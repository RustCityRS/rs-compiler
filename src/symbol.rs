use crate::types::{BaseVarType, Type};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// ── LocalTable ─────────────────────────────────────────────────────────────
// Matches RuneScriptTS `LocalTable` (RuneScript.ts): a flat ordered list of
// all variable declarations within a script, used for slot assignment via
// `getVariableId` (indexOf in type-filtered sublist).

#[derive(Debug, Clone)]
pub struct LocalEntry {
    pub name: String,
    pub var_type: Type,
    pub is_param: bool,
    pub is_array: bool,
}

/// Flat ordered list of all variable declarations within a script.
///
/// `entries` is `pub(crate)` rather than `pub` so the storage shape
/// (currently a `Vec<LocalEntry>`) can change without breaking external
/// consumers. Read access from inside the crate goes through
/// `entries()`; mutations go through `push_param` / `push_local`.
#[derive(Debug, Clone, Default)]
pub struct LocalTable {
    pub(crate) all: Vec<LocalEntry>,
}

impl LocalTable {
    pub fn new() -> Self {
        LocalTable { all: Vec::new() }
    }

    /// Read-only view of the ordered local entries. Use this for iteration.
    pub fn entries(&self) -> &[LocalEntry] {
        &self.all
    }

    /// Release spare capacity once the table is final (post-codegen).
    pub fn shrink_to_fit(&mut self) {
        self.all.shrink_to_fit();
    }

    pub fn push_param(&mut self, name: String, param_type: Type) -> i32 {
        let id = self.get_variable_id_for_next(param_type, false);
        self.all.push(LocalEntry {
            name,
            var_type: param_type,
            is_param: true,
            is_array: false,
        });
        id
    }

    pub fn push_local(&mut self, name: String, var_type: Type, is_array: bool) -> i32 {
        let id = self.get_variable_id_for_next(var_type, is_array);
        self.all.push(LocalEntry {
            name,
            var_type,
            is_param: false,
            is_array,
        });
        id
    }

    /// Matches TS `getVariableId`: indexOf in filtered list.
    fn get_variable_id_for_next(&self, var_type: Type, is_array: bool) -> i32 {
        if is_array {
            self.all.iter().filter(|e| e.is_array).count() as i32
        } else {
            self.all
                .iter()
                .filter(|e| {
                    e.var_type.base_type() == var_type.base_type() && (!e.is_array || e.is_param)
                })
                .count() as i32
        }
    }

    /// Matches TS `getVariableId` for an existing entry by index.
    pub fn get_variable_id(&self, index: usize) -> i32 {
        let entry = &self.all[index];
        if entry.is_array {
            self.all[..index].iter().filter(|e| e.is_array).count() as i32
        } else {
            self.all[..index]
                .iter()
                .filter(|e| {
                    e.var_type.base_type() == entry.var_type.base_type()
                        && (!e.is_array || e.is_param)
                })
                .count() as i32
        }
    }

    /// Matches TS `getLocalCount`.
    pub fn get_local_count(&self, base: BaseVarType) -> u16 {
        self.all
            .iter()
            .filter(|e| e.var_type.base_type() == base && (!e.is_array || e.is_param))
            .count() as u16
    }

    /// Matches TS `getParameterCount`.
    pub fn get_param_count(&self, base: BaseVarType) -> u16 {
        self.all
            .iter()
            .filter(|e| e.is_param && e.var_type.base_type() == base)
            .count() as u16
    }
}

// ── SymbolKind ─────────────────────────────────────────────────────────────

