use super::{Completion, Hover, SymbolKind};
use crate::parser::{ANNOTATIONS, Annotation, annotation_values};
use crate::project::Document;
use crate::token::Kind;
use std::ops::Range;

pub(super) struct Hint<'a> {
    pub annotation: &'a Annotation,
    pub range: Range<usize>,
    pub name: Range<usize>,
    pub value: Range<usize>,
}

pub(super) fn collect(document: &Document) -> Vec<Hint<'_>> {
    let offset = |line: usize, column: usize| {
        document
            .lines
            .line(line.saturating_sub(1))?
            .start
            .checked_add(column.checked_sub(1)?)
    };
    document
        .syntax()
        .into_iter()
        .flat_map(|file| &file.scripts)
        .flat_map(|script| &script.annotations)
        .filter_map(|annotation| {
            let start = offset(annotation.line, annotation.column)?;
            let end = offset(annotation.end_line, annotation.end_column)?;
            let value = offset(annotation.value_line, annotation.value_column)?;
            Some(Hint {
                annotation,
                range: start..end,
                name: start..start + 1 + annotation.name.len(),
                value: value..value + annotation.value.len(),
            })
        })
        .collect()
}

pub(super) fn hover(document: &Document, offset: usize) -> Option<Hover> {
    let hint = collect(document)
        .into_iter()
        .find(|hint| hint.range.contains(&offset))?;
    let annotation = hint.annotation;
    let documentation = match (annotation.name.as_str(), annotation.value.as_str()) {
        ("stack", "component_last") => {
            "Declares the component as the first source argument. The compiler evaluates the remaining arguments first and emits the component last to match the client VM. Unpacking restores the component-first call while preserving bytecode evaluation order."
        }
        ("signature", "fixed") => {
            "Uses the parameter and return types declared in the command signature. This is the default when no signature annotation is present."
        }
        ("signature", "hook") => {
            "An event hook on the active component. A quoted callback is expanded into its script ID, arguments, optional transmit triggers and type signature. Use null to clear the hook."
        }
        ("signature", "hook_component") => {
            "An event hook with an explicit component. A quoted callback is expanded into its script ID, arguments, optional transmit triggers and type signature. Combine with @stack(component_last) to declare the component first. Use null to clear the hook."
        }
        ("signature", "enum") => {
            "An enum lookup whose first two arguments describe its key and value types. The second type argument determines the return type."
        }
        ("signature", "enum_output") => {
            "An enum operation whose first argument describes the enum value type. Unpacking renders that argument as a type name."
        }
        ("signature", "param") => {
            "A parameter lookup whose return type comes from the registered type of its final param argument."
        }
        _ => return None,
    };
    Some(Hover {
        range: hint.range,
        kind: SymbolKind::Command,
        detail: format!("@{}({})", annotation.name, annotation.value),
        documentation: documentation.into(),
    })
}

pub(super) fn completions(
    document: &Document,
    offset: usize,
    limit: usize,
) -> Option<Vec<Completion>> {
    if document.config.is_some() {
        return None;
    }
    let tokens: Vec<_> = document
        .tokens
        .iter()
        .zip(&document.spans)
        .filter(|(token, _)| {
            !matches!(
                token.kind,
                Kind::SingleLineComment | Kind::MultiLineComment | Kind::EndOfFile
            )
        })
        .collect();
    let at_index = tokens
        .iter()
        .rposition(|(token, span)| token.kind == Kind::JumpCall && span.start < offset)?;
    let at = tokens[at_index].1.start;
    let line_start = document.text[..at].rfind('\n').map_or(0, |start| start + 1);
    if !document.text[line_start..at].trim().is_empty() {
        return None;
    }
    let rest = document.text.get(at + 1..offset)?;
    let command_header = |index: usize| {
        tokens
            .get(index + 1)
            .is_some_and(|(_, span)| &document.text[span.start..span.end] == "command")
            && tokens
                .get(index + 2)
                .is_some_and(|(token, _)| token.kind == Kind::Comma)
    };
    let previous_header = tokens[..at_index]
        .iter()
        .rposition(|(token, _)| token.kind == Kind::LBracket)
        .map(command_header);
    let next_header = tokens
        .iter()
        .enumerate()
        .skip(at_index + 1)
        .find(|(_, (token, _))| token.kind == Kind::LBracket)
        .map(|(index, _)| index);
    if next_header.is_some_and(|index| tokens[index].1.start < offset) {
        return None;
    }
    if next_header.is_some_and(|index| !command_header(index))
        || (previous_header == Some(false) && next_header.is_none())
    {
        return None;
    }
    let end_index = next_header.unwrap_or(tokens.len());
    let mut index = at_index;
    while index < end_index {
        if tokens[index].0.kind != Kind::JumpCall {
            return None;
        }
        index += 1;
        if index == end_index {
            break;
        }
        if tokens[index].0.kind != Kind::Identifier {
            return None;
        }
        index += 1;
        if index == end_index {
            break;
        }
        if tokens[index].0.kind != Kind::LParen {
            return None;
        }
        index += 1;
        if index == end_index {
            break;
        }
        if tokens[index].0.kind != Kind::RParen {
            if tokens[index].0.kind != Kind::Identifier {
                return None;
            }
            index += 1;
            if index == end_index {
                break;
            }
        }
        if tokens[index].0.kind != Kind::RParen {
            return None;
        }
        index += 1;
    }
    let (prefix, start, names, detail) = if let Some((name, value)) = rest.split_once('(') {
        if value.contains(')') || value.contains(',') {
            return None;
        }
        let prefix = value.trim_start();
        if !prefix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return None;
        }
        let names = annotation_values(name.trim())?.to_vec();
        (
            prefix,
            offset - prefix.len(),
            names,
            format!("@{} value", name.trim()),
        )
    } else {
        if !rest
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return None;
        }
        (
            rest,
            at + 1,
            ANNOTATIONS.iter().map(|(name, _)| *name).collect(),
            "compiler annotation".into(),
        )
    };
    if !names.iter().any(|name| name.starts_with(prefix)) {
        return None;
    }
    let end = document.text[offset..]
        .find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .map_or(document.text.len(), |end| offset + end);
    Some(
        names
            .into_iter()
            .filter(|name| name.starts_with(prefix))
            .take(limit)
            .map(|name| Completion {
                label: name.into(),
                detail: detail.clone(),
                kind: SymbolKind::Command,
                replacement: start..end,
            })
            .collect(),
    )
}
