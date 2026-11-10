use super::{
    CodeAction, Completion, Hover, Index, Location, SemanticToken, SignatureHelp, Symbol,
    SymbolKind, TextEdit, annotations, config, highlighting, index,
};
use crate::clientscript::hooks::transmit_id;
use crate::diagnostics::{Diagnostic, DiagnosticsCollector, Phase};
use crate::project::{Project, SourceId};
use crate::sources::Scope;
use crate::symbol::SymbolKind as RegistryKind;
use crate::text::Position;
use crate::token::Kind;
use crate::trigger_table;
use crate::types::Type;
use std::collections::HashSet;
use std::ops::Range;

impl Project {
    pub(super) fn symbol_index(&self) -> &Index {
        self.index.get_or_init(|| Index::build(self))
    }

    pub fn hover(&self, source: SourceId, offset: usize) -> Option<Hover> {
        if let Some(hover) = annotations::hover(self.document(source)?, offset) {
            return Some(hover);
        }
        let index = self.symbol_index();
        let Some(reference) = index.at(source, offset) else {
            return config::key_hover(self.document(source)?, offset);
        };
        let symbol = index.symbol(reference.symbol);
        Some(Hover {
            range: reference.location.range.clone(),
            kind: symbol.kind,
            detail: symbol.detail.clone(),
            documentation: symbol.documentation.clone(),
        })
    }

    pub fn definition(&self, source: SourceId, offset: usize) -> Option<Location> {
        let index = self.symbol_index();
        index
            .symbol(index.at(source, offset)?.symbol)
            .definition
            .clone()
    }

    pub fn references(
        &self,
        source: SourceId,
        offset: usize,
        include_declaration: bool,
    ) -> Vec<Location> {
        let index = self.symbol_index();
        let Some(selected) = index.at(source, offset) else {
            return Vec::new();
        };
        index
            .references()
            .filter(|reference| {
                reference.symbol == selected.symbol
                    && (include_declaration || !reference.declaration)
            })
            .map(|reference| reference.location.clone())
            .collect()
    }

    pub fn document_symbols(&self, source: SourceId) -> Vec<&Symbol> {
        self.symbol_index()
            .symbols()
            .filter(|symbol| {
                symbol
                    .definition
                    .as_ref()
                    .is_some_and(|location| location.source == source)
            })
            .collect()
    }

    pub fn workspace_symbols(&self, query: &str, limit: usize) -> Vec<&Symbol> {
        let query = query.to_lowercase();
        self.symbol_index()
            .symbols()
            .filter(|symbol| {
                symbol.container.is_none() && symbol.name.to_lowercase().contains(&query)
            })
            .take(limit)
            .collect()
    }