/// The kind of symbol stored in the symbol table.
#[derive(Debug, Clone)]
pub enum SymbolKind {
    /// A local variable within a script.
    LocalVar {
        var_type: Type,
        /// The local variable slot index.
        slot: i32,
        is_array: bool,
    },
    /// A script parameter (like a local var but set from call args).
    ScriptParam { param_type: Type, slot: i32 },
    /// A compiled script (or script reference).
    Script {
        id: i32,
        trigger: String,
        param_types: Vec<Type>,
        return_types: Vec<Type>,
    },
    /// A game variable (varp, varn, vars, varbit).
    GameVar {
        var_type: Type,
        var_id: i32,
        /// Which game var type: "varp", "varn", "vars", "varbit"
        category: String,
    },
    /// A constant value.
    Constant {
        const_type: Type,
        int_value: Option<i32>,
        string_value: Option<String>,
    },
    /// An engine command.
    Command {
        opcode: i32,
        param_types: Vec<Type>,
        return_types: Vec<Type>,
    },
}

impl SymbolKind {
    pub fn kind_name(&self) -> &'static str {
        match self {
            SymbolKind::LocalVar { .. } => "LocalVar",
            SymbolKind::ScriptParam { .. } => "ScriptParam",
            SymbolKind::Script { .. } => "Script",
            SymbolKind::GameVar { .. } => "GameVar",
            SymbolKind::Constant { .. } => "Constant",
            SymbolKind::Command { .. } => "Command",
        }
    }
}

/// A single symbol entry in the table.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
}

/// A resolved entity id (from loc.pack/npc.pack/stat.pack/etc.) and its type.
///
/// Entity IDs hugely outnumber every other symbol (~22k for a full corpus), so
/// they get this 8-byte `Copy` record instead of a 104-byte `Symbol` — entity
/// lookups only ever need the type and the id.
#[derive(Debug, Clone, Copy)]
pub struct EntityRef {
    pub const_type: Type,
    pub id: i32,
}

/// All ids registered under one entity name. Replaces the former
/// `entity_ids: HashMap<String, Symbol>` + `entity_ids_typed:
/// HashMap<String, HashMap<Type, i32>>` pair (same data, ~6 MB smaller on a full
/// corpus: no 104-byte `Symbol`s and no ~22k nested HashMaps).
#[derive(Debug, Clone, Default)]
pub(crate) struct EntityEntry {
    /// Priority winner for untyped lookup (last `register_entity_id` wins);
    /// `None` for typed-only names (registered via `register_entity_id_typed_only`).
    pub(crate) primary: Option<EntityRef>,
    /// Every `(type, id)` variant under this name, for type-aware lookup.
    /// Tiny (usually one entry); includes the primary's `(type, id)`.
    pub(crate) variants: Vec<(Type, i32)>,
}

impl EntityEntry {
    /// Insert or replace the id for `entity_type` (last write wins per type).
    fn set_variant(&mut self, entity_type: Type, id: i32) {
        if let Some(slot) = self.variants.iter_mut().find(|(t, _)| *t == entity_type) {
            slot.1 = id;
        } else {
            self.variants.push((entity_type, id));
        }
    }
}

