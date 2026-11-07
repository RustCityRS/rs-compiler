use super::fields::{self, ConfigTypes, Section, Value};
use super::{
    Completion, Hover, Index, Location, Reference, SemanticToken, Symbol, SymbolKind, TokenKind,
    index,
};
use crate::project::{Document, Project, SourceId};
use crate::sources::Scope;
use crate::symbol::normalize_comp_name;
use crate::types::Type;
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub(super) fn entity_type(kind: &str) -> Option<Type> {
    match kind {
        "if" | "interface" | "overlayinterface" => Some(Type::Interface),
        "idk" => Some(Type::IdKit),
        "mesanim" => Some(Type::MesAnim),
        "namedobj" => Some(Type::Obj),
        "sprites" => Some(Type::Graphic),
        "writeinv" => Some(Type::WriteinvObj),
        _ => Type::from_name(kind),
    }
}

pub(super) fn key(kind: &str, name: &str) -> String {
    match kind {
        "constant" => format!("constant:{name}"),
        "varp" | "varn" | "vars" | "varbit" | "varc" | "varcs" | "varclansetting" => {
            format!("gamevar:{name}")
        }
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

/// What a file goes by among others of its kind.
fn file_name(path: &Path) -> &str {
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
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
    types: &mut ConfigTypes,
) {
    let no_types = ConfigTypes::default();
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
        // A world map area's sections are its file ids, which nothing names.
        if matches!(kind, "fm" | "fms") {
            continue;
        }
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
        let comments = tokens(document, &no_types)
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
                    let range = start..start + name.trim().len();
                    if kind == "if3" {
                        entry_kind = "component";
                        (
                            format!("{}:{}", file_name(&document.label), name.trim()),
                            range,
                        )
                    } else {
                        (name.trim().to_string(), range)
                    }
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
                types
                    .columns
                    .entry(format!("{current}:{name}"))
                    .or_insert_with(|| {
                        column
                            .split(',')
                            .skip(1)
                            .filter_map(|ty| Type::from_name(ty.trim()))
                            .collect()
                    });
                let start =
                    offset + indent + "column=".len() + column.len() - column.trim_start().len();
                sections.push((
                    format!("{current}:{name}"),
                    start..start + name.len(),
                    offset,
                    "dbcolumn".into(),
                ));
            } else if kind == "param"
                && let Some(ty) = trimmed
                    .strip_prefix("type=")
                    .and_then(|ty| Type::from_name(ty.trim()))
            {
                types.params.entry(current.clone()).or_insert(ty);
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

struct Typed<'a> {
    token: SemanticToken,
    value: Option<Value>,
    table: Option<&'a str>,
}

pub(super) fn tokens(document: &Document, types: &ConfigTypes) -> Vec<SemanticToken> {
    typed_tokens(document, types)
        .into_iter()
        .map(|typed| typed.token)
        .collect()
}

fn typed_tokens<'a>(document: &'a Document, types: &ConfigTypes) -> Vec<Typed<'a>> {
    let text: &'a str = &document.text;
    let bytes = text.as_bytes();
    let file_kind = document.config.as_deref().unwrap_or_default();
    let mut result = Vec::new();
    let mut i = 0;
    let mut value = false;
    let mut section = false;
    let mut key = "";
    let mut values = None;
    let mut part = 0;
    let mut first = "";
    let mut context = Section::default();
    let mut quoted = false;
    // These files are read whole: nothing in a value starts a comment.
    let plain = matches!(file_kind, "if3" | "wma");
    while i < bytes.len() {
        let start = i;
        if bytes[i].is_ascii_whitespace() && !quoted {
            if bytes[i] == b'\n' {
                value = false;
                section = false;
                key = "";
                values = None;
            }
            i += 1;
            continue;
        }
        let slot = values.and_then(|values| fields::slot(values, part, first, &context, types));
        // A tile's parts end at `:` and `@` as well.
        let tile = values.is_some_and(|values| values.contains(&Value::Tile));
        let kind = if quoted {
            // A name in quotes is one name, whatever it holds.
            if bytes[i] == b'"' {
                quoted = false;
                i += 1;
                TokenKind::Operator
            } else {
                while i < bytes.len() && !b"\"\r\n".contains(&bytes[i]) {
                    i += 1;
                }
                quoted = i < bytes.len() && bytes[i] == b'"';
                TokenKind::EnumMember
            }
        } else if value && bytes[i] == b'"' && matches!(slot, Some(Value::Config(_))) {
            quoted = true;
            i += 1;
            TokenKind::Operator
        } else if text[i..].starts_with("//") && !(plain && value) {
            i += text[i..].find('\n').unwrap_or(bytes.len() - i);
            TokenKind::Comment
        } else if text[i..].starts_with("/*") && !(plain && value) {
            i += text[i + 2..]
                .find("*/")
                .map_or(bytes.len() - i, |end| end + 4);
            if text[start..i].contains('\n') {
                value = false;
                section = false;
                key = "";
                values = None;
            }
            TokenKind::Comment
        } else if matches!(slot, Some(Value::Text | Value::Config(Type::Graphic))) {
            // Text and sprite names run on past any comma; `//` ends them as it
            // does when the file is packed.
            let line = text[i..].split('\n').next().unwrap_or_default();
            i += if plain {
                line.trim_end_matches('\r').len()
            } else {
                line.split("//").next().unwrap_or(line).trim_end().len()
            };
            let word = &text[start..i];
            if slot == Some(Value::Text) {
                TokenKind::String
            } else if word == "null" {
                TokenKind::Keyword
            } else if number(word) {
                TokenKind::Number
            } else {
                TokenKind::EnumMember
            }
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
            context = Section::default();
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
        } else if value && (bytes[i] == b',' || tile && bytes[i] == b'@') {
            part += 1;
            i += 1;
            TokenKind::Operator
        } else if value && tile && bytes[i] == b':' {
            i += 1;
            TokenKind::Operator
        } else if bytes[i] == b'=' {
            if !value {
                values = fields::field(file_kind, key);
                part = 0;
                first = "";
            }
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
            if matches!(slot, Some(Value::Config(_))) {
                // A name can hold any character: it ends with its part.
                while i < bytes.len()
                    && !b",\r\n".contains(&bytes[i])
                    && !bytes[i..].starts_with(b"//")
                    && !(tile && bytes[i] == b'@')
                {
                    i += 1;
                }
                while bytes[i - 1].is_ascii_whitespace() {
                    i -= 1;
                }
            } else {
                while i < bytes.len() && config_ident(bytes[i]) && !(tile && bytes[i] == b':') {
                    i += 1;
                }
            }
            let word = &text[start..i];
            if !value && text[i..].trim_start_matches([' ', '\t']).starts_with('=') {
                key = word;
                TokenKind::Property
            } else if slot == Some(Value::Hook) && part == 0 && first.is_empty() {
                TokenKind::Function
            } else if matches!(word, "true" | "false" | "yes" | "no" | "null")
                || tile && word == "direct"
            {
                TokenKind::Keyword
            } else if slot == Some(Value::TypeName) && Type::from_name(word).is_some() {
                TokenKind::Type
            } else if number(word) {
                TokenKind::Number
            } else if slot == Some(Value::Tile) {
                TokenKind::Keyword
            } else {
                TokenKind::EnumMember
            }
        } else {
            i += text[i..].chars().next().unwrap().len_utf8();
            TokenKind::Operator
        };
        if value
            && part == 0
            && first.is_empty()
            && !matches!(kind, TokenKind::Operator | TokenKind::Comment)
        {
            first = &text[start..i];
            context.note(key, first);
        }
        let mut token = SemanticToken::new(start..i, kind);
        token.declaration = kind == TokenKind::Namespace || kind == TokenKind::Property;
        token.static_ = kind == TokenKind::Variable;
        token.readonly = token.static_ && bytes[start] == b'^';
        if token.static_ && !value {
            token.declaration = true;
        }
        result.push(Typed {
            token,
            value: slot,
            table: context.table,
        });
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
    types: &ConfigTypes,
    problems: &mut HashMap<SourceId, Vec<(usize, String)>>,
) {
    let names = symbols
        .iter()
        .enumerate()
        .filter(|(_, symbol)| symbol.kind == SymbolKind::Config)
        .map(|(id, symbol)| (symbol.name.as_str(), id))
        .collect::<HashMap<_, _>>();
    let tables = symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Config)
        .filter_map(|symbol| symbol.value_type)
        .collect::<HashSet<_>>();
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
    let vars_known = !project.registry.game_vars.is_empty();
    for document in project.documents().filter(|doc| doc.config.is_some()) {
        let declarations = declared.get(&document.id).map_or(&[][..], Vec::as_slice);
        let mut report = |offset: usize, message: String| {
            let line = document.text[..offset].matches('\n').count() + 1;
            problems
                .entry(document.id)
                .or_default()
                .push((line, message));
        };
        for typed in typed_tokens(document, types) {
            let Typed {
                mut token,
                value,
                table,
            } = typed;
            let prefix = document.text.as_bytes()[token.range.start];
            if token.static_ {
                token.range.start += 1;
            }
            let pos = declarations.partition_point(|range| range.start <= token.range.start);
            let id_of_config =
                token.kind == TokenKind::Number && matches!(value, Some(Value::Config(_)));
            let hook_script = token.kind == TokenKind::Function && value == Some(Value::Hook);
            if (!matches!(token.kind, TokenKind::Variable | TokenKind::EnumMember)
                && !id_of_config
                && !hook_script)
                || pos
                    .checked_sub(1)
                    .is_some_and(|pos| token.range.end <= declarations[pos].end)
            {
                continue;
            }
            let name = &document.text[token.range.clone()];
            let id = match (prefix, value) {
                (b'^', _) => keys.get(&key("constant", name)).copied(),
                (b'%', _) => keys.get(&key("varp", name)).copied(),
                (_, Some(Value::Config(ty))) if id_of_config => name
                    .parse()
                    .ok()
                    .and_then(|id| project.registry.visibility.config_name(ty, id))
                    .and_then(|name| keys.get(&entity_key(ty, name)).copied()),
                (_, Some(Value::Hook)) if hook_script => [
                    (Scope::Client, "clientscript"),
                    (Scope::Global, "clientscript"),
                    (Scope::Client, "proc"),
                    (Scope::Global, "proc"),
                ]
                .into_iter()
                .find_map(|(scope, trigger)| keys.get(&index::key(scope, trigger, name)).copied()),
                (_, Some(Value::Hook)) => name
                    .contains(':')
                    .then(|| keys.get(&entity_key(Type::Component, name)).copied())
                    .flatten()
                    .or_else(|| keys.get(&key("varp", name)).copied())
                    .or_else(|| names.get(name).copied()),
                (_, Some(Value::Component)) => keys
                    .get(&entity_key(
                        Type::Component,
                        &format!("{}:{name}", file_name(&document.label)),
                    ))
                    .copied(),
                (_, Some(Value::Sound)) => [Type::Synth, Type::Vorbis]
                    .into_iter()
                    .find_map(|ty| keys.get(&entity_key(ty, name)).copied()),
                // An object a held item can be, or nothing at all.
                (_, Some(Value::Config(Type::Obj))) if name == "hide" => continue,
                (_, Some(Value::Config(ty))) => {
                    let found = keys.get(&entity_key(ty, name)).copied();
                    if found.is_none()
                        && !is_placeholder(ty, name)
                        && project.registry.lookup_reference(name, ty).is_none()
                        && (tables.contains(&table_of(ty)) || (is_var(ty) && vars_known))
                    {
                        report(token.range.start, mismatch(&names, symbols, ty, name));
                    }
                    found
                }
                (_, Some(Value::AnyVar)) => {
                    let found = keys.get(&key("varp", name)).copied();
                    if found.is_none() && vars_known {
                        report(token.range.start, format!("Unknown var '{name}'"));
                    }
                    found
                }
                (_, Some(Value::ColumnName)) => table.and_then(|table| {
                    keys.get(&key("dbcolumn", &format!("{table}:{name}")))
                        .copied()
                }),
                (_, Some(_)) => continue,
                (_, None) => {
                    let before = &document.text[..token.range.start];
                    let line = before.rsplit('\n').next().unwrap_or(before).trim_start();
                    let field = line.split('=').next().unwrap_or("");
                    if matches!(field, "name" | "desc" | "description" | "examine") {
                        continue;
                    }
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

fn table_of(ty: Type) -> Type {
    if ty == Type::NamedObj { Type::Obj } else { ty }
}

/// The packer takes what a table's entries go by without a name, for any id.
fn is_placeholder(ty: Type, name: &str) -> bool {
    let own = [ty.name()];
    let tables: &[&str] = match ty {
        Type::Graphic => &["graphic", "sprites"],
        Type::MapElement => &["mapelement", "mel"],
        Type::NamedObj => &["obj"],
        Type::IdKit => &["idkit", "idk"],
        Type::Texture => &["material", "texture"],
        Type::Varp => &["varplayer", "varp"],
        Type::Varbit => &["varplayerbit", "varbit"],
        Type::Varn => &["varnpc", "varn"],
        Type::Vars => &["varworld", "vars"],
        Type::Varc => &["varclient", "varc"],
        Type::ParticleEmitter => &["pem"],
        Type::ParticleEffector => &["pef"],
        _ => &own,
    };
    tables.iter().any(|table| {
        name.strip_prefix(table)
            .and_then(|rest| rest.strip_prefix('_'))
            .and_then(|rest| rest.split('_').next())
            .is_some_and(|id| id.parse::<u32>().is_ok())
    })
}

fn is_var(ty: Type) -> bool {
    matches!(ty, Type::Varp | Type::Varn | Type::Vars | Type::Varbit)
}

fn mismatch(names: &HashMap<&str, usize>, symbols: &[Symbol], ty: Type, name: &str) -> String {
    match names.get(name).and_then(|&id| symbols[id].value_type) {
        Some(other) => format!("'{name}' is a {other}, expected {ty}"),
        None => format!("Unknown {ty} '{name}'"),
    }
}

/// Hovering a key shows what its value holds.
pub(super) fn key_hover(document: &Document, offset: usize) -> Option<Hover> {
    let kind = document.config.as_deref()?;
    let text: &str = &document.text;
    let line_start = text.get(..offset)?.rfind('\n').map_or(0, |i| i + 1);
    let line = text[line_start..].split('\n').next()?;
    let (before_equals, _) = line.split_once('=')?;
    let key = before_equals.trim();
    if key.starts_with(['[', '/']) {
        return None;
    }
    let start = line_start + before_equals.len() - before_equals.trim_start().len();
    let range = start..start + key.len();
    if !range.contains(&offset) {
        return None;
    }
    let values = fields::field(kind, key)?;
    Some(Hover {
        range,
        kind: SymbolKind::Variable,
        detail: format!("{key}: {}", fields::describe(values)),
        documentation: String::new(),
    })
}

/// Completes a key of the file, or a value of the type its key holds. `None`
/// when the file's keys are not known.
pub(super) fn completions(
    project: &Project,
    document: &Document,
    offset: usize,
    limit: usize,
) -> Option<Vec<Completion>> {
    let kind = document.config.as_deref()?;
    let table = fields::keys(kind);
    if table.is_empty() {
        return None;
    }
    let before = document.text.get(..offset)?;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let line = before[line_start..].trim_start();
    if line.starts_with('[') || line.starts_with("//") {
        return Some(Vec::new());
    }
    let start = before
        .rfind(|ch: char| !ch.is_ascii_alphanumeric() && !"_:+.-".contains(ch))
        .map_or(0, |i| {
            i + before[i..].chars().next().map_or(1, char::len_utf8)
        });
    let prefix = &before[start..];
    if !line.contains('=') {
        return Some(
            table
                .iter()
                .filter(|(key, _)| key.starts_with(prefix))
                .take(limit)
                .map(|(key, values)| Completion {
                    label: (*key).to_owned(),
                    detail: fields::describe(values),
                    kind: SymbolKind::Variable,
                    replacement: start..offset,
                })
                .collect(),
        );
    }
    let index = project.symbol_index();
    let sigil = before[..start].chars().next_back();
    let mut items = match (sigil, expected_at(before, kind, &index.config_types)) {
        (Some('^'), _) => symbols_of(index, prefix, |symbol| symbol.kind == SymbolKind::Constant),
        (Some('%'), _) | (_, Some(Value::AnyVar)) => symbols_of(index, prefix, |symbol| {
            symbol.kind == SymbolKind::GameVariable
        }),
        (_, Some(Value::Config(ty))) => configs_of(project, index, ty, prefix),
        (_, Some(Value::Flag)) => ["yes", "no"]
            .into_iter()
            .filter(|word| word.starts_with(prefix))
            .map(|word| {
                (
                    (*word).to_owned(),
                    "boolean".to_owned(),
                    SymbolKind::Variable,
                )
            })
            .collect(),
        (_, Some(Value::TypeName)) => Type::all_def_keywords()
            .into_iter()
            .filter_map(|keyword| keyword.strip_prefix("def_"))
            .filter(|name| name.starts_with(prefix))
            .map(|name| (name.to_owned(), "type".to_owned(), SymbolKind::Variable))
            .collect(),
        _ => Vec::new(),
    };
    items.sort_by(|a, b| a.0.cmp(&b.0));
    items.dedup_by(|a, b| a.0 == b.0);
    items.truncate(limit);
    Some(
        items
            .into_iter()
            .map(|(label, detail, kind)| Completion {
                label,
                detail,
                kind,
                replacement: start..offset,
            })
            .collect(),
    )
}

fn symbols_of(
    index: &Index,
    prefix: &str,
    wanted: impl Fn(&Symbol) -> bool,
) -> Vec<(String, String, SymbolKind)> {
    index
        .symbols
        .iter()
        .filter(|symbol| symbol.name.starts_with(prefix) && wanted(symbol))
        .map(|symbol| (symbol.name.clone(), symbol.detail.clone(), symbol.kind))
        .collect()
}

fn configs_of(
    project: &Project,
    index: &Index,
    ty: Type,
    prefix: &str,
) -> Vec<(String, String, SymbolKind)> {
    let same = |other: Type| table_of(other) == table_of(ty);
    let mut items = symbols_of(index, prefix, |symbol| match symbol.kind {
        SymbolKind::Config => symbol.value_type.is_some_and(same),
        SymbolKind::GameVariable => project
            .registry
            .lookup_reference(&symbol.name, ty)
            .is_some(),
        _ => false,
    });
    items.extend(
        project
            .registry
            .entity_ids
            .iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .filter_map(|(name, entry)| {
                let (_, id) = entry.variants.iter().find(|(other, _)| same(*other))?;
                Some((
                    name.clone(),
                    format!("{} {name} = {id}", ty.name()),
                    SymbolKind::Config,
                ))
            }),
    );
    items
}

/// What the value at the end of `before` should be.
fn expected_at(before: &str, kind: &str, types: &ConfigTypes) -> Option<Value> {
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let (key, value) = before[line_start..].split_once('=')?;
    let section_start = before[..line_start].rfind("\n[").map_or(0, |i| i + 1);
    let mut context = Section::default();
    for line in before[section_start..line_start].lines() {
        if let Some((key, value)) = line.split_once('=') {
            context.note(key.trim(), value.trim());
        }
    }
    let parts = value.split(',').collect::<Vec<_>>();
    fields::slot(
        fields::field(kind, key.trim())?,
        parts.len() - 1,
        parts[0].trim(),
        &context,
        types,
    )
}