    /// Rename only source-owned definitions. External config and engine symbols
    /// require their host's editing support, so an incomplete edit is rejected.
    pub fn rename(
        &self,
        source: SourceId,
        offset: usize,
        name: &str,
    ) -> Result<Vec<TextEdit>, String> {
        if name.is_empty()
            || !name.bytes().enumerate().all(|(i, b)| {
                b == b'_'
                    || b.is_ascii_alphabetic()
                    || (i > 0 && (b.is_ascii_digit() || b":+.".contains(&b)))
            })
        {
            return Err("Expected an identifier containing letters, digits or underscores".into());
        }
        let index = self.symbol_index();
        let selected = index
            .at(source, offset)
            .ok_or("No resolved symbol at this position")?;
        let symbol = index.symbol(selected.symbol);
        let name = if matches!(symbol.value_type, Some(Type::Component | Type::DbColumn))
            && let Some((parent, _)) = symbol.name.rsplit_once(':')
        {
            if let Some((new_parent, _)) = name.rsplit_once(':') {
                if parent != new_parent {
                    return Err("Rename the containing interface or table separately".into());
                }
                name.to_string()
            } else {
                format!("{parent}:{name}")
            }
        } else {
            name.to_string()
        };
        if index
            .symbols()
            .filter(|other| other.key == symbol.key)
            .count()
            > 1
        {
            return Err("Resolve duplicate declarations before renaming this symbol".into());
        }
        if symbol.definition.is_none() {
            return Err("This definition is supplied by the host; rename it through the host's config editor".into());
        }
        if index.symbols().any(|other| {
            other.name == name
                && other.key != symbol.key
                && (symbol.kind != SymbolKind::Config || other.value_type == symbol.value_type)
                && other.container == symbol.container
                && (other.scope == symbol.scope
                    || symbol.scope == Some(Scope::Global)
                    || other.scope == Some(Scope::Global))
        }) {
            return Err(format!(
                "Renaming to '{name}' would conflict with an existing definition"
            ));
        }
        if symbol.container.is_some()
            && index.symbols().any(|other| {
                other.name == name
                    && other.key != symbol.key
                    && other.container.is_some()
                    && other
                        .extent
                        .as_ref()
                        .zip(symbol.extent.as_ref())
                        .is_some_and(|(a, b)| {
                            a.source == b.source
                                && a.range.start < b.range.end
                                && b.range.start < a.range.end
                        })
            })
        {
            return Err(format!(
                "Renaming to '{name}' would capture or shadow a variable in an overlapping scope"
            ));
        }
        if Type::from_name(&name).is_some()
            || matches!(
                name.as_str(),
                "if" | "else"
                    | "return"
                    | "while"
                    | "case"
                    | "default"
                    | "calc"
                    | "true"
                    | "false"
                    | "null"
            )
        {
            return Err(format!("'{name}' is reserved by the language"));
        }
        if symbol.container.is_some() && name.bytes().any(|b| b":+.".contains(&b)) {
            return Err("Local names must contain letters, digits or underscores".into());
        }
        let mut edits = self
            .references(source, offset, true)
            .into_iter()
            .map(|location| {
                let old = &self.document(location.source).unwrap().text[location.range.clone()];
                let replacement =
                    if matches!(symbol.value_type, Some(Type::Component | Type::DbColumn)) {
                        if let Some(slot) = old.strip_prefix(&format!("{}:", symbol.name)) {
                            format!("{name}:{slot}")
                        } else if !old.contains(':') {
                            name.rsplit(':').next().unwrap().to_string()
                        } else {
                            name.clone()
                        }
                    } else {
                        name.clone()
                    };
                TextEdit {
                    location,
                    replacement,
                }
            })
            .collect::<Vec<_>>();
        let child_type = match symbol.value_type {
            Some(Type::Interface) => Some(Type::Component),
            Some(Type::DbTable) => Some(Type::DbColumn),
            _ => None,
        };
        if let Some(child_type) = child_type {
            let prefix = format!("{}:", symbol.name);
            for reference in index.references() {
                let child = index.symbol(reference.symbol);
                if child.value_type != Some(child_type) || !child.name.starts_with(&prefix) {
                    continue;
                }
                let old = &self.document(reference.location.source).unwrap().text
                    [reference.location.range.clone()];
                if let Some(suffix) = old.strip_prefix(&prefix) {
                    edits.push(TextEdit {
                        location: reference.location.clone(),
                        replacement: format!("{name}:{suffix}"),
                    });
                }
            }
        }
        edits.sort_by_key(|edit| (edit.location.source, edit.location.range.start));
        edits.dedup_by(|a, b| a.location == b.location);
        Ok(edits)
    }