/// A hierarchical symbol table supporting scoped lookup.
#[derive(Debug, Clone)]
pub struct SymbolTable {
    /// Symbols in this scope.
    symbols: HashMap<String, Symbol>,
    /// Parent scope for hierarchical lookup.
    parent: Option<Box<SymbolTable>>,
    /// Counters for allocating local variable slots.
    next_int_local: i32,
    next_string_local: i32,
    next_long_local: i32,
    /// Total number of local variable declarations (including redeclarations of same name).
    /// The Java compiler counts each def_X statement, even if the slot is reused.
    total_int_decls: i32,
    total_string_decls: i32,
    total_long_decls: i32,
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolTable {
    pub fn new() -> Self {
        SymbolTable {
            symbols: HashMap::new(),
            parent: None,
            next_int_local: 0,
            next_string_local: 0,
            next_long_local: 0,
            total_int_decls: 0,
            total_string_decls: 0,
            total_long_decls: 0,
        }
    }

    /// Create a child scope that inherits from this table.
    pub fn child(parent: SymbolTable) -> Self {
        let int_local = parent.next_int_local;
        let string_local = parent.next_string_local;
        let long_local = parent.next_long_local;
        let int_decls = parent.total_int_decls;
        let string_decls = parent.total_string_decls;
        let long_decls = parent.total_long_decls;
        SymbolTable {
            symbols: HashMap::new(),
            parent: Some(Box::new(parent)),
            next_int_local: int_local,
            next_string_local: string_local,
            next_long_local: long_local,
            total_int_decls: int_decls,
            total_string_decls: string_decls,
            total_long_decls: long_decls,
        }
    }

    /// Create a child scope without consuming the parent (cloning).
    pub fn new_child(&self) -> Self {
        SymbolTable {
            symbols: HashMap::new(),
            parent: Some(Box::new(self.clone())),
            next_int_local: self.next_int_local,
            next_string_local: self.next_string_local,
            next_long_local: self.next_long_local,
            total_int_decls: self.total_int_decls,
            total_string_decls: self.total_string_decls,
            total_long_decls: self.total_long_decls,
        }
    }

    /// Define a symbol in this scope.
    pub fn define(&mut self, name: String, kind: SymbolKind) {
        self.symbols.insert(name.clone(), Symbol { name, kind });
    }

    /// Define a local variable and auto-assign a slot.
    /// `is_array` should be true for array declarations — arrays are excluded from
    /// the local count in the trailer (matching the reference compiler).
    pub fn define_local(&mut self, name: String, var_type: Type, is_array: bool) -> i32 {
        // Count the declaration (Java compiler counts each def_X), but NOT arrays.
        if !is_array {
            match var_type.base_type() {
                crate::types::BaseVarType::Integer => self.total_int_decls += 1,
                crate::types::BaseVarType::String => self.total_string_decls += 1,
                crate::types::BaseVarType::Long => self.total_long_decls += 1,
            }
        }
        // Always advance the slot counter for each declaration, but if the
        // name already exists in this scope, reuse its slot (the new slot
        // becomes a phantom counted in totals but not referenced).
        // This matches the Neptune compiler behavior where redeclaration
        // within the SAME scope reuses the existing slot.
        let new_slot = self.allocate_slot(&var_type);
        let slot = if let Some(existing) = self.symbols.get(&name) {
            match &existing.kind {
                SymbolKind::LocalVar { slot, .. } => *slot,
                _ => new_slot,
            }
        } else {
            new_slot
        };
        self.define(
            name,
            SymbolKind::LocalVar {
                var_type,
                slot,
                is_array,
            },
        );
        slot
    }

    /// Define a script parameter and auto-assign a slot.
    pub fn define_param(&mut self, name: String, param_type: Type) -> i32 {
        match param_type.base_type() {
            crate::types::BaseVarType::Integer => self.total_int_decls += 1,
            crate::types::BaseVarType::String => self.total_string_decls += 1,
            crate::types::BaseVarType::Long => self.total_long_decls += 1,
        }
        let slot = self.allocate_slot(&param_type);
        self.define(name, SymbolKind::ScriptParam { param_type, slot });
        slot
    }

    /// Allocate a slot for a given type.
    fn allocate_slot(&mut self, var_type: &Type) -> i32 {
        match var_type.base_type() {
            crate::types::BaseVarType::Integer => {
                let slot = self.next_int_local;
                self.next_int_local += 1;
                slot
            }
            crate::types::BaseVarType::String => {
                let slot = self.next_string_local;
                self.next_string_local += 1;
                slot
            }
            crate::types::BaseVarType::Long => {
                let slot = self.next_long_local;
                self.next_long_local += 1;
                slot
            }
        }
    }

    /// Look up a symbol by name, searching up through parent scopes.
    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        if let Some(sym) = self.symbols.get(name) {
            return Some(sym);
        }
        if let Some(parent) = &self.parent {
            return parent.lookup(name);
        }
        None
    }

    /// Look up a symbol only in this scope (not parents).
    pub fn lookup_local(&self, name: &str) -> Option<&Symbol> {
        self.symbols.get(name)
    }

    /// Check if a symbol exists in this scope or parent scopes.
    pub fn contains(&self, name: &str) -> bool {
        self.lookup(name).is_some()
    }

