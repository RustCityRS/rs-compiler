use super::{InlayHint, ScriptPointers, SymbolKind, index};
use crate::effects::ScriptEffects;
use crate::project::{Project, SourceId};
use crate::token::Kind;
use std::ops::Range;

impl Project {
    /// Cached AST effects, with pointer requirements attached only when already checked.
    pub fn script_effects(&self, source: SourceId) -> Vec<ScriptEffects> {
        let Some(document) = self.document(source) else {
            return Vec::new();
        };
        let Some(file) = document.syntax() else {
            return Vec::new();
        };
        let Some(effects) = self.effect_summaries() else {
            return Vec::new();
        };
        let requirements = self
            .pointer_requirements
            .as_ref()
            .filter(|(generation, _)| *generation == self.generation)
            .and_then(|(_, sources)| sources.get(&source));
        file.scripts
            .iter()
            .filter_map(|script| {
                if script.trigger == "command" {
                    return None;
                }
                let name = format!("[{},{}]", script.trigger, script.name);
                let summary = effects.scripts.get(&(source, script.line, name.clone()))?;
                let line = document.lines.line(script.line.saturating_sub(1))?;
                let text = &document.text[line.clone()];
                let start = line.start + text.len() - text.trim_start().len();
                Some(ScriptEffects {
                    range: start..start,
                    may_suspend: summary.may_suspend,
                    writes: summary.writes.iter().cloned().collect(),
                    incomplete: summary.incomplete,
                    required: requirements
                        .and_then(|requirements| requirements.get(&name))
                        .copied(),
                })
            })
            .collect()
    }

    /// Entry pointer requirements from the latest completed `check`. Stale or
    /// unavailable analysis produces no annotations.
    pub fn script_pointers(&self, source: SourceId) -> Vec<ScriptPointers> {
        if !self.pointer_requirements_ready() {
            return Vec::new();
        }
        let Some(document) = self.document(source) else {
            return Vec::new();
        };
        let Some(file) = document.syntax() else {
            return Vec::new();
        };
        let Some(requirements) = self.pointer_requirements.as_ref().unwrap().1.get(&source) else {
            return Vec::new();
        };
        file.scripts
            .iter()
            .filter(|script| script.trigger != "command")
            .filter_map(|script| {
                let name = format!("[{},{}]", script.trigger, script.name);
                let required = *requirements.get(&name)?;
                let line = document.lines.line(script.line.saturating_sub(1))?;
                let text = &document.text[line.clone()];
                let start = line.start + text.len() - text.trim_start().len();
                Some(ScriptPointers {
                    range: start..start,
                    required,
                })
            })
            .collect()
    }

    pub fn inlay_hints(&self, source: SourceId, range: Range<usize>) -> Vec<InlayHint> {
        let Some(document) = self.document(source) else {
            return Vec::new();
        };
        if document.config.is_some() {
            return Vec::new();
        }
        let index = self.symbol_index();
        let hooks = index.hooks.get(&source).map_or(&[][..], Vec::as_slice);
        let tokens = index::tokens(document, &|name| index.is_compound(name), hooks);
        let mut hints = Vec::new();
        for reference in index
            .references_in(source)
            .filter(|reference| !reference.declaration)
        {
            let span = &reference.location.range;
            if !range.contains(&span.start)
                || document
                    .tokens
                    .iter()
                    .zip(&document.spans)
                    .any(|(token, range)| {
                        matches!(token.kind, Kind::StringLiteral | Kind::RawStringLiteral)
                            && range.contains(&span.start)
                            && !hooks.iter().any(|hook| hook.range == *range)
                    })
            {
                continue;
            }
            let symbol = index.symbol(reference.symbol);
            if symbol.kind == SymbolKind::Constant {
                if let Some(ty) = symbol.value_type {
                    hints.push(InlayHint {
                        offset: span.end,
                        label: format!(": {}", ty.name()),
                        parameter: false,
                    });
                }
                continue;
            }
            if !matches!(
                symbol.kind,
                SymbolKind::Command
                    | SymbolKind::Procedure
                    | SymbolKind::Trigger
                    | SymbolKind::ClientScript
            ) {
                continue;
            }
            let next = tokens.partition_point(|(_, range)| range.start < span.end);
            let mut end = span.end;
            if tokens
                .get(next)
                .is_some_and(|(token, _)| token.kind == Kind::LParen)
            {
                let mut depth = 0;
                let mut argument = 0;
                let mut argument_start = true;
                let mut literal_end = 0;
                for (token, token_range) in &tokens[next..] {
                    if token_range.start < literal_end {
                        continue;
                    }
                    if matches!(token.kind, Kind::StringLiteral | Kind::RawStringLiteral) {
                        literal_end = token_range.end;
                    }
                    if depth == 1
                        && argument_start
                        && !matches!(
                            token.kind,
                            Kind::RParen | Kind::SingleLineComment | Kind::MultiLineComment
                        )
                    {
                        if let Some((name, _)) = symbol.parameters.get(argument) {
                            hints.push(InlayHint {
                                offset: token_range.start,
                                label: format!("{}:", name.trim_start_matches('$')),
                                parameter: true,
                            });
                        }
                        argument_start = false;
                    }
                    match token.kind {
                        Kind::LParen => depth += 1,
                        Kind::RParen => {
                            depth -= 1;
                            if depth == 0 {
                                end = token_range.end;
                                break;
                            }
                        }
                        Kind::Comma if depth == 1 => {
                            argument += 1;
                            argument_start = true;
                        }
                        _ => {}
                    }
                }
            } else if !symbol.parameters.is_empty() {
                continue;
            }
            if !symbol.returns.is_empty() {
                hints.push(InlayHint {
                    offset: end,
                    label: format!(
                        "-> {}",
                        symbol
                            .returns
                            .iter()
                            .map(|ty| ty.name())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    parameter: false,
                });
            }
        }
        hints.sort_by_key(|hint| hint.offset);
        hints
    }
}
