use super::{Index, SemanticToken, SymbolKind, TokenKind, annotations};
use crate::lexer::Lexer;
use crate::project::Document;
use crate::token::Kind;
use crate::types::Type;
use std::ops::Range;
use std::path::PathBuf;

const SCRIPT_TRIGGERS: &[&str] = &[
    "proc",
    "label",
    "debugproc",
    "clientscript",
    "queue",
    "timer",
    "softtimer",
    "walktrigger",
    "command",
    "controller_queue",
    "npc_queue",
];
const SCRIPT_COMMANDS: &[&str] = &[
    "queue",
    "timer",
    "softtimer",
    "walktrigger",
    "settimer",
    "cleartimer",
    "clearsofttimer",
    "gosub",
    "jump",
    "controller_queue",
    "npc_queue",
];

pub(super) fn script(document: &Document, index: &Index) -> Vec<SemanticToken> {
    let mut result = Vec::new();
    code(document, index, 0..document.text.len(), &mut result, 0);
    result
}

fn lex(text: &str, path: &PathBuf, offset: usize) -> Vec<(Kind, Range<usize>)> {
    let (tokens, spans, _) = Lexer::new(text, path).tokenize_spanned();
    let mut result = Vec::new();
    let mut end = 0;
    for (token, span) in tokens.iter().zip(spans) {
        recover(text, end..span.start, offset, &mut result);
        if token.kind != Kind::EndOfFile {
            result.push((token.kind, offset + span.start..offset + span.end));
        }
        end = span.end;
    }
    recover(text, end..text.len(), offset, &mut result);
    result
}

fn recover(text: &str, range: Range<usize>, offset: usize, out: &mut Vec<(Kind, Range<usize>)>) {
    let mut i = range.start;
    while i < range.end {
        let tail = &text[i..range.end];
        let ch = tail.chars().next().unwrap();
        if ch.is_whitespace() {
            i += ch.len_utf8();
            continue;
        }
        let (kind, len) = if tail.starts_with('#') {
            (Kind::Hash, tail.find('\n').unwrap_or(tail.len()))
        } else if tail.starts_with("/*") {
            (Kind::MultiLineComment, tail.len())
        } else if tail.starts_with('"') {
            (Kind::StringLiteral, tail.len())
        } else if tail.starts_with("r\"") {
            (Kind::RawStringLiteral, tail.len())
        } else if tail.starts_with('\'') {
            (Kind::CharLiteral, tail.len())
        } else {
            (Kind::BinaryOperator, ch.len_utf8())
        };
        out.push((kind, offset + i..offset + i + len));
        i += len;
    }
}

fn identifier(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Identifier | Kind::Trigger | Kind::Command | Kind::Def
    )
}

/// A word the lexer reads as a keyword, which after a sigil is still a name:
/// `^false` is a constant, not the boolean.
fn keyword(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::If
            | Kind::Else
            | Kind::While
            | Kind::Switch
            | Kind::Case
            | Kind::Default
            | Kind::Calc
            | Kind::Null
            | Kind::BooleanTrue
            | Kind::BooleanFalse
    )
}