    /// Save the current slot counters. Used before entering a block scope.
    pub fn save_slots(&self) -> (i32, i32, i32) {
        (
            self.next_int_local,
            self.next_string_local,
            self.next_long_local,
        )
    }

    /// Restore slot counters to a saved state, but only if higher counters haven't
    /// been set by a sibling scope. This is used between sibling blocks (if/else)
    /// so each branch starts from the same slot baseline.
    pub fn restore_slots(&mut self, saved: (i32, i32, i32)) {
        self.next_int_local = saved.0;
        self.next_string_local = saved.1;
        self.next_long_local = saved.2;
    }

    /// Advance slot counters to be at least as high as the given values.
    /// Used after a block scope to ensure the next sibling doesn't reuse slots.
    pub fn merge_slots(&mut self, other: (i32, i32, i32)) {
        self.next_int_local = self.next_int_local.max(other.0);
        self.next_string_local = self.next_string_local.max(other.1);
        self.next_long_local = self.next_long_local.max(other.2);
    }

    /// Merge total declaration counts from a child scope.
    pub fn merge_decls(&mut self, child: &SymbolTable) {
        self.total_int_decls = child.total_int_decls;
        self.total_string_decls = child.total_string_decls;
        self.total_long_decls = child.total_long_decls;
    }

    /// Get the total int local count (includes redeclarations, matching Java compiler).
    pub fn int_local_count(&self) -> u16 {
        self.total_int_decls as u16
    }

    /// Get the total string local count.
    pub fn string_local_count(&self) -> u16 {
        self.total_string_decls as u16
    }

    /// Get the total long local count.
    pub fn long_local_count(&self) -> u16 {
        self.total_long_decls as u16
    }

    /// Reset local variable counters (for starting a new script).
    pub fn reset_locals(&mut self) {
        self.next_int_local = 0;
        self.next_string_local = 0;
        self.next_long_local = 0;
    }

    /// Iterate over all symbols in this scope.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Symbol)> {
        self.symbols.iter()
    }
}

