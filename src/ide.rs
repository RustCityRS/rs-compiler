//! Editor queries over the same documents and definitions used for compilation.
//! Locations and edits use UTF-8 byte ranges; `text::LineIndex` converts UTF-16.
mod actions;
mod config;
mod fields;
mod highlighting;
mod hints;
mod hooks;
mod index;
mod queries;

use crate::diagnostics::Applicability;
use crate::project::SourceId;
use crate::sources::Scope;
use crate::types::Type;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub source: SourceId,
    pub range: Range<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    Procedure,
    ClientScript,
    Trigger,
    Parameter,
    Variable,
    Constant,
    GameVariable,
    Command,
    Config,
}

#[derive(Clone, Debug)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub detail: String,
    pub documentation: String,
    pub value_type: Option<Type>,
    pub definition: Option<Location>,
    pub extent: Option<Location>,
    pub parameters: Vec<(String, Type)>,
    pub returns: Vec<Type>,
    pub(crate) container: Option<(SourceId, usize)>,
    pub(crate) scope: Option<Scope>,
    pub(crate) key: String,
}

#[derive(Clone, Debug)]
pub struct Completion {
    pub label: String,
    pub detail: String,
    pub kind: SymbolKind,
    pub replacement: Range<usize>,
}

#[derive(Clone, Debug)]
pub struct Hover {
    pub range: Range<usize>,
    pub kind: SymbolKind,
    pub detail: String,
    pub documentation: String,
}

#[derive(Clone, Debug)]
pub struct SignatureHelp {
    pub label: String,
    pub parameters: Vec<(String, Type)>,
    pub active_parameter: usize,
    pub documentation: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub location: Location,
    pub replacement: String,
}

#[derive(Clone, Debug)]
pub struct CodeAction {
    pub title: String,
    pub kind: &'static str,
    pub edits: Vec<TextEdit>,
    pub applicability: Applicability,
}

#[derive(Clone, Debug)]
pub struct InlayHint {
    pub offset: usize,
    pub label: String,
    pub parameter: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Keyword,
    Function,
    Parameter,
    Variable,
    Property,
    Type,
    Number,
    String,
    Comment,
    Operator,
    EnumMember,
    Namespace,
    Decorator,
    Macro,
}

#[derive(Clone, Debug)]
pub struct SemanticToken {
    pub range: Range<usize>,
    pub kind: TokenKind,
    pub declaration: bool,
    pub readonly: bool,
    pub static_: bool,
    pub default_library: bool,
}

impl SemanticToken {
    fn new(range: Range<usize>, kind: TokenKind) -> Self {
        Self {
            range,
            kind,
            declaration: false,
            readonly: false,
            static_: false,
            default_library: false,
        }
    }
}

pub(crate) struct Reference {
    location: Location,
    symbol: usize,
    declaration: bool,
}

/// What configs and the registry declare. Kept while only scripts change.
pub(crate) struct Configs {
    key: u64,
    symbols: Vec<Symbol>,
    keys: HashMap<String, usize>,
    references: Vec<Reference>,
    links: Vec<index::Link>,
    compounds: HashSet<String>,
    config_types: fields::ConfigTypes,
    config_problems: HashMap<SourceId, Vec<(usize, String)>>,
}

pub(crate) struct Index {
    configs: Arc<Configs>,
    symbols: Vec<Symbol>,
    entities: Vec<Symbol>,
    references: Vec<Reference>,
    compounds: HashSet<String>,
    hooks: HashMap<SourceId, Vec<hooks::Hook>>,
}
