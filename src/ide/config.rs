use super::{Location, Reference, SemanticToken, Symbol, SymbolKind, TokenKind, index};
use crate::project::{Document, Project};
use crate::sources::Scope;
use crate::symbol::normalize_comp_name;
use crate::types::Type;
use std::collections::HashMap;

pub(super) fn entity_type(kind: &str) -> Option<Type> {
    match kind {
        "if" | "interface" | "overlayinterface" => Some(Type::Interface),
        "idk" => Some(Type::IdKit),
        "mesanim" => Some(Type::MesAnim),
        "namedobj" => Some(Type::Obj),
        _ => Type::from_name(kind),
    }
}

pub(super) fn key(kind: &str, name: &str) -> String {
    match kind {
        "constant" => format!("constant:{name}"),
        "varp" | "varn" | "vars" | "varbit" | "varc" | "varcstr" => format!("gamevar:{name}"),
        "command" | "cs2_command" => format!("command:{name}"),
        _ => format!(
            "config:{}:{name}",
            entity_type(kind).map_or(kind, |ty| ty.name())
        ),
    }
}

pub(super) fn entity_key(ty: Type, name: &str) -> String {
    if matches!(ty, Type::Component | Type::Interface)
        && let Some((interface, component)) = name.split_once(':')
    {
        return key(
            "component",
            &format!("{interface}:{}", normalize_comp_name(component)),
        );
    }
    if ty == Type::DbColumn
        && let Some((column, slot)) = name.rsplit_once(':')
        && column.contains(':')
        && slot.parse::<u32>().is_ok()
    {
        return key("dbcolumn", column);
    }
    key(ty.name(), name)
}

fn lines(document: &Document) -> impl Iterator<Item = (usize, &str)> {
    let mut offset = 0;
    document.text.split_inclusive('\n').map(move |line| {
        let start = offset;
        offset += line.len();
        (start, line)
    })
}