/// The root symbol table manager that holds all global symbols.
///
/// Field visibility is `pub(crate)` rather than `pub` so external consumers
/// only touch the registry through accessor methods (`lookup_command`,
/// `lookup_entity_id`, `register_script`, etc.). This lets internal storage
/// change — e.g. swapping `HashMap<String, _>` for an interned-string map —
/// without breaking the lib's public surface.
#[derive(Debug, Clone)]
pub struct SymbolRegistry {
    /// All registered scripts by name (last registration wins). The `Symbol` is
    /// shared (`Arc`) with `scripts_by_trigger` so each script's symbol — and its
    /// param/return type vectors — is stored once, not once per map.
    pub(crate) scripts: HashMap<String, Arc<Symbol>>,
    /// All registered commands by name.
    pub(crate) commands: HashMap<String, Symbol>,
    /// All registered game variables by name.
    pub(crate) game_vars: HashMap<String, Symbol>,
    /// All registered constants by name (from .constant files).
    pub(crate) constants: HashMap<String, Symbol>,
    /// Entity IDs from pack files (loc.pack, npc.pack, stat.pack, etc.), keyed by
    /// name. Each entry carries a priority winner for plain-identifier lookup
    /// plus all per-type variants for type-aware lookup (so `smokepuff` resolves
    /// to synth=164 in sound_synth() but spotanim=86 in spotanim_map(), matching
    /// the Java compiler). Constants override only `^name` refs.
    pub(crate) entity_ids: HashMap<String, EntityEntry>,
    /// Parameter types for engine commands, parsed from engine.rs2.
    /// Maps command name → ordered list of parameter types.
    pub(crate) command_param_types: HashMap<String, Vec<Type>>,
    /// Type-char values for RS2 type names (int, string, coord, stat, etc.).
    /// Checked last in identifier resolution so commands take priority over type names.
    pub(crate) type_chars: HashMap<String, i32>,
    /// Script name to ID mapping (by name, last registration wins).
    pub(crate) script_ids: HashMap<String, i32>,
    /// Proc script IDs: name → ID for trigger="proc" scripts.
    pub(crate) proc_script_ids: HashMap<String, i32>,
    /// Label script IDs: name → ID for trigger="label" scripts.
    pub(crate) label_script_ids: HashMap<String, i32>,
    /// Trigger-specific script IDs: "trigger:name" → ID.
    /// Used so queue(foo) resolves to the [queue,foo] script, not [proc,foo].
    pub(crate) trigger_script_ids: HashMap<String, i32>,
    /// Full script symbols keyed by "trigger:name".
    /// Unlike `scripts` (name-only key, last-write-wins), this preserves
    /// all trigger variants so `[proc,fib]` isn't overwritten by `[debugproc,fib]`.
    /// Shares each `Symbol` (`Arc`) with `scripts`.
    pub(crate) scripts_by_trigger: HashMap<String, Arc<Symbol>>,
    /// Pre-assigned script IDs loaded from script.pack: "trigger:name" → ID.
    /// When present, overrides sequential assignment so IDs match the Java compiler.
    pub(crate) preloaded_script_ids: HashMap<String, i32>,
    /// DB column types: "table:column" → first field Type (from dbcolumn.pack).
    /// Used by db_find to determine the implicit BaseVarType argument.
    pub(crate) dbcolumn_types: HashMap<String, Type>,
    /// Component lookup: "interface_name:component_name" → packed (iface_id << 16 | comp_id).
    pub(crate) components: HashMap<String, i32>,
    /// Interface name → ID mapping (for component resolution).
    pub(crate) interface_ids: HashMap<String, i32>,
    /// Scripts defined below `#testscript` — keyed by "trigger:name".
    pub(crate) test_scripts: HashSet<String>,
    /// Next available script ID.
    next_script_id: i32,
}

impl Default for SymbolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolRegistry {
    pub fn new() -> Self {
        SymbolRegistry {
            scripts: HashMap::new(),
            commands: HashMap::new(),
            game_vars: HashMap::new(),
            constants: HashMap::new(),
            entity_ids: HashMap::new(),
            command_param_types: HashMap::new(),
            type_chars: HashMap::new(),
            script_ids: HashMap::new(),
            proc_script_ids: HashMap::new(),
            label_script_ids: HashMap::new(),
            trigger_script_ids: HashMap::new(),
            scripts_by_trigger: HashMap::new(),
            preloaded_script_ids: HashMap::new(),
            dbcolumn_types: HashMap::new(),
            components: HashMap::new(),
            interface_ids: HashMap::new(),
            test_scripts: HashSet::new(),
            next_script_id: 0,
        }
    }

    /// Register a script by name with its type information.
    pub fn register_script(
        &mut self,
        name: String,
        trigger: String,
        param_types: Vec<Type>,
        return_types: Vec<Type>,
    ) -> i32 {
        let key = format!("{}:{}", trigger, name);
        let id = if let Some(&preloaded) = self.preloaded_script_ids.get(&key) {
            preloaded
        } else {
            let id = self.next_script_id;
            self.next_script_id += 1;
            id
        };

        // proc/label ids are keyed by bare name (not `trigger:name`), so they
        // can't be derived from the trigger-keyed map without a per-lookup
        // concat — keep these small dedicated maps.
        if trigger == "proc" {
            self.proc_script_ids.insert(name.clone(), id);
        } else if trigger == "label" {
            self.label_script_ids.insert(name.clone(), id);
        }
        // The id lives in the `Symbol`; `script_id` / `script_id_for_trigger`
        // read it from `scripts` / `scripts_by_trigger` (the `*_script_ids` maps
        // are now only the clientscript-pack fallback), so we don't duplicate it.
        let symbol = Arc::new(Symbol {
            name: name.clone(),
            kind: SymbolKind::Script {
                id,
                trigger,
                param_types,
                return_types,
            },
        });
        self.scripts_by_trigger.insert(key, Arc::clone(&symbol));
        self.scripts.insert(name, symbol);

        id
    }

