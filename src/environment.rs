//! Engine semantics supplied by an emulator or editor host.
use crate::pointer::{PointerHolder, PointerSet};
use crate::symbol::SymbolRegistry;
use crate::types::Type;
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct Trigger {
    pub byte: Option<u8>,
    pub subject_type: Option<Type>,
    pub validates_subject: bool,
    pub is_button: bool,
    pub allows_returns: bool,
}

pub type LookupResolver = fn(trigger: &str, subject: &str, registry: &SymbolRegistry) -> i32;

/// A complete engine contract. Empty maps mean unsupported, not an implicit
/// fallback to a particular game's revision. Legacy directory adapters retain
/// their existing defaults until a host supplies this profile.
#[derive(Clone, Debug, Default)]
pub struct EngineProfile {
    pub triggers: HashMap<String, Trigger>,
    pub command_pointers: HashMap<String, PointerHolder>,
    pub trigger_pointers: HashMap<String, PointerSet>,
    pub lookup_key: Option<LookupResolver>,
}
