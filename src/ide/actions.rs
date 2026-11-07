use super::{CodeAction, Location, SymbolKind, TextEdit, index};
use crate::diagnostics::{Applicability, Diagnostic};
use crate::find_testscript_line;
use crate::lexer::Lexer;
use crate::parser::{Parser, Statement};
use crate::project::{Document, Project, SourceId};
use crate::token::Kind;
use crate::types::Type;
use std::collections::HashSet;
use std::ops::Range;

fn distance(left: &str, right: &str) -> usize {
    let mut row: Vec<usize> = (0..=right.len()).collect();
    for (i, a) in left.bytes().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, b) in right.bytes().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (row[j + 1] + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(a != b));
            diagonal = old;
        }
    }
    row[right.len()]
}

fn insertion(document: &Document, offset: usize) -> usize {
    find_testscript_line(&document.text)
        .and_then(|line| document.lines.line(line.saturating_sub(1)))
        .map(|line| line.start)
        .filter(|marker| offset < *marker)
        .unwrap_or(document.text.len())
}

impl Project {
    pub fn quick_fixes(&self, source: SourceId, diagnostics: &[Diagnostic]) -> Vec<CodeAction> {
        let mut actions = self.code_actions(diagnostics);
        let Some(document) = self.document(source) else {
            return actions;
        };
        let index = self.symbol_index();
        for diagnostic in diagnostics {
            if diagnostic.message.contains("from production code") {
                continue;
            }
            let Some(line) = document.lines.line(diagnostic.line.saturating_sub(1)) else {
                continue;
            };
            if diagnostic.message.starts_with("Type mismatch:") {
                let types = diagnostic.message.split('\'').collect::<Vec<_>>();
                if let (Some(given), Some(expected)) = (types.get(1), types.get(3)) {
                    for (declared, inferred) in [(given, expected), (expected, given)] {
                        if Type::from_name(inferred).is_none() {
                            continue;
                        }
                        if let Some(range) = document.tokens.iter().zip(&document.spans).find_map(
                            |(token, range)| {
                                (token.kind == Kind::Def
                                    && line.contains(&range.start)
                                    && document.text[range.clone()] == format!("def_{declared}"))
                                .then_some(range.clone())
                            },
                        ) {
                            actions.push(CodeAction {
                                title: format!("Change declared type to 'def_{inferred}'"),
                                kind: "quickfix",
                                edits: vec![TextEdit {
                                    location: Location { source, range },
                                    replacement: format!("def_{inferred}"),
                                }],
                                applicability: Applicability::MaybeIncorrect,
                            });
                        }
                    }
                }
            }
            let Some(raw) = diagnostic.message.split('\'').nth(1) else {
                continue;
            };
            let name = raw.trim_start_matches(['~', '@', '^', '%', '$']);
            let Some(range) = document
                .spans
                .iter()
                .find(|range| {
                    line.contains(&range.start) && document.text[(*range).clone()] == *name
                })
                .cloned()
            else {
                continue;
            };
            if diagnostic.message.contains("bare value")
                && let Some(symbol) = index
                    .symbols
                    .iter()
                    .find(|symbol| symbol.kind == SymbolKind::Command && symbol.name == name)
            {
                let args = symbol
                    .parameters
                    .iter()
                    .map(|(_, ty)| match ty {
                        Type::String => "\"\"",
                        Type::Boolean => "false",
                        Type::Long => "0L",
                        _ => "0",
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                actions.push(CodeAction {
                    title: format!("Call '{name}' with arguments"),
                    kind: "quickfix",
                    edits: vec![TextEdit {
                        location: Location {
                            source,
                            range: range.clone(),
                        },
                        replacement: format!("{name}({args})"),
                    }],
                    applicability: Applicability::HasPlaceholders,
                });
                continue;
            }
            let kind = if diagnostic.message.contains("constant") {
                Some(SymbolKind::Constant)
            } else if diagnostic.message.contains("proc") {
                Some(SymbolKind::Procedure)
            } else if diagnostic.message.contains("command") {
                Some(SymbolKind::Command)
            } else if diagnostic.message.contains("label") {
                Some(SymbolKind::Trigger)
            } else {
                None
            };
            if diagnostic.message.contains("resolve") || diagnostic.message.contains("Unknown") {
                let mut candidates = index
                    .symbols
                    .iter()
                    .filter(|symbol| {
                        kind.is_none_or(|kind| symbol.kind == kind)
                            && symbol.name.len().abs_diff(name.len()) <= 3
                            && index::visible(symbol, document, range.start)
                    })
                    .map(|symbol| (distance(name, &symbol.name), &symbol.name))
                    .filter(|(distance, _)| *distance > 0 && *distance <= 3)
                    .collect::<Vec<_>>();
                candidates.sort();
                candidates.dedup();
                for (_, candidate) in candidates.into_iter().take(3) {
                    actions.push(CodeAction {
                        title: format!("Change to '{candidate}'"),
                        kind: "quickfix",
                        edits: vec![TextEdit {
                            location: Location {
                                source,
                                range: range.clone(),
                            },
                            replacement: candidate.clone(),
                        }],
                        applicability: Applicability::MaybeIncorrect,
                    });
                }
                if matches!(kind, Some(SymbolKind::Procedure | SymbolKind::Trigger)) {
                    let trigger = if kind == Some(SymbolKind::Procedure) {
                        "proc"
                    } else {
                        "label"
                    };
                    let insertion = insertion(document, range.start);
                    actions.push(CodeAction {
                        title: format!("Create [{trigger},{name}]"),
                        kind: "quickfix",
                        edits: vec![TextEdit {
                            location: Location {
                                source,
                                range: insertion..insertion,
                            },
                            replacement: format!("\n\n[{trigger},{name}]\nreturn;\n"),
                        }],
                        applicability: Applicability::MaybeIncorrect,
                    });
                }
                if diagnostic.message.contains("symbol") {
                    actions.push(CodeAction {
                        title: format!("Quote '{name}' as a string"),
                        kind: "quickfix",
                        edits: vec![TextEdit {
                            location: Location {
                                source,
                                range: range.clone(),
                            },
                            replacement: format!("\"{name}\""),
                        }],
                        applicability: Applicability::MaybeIncorrect,
                    });
                }
            }
        }
        actions
    }

    pub fn extract_proc(
        &self,
        source: SourceId,
        range: Range<usize>,
        name: &str,
    ) -> Result<CodeAction, String> {
        let document = self.document(source).ok_or("Unknown source")?;
        if range.is_empty() || document.config.is_some() {
            return Err("Select statements inside a script".into());
        }
        let text = document
            .text
            .get(range.clone())
            .ok_or("Invalid selection")?;
        let index = self.symbol_index();
        if index
            .symbols
            .iter()
            .any(|symbol| symbol.container.is_none() && symbol.name == name)
        {
            return Err("A script with this name already exists".into());
        }
        if name.is_empty()
            || !name
                .bytes()
                .enumerate()
                .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
        {
            return Err("Invalid procedure name".into());
        }
        let enclosing = index
            .symbols
            .iter()
            .find(|symbol| {
                symbol.container.is_none()
                    && symbol.kind != SymbolKind::Command
                    && symbol.extent.as_ref().is_some_and(|extent| {
                        extent.source == source
                            && extent.range.start < range.start
                            && range.end <= extent.range.end
                    })
            })
            .ok_or("Selection must stay inside one script")?;
        let selected = document
            .tokens
            .iter()
            .zip(&document.spans)
            .filter(|(_, span)| range.start <= span.start && span.start < range.end)
            .collect::<Vec<_>>();
        if selected
            .iter()
            .any(|(token, _)| token.kind == Kind::JumpCall)
        {
            return Err("A jump cannot be extracted into a returning procedure".into());
        }
        let mut params = Vec::new();
        let mut outputs = Vec::new();
        let mut seen = HashSet::new();
        for reference in index
            .references
            .iter()
            .filter(|r| r.location.source == source && range.contains(&r.location.range.start))
        {
            let symbol = &index.symbols[reference.symbol];
            if symbol.container.is_none() || !seen.insert(reference.symbol) {
                continue;
            }
            let defined_inside = symbol
                .definition
                .as_ref()
                .is_some_and(|definition| range.contains(&definition.range.start));
            if !defined_inside {
                params.push(symbol);
            }
            let used_after = index.references.iter().any(|r| {
                r.symbol == reference.symbol
                    && r.location.source == source
                    && r.location.range.start >= range.end
            });
            let assigned = index
                .references
                .iter()
                .filter(|r| {
                    r.symbol == reference.symbol
                        && r.location.source == source
                        && range.contains(&r.location.range.start)
                })
                .any(|r| {
                    document
                        .spans
                        .iter()
                        .position(|span| *span == r.location.range)
                        .is_some_and(|i| {
                            document
                                .tokens
                                .iter()
                                .skip(i + 1)
                                .find(|token| {
                                    !matches!(
                                        token.kind,
                                        Kind::Comma | Kind::LocalVar | Kind::Identifier
                                    )
                                })
                                .is_some_and(|token| token.kind == Kind::Equals)
                        })
                });
            if used_after && (defined_inside || assigned) {
                outputs.push((symbol, defined_inside));
            }
        }
        let has_return = selected.iter().any(|(token, _)| token.kind == Kind::Return);
        let probe = format!("[proc,{name}]\n{text}");
        let tokens = Lexer::new(&probe, &document.label)
            .tokenize()
            .map_err(|_| "Select complete statements")?;
        let (parsed, errors) = Parser::new(tokens, &document.label).parse_recovering();
        if !errors.is_empty() || parsed.scripts.len() != 1 {
            return Err("Select complete statements".into());
        }
        if has_return
            && !parsed.scripts[0]
                .body
                .last()
                .is_some_and(|statement| matches!(statement, Statement::Return { .. }))
        {
            return Err("A selection with returns must end with an unconditional return".into());
        }
        if params.iter().any(|symbol| {
            symbol.definition.as_ref().is_some_and(|definition| {
                document
                    .spans
                    .iter()
                    .position(|span| *span == definition.range)
                    .is_some_and(|i| {
                        document
                            .tokens
                            .get(i + 1)
                            .is_some_and(|token| token.kind == Kind::LParen)
                    })
            })
        }) {
            return Err("An array local cannot be passed as a scalar parameter".into());
        }
        let parameters = params
            .iter()
            .map(|symbol| {
                format!(
                    "{} ${}",
                    symbol.value_type.unwrap_or(Type::Int).name(),
                    symbol.name
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let arguments = params
            .iter()
            .map(|symbol| format!("${}", symbol.name))
            .collect::<Vec<_>>()
            .join(", ");
        let returns = if has_return {
            enclosing
                .returns
                .iter()
                .map(|ty| ty.name())
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            outputs
                .iter()
                .map(|(symbol, _)| symbol.value_type.unwrap_or(Type::Int).name())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let values = outputs
            .iter()
            .map(|(symbol, _)| format!("${}", symbol.name))
            .collect::<Vec<_>>()
            .join(", ");
        let mut replacement = String::new();
        for (symbol, new) in outputs.iter().filter(|_| !has_return) {
            if *new {
                replacement.push_str(&format!(
                    "def_{} ${};\n",
                    symbol.value_type.unwrap_or(Type::Int).name(),
                    symbol.name
                ));
            }
        }
        if has_return && !enclosing.returns.is_empty() {
            replacement.push_str("return(");
        } else if !has_return && !outputs.is_empty() {
            replacement.push_str(&format!("{values} = "));
        }
        replacement.push_str(&format!("~{name}({arguments})"));
        replacement.push_str(if has_return && !enclosing.returns.is_empty() {
            ");"
        } else {
            ";"
        });
        if has_return && enclosing.returns.is_empty() {
            replacement.push_str("\nreturn;");
        }
        let body = format!(
            "\n\n[proc,{name}]({parameters}){}\n{}\n{}\n",
            if returns.is_empty() {
                String::new()
            } else {
                format!("({returns})")
            },
            text.trim(),
            if has_return {
                String::new()
            } else if outputs.is_empty() {
                "return;".into()
            } else {
                format!("return({values});")
            }
        );
        let insertion = insertion(document, range.start);
        Ok(CodeAction {
            title: format!("Extract procedure '{name}'"),
            kind: "refactor.extract",
            edits: vec![
                TextEdit {
                    location: Location { source, range },
                    replacement,
                },
                TextEdit {
                    location: Location {
                        source,
                        range: insertion..insertion,
                    },
                    replacement: body,
                },
            ],
            applicability: Applicability::MaybeIncorrect,
        })
    }
}