    /// Register a command (engine function).
    pub fn register_command(
        &mut self,
        name: String,
        opcode: i32,
        param_types: Vec<Type>,
        return_types: Vec<Type>,
    ) {
        self.command_param_types
            .insert(name.clone(), param_types.clone());
        self.commands.insert(
            name.clone(),
            Symbol {
                name,
                kind: SymbolKind::Command {
                    opcode,
                    param_types,
                    return_types,
                },
            },
        );
    }

    /// Register a game variable.
    pub fn register_game_var(
        &mut self,
        name: String,
        var_type: Type,
        var_id: i32,
        category: String,
    ) {
        self.game_vars.insert(
            name.clone(),
            Symbol {
                name,
                kind: SymbolKind::GameVar {
                    var_type,
                    var_id,
                    category,
                },
            },
        );
    }

    /// Register a constant value (from .constant files).
    pub fn register_constant(
        &mut self,
        name: String,
        const_type: Type,
        int_value: Option<i32>,
        string_value: Option<String>,
    ) {
        self.constants.insert(
            name.clone(),
            Symbol {
                name,
                kind: SymbolKind::Constant {
                    const_type,
                    int_value,
                    string_value,
                },
            },
        );
    }

    /// Register an entity ID (from stat.pack, npc.pack, loc.pack, etc.).
    /// These are stored separately from .constant files values so plain
    /// identifier resolution uses entity IDs, while ^name uses .constant files.
    pub fn register_entity_id(&mut self, name: String, entity_type: Type, id: i32) {
        let entry = self.entity_ids.entry(name).or_default();
        // Priority winner for untyped lookup (last write wins).
        entry.primary = Some(EntityRef {
            const_type: entity_type,
            id,
        });
        entry.set_variant(entity_type, id);
    }

    /// Register an entity ID that should only be resolvable via a typed
    /// (hint-driven) lookup. Skips the flat `entity_ids` map so the name
    /// does not shadow trigger names or other higher-priority resolutions
    /// when used in untyped expression contexts.
    ///
    /// Used for NpcMode names like `oploc1`/`apnpc2` that share their
    /// spelling with `ServerTriggerType` byte names — `npc_setmode(oploc1)`
    /// still works (typed lookup hits `Type::NpcMode`), but bare `oploc1`
    /// in e.g. `test_op(oploc1, …)` falls through to `trigger_table::byte`.
    pub fn register_entity_id_typed_only(&mut self, name: String, entity_type: Type, id: i32) {
        self.entity_ids
            .entry(name)
            .or_default()
            .set_variant(entity_type, id);
    }

    /// Look up an entity ID for a specific expected type. Returns `None` if the
    /// name is not registered for that type.
    /// Normalizes the name (lowercase + spaces→underscores) matching TS normalizeName for Basic symbols.
    pub fn lookup_entity_id_typed(&self, name: &str, expected: Type) -> Option<i32> {
        let normalized = name.to_lowercase().replace(' ', "_");
        self.entity_ids
            .get(&normalized)
            .and_then(|e| e.variants.iter().find(|(t, _)| *t == expected))
            .map(|(_, id)| *id)
    }

    /// Look up a script by name (last registration wins if multiple triggers share the name).
    pub fn lookup_script(&self, name: &str) -> Option<&Symbol> {
        self.scripts.get(name).map(|s| s.as_ref())
    }

    /// Look up a script by trigger and name (avoids name collisions between triggers).
    pub fn lookup_script_by_trigger(&self, trigger: &str, name: &str) -> Option<&Symbol> {
        self.scripts_by_trigger
            .get(&format!("{}:{}", trigger, name))
            .map(|s| s.as_ref())
    }

    /// Look up a command by name.
    pub fn lookup_command(&self, name: &str) -> Option<&Symbol> {
        self.commands.get(name)
    }