    /// Bounded completion lists prevent copying entire cache registries on each keystroke.
    pub fn completions(&self, source: SourceId, offset: usize, limit: usize) -> Vec<Completion> {
        let Some(document) = self.document(source) else {
            return Vec::new();
        };
        if !document.text.is_char_boundary(offset) {
            return Vec::new();
        }
        if let Some(items) = annotations::completions(document, offset, limit) {
            return items;
        }
        if let Some(items) = config::completions(self, document, offset, limit) {
            return items;
        }
        let Some(before) = document.text.get(..offset) else {
            return Vec::new();
        };
        let start = before
            .rfind(|ch: char| !ch.is_ascii_alphanumeric() && !"_:+.".contains(ch))
            .map_or(0, |i| i + 1);
        let prefix = &before[start..];
        let index = self.symbol_index();
        if !matches!(before[..start].chars().next_back(), Some('$' | '%' | '^'))
            && let Some(kind) = index
                .hooks
                .get(&source)
                .and_then(|hooks| hooks.iter().find_map(|hook| hook.transmit_at(offset)))
        {
            return index
                .symbols()
                .filter(|symbol| {
                    symbol.name.starts_with(prefix)
                        && index::visible(symbol, document, offset)
                        && transmit_id(&self.registry, kind, &symbol.name).is_some()
                })
                .take(limit)
                .map(|symbol| Completion {
                    label: symbol.name.clone(),
                    detail: symbol.detail.clone(),
                    kind: symbol.kind,
                    replacement: start..offset,
                })
                .collect();
        }
        if index.hooks.get(&source).is_some_and(|hooks| {
            hooks
                .iter()
                .any(|hook| hook.callback.start <= offset && offset <= hook.callback.end)
        }) {
            return index
                .symbols()
                .filter(|symbol| {
                    matches!(
                        symbol.kind,
                        SymbolKind::ClientScript | SymbolKind::Procedure
                    ) && symbol.scope == Some(Scope::Client)
                        && symbol.name.starts_with(prefix)
                })
                .take(limit)
                .map(|symbol| Completion {
                    label: symbol.name.clone(),
                    detail: symbol.detail.clone(),
                    kind: symbol.kind,
                    replacement: start..offset,
                })
                .collect();
        }
        let sigil = before[..start].chars().next_back();
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        let line = before.rsplit('\n').next().unwrap_or(before).trim_start();
        let header = line.strip_prefix('[').filter(|line| !line.contains(']'));
        if header.is_some_and(|header| !header.contains(',')) {
            let names = self.registry.profile.as_ref().map_or_else(
                || trigger_table::names().collect::<Vec<_>>(),
                |profile| profile.triggers.keys().map(String::as_str).collect(),
            );
            return names
                .into_iter()
                .filter(|name| name.starts_with(prefix))
                .map(|name| Completion {
                    label: name.into(),
                    detail: "trigger".into(),
                    kind: SymbolKind::Trigger,
                    replacement: start..offset,
                })
                .take(limit)
                .collect();
        }
        let trigger = header
            .and_then(|header| header.split_once(','))
            .map(|(trigger, _)| trigger.trim());
        let expected = trigger
            .and_then(|trigger| self.registry.trigger(trigger))
            .and_then(|trigger| trigger.subject_type)
            .or_else(|| {
                self.signature_help(source, offset).and_then(|help| {
                    help.parameters
                        .get(help.active_parameter)
                        .map(|(_, ty)| *ty)
                })
            });
        let matches_type = |ty: Type| {
            expected.is_none_or(|expected| {
                expected == Type::Any
                    || expected == ty
                    || matches!(
                        (expected, ty),
                        (Type::NamedObj, Type::Obj) | (Type::Obj, Type::NamedObj)
                    )
                    || (trigger.is_some() && expected == Type::Interface && ty == Type::Component)
            })
        };
        for symbol in self.symbol_index().symbols() {
            if !symbol.name.starts_with(prefix) || !index::visible(symbol, document, offset) {
                continue;
            }
            let inaccessible = match symbol.kind {
                SymbolKind::Constant => self.registry.visibility.check(&document.label, "constant", &symbol.name, true).is_some(),
                SymbolKind::GameVariable => self.registry.lookup_game_var(&symbol.name).is_some_and(|variable| {
                    matches!(&variable.kind, RegistryKind::GameVar { category, .. } if self.registry.visibility.check(&document.label, category, &symbol.name, true).is_some())
                }),
                SymbolKind::Config => symbol.value_type.is_some_and(|ty| self.registry.visibility.typed(&document.label, &symbol.name, ty, &self.registry).is_some()),
                _ => false,
            };
            if inaccessible {
                continue;
            }
            if !matches!(sigil, Some('$' | '^' | '%' | '~' | '@')) {
                if let Some(trigger) = trigger.filter(|_| expected.is_none()) {
                    if !symbol.key.ends_with(&format!(":{trigger}:{}", symbol.name)) {
                        continue;
                    }
                } else if let Some(expected) = expected {
                    let script_type = matches!(
                        expected,
                        Type::Proc
                            | Type::Label
                            | Type::Queue
                            | Type::Timer
                            | Type::SoftTimer
                            | Type::Walktrigger
                            | Type::ControllerQueue
                            | Type::NpcQueue
                            | Type::ClientScript
                    );
                    if script_type {
                        if !symbol
                            .key
                            .ends_with(&format!(":{}:{}", expected.name(), symbol.name))
                        {
                            continue;
                        }
                    } else if symbol.kind != SymbolKind::Config
                        || !symbol.value_type.is_some_and(matches_type)
                    {
                        continue;
                    }
                }
            }
            let matches = match sigil {
                Some('$') => symbol.container.is_some(),
                Some('^') => symbol.kind == SymbolKind::Constant,
                Some('%') => symbol.kind == SymbolKind::GameVariable,
                Some('~') => symbol.kind == SymbolKind::Procedure,
                Some('@') => symbol.key.contains(":label:"),
                _ => {
                    symbol.container.is_none()
                        && !matches!(symbol.kind, SymbolKind::Constant | SymbolKind::GameVariable)
                }
            };
            if matches && seen.insert(symbol.name.clone()) {
                result.push(Completion {
                    label: symbol.name.clone(),
                    detail: symbol.detail.clone(),
                    kind: symbol.kind,
                    replacement: start..offset,
                });
            }
        }
        if !matches!(sigil, Some('$' | '^' | '%' | '~' | '@'))
            && trigger.is_none_or(|_| expected.is_some())
        {
            let mut configs = self
                .registry
                .entity_ids
                .iter()
                .filter(|(name, entry)| {
                    name.starts_with(prefix)
                        && entry.variants.iter().any(|(ty, _)| {
                            matches_type(*ty)
                                && self
                                    .registry
                                    .visibility
                                    .typed(&document.label, name, *ty, &self.registry)
                                    .is_none()
                        })
                })
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>();
            configs.sort_unstable();
            for name in configs.into_iter().take(limit) {
                if seen.insert(name.to_string()) {
                    let entry = &self.registry.entity_ids[name];
                    result.push(Completion {
                        label: name.into(),
                        detail: entry
                            .variants
                            .iter()
                            .map(|(ty, id)| format!("{} {name} = {id}", ty.name()))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        kind: SymbolKind::Config,
                        replacement: start..offset,
                    });
                }
            }
        }
        if sigil.is_none_or(|sigil| !"$^%~@".contains(sigil))
            && expected.is_none()
            && header.is_none()
        {
            let keywords =
                "if else while return case default calc switch_int switch_string true false null";
            let types = "int boolean string long char colour coord loc npc obj namedobj player_uid npc_uid seq spotanim synth midi jingle graphic fontmetrics model texture idkit mesanim hitmark stat component iftype setposh setposv setsize settextalignh settextalignv interface toplevelinterface overlayinterface inv enum struct param dbtable dbrow dbcolumn category varp varn vars varpbit mapzone maparea mapelement locshape npc_mode npc_stat area hunt movespeed entityoverlay stringvector writeinvobj controller regioncoord region_uid label proc clientscript queue softtimer timer walktrigger any";
            for name in keywords
                .split_whitespace()
                .map(str::to_string)
                .chain(types.split_whitespace().map(|ty| format!("def_{ty}")))
            {
                if name.starts_with(prefix) && seen.insert(name.clone()) {
                    result.push(Completion {
                        label: name.clone(),
                        detail: "keyword".into(),
                        kind: SymbolKind::Variable,
                        replacement: start..offset,
                    });
                }
            }
        }
        result.sort_by(|a, b| a.label.cmp(&b.label));
        result.truncate(limit);
        result
    }

