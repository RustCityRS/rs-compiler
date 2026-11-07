use super::{InlayHint, SymbolKind, index};
use crate::project::{Project, SourceId};
use crate::token::Kind;
use std::ops::Range;

impl Project {
    pub fn inlay_hints(&self, source: SourceId, range: Range<usize>) -> Vec<InlayHint> {
        let Some(document) = self.document(source) else {
            return Vec::new();
        };
        if document.config.is_some() {
            return Vec::new();
        }
        let index = self.symbol_index();
        let hooks = index.hooks.get(&source).map_or(&[][..], Vec::as_slice);
        let tokens = index::tokens(document, &index.compounds, hooks);
        let mut hints = Vec::new();
        for reference in index
            .references
            .iter()
            .filter(|reference| reference.location.source == source && !reference.declaration)
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
            let symbol = &index.symbols[reference.symbol];
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