pub(super) fn declarations(
    project: &Project,
    symbols: &mut Vec<Symbol>,
    keys: &mut HashMap<String, usize>,
    references: &mut Vec<Reference>,
) {
    let mut documents = project
        .documents()
        .filter(|doc| doc.config.is_some())
        .collect::<Vec<_>>();
    documents.sort_by_key(|doc| {
        (
            doc.config.as_ref().unwrap().starts_with("pack:"),
            doc.label.to_string_lossy().contains("_unpack"),
        )
    });
    for document in documents {
        let kind = document.config.as_deref().unwrap();
        let pack = kind.strip_prefix("pack:");
        let kind = pack.unwrap_or(kind);
        let interfaces = if pack == Some("interface") {
            lines(document)
                .filter_map(|(_, line)| {
                    let (id, name) = line.trim().split_once('=')?;
                    Some((id.parse::<u32>().ok()?, name.trim()))
                })
                .collect::<HashMap<_, _>>()
        } else {
            HashMap::new()
        };
        let comments = tokens(document)
            .into_iter()
            .filter(|token| token.kind == TokenKind::Comment)
            .map(|token| token.range)
            .collect::<Vec<_>>();
        let mut sections = Vec::new();
        let mut current = String::new();
        for (offset, line) in lines(document) {
            let line = line.trim_end_matches(['\r', '\n']);
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.is_empty() {
                continue;
            }
            let indent = line.len() - trimmed.len();
            let pos = comments.partition_point(|range| range.start <= offset + indent);
            if pos
                .checked_sub(1)
                .is_some_and(|pos| comments[pos].contains(&(offset + indent)))
            {
                continue;
            }
            let mut entry_kind = kind;
            let entry = if pack.is_some() {
                trimmed.split_once('=').and_then(|(id, value)| {
                    let name = value.split("//").next().unwrap().trim();
                    let start = offset + indent + trimmed.len() - value.trim_start().len();
                    let qualified = if kind == "interface"
                        && let Some((interface, component)) = id.split_once(':')
                    {
                        let interface = interfaces.get(&interface.parse::<u32>().ok()?)?;
                        component.parse::<u32>().ok()?;
                        entry_kind = "component";
                        format!("{interface}:{name}")
                    } else {
                        id.trim().parse::<u32>().ok()?;
                        if kind == "interface" && name.contains(':') {
                            entry_kind = "component";
                        }
                        name.to_string()
                    };
                    Some((qualified, start..start + name.len()))
                })
            } else if let Some(rest) = trimmed.strip_prefix('[') {
                rest.split_once(']').map(|(name, _)| {
                    if matches!(kind, "if" | "interface") && name.contains(':') {
                        entry_kind = "component";
                    }
                    let start = offset + indent + 1 + name.len() - name.trim_start().len();
                    (name.trim().to_string(), start..start + name.trim().len())
                })
            } else if kind == "constant" {
                trimmed.split_once('=').map(|(name, _)| {
                    let name = name.trim().trim_start_matches('^');
                    let start = offset + indent + usize::from(trimmed.starts_with('^'));
                    (name.to_string(), start..start + name.len())
                })
            } else {
                None
            };
            if let Some((name, range)) = entry {
                if name.is_empty() {
                    continue;
                }
                current = name.clone();
                sections.push((name, range, offset, entry_kind.to_string()));
            } else if kind == "dbtable"
                && let Some(column) = trimmed.strip_prefix("column=")
            {
                let name = column.split(',').next().unwrap().trim();
                let start =
                    offset + indent + "column=".len() + column.len() - column.trim_start().len();
                sections.push((
                    format!("{current}:{name}"),
                    start..start + name.len(),
                    offset,
                    "dbcolumn".into(),
                ));
            }
        }
        for (i, (name, range, start, kind)) in sections.iter().enumerate() {
            let symbol_key = if pack.is_some() && matches!(kind.as_str(), "script" | "clientscript")
            {
                let Some((trigger, name)) = name.trim_matches(['[', ']']).split_once(',') else {
                    continue;
                };
                let found = keys
                    .get(&index::key(Scope::Global, trigger, name))
                    .or_else(|| keys.get(&index::key(document.scope, trigger, name)))
                    .copied();
                let Some(id) = found else { continue };
                references.push(Reference {
                    location: Location {
                        source: document.id,
                        range: range.start + 1 + trigger.len() + 1..range.end - 1,
                    },
                    symbol: id,
                    declaration: true,
                });
                continue;
            } else {
                entity_type(kind).map_or_else(|| key(kind, name), |ty| entity_key(ty, name))
            };
            let end = sections[i + 1..]
                .iter()
                .find(|entry| kind != "dbtable" || entry.3 == "dbtable")
                .map_or(document.text.len(), |entry| entry.2);
            let location = Location {
                source: document.id,
                range: range.clone(),
            };
            let id = *keys.entry(symbol_key.clone()).or_insert_with(|| {
                let id = symbols.len();
                symbols.push(Symbol {
                    name: name.clone(),
                    kind: SymbolKind::Config,
                    detail: format!("{kind} {name}"),
                    documentation: String::new(),
                    value_type: entity_type(kind),
                    definition: None,
                    extent: None,
                    parameters: Vec::new(),
                    returns: Vec::new(),
                    container: None,
                    scope: None,
                    key: symbol_key,
                });
                id
            });
            let symbol = &mut symbols[id];
            if symbol.definition.is_none() {
                symbol.definition = Some(location.clone());
                symbol.extent = Some(Location {
                    source: document.id,
                    range: *start..end,
                });
                symbol.documentation = document.text[*start..end].trim().to_string();
            }
            references.push(Reference {
                location,
                symbol: id,
                declaration: true,
            });
        }
    }
}