    pub fn signature_help(&self, source: SourceId, offset: usize) -> Option<SignatureHelp> {
        let document = self.document(source)?;
        let tokens = index::tokens(
            document,
            &|name| self.symbol_index().is_compound(name),
            self.symbol_index()
                .hooks
                .get(&source)
                .map_or(&[][..], Vec::as_slice),
        );
        let mut stack: Vec<(Option<usize>, usize)> = Vec::new();
        let mut previous = None;
        let index = self.symbol_index();
        for (token, range) in tokens.iter().filter(|(_, range)| range.start < offset) {
            match token.kind {
                Kind::LParen => {
                    stack.push((
                        previous
                            .and_then(|position| index.at(source, position))
                            .map(|reference| reference.symbol),
                        0,
                    ));
                }
                Kind::LBrace | Kind::LBracket => stack.push((None, 0)),
                Kind::RParen | Kind::RBrace | Kind::RBracket => {
                    stack.pop();
                }
                Kind::Comma => {
                    if let Some((_, argument)) = stack.last_mut() {
                        *argument += 1;
                    }
                }
                Kind::SingleLineComment
                | Kind::MultiLineComment
                | Kind::StringLiteral
                | Kind::RawStringLiteral => continue,
                _ => {}
            }
            previous = Some(range.start);
        }
        let (symbol, argument) = stack
            .into_iter()
            .rev()
            .find_map(|(symbol, argument)| symbol.map(|symbol| (symbol, argument)))?;
        let symbol = index.symbol(symbol);
        Some(SignatureHelp {
            label: symbol.detail.clone(),
            parameters: symbol.parameters.clone(),
            active_parameter: argument,
            documentation: symbol.documentation.clone(),
        })
    }

