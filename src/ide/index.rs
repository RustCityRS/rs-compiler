use super::{
    Configs, Index, Location, Reference, Symbol, SymbolKind, annotations, config, fields,
    highlighting, hooks,
};
use crate::lexer::Lexer;
use crate::parallel_map_balanced;
use crate::parser::Statement;
use crate::project::{Document, Project, SourceId};
use crate::sources::Scope;
use crate::symbol::{SymbolKind as RegistryKind, SymbolRegistry};
use crate::token::Kind;
use crate::types::Type;
use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::iter::from_fn;
use std::ops::Range;
use std::sync::Arc;

pub(super) const CONFIG: usize = 1 << (usize::BITS - 1);
const ENTITY: usize = 1 << (usize::BITS - 2);

pub(super) fn key(scope: Scope, trigger: &str, name: &str) -> String {
    format!("{scope:?}:{trigger}:{name}")
}
pub(super) fn local_key(source: SourceId, start: usize, name: &str) -> String {
    format!("{}:{start}:${name}", source.0)
}

pub(super) fn visible(symbol: &Symbol, document: &Document, offset: usize) -> bool {
    if let Some((source, start)) = symbol.container {
        return source == document.id
            && symbol
                .extent
                .as_ref()
                .is_some_and(|extent| start <= offset && offset <= extent.range.end)
            && symbol
                .definition
                .as_ref()
                .is_none_or(|definition| definition.range.start <= offset);
    }
    if symbol.kind == SymbolKind::Command {
        return true;
    }
    symbol
        .scope
        .is_none_or(|scope| scope == Scope::Global || scope == document.scope)
}

#[derive(Clone)]
pub(super) struct Lexeme<'a> {
    pub(super) kind: Kind,
    pub(super) value: Cow<'a, str>,
}

pub(super) fn tokens<'a>(
    document: &'a Document,
    compound: &dyn Fn(&str) -> bool,
    hooks: &'a [hooks::Hook],
) -> Vec<(Lexeme<'a>, Range<usize>)> {
    let text: &'a str = &document.text;
    let mut result = Vec::new();
    let annotations = annotations::collect(document);
    for (token, range) in document.tokens.iter().zip(&document.spans) {
        if annotations
            .iter()
            .any(|hint| hint.range.contains(&range.start))
        {
            continue;
        }
        if let Ok(hook) = hooks.binary_search_by_key(&range.start, |hook| hook.range.start)
            && hooks[hook].range == *range
        {
            result.extend(hooks[hook].tokens.iter().map(|(token, range)| {
                (
                    Lexeme {
                        kind: token.kind,
                        value: Cow::Borrowed(token.value.as_str()),
                    },
                    range.clone(),
                )
            }));
        } else if token.kind == Kind::StringLiteral {
            embedded_tokens(document, range.clone(), &mut result, 0);
        }
        result.push((
            Lexeme {
                kind: token.kind,
                value: Cow::Borrowed(&text[range.clone()]),
            },
            range.clone(),
        ));
    }
    result.sort_by_key(|(_, range)| range.start);
    let mut qualified = Vec::with_capacity(result.len());
    let mut i = 0;
    while i < result.len() {
        let (mut token, mut range) = result[i].clone();
        i += 1;
        if matches!(token.kind, Kind::Identifier | Kind::Trigger | Kind::Command) {
            let mut candidate: Option<String> = None;
            let mut end = range.end;
            let mut j = i;
            while j + 1 < result.len()
                && result[j].0.kind == Kind::BinaryOperator
                && result[j].0.value == "+"
                && matches!(
                    result[j + 1].0.kind,
                    Kind::Identifier | Kind::Trigger | Kind::Command | Kind::Number
                )
                && end == result[j].1.start
                && result[j].1.end == result[j + 1].1.start
            {
                let candidate = candidate.get_or_insert_with(|| token.value.to_string());
                candidate.push('+');
                candidate.push_str(&result[j + 1].0.value);
                end = result[j + 1].1.end;
                j += 2;
                if compound(candidate) {
                    token.value = Cow::Owned(candidate.clone());
                    token.kind = Kind::Identifier;
                    range.end = end;
                    i = j;
                }
            }
            while i + 1 < result.len()
                && result[i].0.kind == Kind::Colon
                && matches!(
                    result[i + 1].0.kind,
                    Kind::Identifier | Kind::Trigger | Kind::Command | Kind::Number
                )
                && range.end == result[i].1.start
                && result[i].1.end == result[i + 1].1.start
                && result.get(i + 2).is_none_or(|(token, _)| {
                    matches!(
                        token.kind,
                        Kind::RParen
                            | Kind::Comma
                            | Kind::Semicolon
                            | Kind::RBrace
                            | Kind::RBracket
                            | Kind::Colon
                            | Kind::ComparisonOperator
                            | Kind::LogicalAnd
                            | Kind::LogicalOr
                            | Kind::BinaryOperator
                            | Kind::EndOfFile
                    )
                })
            {
                let value = token.value.to_mut();
                value.push(':');
                value.push_str(&result[i + 1].0.value);
                token.kind = Kind::Identifier;
                range.end = result[i + 1].1.end;
                i += 2;
            }
        }
        qualified.push((token, range));
    }
    qualified
}

