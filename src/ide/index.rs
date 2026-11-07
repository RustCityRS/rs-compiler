use super::{Index, Location, Reference, Symbol, SymbolKind, config, highlighting, hooks};
use crate::lexer::Lexer;
use crate::parser::Statement;
use crate::project::{Document, Project, SourceId};
use crate::sources::Scope;
use crate::symbol::SymbolKind as RegistryKind;
use crate::token::{Kind, Token};
use crate::types::Type;
use std::collections::{HashMap, HashSet};
use std::ops::Range;

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

pub(super) fn tokens(
    document: &Document,
    compounds: &HashSet<String>,
    hooks: &[hooks::Hook],
) -> Vec<(Token, Range<usize>)> {
    let mut result = Vec::new();
    for (token, range) in document.tokens.iter().zip(&document.spans) {
        if let Some(hook) = hooks.iter().find(|hook| hook.range == *range) {
            result.extend(hook.tokens.iter().cloned());
        } else if token.kind == Kind::StringLiteral {
            embedded_tokens(document, range.clone(), token.line, &mut result, 0);
        }
        result.push((
            Token::new(
                token.kind,
                document.text[range.clone()].to_owned(),
                token.line,
                1,
            ),
            range.clone(),
        ));
    }
    result.sort_by_key(|(_, range)| range.start);
    let mut qualified = Vec::new();
    let mut i = 0;
    while i < result.len() {
        let (mut token, mut range) = result[i].clone();
        i += 1;
        if matches!(token.kind, Kind::Identifier | Kind::Trigger | Kind::Command) {
            let mut candidate = token.value.clone();
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
                candidate.push('+');
                candidate.push_str(&result[j + 1].0.value);
                end = result[j + 1].1.end;
                j += 2;
                if compounds.contains(&candidate) {
                    token.value = candidate.clone();
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
                token.value.push(':');
                token.value.push_str(&result[i + 1].0.value);
                token.kind = Kind::Identifier;
                range.end = result[i + 1].1.end;
                i += 2;
            }
        }
        qualified.push((token, range));
    }
    qualified
}

fn embedded_tokens(
    document: &Document,
    range: Range<usize>,
    line: usize,
    result: &mut Vec<(Token, Range<usize>)>,
    depth: usize,
) {
    if depth > 32 {
        return;
    }
    for (part, code) in highlighting::interpolations(&document.text[range.clone()]) {
        let start = range.start + part.start + 1;
        let end = range.start + part.end - 1;
        let expression = &document.text[start..end];
        if expression.starts_with("p,") || expression.starts_with("n,") {
            result.push((
                Token::new(Kind::Identifier, expression.to_string(), line, 1),
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
                    embedded_tokens(document, span.clone(), line, result, depth + 1);
                }
                result.push((
                    Token::new(token.kind, document.text[span.clone()].to_string(), line, 1),
                    span,
                ));
            }
        }
    }
}