    /// Config values that do not name what their key holds.
    pub fn config_diagnostics(&self) -> DiagnosticsCollector {
        let mut diagnostics = DiagnosticsCollector::new();
        for (source, problems) in self.configs().problems() {
            let Some(document) = self.document(*source) else {
                continue;
            };
            for (line, message) in problems {
                diagnostics.warning(
                    document.label.clone(),
                    *line,
                    0,
                    message.clone(),
                    Phase::TypeChecking,
                );
            }
        }
        diagnostics
    }

    pub fn semantic_tokens(&self, source: SourceId) -> Vec<SemanticToken> {
        let Some(document) = self.document(source) else {
            return Vec::new();
        };
        if document.config.is_some() {
            // Never waits for the index: until it exists, values typed by a
            // declaration read as plain words.
            return match self.configs.lock().unwrap().as_ref() {
                Some(configs) => config::tokens(document, configs.types()),
                None => config::tokens(document, &Default::default()),
            };
        }
        highlighting::script(document, self.symbol_index())
    }

    pub fn folding_ranges(&self, source: SourceId) -> Vec<Range<usize>> {
        let Some(document) = self.document(source) else {
            return Vec::new();
        };
        let mut stack = Vec::new();
        let mut ranges = Vec::new();
        for (token, span) in document.tokens.iter().zip(&document.spans) {
            match token.kind {
                Kind::LBrace => stack.push(span.start),
                Kind::RBrace => {
                    if let Some(start) = stack.pop() {
                        ranges.push(start..span.end);
                    }
                }
                Kind::MultiLineComment => ranges.push(span.clone()),
                _ => {}
            }
        }
        ranges.extend(
            self.document_symbols(source)
                .into_iter()
                .filter(|symbol| symbol.container.is_none())
                .filter_map(|symbol| symbol.extent.as_ref().map(|extent| extent.range.clone())),
        );
        ranges.retain(|range| document.text[range.clone()].contains('\n'));
        ranges.sort_by_key(|range| (range.start, range.end));
        ranges
    }

    pub fn selection_ranges(&self, source: SourceId, offset: usize) -> Vec<Range<usize>> {
        let Some(document) = self.document(source) else {
            return Vec::new();
        };
        let mut ranges = self
            .folding_ranges(source)
            .into_iter()
            .filter(|range| range.contains(&offset))
            .collect::<Vec<_>>();
        if let Some(range) = document.spans.iter().find(|range| range.contains(&offset)) {
            ranges.push(range.clone());
        }
        if let Some(position) = document.lines.position(&document.text, offset)
            && let Some(line) = document.lines.line(position.line)
        {
            ranges.push(line);
        }
        ranges.push(0..document.text.len());
        ranges.sort_by_key(|range| range.len());
        ranges.dedup();
        let mut nested: Vec<Range<usize>> = Vec::new();
        for range in ranges {
            if nested
                .last()
                .is_none_or(|inner| range.start <= inner.start && range.end >= inner.end)
            {
                nested.push(range);
            }
        }
        nested
    }