fn embedded_tokens<'a>(
    document: &'a Document,
    range: Range<usize>,
    result: &mut Vec<(Lexeme<'a>, Range<usize>)>,
    depth: usize,
) {
    if depth > 32 {
        return;
    }
    let text: &'a str = &document.text;
    for (part, code) in highlighting::interpolations(&text[range.clone()]) {
        let start = range.start + part.start + 1;
        let end = range.start + part.end - 1;
        let expression = &text[start..end];
        if expression.starts_with("p,") || expression.starts_with("n,") {
            result.push((
                Lexeme {
                    kind: Kind::Identifier,
                    value: Cow::Borrowed(expression),
                },
                start..end,
            ));
        } else if code {
            let (inner, spans, _) = Lexer::new(expression, &document.label).tokenize_spanned();
            for (token, span) in inner
                .into_iter()
                .zip(spans)
                .filter(|(token, _)| token.kind != Kind::EndOfFile)
            {
                let span = start + span.start..start + span.end;
                if token.kind == Kind::StringLiteral {
                    embedded_tokens(document, span.clone(), result, depth + 1);
                }
                result.push((
                    Lexeme {
                        kind: token.kind,
                        value: Cow::Borrowed(&text[span.clone()]),
                    },
                    span,
                ));
            }
        }
    }
}

/// A config reference to a script, settled once the scripts are known. When no
/// script matches, the references inside it stand instead.
pub(crate) struct Link {
    pub(super) location: Location,
    pub(super) scripts: Vec<String>,
    pub(super) symbol: Option<usize>,
    pub(super) declaration: bool,
    pub(super) fallback: Vec<Link>,
}

struct Found {
    range: Range<usize>,
    probes: Vec<String>,
    symbol: Option<usize>,
    entity: Option<(String, String, Option<Type>)>,
}