impl Index {
    pub(super) fn build(project: &Project) -> Self {
        let names = project.registry.hook_commands.keys().cloned().collect();
        let hooks: HashMap<_, _> = project
            .documents
            .values()
            .filter(|document| document.config.is_none())
            .map(|document| (document.id, hooks::collect(document, &names)))
            .collect();
        let mut symbols = Vec::new();
        for document in project.documents.values() {
            let Some(file) = document.syntax() else {
                continue;
            };
            let mut headers = Vec::new();
            let mut blocks = Vec::new();
            let mut stack = Vec::new();
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
                    .take_while(|line| line.trim().starts_with("//"))
                    .map(|line| line.trim().trim_start_matches('/').trim())
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
                    let location = document
                        .tokens
                        .iter()
                        .zip(&document.spans)
                        .enumerate()
                        .find(|(i, (token, range))| {
                            *i > 1
                                && document.tokens[*i - 1].kind == Kind::LocalVar
                                && (if kind == SymbolKind::Parameter {
                                    Type::from_name(&document.text[document.spans[*i - 2].clone()])
                                        .is_some()
                                } else {
                                    document.tokens[*i - 2].kind == Kind::Def
                                })
                                && document.text[(*range).clone()] == *name
                                && (token.line == line || kind == SymbolKind::Parameter)
                                && range.start >= start
                                && range.start < end
                                && !selected.contains(&range.start)
                        })
                        .map(|(_, (_, range))| Location {
                            source: document.id,
                            range: range.clone(),
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
        }
        let mut keys: HashMap<String, usize> = symbols
            .iter()
            .enumerate()
            .map(|(i, symbol)| (symbol.key.clone(), i))
            .collect();
        for (name, symbol) in project
            .registry
            .commands()
            .chain(project.registry.constants())
            .chain(project.registry.game_variables())
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
            if keys.contains_key(&key) {
                continue;
            }
            keys.insert(key.clone(), symbols.len());
            symbols.push(Symbol {
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
            });
        }
        for symbol in &mut symbols {
            if symbol.kind != SymbolKind::Command {
                continue;
            }
            let name = symbol.name.trim_start_matches('.');
            if let Some(&explicit) = project.registry.hook_commands.get(name) {
                symbol.parameters = vec![("hook".into(), Type::ClientScript)];
                if explicit {
                    symbol
                        .parameters
                        .push(("component".into(), Type::Component));
                }
                symbol.detail = format!(
                    "{}(hook $hook{})",
                    symbol.name,
                    if explicit {
                        ", component $component"
                    } else {
                        ""
                    }
                );
            } else if name == "runclientscript" {
                symbol.parameters = vec![("hook".into(), Type::ClientScript)];
                symbol.detail = format!("{}(hook $hook)", symbol.name);
            }
        }
        let mut references = Vec::new();
        config::declarations(project, &mut symbols, &mut keys, &mut references);
        let compounds = symbols
            .iter()
            .map(|symbol| &symbol.name)
            .chain(project.registry.entity_ids.keys())
            .filter(|name| name.contains('+'))
            .cloned()
            .collect::<HashSet<_>>();
        let mut definitions = HashSet::new();
        let mut locals: HashMap<SourceId, HashMap<String, Vec<usize>>> = HashMap::new();
        for (i, symbol) in symbols.iter().enumerate() {
            if let Some((source, _)) = symbol.container {
                locals
                    .entry(source)
                    .or_default()
                    .entry(symbol.name.clone())
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
        for document in project.documents.values() {
            let document_hooks = hooks.get(&document.id).map_or(&[][..], Vec::as_slice);
            let all = tokens(document, &compounds, document_hooks);
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
                            .get(&key(Scope::Client, trigger, &token.value))
                            .copied();
                    }
                    keys.get(&key(Scope::Global, trigger, &token.value))
                        .or_else(|| keys.get(&key(document.scope, trigger, &token.value)))
                        .copied()
                };
                let expected = if callback {
                    Some(Type::ClientScript)
                } else if transmit_kind.is_some() {
                    None
                } else {
                    calls.last().and_then(|(symbol, argument)| {
                        symbol.and_then(|symbol| {
                            symbols[symbol].parameters.get(*argument).map(|(_, ty)| *ty)
                        })
                    })
                };
                let mut resolved = match previous {
                    Some(Kind::LocalVar) => locals
                        .get(&document.id)
                        .and_then(|locals| locals.get(token.value.as_str()))
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
                    Some(Kind::ConstantVar) => {
                        keys.get(&format!("constant:{}", token.value)).copied()
                    }
                    Some(Kind::GameVar) => keys.get(&format!("gamevar:{}", token.value)).copied(),
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
                            | Type::Walktrigger,
                        ) => script(expected.unwrap().name()),
                        _ if transmit_kind.is_some() => {
                            let kind = transmit_kind.unwrap();
                            if kind.starts_with("var") {
                                keys.get(&format!("gamevar:{}", token.value)).copied()
                            } else {
                                Type::from_name(kind).and_then(|ty| {
                                    keys.get(&config::entity_key(ty, &token.value)).copied()
                                })
                            }
                        }
                        _ => keys
                            .get(&format!("command:{}", token.value))
                            .or_else(|| {
                                expected
                                    .and_then(|ty| keys.get(&config::entity_key(ty, &token.value)))
                            })
                            .copied(),
                    },
                };
                if resolved.is_none() && token.value.contains(',') {
                    resolved = keys.get(&config::key("mesanim", &token.value)).copied();
                }
                if resolved.is_none() && token.value.contains(':') {
                    resolved = [Type::Component, Type::DbColumn]
                        .into_iter()
                        .find_map(|ty| keys.get(&config::entity_key(ty, &token.value)).copied());
                }
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
                    && let Some(entry) = project.registry.entity_ids.get(&token.value)
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
                    if let Some(&symbol) = keys.get(&key) {
                        references.push(Reference {
                            location: Location {
                                source: document.id,
                                range: range.clone(),
                            },
                            symbol,
                            declaration: false,
                        });
                        previous_symbol = Some(symbol);
                        continue;
                    }
                    let id = symbols.len();
                    keys.insert(key.clone(), id);
                    symbols.push(Symbol {
                        name: token.value.clone(),
                        kind: SymbolKind::Config,
                        detail: entry
                            .variants
                            .iter()
                            .map(|(ty, id)| format!("{} {} = {id}", ty.name(), token.value))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        documentation: String::new(),
                        value_type: ty,
                        definition: None,
                        extent: None,
                        parameters: Vec::new(),
                        returns: Vec::new(),
                        container: None,
                        scope: None,
                        key,
                    });
                    resolved = Some(id);
                }
                if let Some(symbol) = resolved {
                    references.push(Reference {
                        location: Location {
                            source: document.id,
                            range: range.clone(),
                        },
                        symbol,
                        declaration: false,
                    });
                }
                previous_symbol = resolved;
            }
        }
        config::references(project, &keys, &symbols, &mut references);
        for symbol in &symbols {
            if symbol.kind != SymbolKind::Trigger {
                continue;
            }
            let Some(trigger) = symbol.key.split(':').nth(1) else {
                continue;
            };
            let Some(ty) = project
                .registry
                .trigger(trigger)
                .and_then(|trigger| trigger.subject_type)
            else {
                continue;
            };
            if let Some(&entity) = keys.get(&config::entity_key(ty, &symbol.name))
                && let Some(location) = &symbol.definition
            {
                references.push(Reference {
                    location: location.clone(),
                    symbol: entity,
                    declaration: false,
                });
            }
        }
        references
            .sort_by_key(|reference| (reference.location.source, reference.location.range.start));
        references.dedup_by(|a, b| a.location == b.location && a.symbol == b.symbol);
        Self {
            symbols,
            references,
            compounds,
            hooks,
        }
    }

    pub(super) fn at(&self, source: SourceId, offset: usize) -> Option<&Reference> {
        let i = self.references.partition_point(|reference| {
            (reference.location.source, reference.location.range.start) <= (source, offset)
        });
        i.checked_sub(1)
            .and_then(|i| self.references.get(i))
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