    /// Reindent without rewriting expressions, literals or comments.
    /// Each edit is confined to whitespace outside string tokens.
    pub fn format(&self, source: SourceId, tab_size: usize, spaces: bool) -> Vec<TextEdit> {
        let Some(document) = self.document(source) else {
            return Vec::new();
        };
        let mut edits = Vec::new();
        let mut depth = 0usize;
        let mut token_index = 0;
        let protected = document
            .tokens
            .iter()
            .zip(&document.spans)
            .filter(|(token, _)| {
                matches!(
                    token.kind,
                    Kind::StringLiteral | Kind::RawStringLiteral | Kind::MultiLineComment
                )
            })
            .map(|(_, span)| span.clone())
            .collect::<Vec<_>>();
        let unit = if spaces {
            " ".repeat(tab_size.clamp(1, 16))
        } else {
            "\t".into()
        };
        for line in 0..document.lines.line_count() {
            let range = document.lines.line(line).unwrap();
            let text = &document.text[range.clone()];
            let content = text.trim_end_matches(['\r', '\n']);
            let next = protected.partition_point(|span| span.end <= range.start);
            let protected_line = protected[next..]
                .iter()
                .take_while(|span| span.start < range.end)
                .any(|span| span.start < range.start || span.end > range.start + content.len());
            let trimmed = content.trim();
            let indent = depth.saturating_sub(usize::from(trimmed.starts_with('}')));
            let formatted = if trimmed.is_empty() {
                String::new()
            } else {
                format!("{}{trimmed}", unit.repeat(indent))
            };
            if !protected_line && content != formatted {
                edits.push(TextEdit {
                    location: Location {
                        source,
                        range: range.start..range.start + content.len(),
                    },
                    replacement: formatted,
                });
            }
            while token_index < document.tokens.len()
                && document.spans[token_index].start < range.end
            {
                match document.tokens[token_index].kind {
                    Kind::LBrace => depth += 1,
                    Kind::RBrace => depth = depth.saturating_sub(1),
                    _ => {}
                }
                token_index += 1;
            }
        }
        edits
    }

    pub fn code_actions(&self, diagnostics: &[Diagnostic]) -> Vec<CodeAction> {
        let mut actions = Vec::new();
        for diagnostic in diagnostics {
            for help in &diagnostic.help {
                let mut edits = Vec::new();
                for suggestion in &help.suggestions {
                    let Some(document) = self
                        .documents
                        .values()
                        .find(|document| document.label == suggestion.file)
                    else {
                        continue;
                    };
                    let start = document
                        .lines
                        .line(suggestion.line_range.0.saturating_sub(1))
                        .map_or(document.text.len(), |range| range.start);
                    let end = document
                        .lines
                        .line(suggestion.line_range.1.saturating_sub(1))
                        .map_or(document.text.len(), |range| range.end);
                    edits.push(TextEdit {
                        location: Location {
                            source: document.id,
                            range: start..end.max(start),
                        },
                        replacement: suggestion.replacement.clone(),
                    });
                }
                if !edits.is_empty() {
                    actions.push(CodeAction {
                        title: help.message.clone(),
                        kind: "quickfix",
                        edits,
                        applicability: help.applicability,
                    });
                }
            }
        }
        actions
    }

    /// Verbose source excerpts from the current buffer, including unsaved edits.
    pub fn render_diagnostic(&self, diagnostic: &Diagnostic) -> String {
        let mut text = format!("{} [{}]\n", diagnostic, diagnostic.code());
        if let Some(document) = self
            .documents
            .values()
            .find(|document| document.label == diagnostic.file)
        {
            let range =
                document
                    .lines
                    .diagnostic_range(&document.text, diagnostic.line, diagnostic.column);
            if let Some(Position { line, .. }) =
                document.lines.position(&document.text, range.start)
                && let Some(line_range) = document.lines.line(line)
            {
                let excerpt = document.text[line_range.clone()].trim_end_matches(['\r', '\n']);
                let column = document.text[line_range.start..range.start].chars().count();
                text.push_str(&format!(
                    "{:>5} | {excerpt}\n      | {}^\n",
                    line + 1,
                    " ".repeat(column)
                ));
            }
        }
        for help in &diagnostic.help {
            text.push_str(&format!("help: {}\n", help.message));
        }
        text
    }
}

impl Diagnostic {
    /// Stable phase codes; message wording is not part of the diagnostic API.
    pub fn code(&self) -> &'static str {
        match self.phase {
            Phase::Lexing => "RUNEC1001",
            Phase::Parsing => "RUNEC1002",
            Phase::SymbolRegistration => "RUNEC2001",
            Phase::TypeChecking => "RUNEC2002",
            Phase::CodeGeneration => "RUNEC3001",
            Phase::PointerCheck => "RUNEC4001",
        }
    }
}