fn code(
    document: &Document,
    index: &Index,
    range: Range<usize>,
    out: &mut Vec<SemanticToken>,
    depth: usize,
) {
    if depth > 32 {
        out.push(SemanticToken::new(range, TokenKind::String));
        return;
    }
    let text = &document.text;
    let tokens = index
        .hooks
        .get(&document.id)
        .and_then(|hooks| hooks.iter().find(|hook| hook.range == range))
        .map_or_else(
            || lex(&text[range.clone()], &document.label, range.start),
            |hook| {
                hook.tokens
                    .iter()
                    .map(|(token, span)| (token.kind, span.clone()))
                    .collect()
            },
        );
    let mut signature_end = 0;
    let annotations = annotations::collect(document);
    let mut i = 0;
    while i < tokens.len() {
        let (kind, mut span) = tokens[i].clone();
        let next = tokens.get(i + 1);
        if let Some(hint) = annotations
            .iter()
            .find(|hint| hint.range.contains(&span.start))
        {
            let token_kind = if span.start == hint.name.start {
                span = hint.name.clone();
                while tokens
                    .get(i + 1)
                    .is_some_and(|(_, next)| next.end <= span.end)
                {
                    i += 1;
                }
                TokenKind::Decorator
            } else if span == hint.value {
                TokenKind::EnumMember
            } else if matches!(kind, Kind::SingleLineComment | Kind::MultiLineComment) {
                TokenKind::Comment
            } else {
                TokenKind::Operator
            };
            out.push(SemanticToken::new(span, token_kind));
            i += 1;
            continue;
        }
        if kind == Kind::LBracket
            && next.is_some_and(|(kind, _)| identifier(*kind))
            && tokens
                .get(i + 2)
                .is_some_and(|(kind, _)| *kind == Kind::Comma)
        {
            out.push(SemanticToken::new(span, TokenKind::Decorator));
            out.push(SemanticToken::new(tokens[i + 1].1.clone(), TokenKind::Type));
            out.push(SemanticToken::new(
                tokens[i + 2].1.clone(),
                TokenKind::Operator,
            ));
            let trigger = &text[tokens[i + 1].1.clone()];
            i += 3;
            if let Some((_, subject)) = tokens.get(i) {
                let start = subject.start;
                let mut end = start;
                while i < tokens.len()
                    && tokens[i].0 != Kind::RBracket
                    && !text[start..tokens[i].1.end].contains('\n')
                {
                    end = tokens[i].1.end;
                    i += 1;
                }
                if start < end {
                    let named = SCRIPT_TRIGGERS.contains(&trigger);
                    let mut token = SemanticToken::new(
                        start..end,
                        if named {
                            TokenKind::Function
                        } else {
                            TokenKind::EnumMember
                        },
                    );
                    token.declaration = named;
                    token.default_library = trigger == "command";
                    out.push(token);
                }
            }
            if tokens
                .get(i)
                .is_some_and(|(kind, _)| *kind == Kind::RBracket)
            {
                out.push(SemanticToken::new(
                    tokens[i].1.clone(),
                    TokenKind::Decorator,
                ));
                i += 1;
                let mut j = i;
                for _ in 0..2 {
                    if tokens.get(j).is_none_or(|(kind, _)| *kind != Kind::LParen) {
                        break;
                    }
                    let mut nesting = 0;
                    while j < tokens.len() {
                        match tokens[j].0 {
                            Kind::LParen => nesting += 1,
                            Kind::RParen => nesting -= 1,
                            _ => {}
                        }
                        signature_end = tokens[j].1.end;
                        j += 1;
                        if nesting == 0 {
                            break;
                        }
                    }
                }
            }
            continue;
        }
        if matches!(kind, Kind::StringLiteral | Kind::RawStringLiteral)
            && index
                .hooks
                .get(&document.id)
                .is_some_and(|hooks| hooks.iter().any(|hook| hook.range == span))
        {
            let open = span.start + if kind == Kind::RawStringLiteral { 2 } else { 1 };
            out.push(SemanticToken::new(span.start..open, TokenKind::String));
            code(document, index, span.clone(), out, depth + 1);
            if document.text[span.clone()].ends_with('"') {
                out.push(SemanticToken::new(
                    span.end - 1..span.end,
                    TokenKind::String,
                ));
            }
            i += 1;
            continue;
        }
        if kind == Kind::StringLiteral {
            string(document, index, span, out, depth);
            i += 1;
            continue;
        }
        let mut symbol_offset = span.start;
        let prefix = matches!(
            kind,
            Kind::LocalVar | Kind::GameVar | Kind::ConstantVar | Kind::ScriptCall | Kind::JumpCall
        );
        if prefix
            && let Some((next_kind, name)) = next
            && (identifier(*next_kind) || keyword(*next_kind))
            && span.end == name.start
        {
            symbol_offset = name.start;
            span.end = name.end;
            i += 1;
        }
        let reference = index
            .at(document.id, symbol_offset)
            .filter(|reference| reference.location.range.start == symbol_offset);
        let symbol = reference.map(|reference| index.symbol(reference.symbol));
        if let Some(reference) = reference
            && reference.location.range.end > span.end
            && reference.location.range.end <= range.end
        {
            span.end = reference.location.range.end;
            while tokens
                .get(i + 1)
                .is_some_and(|(_, next)| next.end <= span.end)
            {
                i += 1;
            }
        }
        let name = &text[symbol_offset..span.end];
        let type_position = span.start < signature_end
            || tokens
                .get(i + 1)
                .is_some_and(|(kind, _)| *kind == Kind::LocalVar);
        let script_argument = i >= 2
            && tokens[i - 1].0 == Kind::LParen
            && SCRIPT_COMMANDS.contains(&&text[tokens[i - 2].1.clone()]);
        let token_kind = match kind {
            Kind::SingleLineComment | Kind::MultiLineComment => TokenKind::Comment,
            Kind::RawStringLiteral | Kind::CharLiteral => TokenKind::String,
            Kind::Number | Kind::LongLiteral | Kind::CoordLiteral => TokenKind::Number,
            Kind::Hash => TokenKind::Macro,
            Kind::LocalVar => {
                if symbol.is_some_and(|s| s.kind == SymbolKind::Parameter) {
                    TokenKind::Parameter
                } else {
                    TokenKind::Variable
                }
            }
            Kind::ConstantVar | Kind::GameVar => TokenKind::Variable,
            Kind::ScriptCall | Kind::JumpCall => TokenKind::Function,
            Kind::Def => TokenKind::Type,
            Kind::Identifier | Kind::Trigger | Kind::Command => {
                if Type::from_name(name).is_some()
                    && (type_position
                        || matches!(name, "int" | "string" | "long" | "char" | "boolean"))
                {
                    TokenKind::Type
                } else {
                    match symbol.map(|s| s.kind) {
                        Some(
                            SymbolKind::Command
                            | SymbolKind::Procedure
                            | SymbolKind::ClientScript
                            | SymbolKind::Trigger,
                        ) => TokenKind::Function,
                        Some(SymbolKind::Parameter) => TokenKind::Parameter,
                        Some(SymbolKind::Constant | SymbolKind::GameVariable) => {
                            TokenKind::Variable
                        }
                        Some(SymbolKind::Config) => TokenKind::EnumMember,
                        _ if script_argument
                            || tokens
                                .get(i + 1)
                                .is_some_and(|(kind, _)| *kind == Kind::LParen) =>
                        {
                            TokenKind::Function
                        }
                        _ => TokenKind::Variable,
                    }
                }
            }
            Kind::If
            | Kind::Else
            | Kind::While
            | Kind::Return
            | Kind::Case
            | Kind::Default
            | Kind::Calc
            | Kind::Switch
            | Kind::Null
            | Kind::BooleanTrue
            | Kind::BooleanFalse => TokenKind::Keyword,
            _ => TokenKind::Operator,
        };
        let mut token = SemanticToken::new(span, token_kind);
        token.declaration = reference.is_some_and(|r| r.declaration);
        token.readonly =
            kind == Kind::ConstantVar || symbol.is_some_and(|s| s.kind == SymbolKind::Constant);
        token.static_ = matches!(kind, Kind::ConstantVar | Kind::GameVar)
            || symbol
                .is_some_and(|s| matches!(s.kind, SymbolKind::Constant | SymbolKind::GameVariable));
        token.default_library = token_kind == TokenKind::Function
            && !prefix
            && !script_argument
            && symbol.is_none_or(|s| s.kind == SymbolKind::Command);
        out.push(token);
        i += 1;
    }
}