    /// Look up a game variable by name.
    pub fn lookup_game_var(&self, name: &str) -> Option<&Symbol> {
        self.game_vars.get(name)
    }

    /// Look up a constant by name (from .constant files).
    pub fn lookup_constant(&self, name: &str) -> Option<&Symbol> {
        self.constants.get(name)
    }

    /// Look up an entity ID by name (from stat.pack, npc.pack, etc.).
    /// Normalizes the name matching TS normalizeName for Basic symbols.
    pub fn lookup_entity_id(&self, name: &str) -> Option<EntityRef> {
        let normalized = name.to_lowercase().replace(' ', "_");
        self.entity_ids.get(&normalized).and_then(|e| e.primary)
    }

    /// Free registration-only scratch once every script is registered. The
    /// pre-assigned IDs from `script.pack` are consulted only while assigning
    /// ids in `register_script`; codegen and pointer-checking never read them.
    /// Replacing (not `clear`-ing) the map returns its buckets to the allocator.
    pub(crate) fn drop_registration_scratch(&mut self) {
        self.preloaded_script_ids = HashMap::new();
    }

    /// Get the script ID for a given name. Reads the id from the registered
    /// script's `Symbol`; falls back to `script_ids` (clientscript-pack entries,
    /// which are never registered as RS2). Registered scripts take precedence,
    /// matching the old insert order (clientscripts loaded, then `register_script`
    /// overwrote by name).
    pub fn script_id(&self, name: &str) -> Option<i32> {
        if let Some(s) = self.scripts.get(name)
            && let SymbolKind::Script { id, .. } = &s.kind
        {
            return Some(*id);
        }
        self.script_ids.get(name).copied()
    }

    /// Get the proc script ID for a given name.
    pub fn proc_script_id(&self, name: &str) -> Option<i32> {
        self.proc_script_ids.get(name).copied()
    }

    /// Get the label script ID for a given name.
    pub fn label_script_id(&self, name: &str) -> Option<i32> {
        self.label_script_ids.get(name).copied()
    }

    pub fn script_id_for_trigger(&self, trigger: &str, name: &str) -> Option<i32> {
        let key = format!("{}:{}", trigger, name);
        if let Some(s) = self.scripts_by_trigger.get(&key)
            && let SymbolKind::Script { id, .. } = &s.kind
        {
            return Some(*id);
        }
        // Fallback: clientscript-pack ids (keyed `clientscript:name`), never
        // registered as RS2 scripts.
        self.trigger_script_ids.get(&key).copied()
    }

    pub fn command_opcode(&self, name: &str) -> Option<i32> {
        self.commands.get(name).and_then(|sym| {
            if let SymbolKind::Command { opcode, .. } = &sym.kind {
                Some(*opcode)
            } else {
                None
            }
        })
    }

    pub fn mark_test_script(&mut self, trigger: &str, name: &str) {
        self.test_scripts.insert(format!("{}:{}", trigger, name));
    }

    pub fn is_test_script(&self, trigger: &str, name: &str) -> bool {
        self.test_scripts.contains(&format!("{}:{}", trigger, name))
    }

    /// Look up a component by interface_name:component_name.
    /// Returns the packed (iface_id << 16 | comp_id) value.
    /// Normalizes the component name so 647-style `com2` and 225-style `com_2`
    /// both resolve against whichever form the pack was written in.
    pub fn lookup_component(&self, iface_name: &str, comp_name: &str) -> Option<i32> {
        let key = format!("{}:{}", iface_name, normalize_comp_name(comp_name));
        self.components.get(&key).copied()
    }
}

