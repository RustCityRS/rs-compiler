use super::index::Lexeme;
use super::{InlayHint, ScriptPointers, Symbol, SymbolKind, index};
use crate::effects::ScriptEffects;
use crate::project::{Project, SourceId};
use crate::token::Kind;
use crate::types::{BaseVarType, Type};
use std::ops::Range;

/// Literals and `calc` say nothing about their role; names and calls do.
fn unclear(argument: &[(Lexeme<'_>, Range<usize>)]) -> bool {
    match argument.first().map(|(token, _)| token.kind) {
        Some(
            Kind::Number
            | Kind::LongLiteral
            | Kind::StringLiteral
            | Kind::RawStringLiteral
            | Kind::BooleanTrue
            | Kind::BooleanFalse
            | Kind::CharLiteral
            | Kind::CoordLiteral
            | Kind::Null
            | Kind::Calc,
        ) => true,
        Some(Kind::BinaryOperator) => {
            argument[0].0.value == "-"
                && argument.get(1).is_some_and(|(token, _)| {
                    matches!(token.kind, Kind::Number | Kind::LongLiteral)
                })
        }
        _ => false,
    }
}

/// The name without `$` or a numeric suffix, and whether it had the suffix.
fn stem(name: &str) -> (&str, bool) {
    let name = name.trim_start_matches('$');
    let stem = name.trim_end_matches(|c: char| c.is_ascii_digit());
    (stem, stem.len() < name.len())
}

/// The type a name spells. `command` and `type` parse as `int` but are not its name.
fn spelled_type(name: &str) -> Option<Type> {
    Type::from_name(name).filter(|ty| *ty != Type::Int || name == "int")
}

/// Whether a parameter name tells more than its type, its numbered siblings and
/// the call's own name. Hook parameters take callbacks, which name themselves.
fn descriptive(name: &str, ty: Type, callee: &Symbol) -> bool {
    if ty == Type::ClientScript {
        return false;
    }
    let (stem, numbered) = stem(name);
    let stack = match ty.base_type() {
        BaseVarType::Integer => "int",
        BaseVarType::String => "string",
        BaseVarType::Long => "long",
    };
    if stem == ty.name()
        || stem == ty.declared_name()
        || stem == stack
        || spelled_type(stem) == Some(ty)
    {
        return false;
    }
    if numbered
        && (spelled_type(stem).is_some()
            || callee
                .parameters
                .iter()
                .filter(|(other, _)| self::stem(other) == (stem, true))
                .count()
                > 1)
    {
        return false;
    }
    !(stem.len() >= 3 && callee.name.contains(stem))
}

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
            ) || symbol.parameters.len() < 2
            {
                continue;
            }
            let next = tokens.partition_point(|(_, range)| range.start < span.end);
            if tokens
                .get(next)
                .is_none_or(|(token, _)| token.kind != Kind::LParen)
            {
                continue;
            }
            let end = hooks[..hooks.partition_point(|hook| hook.range.start <= span.start)]
                .last()
                .filter(|hook| hook.range.contains(&span.start))
                .map_or(usize::MAX, |hook| hook.range.end);
            let mut depth = 0;
            let mut argument = 0;
            let mut argument_start = true;
            let mut literal_end = 0;
            for (i, (token, token_range)) in tokens.iter().enumerate().skip(next) {
                if token_range.start >= end {
                    break;
                }
                if token_range.start < literal_end {
                    continue;
                }
                if token.kind == Kind::Semicolon {
                    break;
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
                    if let Some((name, ty)) = symbol.parameters.get(argument)
                        && unclear(&tokens[i..])
                        && descriptive(name, *ty, symbol)
                    {
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
        }
        hints.sort_by_key(|hint| hint.offset);
        hints
    }
}