pub(super) fn tokens(document: &Document) -> Vec<SemanticToken> {
    let text = &document.text;
    let bytes = text.as_bytes();
    let mut result = Vec::new();
    let mut i = 0;
    let mut value = false;
    let mut section = false;
    while i < bytes.len() {
        let start = i;
        if bytes[i].is_ascii_whitespace() {
            if bytes[i] == b'\n' {
                value = false;
                section = false;
            }
            i += 1;
            continue;
        }
        let kind = if text[i..].starts_with("//") {
            i += text[i..].find('\n').unwrap_or(bytes.len() - i);
            TokenKind::Comment
        } else if text[i..].starts_with("/*") {
            i += text[i + 2..]
                .find("*/")
                .map_or(bytes.len() - i, |end| end + 4);
            if text[start..i].contains('\n') {
                value = false;
                section = false;
            }
            TokenKind::Comment
        } else if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'\n' {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            // An escape may precede a multibyte character.
            while !text.is_char_boundary(i) {
                i += 1;
            }
            TokenKind::String
        } else if !value && bytes[i] == b'[' {
            section = true;
            i += 1;
            TokenKind::Decorator
        } else if section && bytes[i] == b']' {
            section = false;
            i += 1;
            TokenKind::Decorator
        } else if section {
            while i < bytes.len() && !b"]\r\n".contains(&bytes[i]) {
                i += 1;
            }
            while i > start && bytes[i - 1].is_ascii_whitespace() {
                i -= 1;
            }
            TokenKind::Namespace
        } else if bytes[i] == b'=' {
            value = true;
            i += 1;
            TokenKind::Operator
        } else if b"^%".contains(&bytes[i]) {
            i += 1;
            while i < bytes.len() && config_ident(bytes[i]) {
                i += 1;
            }
            TokenKind::Variable
        } else if config_ident(bytes[i]) {
            i += 1;
            while i < bytes.len() && config_ident(bytes[i]) {
                i += 1;
            }
            let word = &text[start..i];
            if !value && text[i..].trim_start_matches([' ', '\t']).starts_with('=') {
                TokenKind::Property
            } else if matches!(word, "true" | "false" | "yes" | "no" | "null") {
                TokenKind::Keyword
            } else if number(word) {
                TokenKind::Number
            } else {
                TokenKind::EnumMember
            }
        } else {
            i += text[i..].chars().next().unwrap().len_utf8();
            TokenKind::Operator
        };
        let mut token = SemanticToken::new(start..i, kind);
        token.declaration = kind == TokenKind::Namespace || kind == TokenKind::Property;
        token.static_ = kind == TokenKind::Variable;
        token.readonly = token.static_ && bytes[start] == b'^';
        if token.static_ && !value {
            token.declaration = true;
        }
        result.push(token);
    }
    result
}

fn config_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"_:.+-".contains(&byte)
}

fn number(word: &str) -> bool {
    let word = word.trim_start_matches('-');
    if let Some(hex) = word.strip_prefix("0x") {
        return !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit());
    }
    !word.is_empty() && word.bytes().all(|b| b.is_ascii_digit() || b == b'_')
}

pub(super) fn references(
    project: &Project,
    keys: &HashMap<String, usize>,
    symbols: &[Symbol],
    references: &mut Vec<Reference>,
) {
    let names = symbols
        .iter()
        .enumerate()
        .filter(|(_, symbol)| symbol.kind == SymbolKind::Config)
        .map(|(id, symbol)| (symbol.name.as_str(), id))
        .collect::<HashMap<_, _>>();
    let mut declared = HashMap::<_, Vec<_>>::new();
    for reference in references.iter() {
        declared
            .entry(reference.location.source)
            .or_default()
            .push(reference.location.range.clone());
    }
    for ranges in declared.values_mut() {
        ranges.sort_by_key(|range| range.start);
    }
    for document in project.documents().filter(|doc| doc.config.is_some()) {
        let declarations = declared.get(&document.id).map_or(&[][..], Vec::as_slice);
        for mut token in tokens(document) {
            let prefix = document.text.as_bytes()[token.range.start];
            if token.static_ {
                token.range.start += 1;
            }
            let pos = declarations.partition_point(|range| range.start <= token.range.start);
            if !matches!(token.kind, TokenKind::Variable | TokenKind::EnumMember)
                || pos
                    .checked_sub(1)
                    .is_some_and(|pos| token.range.end <= declarations[pos].end)
            {
                continue;
            }
            let name = &document.text[token.range.clone()];
            let before = &document.text[..token.range.start];
            let line = before.rsplit('\n').next().unwrap_or(before).trim_start();
            let field = line.split('=').next().unwrap_or("");
            if matches!(field, "name" | "desc" | "description" | "examine") {
                continue;
            }
            let id = match prefix {
                b'^' => keys.get(&key("constant", name)).copied(),
                b'%' => keys.get(&key("varp", name)).copied(),
                _ => {
                    let kind = if field.starts_with("model") {
                        "model"
                    } else if field.starts_with("recol") || field.starts_with("retex") {
                        continue;
                    } else {
                        field.trim_end_matches(|ch: char| ch.is_ascii_digit())
                    };
                    keys.get(&key(kind, name))
                        .copied()
                        .or_else(|| names.get(name).copied())
                }
            };
            if let Some(symbol) = id {
                references.push(Reference {
                    location: Location {
                        source: document.id,
                        range: token.range,
                    },
                    symbol,
                    declaration: false,
                });
            }
        }
    }
}