pub(crate) fn interpolations(raw: &str) -> Vec<(Range<usize>, bool)> {
    let bytes = raw.as_bytes();
    let mut result = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i = (i + 2).min(bytes.len());
            continue;
        }
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        let mut parens = 0usize;
        let mut quote = None;
        while i < bytes.len() {
            let ch = bytes[i];
            if quote.is_some() && ch == b'\\' {
                i = (i + 2).min(bytes.len());
                continue;
            }
            if quote == Some(ch) {
                quote = None;
            } else if quote.is_none() {
                match ch {
                    b'"' | b'\'' | b'`' => quote = Some(ch),
                    b'(' => parens += 1,
                    b')' => parens = parens.saturating_sub(1),
                    b'>' if parens == 0 => break,
                    _ => {}
                }
            }
            i += 1;
        }
        if i == bytes.len() {
            break;
        }
        let inner = raw[start + 1..i].trim();
        let name = inner.strip_prefix('.').unwrap_or(inner);
        let tag = name.trim_start_matches('/').split('=').next().unwrap_or("");
        let formatting = matches!(
            tag,
            "br" | "lt"
                | "gt"
                | "nbsp"
                | "shy"
                | "times"
                | "euro"
                | "copy"
                | "reg"
                | "u"
                | "str"
                | "shad"
                | "trans"
                | "col"
                | "img"
        );
        let code = !formatting
            && (inner.starts_with(['$', '%', '^', '~', '"'])
                || inner.contains('(')
                || (!name.is_empty()
                    && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')));
        result.push((start..i + 1, code));
        i += 1;
    }
    result
}

fn string(
    document: &Document,
    index: &Index,
    range: Range<usize>,
    out: &mut Vec<SemanticToken>,
    depth: usize,
) {
    let raw = &document.text[range.clone()];
    let mut start = range.start;
    for (part, expression) in interpolations(raw) {
        let open = range.start + part.start;
        let end = range.start + part.end;
        if start < open {
            out.push(SemanticToken::new(start..open, TokenKind::String));
        }
        if expression {
            out.push(SemanticToken::new(open..open + 1, TokenKind::Keyword));
            code(document, index, open + 1..end - 1, out, depth + 1);
            out.push(SemanticToken::new(end - 1..end, TokenKind::Keyword));
        } else {
            out.push(SemanticToken::new(open..end, TokenKind::Keyword));
        }
        start = end;
    }
    if start < range.end {
        out.push(SemanticToken::new(start..range.end, TokenKind::String));
    }
}