/// Strip underscores at alpha→digit boundaries to unify 647-style `com2`
/// with 225-style `com_2`. Both normalize to `com2`.
pub(crate) fn normalize_comp_name(name: &str) -> String {
    let bytes = name.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'_'
            && i > 0
            && bytes[i - 1].is_ascii_alphabetic()
            && i + 1 < bytes.len()
            && bytes[i + 1].is_ascii_digit()
        {
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    unsafe { String::from_utf8_unchecked(out) }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── normalize_comp_name ────────────────────────────────────────────

    #[test]
    fn normalize_strips_underscore_before_digits() {
        assert_eq!(normalize_comp_name("com_2"), "com2");
        assert_eq!(normalize_comp_name("com_10"), "com10");
        assert_eq!(normalize_comp_name("com_0"), "com0");
    }

    #[test]
    fn normalize_preserves_already_compact_names() {
        assert_eq!(normalize_comp_name("com2"), "com2");
        assert_eq!(normalize_comp_name("com10"), "com10");
        assert_eq!(normalize_comp_name("com0"), "com0");
    }

    #[test]
    fn normalize_preserves_non_digit_underscores() {
        assert_eq!(normalize_comp_name("close_button"), "close_button");
        assert_eq!(normalize_comp_name("thin_swords"), "thin_swords");
        assert_eq!(normalize_comp_name("title"), "title");
    }

    #[test]
    fn normalize_only_strips_alpha_digit_boundary() {
        // underscore between two digits — not stripped
        assert_eq!(normalize_comp_name("layer_2_3"), "layer2_3");
        // underscore between digit and alpha — not stripped
        assert_eq!(normalize_comp_name("3_panel"), "3_panel");
    }

    // ── lookup_component (647 pack style) ──────────────────────────────

    fn registry_647_style() -> SymbolRegistry {
        let mut r = SymbolRegistry::new();
        r.interface_ids.insert("multi2".into(), 228);
        // 647 pack registers `com0`..`com9` (no underscore)
        for i in 0..10 {
            let key = format!("multi2:com{}", i);
            r.components.insert(key, (228 << 16) | i);
        }
        r
    }

    #[test]
    fn lookup_647_pack_with_647_reference() {
        let r = registry_647_style();
        // Script uses the same 647 naming: `multi2:com2`
        assert_eq!(r.lookup_component("multi2", "com2"), Some((228 << 16) | 2));
        assert_eq!(r.lookup_component("multi2", "com9"), Some((228 << 16) | 9));
    }

    #[test]
    fn lookup_647_pack_with_225_reference() {
        let r = registry_647_style();
        // Script uses 225 naming: `multi2:com_2` — must still resolve
        assert_eq!(r.lookup_component("multi2", "com_2"), Some((228 << 16) | 2));
        assert_eq!(r.lookup_component("multi2", "com_9"), Some((228 << 16) | 9));
        assert_eq!(r.lookup_component("multi2", "com_0"), Some(228 << 16));
    }

    // ── lookup_component (225 pack style) ──────────────────────────────

    fn registry_225_style() -> SymbolRegistry {
        let mut r = SymbolRegistry::new();
        r.interface_ids.insert("multi2".into(), 228);
        // 225 pack registers `multi2:com_0`..`com_9` — but through
        // symloader normalization these become `com0`..`com9`.
        for i in 0..10 {
            let key = format!("multi2:com{}", i);
            r.components.insert(key, 2459 + i);
        }
        r
    }

    #[test]
    fn lookup_225_pack_with_225_reference() {
        let r = registry_225_style();
        assert_eq!(r.lookup_component("multi2", "com_0"), Some(2459));
        assert_eq!(r.lookup_component("multi2", "com_5"), Some(2464));
    }

    #[test]
    fn lookup_225_pack_with_647_reference() {
        let r = registry_225_style();
        assert_eq!(r.lookup_component("multi2", "com0"), Some(2459));
        assert_eq!(r.lookup_component("multi2", "com5"), Some(2464));
    }

    // ── lookup_component (miss) ────────────────────────────────────────

    #[test]
    fn lookup_returns_none_for_unknown_component() {
        let r = registry_647_style();
        assert_eq!(r.lookup_component("multi2", "nonexistent"), None);
        assert_eq!(r.lookup_component("unknown_iface", "com0"), None);
    }
}
