use crate::clientscript::hooks::transmit_kind;
use crate::lexer::Lexer;
use crate::project::Document;
use crate::symbol::SymbolRegistry;
use crate::token::{Kind, Token};
use std::ops::Range;

pub(super) struct Hook {
    pub range: Range<usize>,
    pub callback: Range<usize>,
    pub tokens: Vec<(Token, Range<usize>)>,
    pub transmits: Option<(Range<usize>, &'static str)>,
}

impl Hook {
    pub fn transmit_at(&self, offset: usize) -> Option<&'static str> {
        self.transmits.as_ref().and_then(|(range, kind)| {
            (range.start <= offset && offset <= range.end).then_some(*kind)
        })
    }
}

pub(super) fn collect(document: &Document, registry: &SymbolRegistry) -> Vec<Hook> {
    let mut hooks = Vec::new();
    let significant: Vec<_> = document
        .tokens
        .iter()
        .zip(&document.spans)
        .filter(|(token, _)| {
            !matches!(token.kind, Kind::SingleLineComment | Kind::MultiLineComment)
        })
        .collect();
    let mut calls = Vec::new();
    for (i, (token, span)) in significant.iter().enumerate() {
        match token.kind {
            Kind::LParen => {
                calls.push((i, 0));
                continue;
            }
            Kind::RParen => {
                calls.pop();
                continue;
            }
            Kind::Comma => {
                if let Some((_, argument)) = calls.last_mut() {
                    *argument += 1;
                }
                continue;
            }
            _ => {}
        }
        if !matches!(token.kind, Kind::StringLiteral | Kind::RawStringLiteral)
            || i == 0
            || !matches!(significant[i - 1].0.kind, Kind::LParen | Kind::Comma)
        {
            continue;
        }
        let Some(&(open, argument)) = calls.last() else {
            continue;
        };
        let Some(mut command) = open.checked_sub(1) else {
            continue;
        };
        if &document.text[significant[command].1.clone()] == "*" && command > 0 {
            command -= 1;
        }
        let name = &document.text[significant[command].1.clone()];
        if !registry.hook_commands.contains_key(name) && name != "runclientscript"
            || argument != usize::from(registry.component_first_command(name))
        {
            continue;
        }
        let range = (*span).clone();
        let raw = &document.text[range.clone()];
        let start = if raw.starts_with("r\"") { 2 } else { 1 };
        let end = raw.len() - usize::from(raw.ends_with('"'));
        if start > end {
            continue;
        }
        let mut text = String::new();
        let mut offsets = Vec::new();
        let mut pos = start;
        while pos < end {
            let begin = pos;
            if raw.as_bytes()[pos] == b'\\'
                && pos + 1 < end
                && matches!(raw.as_bytes()[pos + 1], b'\\' | b'"' | b'<')
            {
                pos += 1;
            }
            let ch = raw[pos..].chars().next().unwrap();
            text.push(ch);
            for byte in 0..ch.len_utf8() {
                offsets.push(range.start + if byte == 0 { begin } else { pos + byte });
            }
            pos += ch.len_utf8();
        }
        offsets.push(range.start + end);
        let (tokens, spans, _) = Lexer::new(&text, &document.label).tokenize_spanned();
        let tokens: Vec<_> = tokens
            .into_iter()
            .zip(spans)
            .filter(|(token, _)| token.kind != Kind::EndOfFile)
            .map(|(mut token, span)| {
                token.line += significant[i].0.line - 1;
                (token, offsets[span.start]..offsets[span.end])
            })
            .collect();
        let callback = tokens
            .first()
            .map_or(range.start + start..range.start + start, |(_, span)| {
                span.clone()
            });
        let transmits = transmit_kind(name).and_then(|kind| {
            let open = tokens
                .iter()
                .find(|(token, _)| token.kind == Kind::LBrace)?
                .1
                .end;
            let close = tokens
                .iter()
                .rfind(|(token, _)| token.kind == Kind::RBrace)
                .map_or(range.end - usize::from(raw.ends_with('"')), |(_, span)| {
                    span.start
                });
            Some((open..close, kind))
        });
        hooks.push(Hook {
            range,
            callback,
            tokens,
            transmits,
        });
    }
    hooks
}