fn script_symbols(document: &Document) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    let Some(file) = document.syntax() else {
        return symbols;
    };
    let mut headers = Vec::new();
    let mut blocks = Vec::new();
    let mut stack = Vec::new();
    let mut declared: HashMap<&str, Vec<usize>> = HashMap::new();
    for i in 2..document.tokens.len() {
        if document.tokens[i - 1].kind == Kind::LocalVar {
            declared
                .entry(&document.text[document.spans[i].clone()])
                .or_default()
                .push(i);
        }
    }
    for (i, token) in document.tokens.iter().enumerate() {
        if token.kind == Kind::LBracket
            && document
                .tokens
                .get(i + 2)
                .is_some_and(|token| token.kind == Kind::Comma)
            && let Some(end) = document.tokens[i + 3..]
                .iter()
                .position(|token| token.kind == Kind::RBracket)
                .map(|end| i + 3 + end)
        {
            let name = document.spans[i + 3].start..document.spans[end].start;
            headers.push((document.spans[i].start, name));
        }
        match token.kind {
            Kind::LBrace => stack.push(document.spans[i].start),
            Kind::RBrace => {
                if let Some(start) = stack.pop() {
                    blocks.push(start..document.spans[i].end);
                }
            }
            _ => {}
        }
    }
    for (number, script) in file.scripts.iter().enumerate() {
        let line_start = document
            .lines
            .line(script.line.saturating_sub(1))
            .map_or(0, |line| line.start);
        let header = headers.iter().enumerate().find(|(_, (start, name))| {
            *start >= line_start && document.text[name.clone()].trim() == script.name
        });
        let (start, selection, end) = if let Some((index, (start, name))) = header {
            (
                *start,
                name.start..name.start + document.text[name.clone()].trim_end().len(),
                headers
                    .get(index + 1)
                    .map_or(document.text.len(), |(start, _)| *start),
            )
        } else {
            (
                line_start,
                line_start..line_start,
                file.scripts
                    .get(number + 1)
                    .and_then(|script| document.lines.line(script.line.saturating_sub(1)))
                    .map_or(document.text.len(), |line| line.start),
            )
        };
        let symbol_key = if script.trigger == "command" {
            format!("command:{}", script.name)
        } else {
            key(document.scope, &script.trigger, &script.name)
        };
        let params = script
            .params
            .iter()
            .map(|param| (param.name.clone(), param.param_type))
            .collect::<Vec<_>>();
        let parameters = params
            .iter()
            .map(|(name, ty)| format!("{} ${name}", ty.name()))
            .collect::<Vec<_>>()
            .join(", ");
        let returns = script
            .return_types
            .iter()
            .map(Type::name)
            .collect::<Vec<_>>()
            .join(", ");
        let documentation = document.text[..line_start]
            .lines()
            .rev()
            .enumerate()
            .take_while(|(offset, line)| {
                let number = script.line.saturating_sub(offset + 1);
                line.trim().starts_with("//")
                    || script
                        .annotations
                        .iter()
                        .any(|annotation| (annotation.line..=annotation.end_line).contains(&number))
            })
            .filter_map(|(_, line)| {
                line.trim()
                    .strip_prefix("//")
                    .map(|line| line.trim_start_matches('/').trim())
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        symbols.push(Symbol {
            name: script.name.clone(),
            kind: match script.trigger.as_str() {
                "proc" => SymbolKind::Procedure,
                "clientscript" => SymbolKind::ClientScript,
                "command" => SymbolKind::Command,
                _ => SymbolKind::Trigger,
            },
            detail: format!(
                "[{},{}]({parameters})({returns})",
                script.trigger, script.name
            ),
            documentation,
            value_type: script.return_types.first().copied(),
            definition: Some(Location {
                source: document.id,
                range: selection,
            }),
            extent: Some(Location {
                source: document.id,
                range: start..end,
            }),
            parameters: params,
            returns: script.return_types.clone(),
            container: None,
            scope: Some(document.scope),
            key: symbol_key,
        });
        let mut locals = script
            .params
            .iter()
            .map(|param| {
                (
                    param.name.as_str(),
                    param.param_type,
                    script.line,
                    SymbolKind::Parameter,
                )
            })
            .collect::<Vec<_>>();
        collect_locals(&script.body, &mut locals);
        let mut selected = HashSet::new();
        for (name, ty, line, kind) in locals {
            let location = declared
                .get(name)
                .into_iter()
                .flatten()
                .copied()
                .find(|&i| {
                    let range = &document.spans[i];
                    (if kind == SymbolKind::Parameter {
                        Type::from_name(&document.text[document.spans[i - 2].clone()]).is_some()
                    } else {
                        document.tokens[i - 2].kind == Kind::Def
                    }) && (document.tokens[i].line == line || kind == SymbolKind::Parameter)
                        && range.start >= start
                        && range.start < end
                        && !selected.contains(&range.start)
                })
                .map(|i| Location {
                    source: document.id,
                    range: document.spans[i].clone(),
                });
            if let Some(location) = &location {
                selected.insert(location.range.start);
            }
            let extent = if kind == SymbolKind::Parameter {
                start..end
            } else {
                location
                    .as_ref()
                    .and_then(|location| {
                        blocks
                            .iter()
                            .filter(|block| block.contains(&location.range.start))
                            .min_by_key(|block| block.len())
                    })
                    .cloned()
                    .unwrap_or(start..end)
            };
            symbols.push(Symbol {
                name: name.into(),
                kind,
                detail: format!("{} ${name}", ty.name()),
                documentation: String::new(),
                value_type: Some(ty),
                definition: location,
                extent: Some(Location {
                    source: document.id,
                    range: extent.clone(),
                }),
                parameters: Vec::new(),
                returns: Vec::new(),
                container: Some((document.id, extent.start)),
                scope: Some(document.scope),
                key: local_key(document.id, extent.start, name),
            });
        }
    }
    symbols
}

fn hook_parameters(symbol: &mut Symbol, registry: &SymbolRegistry) {
    if symbol.kind != SymbolKind::Command {
        return;
    }
    let name = symbol.name.trim_start_matches('.');
    if let Some(&explicit) = registry.hook_commands.get(name) {
        let component_first = explicit && registry.component_first_command(name);
        symbol.parameters = vec![("hook".into(), Type::ClientScript)];
        if explicit {
            symbol.parameters.insert(
                usize::from(!component_first),
                ("component".into(), Type::Component),
            );
        }
        symbol.detail = format!(
            "{}({})",
            symbol.name,
            if component_first {
                "component $component, hook $hook"
            } else if explicit {
                "hook $hook, component $component"
            } else {
                "hook $hook"
            }
        );
    } else if name == "runclientscript" {
        symbol.parameters = vec![("hook".into(), Type::ClientScript)];
        symbol.detail = format!("{}(hook $hook)", symbol.name);
    }
}

fn link(link: &Link, keys: &HashMap<&str, usize>, references: &mut Vec<Reference>) {
    match link
        .scripts
        .iter()
        .find_map(|key| keys.get(key.as_str()).copied())
        .or(link.symbol)
    {
        Some(symbol) => references.push(Reference {
            location: link.location.clone(),
            symbol,
            declaration: link.declaration,
        }),
        None => {
            for inner in &link.fallback {
                self::link(inner, keys, references);
            }
        }
    }
}

fn sort(references: &mut Vec<Reference>) {
    references.sort_by_key(|reference| (reference.location.source, reference.location.range.start));
    references.dedup_by(|a, b| a.location == b.location && a.symbol == b.symbol);
}

fn within(references: &[Reference], source: SourceId) -> &[Reference] {
    let start = references.partition_point(|reference| reference.location.source < source);
    let end = references.partition_point(|reference| reference.location.source <= source);
    &references[start..end]
}

fn merge<'a>(a: &'a [Reference], b: &'a [Reference]) -> impl Iterator<Item = &'a Reference> {
    let (mut a, mut b) = (a.iter().peekable(), b.iter().peekable());
    from_fn(move || match (a.peek(), b.peek()) {
        (Some(x), Some(y))
            if (y.location.source, y.location.range.start)
                < (x.location.source, x.location.range.start) =>
        {
            b.next()
        }
        (Some(_), _) => a.next(),
        _ => b.next(),
    })
}

impl Project {
    pub(super) fn configs(&self) -> Arc<Configs> {
        let mut commands = self
            .documents
            .values()
            .filter_map(Document::syntax)
            .flat_map(|file| &file.scripts)
            .filter(|script| script.trigger == "command")
            .map(|script| format!("command:{}", script.name))
            .collect::<Vec<_>>();
        commands.sort_unstable();
        commands.dedup();
        let registry = &self.registry;
        let mut hasher = DefaultHasher::new();
        (
            self.config_generation,
            self.registry_generation,
            registry.commands.len(),
            registry.constants.len(),
            registry.game_vars.len(),
            registry.hook_commands.len(),
            registry.entity_ids.len(),
            &commands,
        )
            .hash(&mut hasher);
        let key = hasher.finish();
        let mut cached = self.configs.lock().unwrap();
        if let Some(configs) = cached.as_ref().filter(|configs| configs.key == key) {
            return configs.clone();
        }
        let configs = Arc::new(Configs::build(self, &commands.into_iter().collect(), key));
        *cached = Some(configs.clone());
        configs
    }
}

impl Configs {
    fn build(project: &Project, commands: &HashSet<String>, key: u64) -> Self {
        let registry = &project.registry;
        let mut symbols = Vec::new();
        let mut keys = HashMap::new();
        for (name, symbol) in registry
            .commands()
            .chain(registry.constants())
            .chain(registry.game_variables())
        {
            let (kind, prefix, ty, params, returns, detail) = match &symbol.kind {
                RegistryKind::Command {
                    param_types,
                    return_types,
                    ..
                } => (
                    SymbolKind::Command,
                    "command",
                    return_types.first().copied(),
                    param_types.clone(),
                    return_types.clone(),
                    format!(
                        "{name}({})({})",
                        param_types
                            .iter()
                            .map(Type::name)
                            .collect::<Vec<_>>()
                            .join(", "),
                        return_types
                            .iter()
                            .map(Type::name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ),
                RegistryKind::Constant {
                    const_type,
                    int_value,
                    string_value,
                } => (
                    SymbolKind::Constant,
                    "constant",
                    Some(*const_type),
                    Vec::new(),
                    Vec::new(),
                    format!(
                        "{} ^{name} = {}",
                        const_type.name(),
                        string_value
                            .as_ref()
                            .map(|s| format!("{s:?}"))
                            .unwrap_or_else(|| int_value.unwrap_or_default().to_string())
                    ),
                ),
                RegistryKind::GameVar {
                    var_type,
                    category,
                    var_id,
                } => (
                    SymbolKind::GameVariable,
                    "gamevar",
                    Some(*var_type),
                    Vec::new(),
                    Vec::new(),
                    format!("{} %{name} ({category}, ID {var_id})", var_type.name()),
                ),
                _ => continue,
            };
            let key = format!("{prefix}:{name}");
            if commands.contains(&key) || keys.contains_key(&key) {
                continue;
            }
            keys.insert(key.clone(), symbols.len());
            let mut symbol = Symbol {
                name: name.into(),
                kind,
                detail,
                documentation: String::new(),
                value_type: ty,
                definition: None,
                extent: None,
                parameters: params
                    .into_iter()
                    .enumerate()
                    .map(|(i, ty)| (format!("arg{i}"), ty))
                    .collect(),
                returns,
                container: None,
                scope: None,
                key,
            };
            hook_parameters(&mut symbol, registry);
            symbols.push(symbol);
        }
        let mut references = Vec::new();
        let mut links = Vec::new();
        let mut config_types = fields::ConfigTypes::default();
        config::declarations(
            project,
            commands,
            &mut symbols,
            &mut keys,
            &mut references,
            &mut links,
            &mut config_types,
        );
        let compounds = symbols
            .iter()
            .map(|symbol| &symbol.name)
            .chain(registry.entity_ids.keys())
            .filter(|name| name.contains('+'))
            .cloned()
            .collect();
        for (i, symbol) in symbols.iter().enumerate() {
            if let Some(location) = &symbol.definition {
                references.push(Reference {
                    location: location.clone(),
                    symbol: CONFIG | i,
                    declaration: true,
                });
            }
        }
        let mut config_problems = HashMap::new();
        config::references(
            project,
            commands,
            &keys,
            &symbols,
            &mut references,
            &mut links,
            &config_types,
            &mut config_problems,
        );
        sort(&mut references);
        Self {
            key,
            symbols,
            keys,
            references,
            links,
            compounds,
            config_types,
            config_problems,
        }
    }

    pub(super) fn problems(&self) -> &HashMap<SourceId, Vec<(usize, String)>> {
        &self.config_problems
    }

    pub(super) fn types(&self) -> &fields::ConfigTypes {
        &self.config_types
    }
}

impl Index {
    pub(super) fn build(project: &Project) -> Self {
        let registry = &project.registry;
        let documents = project.documents.values().collect::<Vec<_>>();
        let parts = parallel_map_balanced(&documents, |document| {
            (
                document
                    .config
                    .is_none()
                    .then(|| hooks::collect(document, registry)),
                script_symbols(document),
            )
        });
        let mut hooks = HashMap::new();
        let mut symbols = Vec::new();
        for (document, (found, mut declared)) in documents.iter().zip(parts) {
            if let Some(found) = found {
                hooks.insert(document.id, found);
            }
            symbols.append(&mut declared);
        }
        for symbol in &mut symbols {
            hook_parameters(symbol, registry);
        }
        let configs = project.configs();
        let compounds = symbols
            .iter()
            .filter(|symbol| symbol.name.contains('+') && !configs.compounds.contains(&symbol.name))
            .map(|symbol| symbol.name.clone())
            .collect::<HashSet<_>>();
        let keys = symbols
            .iter()
            .enumerate()
            .filter(|(_, symbol)| symbol.container.is_none())
            .map(|(i, symbol)| (symbol.key.as_str(), i))
            .collect::<HashMap<_, _>>();
        let mut references = Vec::new();
        let mut definitions = HashSet::new();
        let mut locals: HashMap<SourceId, HashMap<&str, Vec<usize>>> = HashMap::new();
        for (i, symbol) in symbols.iter().enumerate() {
            if let Some((source, _)) = symbol.container {
                locals
                    .entry(source)
                    .or_default()
                    .entry(symbol.name.as_str())
                    .or_default()
                    .push(i);
            }
            if let Some(location) = &symbol.definition {
                definitions.insert((location.source, location.range.start, location.range.end));
                references.push(Reference {
                    location: location.clone(),
                    symbol: i,
                    declaration: true,
                });
            }
        }
        let compound = |name: &str| configs.compounds.contains(name) || compounds.contains(name);
        let lookup = |key: &str| {
            keys.get(key)
                .copied()
                .or_else(|| configs.keys.get(key).map(|&i| CONFIG | i))
        };
        let parameters = |id: usize| -> &[(String, Type)] {
            if id & CONFIG != 0 {
                &configs.symbols[id ^ CONFIG].parameters
            } else if id & ENTITY != 0 {
                &[]
            } else {
                &symbols[id].parameters
            }
        };
        let found = parallel_map_balanced(&documents, |document| {
            let mut out = (Vec::new(), Vec::new());
            if document.config.is_some() {
                return out;
            }
            let document_hooks = hooks.get(&document.id).map_or(&[][..], Vec::as_slice);
            let all = tokens(document, &compound, document_hooks);
            let significant = all
                .iter()
                .filter(|(token, _)| {
                    !matches!(
                        token.kind,
                        Kind::SingleLineComment
                            | Kind::MultiLineComment
                            | Kind::StringLiteral
                            | Kind::RawStringLiteral
                    )
                })
                .collect::<Vec<_>>();
            let mut calls: Vec<(Option<usize>, usize)> = Vec::new();
            let mut previous_symbol = None;
            for (i, (token, range)) in significant.iter().enumerate() {
                match token.kind {
                    Kind::LParen => {
                        calls.push((previous_symbol, 0));
                        previous_symbol = None;
                    }
                    Kind::LBrace | Kind::LBracket => {
                        calls.push((None, 0));
                        previous_symbol = None;
                    }
                    Kind::RParen | Kind::RBrace | Kind::RBracket => {
                        calls.pop();
                        previous_symbol = None;
                    }
                    Kind::Comma => {
                        if let Some((_, argument)) = calls.last_mut() {
                            *argument += 1;
                        }
                        previous_symbol = None;
                    }
                    _ if !matches!(
                        token.kind,
                        Kind::Identifier | Kind::Trigger | Kind::Command
                    ) =>
                    {
                        previous_symbol = None
                    }
                    _ => {}
                }
                if !matches!(token.kind, Kind::Identifier | Kind::Trigger | Kind::Command) {
                    continue;
                }
                if definitions.contains(&(document.id, range.start, range.end)) {
                    continue;
                }
                let previous = i.checked_sub(1).map(|j| significant[j].0.kind);
                let callback = document_hooks.iter().any(|hook| hook.callback == *range);
                let transmit_kind = document_hooks
                    .iter()
                    .find_map(|hook| hook.transmit_at(range.start));
                let script = |trigger: &str| {
                    if callback {
                        return keys
                            .get(key(Scope::Client, trigger, &token.value).as_str())
                            .copied();
                    }
                    keys.get(key(Scope::Global, trigger, &token.value).as_str())
                        .or_else(|| keys.get(key(document.scope, trigger, &token.value).as_str()))
                        .copied()
                };
                let expected = if callback {
                    Some(Type::ClientScript)
                } else if transmit_kind.is_some() {
                    None
                } else {
                    calls.last().and_then(|(symbol, argument)| {
                        symbol
                            .and_then(|symbol| parameters(symbol).get(*argument).map(|(_, ty)| *ty))
                    })
                };
                // A config a script names first comes later, so a miss may still find one.
                let mut probes = Vec::new();
                let mut find = |key: String| {
                    let found = lookup(&key);
                    if found.is_none() && key.starts_with("config:") {
                        probes.push(key);
                    }
                    found
                };
                let mut resolved = match previous {
                    Some(Kind::LocalVar) => locals
                        .get(&document.id)
                        .and_then(|locals| locals.get(token.value.as_ref()))
                        .and_then(|ids| {
                            ids.iter()
                                .copied()
                                .filter(|&id| visible(&symbols[id], document, range.start))
                                .max_by_key(|&id| {
                                    (
                                        symbols[id].container,
                                        symbols[id]
                                            .definition
                                            .as_ref()
                                            .map(|location| location.range.start),
                                    )
                                })
                        }),
                    Some(Kind::ScriptCall) => script("proc"),
                    Some(Kind::JumpCall) => script("label"),
                    Some(Kind::ConstantVar) => find(format!("constant:{}", token.value)),
                    Some(Kind::GameVar) => find(format!("gamevar:{}", token.value)),
                    _ => match expected {
                        Some(Type::ClientScript) => {
                            script("clientscript").or_else(|| script("proc"))
                        }
                        Some(
                            Type::Proc
                            | Type::Label
                            | Type::Queue
                            | Type::Timer
                            | Type::SoftTimer
                            | Type::Walktrigger
                            | Type::ControllerQueue
                            | Type::NpcQueue,
                        ) => script(expected.unwrap().name()),
                        _ if transmit_kind.is_some() => {
                            let kind = transmit_kind.unwrap();
                            if kind.starts_with("var") {
                                find(format!("gamevar:{}", token.value))
                            } else {
                                Type::from_name(kind)
                                    .and_then(|ty| find(config::entity_key(ty, &token.value)))
                            }
                        }
                        _ => find(format!("command:{}", token.value)).or_else(|| {
                            expected.and_then(|ty| find(config::entity_key(ty, &token.value)))
                        }),
                    },
                };
                if resolved.is_none() && token.value.contains(',') {
                    resolved = find(config::key("mesanim", &token.value));
                }
                if resolved.is_none() && token.value.contains(':') {
                    resolved = [Type::Component, Type::DbColumn]
                        .into_iter()
                        .find_map(|ty| find(config::entity_key(ty, &token.value)));
                }
                let mut entity = None;
                if resolved.is_none()
                    && !matches!(
                        previous,
                        Some(
                            Kind::LocalVar
                                | Kind::ScriptCall
                                | Kind::JumpCall
                                | Kind::ConstantVar
                                | Kind::GameVar
                        )
                    )
                    && let Some(entry) = registry.entity_ids.get(token.value.as_ref())
                {
                    let ty = expected
                        .filter(|expected| {
                            entry.variants.iter().any(|(ty, _)| {
                                ty == expected || (*ty == Type::Obj && *expected == Type::NamedObj)
                            })
                        })
                        .or_else(|| entry.variants.first().map(|(ty, _)| *ty));
                    let key = ty.map_or_else(
                        || format!("config:{}", token.value),
                        |ty| config::entity_key(ty, &token.value),
                    );
                    resolved = lookup(&key);
                    if resolved.is_none() {
                        entity = Some((token.value.to_string(), key, ty));
                    }
                }
                previous_symbol = resolved.or(entity.as_ref().map(|_| ENTITY));
                if probes.is_empty() && entity.is_none() {
                    if let Some(symbol) = resolved {
                        out.0.push(Reference {
                            location: Location {
                                source: document.id,
                                range: range.clone(),
                            },
                            symbol,
                            declaration: false,
                        });
                    }
                } else {
                    out.1.push(Found {
                        range: range.clone(),
                        probes,
                        symbol: resolved,
                        entity,
                    });
                }
            }
            out
        });
        let mut entities = Vec::new();
        let mut entity_keys: HashMap<String, usize> = HashMap::new();
        for (document, (resolved, found)) in documents.iter().zip(found) {
            references.extend(resolved);
            for Found {
                range,
                probes,
                symbol,
                entity,
            } in found
            {
                let symbol = probes
                    .iter()
                    .find_map(|key| entity_keys.get(key).map(|&i| ENTITY | i))
                    .or(symbol)
                    .or_else(|| {
                        let (name, key, value_type) = entity?;
                        Some(
                            ENTITY
                                | *entity_keys.entry(key.clone()).or_insert_with(|| {
                                    entities.push(Symbol {
                                        detail: registry.entity_ids[&name]
                                            .variants
                                            .iter()
                                            .map(|(ty, id)| format!("{} {name} = {id}", ty.name()))
                                            .collect::<Vec<_>>()
                                            .join("\n"),
                                        name,
                                        kind: SymbolKind::Config,
                                        documentation: String::new(),
                                        value_type,
                                        definition: None,
                                        extent: None,
                                        parameters: Vec::new(),
                                        returns: Vec::new(),
                                        container: None,
                                        scope: None,
                                        key,
                                    });
                                    entities.len() - 1
                                }),
                        )
                    });
                if let Some(symbol) = symbol {
                    references.push(Reference {
                        location: Location {
                            source: document.id,
                            range,
                        },
                        symbol,
                        declaration: false,
                    });
                }
            }
        }
        for item in &configs.links {
            link(item, &keys, &mut references);
        }
        for symbol in &symbols {
            if symbol.kind != SymbolKind::Trigger {
                continue;
            }
            let Some(trigger) = symbol.key.split(':').nth(1) else {
                continue;
            };
            let Some(ty) = registry
                .trigger(trigger)
                .and_then(|trigger| trigger.subject_type)
            else {
                continue;
            };
            let key = config::entity_key(ty, &symbol.name);
            if let Some(entity) = configs
                .keys
                .get(&key)
                .map(|&i| CONFIG | i)
                .or_else(|| entity_keys.get(&key).map(|&i| ENTITY | i))
                && let Some(location) = &symbol.definition
            {
                references.push(Reference {
                    location: location.clone(),
                    symbol: entity,
                    declaration: false,
                });
            }
        }
        sort(&mut references);
        Self {
            configs,
            symbols,
            entities,
            references,
            compounds,
            hooks,
        }
    }

    pub(super) fn symbol(&self, id: usize) -> &Symbol {
        if id & CONFIG != 0 {
            &self.configs.symbols[id ^ CONFIG]
        } else if id & ENTITY != 0 {
            &self.entities[id ^ ENTITY]
        } else {
            &self.symbols[id]
        }
    }

    /// Scripts and their locals, then the registry and configs, then the
    /// configs a script names first.
    pub(super) fn symbols(&self) -> impl Iterator<Item = &Symbol> {
        self.symbols
            .iter()
            .chain(&self.configs.symbols)
            .chain(&self.entities)
    }

    pub(super) fn references(&self) -> impl Iterator<Item = &Reference> {
        merge(&self.references, &self.configs.references)
    }

    pub(super) fn references_in(&self, source: SourceId) -> impl Iterator<Item = &Reference> {
        merge(
            within(&self.references, source),
            within(&self.configs.references, source),
        )
    }

    pub(super) fn is_compound(&self, name: &str) -> bool {
        self.configs.compounds.contains(name) || self.compounds.contains(name)
    }

    pub(super) fn config_types(&self) -> &fields::ConfigTypes {
        &self.configs.config_types
    }

    pub(super) fn at(&self, source: SourceId, offset: usize) -> Option<&Reference> {
        fn last(references: &[Reference], source: SourceId, offset: usize) -> Option<&Reference> {
            let i = references.partition_point(|reference| {
                (reference.location.source, reference.location.range.start) <= (source, offset)
            });
            i.checked_sub(1).map(|i| &references[i])
        }
        let start =
            |reference: &Reference| (reference.location.source, reference.location.range.start);
        match (
            last(&self.references, source, offset),
            last(&self.configs.references, source, offset),
        ) {
            (Some(script), Some(config)) => Some(if start(config) >= start(script) {
                config
            } else {
                script
            }),
            (script, config) => script.or(config),
        }
        .filter(|reference| {
            reference.location.source == source && offset <= reference.location.range.end
        })
    }
}

fn collect_locals<'a>(
    statements: &'a [Statement],
    out: &mut Vec<(&'a str, Type, usize, SymbolKind)>,
) {
    for statement in statements {
        match statement {
            Statement::VarDeclaration {
                var_type,
                name,
                line,
                ..
            } => out.push((name, *var_type, *line, SymbolKind::Variable)),
            Statement::ArrayDeclaration {
                element_type,
                name,
                line,
                ..
            } => out.push((name, *element_type, *line, SymbolKind::Variable)),
            Statement::If {
                body,
                else_if,
                else_body,
                ..
            } => {
                collect_locals(body, out);
                for (_, body, _) in else_if {
                    collect_locals(body, out);
                }
                if let Some(body) = else_body {
                    collect_locals(body, out);
                }
            }
            Statement::While { body, .. } => collect_locals(body, out),
            Statement::Switch { cases, default, .. } => {
                for case in cases {
                    collect_locals(&case.body, out);
                }
                if let Some(body) = default {
                    collect_locals(body, out);
                }
            }
            _ => {}
        }
    }
}
