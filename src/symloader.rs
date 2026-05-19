//! Loads pack files and populates the SymbolRegistry.
//!
//! Pack files use 2004scape's `id=name` format (matching CompilerTypeInfo.load).
//! Constants are loaded from `.constant` files in the scripts directory.

use std::fs;
use std::path::Path;

use tracing::{info, warn};

use crate::symbol::{SymbolKind, SymbolRegistry};
use crate::types::Type;

/// Parse the parameter types declared in an `engine.rs2` file and populate
/// `registry.command_param_types`. Lines like:
///   `[command,sound_synth](synth $sound, int $loops, int $delay)`
/// are parsed to extract the ordered list of parameter types per command.
pub fn load_engine_command_params(registry: &mut SymbolRegistry, engine_rs2: &Path) {
    let text = match fs::read_to_string(engine_rs2) {
        Ok(t) => t,
        Err(_) => return,
    };
    for line in text.lines() {
        let line = line.trim();
        // Match lines starting with `[command,<name>]`
        if !line.starts_with("[command,") {
            continue;
        }
        // Extract command name (between the comma and closing bracket)
        let rest = &line["[command,".len()..];
        let name_end = match rest.find(']') {
            Some(i) => i,
            None => continue,
        };
        let name = rest[..name_end].trim().to_string();

        // Extract parameter list between first `(` and matching `)`
        let after_bracket = &rest[name_end + 1..];
        let params_start = match after_bracket.find('(') {
            Some(i) => i + 1,
            None => {
                // No params at all
                registry.command_param_types.insert(name, Vec::new());
                continue;
            }
        };
        let params_end = match after_bracket[params_start..].find(')') {
            Some(i) => params_start + i,
            None => continue,
        };
        let params_str = &after_bracket[params_start..params_end];

        let mut param_types: Vec<Type> = Vec::new();
        for param in params_str.split(',') {
            let param = param.trim();
            if param.is_empty() {
                continue;
            }
            // Each param is like `synth $sound` or `int $loops` or just `int`
            let type_name = param.split_whitespace().next().unwrap_or(param);
            if let Some(t) = Type::from_name(type_name) {
                param_types.push(t);
            } else {
                // Unknown type — push Int as placeholder so arg count stays correct
                param_types.push(Type::Int);
            }
        }
        registry
            .command_param_types
            .insert(name.clone(), param_types.clone());

        if let Some(sym) = registry.commands.get_mut(&name) {
            if let SymbolKind::Command {
                param_types: ref mut pt,
                ..
            } = sym.kind
            {
                *pt = param_types;
            }
        }

        // Parse return types from the second parenthesized group: `(param_types)(return_types)`
        let after_params = &after_bracket[params_end + 1..];
        if let Some(ret_start) = after_params.find('(') {
            if let Some(ret_end) = after_params[ret_start + 1..].find(')') {
                let ret_str = &after_params[ret_start + 1..ret_start + 1 + ret_end];
                let mut return_types: Vec<Type> = Vec::new();
                for ret in ret_str.split(',') {
                    let ret = ret.trim();
                    if ret.is_empty() {
                        continue;
                    }
                    if let Some(t) = Type::from_name(ret) {
                        return_types.push(t);
                    }
                }
                if !return_types.is_empty() {
                    // Update the command's return types in the registry
                    if let Some(sym) = registry.commands.get_mut(&name) {
                        if let SymbolKind::Command {
                            return_types: ref mut rt,
                            ..
                        } = sym.kind
                        {
                            *rt = return_types;
                        }
                    }
                }
            }
        }
    }
}

/// Patch return types for commands whose signatures are not in engine.rs2 but are
/// known to return values in the Neptune compiler. Without this, statement-level
/// calls to these commands will be missing POP_INT_DISCARD / POP_STRING_DISCARD.
pub fn patch_command_return_types(registry: &mut SymbolRegistry) {
    let patches: &[(&str, Vec<Type>)] = &[
        ("db_find_with_count", vec![Type::Int]),
        ("db_find_refine_with_count", vec![Type::Int]),
        ("db_listall_with_count", vec![Type::Int]),
    ];
    for (name, ret_types) in patches {
        if let Some(sym) = registry.commands.get_mut(*name) {
            if let SymbolKind::Command { return_types, .. } = &mut sym.kind {
                if return_types.is_empty() {
                    *return_types = ret_types.clone();
                }
            }
        }
    }
}

/// Load script.pack: `id=[trigger,name]` — pre-assigns script IDs so the
/// Rust compiler uses the same IDs as the Java compiler.
fn load_script_pack(registry: &mut SymbolRegistry, path: &Path) {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return,
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (id_str, rest) = match line.split_once('=') {
            Some(pair) => pair,
            None => continue,
        };
        let id: i32 = match id_str.parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let rest = rest.trim();
        // Expected format: [trigger,name]
        if !rest.starts_with('[') || !rest.ends_with(']') {
            continue;
        }
        let inner = &rest[1..rest.len() - 1];
        let comma = match inner.find(',') {
            Some(i) => i,
            None => continue,
        };
        let trigger = &inner[..comma];
        let name = &inner[comma + 1..];
        if trigger.is_empty() || name.is_empty() {
            continue;
        }
        let key = format!("{}:{}", trigger, name);
        registry.preloaded_script_ids.insert(key, id);
    }
}

/// Generate/update script.pack from .rs2 files, matching 2004scape's regenScriptPack().
/// Scans all [trigger,name] declarations, preserves existing IDs, assigns new ones.
pub fn generate_script_pack(scripts_dir: &Path, pack_dir: &Path) {
    use std::collections::BTreeMap;

    let pack_path = pack_dir.join("script.pack");

    // Load existing entries
    let mut id_to_name: BTreeMap<i32, String> = BTreeMap::new();
    let mut name_to_id: std::collections::HashMap<String, i32> = std::collections::HashMap::new();
    let mut max_id: i32 = 0;

    if let Ok(text) = fs::read_to_string(&pack_path) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some((id_str, name)) = line.split_once('=') {
                if let Ok(id) = id_str.parse::<i32>() {
                    let name = name.trim().to_string();
                    name_to_id.insert(name.clone(), id);
                    id_to_name.insert(id, name);
                    if id >= max_id {
                        max_id = id + 1;
                    }
                }
            }
        }
    }

    // Scan .rs2 files for [trigger,name] declarations
    let mut rs2_files = Vec::new();
    collect_rs2_files(scripts_dir, &mut rs2_files);
    rs2_files.sort();

    for path in &rs2_files {
        // Skip engine.rs2 (command signatures, not compiled scripts)
        if path.file_name().and_then(|n| n.to_str()) == Some("engine.rs2") {
            continue;
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        for line in text.lines() {
            let line = line.split("//").next().unwrap_or("").trim();
            if !line.starts_with('[') {
                continue;
            }
            let bracket_end = match line.find(']') {
                Some(i) => i,
                None => continue,
            };
            let full_name = &line[..bracket_end + 1]; // e.g. "[proc,foo]"
            let inner = &line[1..bracket_end];
            let comma = match inner.find(',') {
                Some(i) => i,
                None => continue,
            };
            let trigger = &inner[..comma];
            // Skip command declarations
            if trigger == "command" {
                continue;
            }

            if !name_to_id.contains_key(full_name) {
                name_to_id.insert(full_name.to_string(), max_id);
                id_to_name.insert(max_id, full_name.to_string());
                max_id += 1;
            }
        }
    }

    // Save
    if let Err(e) = std::fs::create_dir_all(pack_dir) {
        warn!(target: "rs_compiler", "Failed to create pack dir: {}", e);
        return;
    }
    let mut out = String::new();
    for (id, name) in &id_to_name {
        out.push_str(&format!("{}={}\n", id, name));
    }
    if let Err(e) = fs::write(&pack_path, &out) {
        warn!(target: "rs_compiler", "Failed to write script.pack: {}", e);
    } else {
        info!(target: "rs_compiler", "script.pack: {} entries", id_to_name.len());
    }
}

fn collect_rs2_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut sorted: Vec<std::path::PathBuf> = entries.flatten().map(|e| e.path()).collect();
    sorted.sort();
    for path in sorted {
        if path.is_dir() {
            collect_rs2_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs2") {
            out.push(path);
        }
    }
}

/// Load all .pack files from `pack_dir` into `registry`.
///
/// Pack files use 2004scape format: `id=name` (one per line).
/// This matches how 2004scape's Compiler.ts loads CompilerTypeInfo from pack files.
pub fn load_packs(registry: &mut SymbolRegistry, pack_dir: &Path) {
    if !pack_dir.exists() {
        return;
    }

    // Register engine constants FIRST (baseline defaults).
    // Pack files loaded below may overwrite these with content-specific values.
    register_engine_constants(registry);

    load_commands(registry, &pack_dir.join("command.pack"));

    load_game_vars(registry, &pack_dir.join("varp.pack"), "varp");
    load_game_vars(registry, &pack_dir.join("varn.pack"), "varn");
    load_game_vars(registry, &pack_dir.join("vars.pack"), "vars");
    load_game_vars(registry, &pack_dir.join("varbit.pack"), "varbit");

    // Load entity IDs.
    // Load in ascending priority order: later entries overwrite earlier ones
    // when the same name exists in multiple tables.
    //
    // Priority (highest wins for plain identifier resolution):
    //   Low:    UI types (interface, overlayinterface, component, idk),
    //           abstract types (struct, dbrow, dbtable, dbcolumn, param)
    //   Medium: audio/animation types (synth, seq, spotanim, mesanim, fontmetrics)
    //   High:   core game entity types (stat, obj, npc, loc, inv, category, enum, ...)
    //
    // e.g. "prayer" is in interface.pack (id=5608) AND stat.pack (id=5);
    //      stat.pack wins so stat_sub(prayer,...) resolves to 5.
    // e.g. "stafforb" is in struct.pack (id=45) AND obj.pack (id=567);
    //      obj.pack wins so inv_add(..., stafforb, ...) resolves to 567.
    for (file, type_hint) in &[
        // Lowest priority — category overridden by UI types, synth, and all above
        ("category.pack", Type::Category),
        // UI types (override category)
        ("idk.pack", Type::IdKit),
        ("interface.pack", Type::Interface), // Combined: interfaces + components
        ("overlayinterface.pack", Type::OverlayInterface),
        // Abstract / structural types
        ("struct.pack", Type::Struct),
        ("dbrow.pack", Type::DbRow),
        ("dbtable.pack", Type::DbTable),
        ("dbcolumn.pack", Type::DbColumn),
        ("param.pack", Type::Param),
        // Audio / animation / model types (medium priority)
        ("model.pack", Type::Model),
        ("fontmetrics.pack", Type::FontMetrics),
        ("mesanim.pack", Type::MesAnim),
        ("jingle.pack", Type::Jingle),
        ("midi.pack", Type::Midi),
        ("synth.pack", Type::Synth),
        ("spotanim.pack", Type::Spotanim),
        ("seq.pack", Type::Seq),
        // Core game entity types (highest priority — overwrite everything above)
        ("locshape.pack", Type::LocShape),
        ("hunt.pack", Type::Hunt),
        ("npc_mode.pack", Type::NpcMode),
        ("enum.pack", Type::Enum),
        ("writeinv.pack", Type::WriteinvObj),
        ("inv.pack", Type::Inv),
        ("loc.pack", Type::Loc),
        ("npc_stat.pack", Type::NpcStat),
        ("stat.pack", Type::Stat),
        ("npc.pack", Type::Npc),
        ("obj.pack", Type::Obj),
        ("controller.pack", Type::Controller),
    ] {
        load_entity_ids(registry, &pack_dir.join(file), *type_hint);
    }

    // Register type-name identifiers so they resolve to their CS2 type chars.
    register_type_chars(registry);

    // Load pre-assigned script IDs LAST so they are available before script registration.
    load_script_pack(registry, &pack_dir.join("script.pack"));
}

/// Load a game var pack file: `id=name`
/// Type defaults to Int (matching 2004scape — var types come from config definitions).
/// Load command.pack: `opcode=name`
/// Game commands are provided by the engine, not hardcoded in the compiler.
/// This matches the RuneScriptTS architecture where the consuming engine
/// passes its `ScriptOpcodeMap` as the `symbols['command']` CompilerTypeInfo.
fn load_commands(registry: &mut SymbolRegistry, path: &Path) {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return,
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (id_str, name) = match line.split_once('=') {
            Some(pair) => pair,
            None => continue,
        };
        let opcode: i32 = match id_str.parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let name = name.trim().to_string();
        if name.is_empty() {
            continue;
        }
        registry.register_command(name, opcode, Vec::new(), Vec::new());
    }
}

fn load_game_vars(registry: &mut SymbolRegistry, path: &Path, category: &str) {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return,
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (id_str, name) = match line.split_once('=') {
            Some(pair) => pair,
            None => continue,
        };
        let idx: i32 = match id_str.parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let name = name.trim().to_string();
        if name.is_empty() {
            continue;
        }
        // The parser prefixes names with '.' for secondary entity scope
        // (e.g., .%npc_attacking_uid for varn, .%aggressive_npc for varp on
        // another player). Register the dot-prefixed version for all categories.
        registry.register_game_var(format!(".{}", name), Type::Int, idx, category.to_string());
        registry.register_game_var(name, Type::Int, idx, category.to_string());
    }
}

/// Scan `.dbtable` files under `scripts_dir` and register each column as a
/// compound entity ID `table:column` → packed int, mirroring 2004scape's
/// runescript lookup.
///
/// Packed format: `(tableId << 12) | (colIdx << 4) | tupleNibble`
/// where `tupleNibble` is 0 for the whole tuple and `N+1` for slot N.
///
/// Also populates `registry.dbcolumn_types` so `db_find(music:playbutton, ...)`
/// emits the correct BaseVarType ordinal.
///
/// `dbtable_ids` is the pre-loaded name→id mapping from `dbtable.pack` — the
/// same IDs the runtime uses when indexing rows.
pub fn load_dbtable_configs(
    registry: &mut SymbolRegistry,
    scripts_dir: &Path,
    dbtable_ids: &std::collections::HashMap<String, u16>,
) {
    let mut dbtable_files: Vec<std::path::PathBuf> = Vec::new();
    collect_files_by_ext(scripts_dir, "dbtable", &mut dbtable_files);
    dbtable_files.sort();

    for path in &dbtable_files {
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let lines = strip_comments(&text);

        let mut current_table: Option<String> = None;
        let mut col_idx: u16 = 0;
        for line in &lines {
            // Section header: [tablename]
            if line.starts_with('[') && line.ends_with(']') {
                let name = line[1..line.len() - 1].trim().to_string();
                current_table = Some(name);
                col_idx = 0;
                continue;
            }
            // column=name,type1[,type2...][,INDEXED][,REQUIRED]
            let Some(rest) = line.strip_prefix("column=") else {
                continue;
            };
            let Some(table_name) = current_table.as_deref() else {
                continue;
            };
            let Some(&table_id) = dbtable_ids.get(table_name) else {
                continue;
            };

            let parts: Vec<&str> = rest.split(',').map(str::trim).collect();
            if parts.is_empty() {
                continue;
            }
            let col_name = parts[0];
            if col_name.is_empty() {
                continue;
            }

            // Collect types, skipping INDEXED/REQUIRED flags.
            let types: Vec<Type> = parts[1..]
                .iter()
                .filter(|s| !matches!(**s, "INDEXED" | "REQUIRED"))
                .filter_map(|s| Type::from_name(s))
                .collect();

            let compound = format!("{}:{}", table_name, col_name);
            let packed = ((table_id as i32) << 12) | ((col_idx as i32) << 4);

            // Register whole-tuple form
            registry.register_entity_id(compound.clone(), Type::DbColumn, packed);

            // Register per-slot tuple indexes for multi-field columns
            if types.len() > 1 {
                for (i, _) in types.iter().enumerate() {
                    let slot_compound = format!("{}:{}:{}", table_name, col_name, i);
                    let slot_packed = packed | (((i as i32) + 1) & 0xF);
                    registry.register_entity_id(slot_compound, Type::DbColumn, slot_packed);
                }
            }

            // First-field type drives db_find's BaseVarType ordinal.
            if let Some(first_type) = types.first().copied() {
                registry.dbcolumn_types.insert(compound, first_type);
            }

            col_idx += 1;
        }
    }
}

/// Load dbtable.pack as a `name → id` map. Used by `load_dbtable_configs`.
pub fn load_dbtable_pack(path: &Path) -> std::collections::HashMap<String, u16> {
    let mut out = std::collections::HashMap::new();
    let Ok(text) = fs::read_to_string(path) else {
        return out;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((id_str, name)) = line.split_once('=') else {
            continue;
        };
        let Ok(id) = id_str.parse::<u16>() else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        out.insert(name.to_string(), id);
    }
    out
}

/// Load all `.constant` files from the scripts directory, matching 2004scape's
/// `loadDirExtFull(scripts_dir, '.constant', ...)` behavior.
///
/// Format: `^name = value` per line, with `//` and `/* */` comment stripping.
pub fn load_constant_files(registry: &mut SymbolRegistry, scripts_dir: &Path) {
    let mut constant_files: Vec<std::path::PathBuf> = Vec::new();
    collect_files_by_ext(scripts_dir, "constant", &mut constant_files);
    constant_files.sort();

    for path in &constant_files {
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let lines = strip_comments(&text);
        for line in &lines {
            // Split on '=' into name and value
            let eq_pos = match line.find('=') {
                Some(i) => i,
                None => continue,
            };
            let mut name = line[..eq_pos].trim();
            let value = line[eq_pos + 1..].trim();

            // Strip leading ^ from name
            if let Some(stripped) = name.strip_prefix('^') {
                name = stripped;
            }

            if name.is_empty() {
                continue;
            }

            // Strip surrounding quotes from string values
            let val_str = if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
                &value[1..value.len() - 1]
            } else {
                value
            };

            register_constant_value(registry, name.to_string(), val_str);
        }
    }
}

/// Register a constant value, auto-detecting its type (int, hex, coord, or string).
fn register_constant_value(registry: &mut SymbolRegistry, name: String, val_str: &str) {
    if let Ok(i) = val_str.parse::<i32>() {
        registry.register_constant(name, Type::Int, Some(i), None);
    } else if let Some(hex_str) = val_str
        .strip_prefix("0x")
        .or_else(|| val_str.strip_prefix("0X"))
    {
        let v = u32::from_str_radix(hex_str, 16).unwrap_or(0) as i32;
        registry.register_constant(name, Type::Int, Some(v), None);
    } else if let Some(packed) = parse_coord_string(val_str) {
        registry.register_constant(name, Type::Coord, Some(packed), None);
    } else {
        registry.register_constant(name, Type::String, None, Some(val_str.to_string()));
    }
}

/// Strip `//` single-line and `/* */` multi-line comments from source text,
/// matching 2004scape's `loadFileFull()` behavior.
fn strip_comments(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut multi_depth: usize = 0;

    for line in text.lines() {
        let mut line = line.trim().to_string();

        // Handle continuation of multi-line comments
        if multi_depth > 0 {
            while let Some(start) = line.find("/*") {
                line = line[start + 2..].trim_start().to_string();
                multi_depth += 1;
            }
            while let Some(end) = line.find("*/") {
                line = line[end + 2..].trim_start().to_string();
                multi_depth -= 1;
                if multi_depth == 0 {
                    break;
                }
            }
            if multi_depth > 0 {
                continue;
            }
        }

        if line.is_empty() {
            continue;
        }

        // Strip single-line comments
        if let Some(idx) = line.find("//") {
            line = line[..idx].trim_end().to_string();
            if line.is_empty() {
                continue;
            }
        }

        // Strip inline multi-line comments
        if let Some(start) = line.find("/*") {
            if let Some(end) = line.find("*/") {
                line = format!("{}{}", &line[..start], &line[end + 2..]);
            } else {
                line = line[..start].to_string();
                multi_depth += 1;
            }
            if line.is_empty() {
                continue;
            }
        }

        result.push(line);
    }

    result
}

/// Scan `.varp`/`.varn`/`.vars`/`.varbit` configs and refine the type
/// of each matching game-var. `load_game_vars` defaults everything to
/// `Type::Int`; the real type comes from the `type=<typename>` line
/// in the config block.
pub fn load_game_var_types(registry: &mut SymbolRegistry, scripts_dir: &Path) {
    // Track the next auto-assigned ID per category for vars only found in config files.
    let mut next_auto_id: std::collections::HashMap<String, i32> = std::collections::HashMap::new();
    for ext in &["varp", "varn", "vars", "varbit"] {
        // Compute the next available ID (one past the highest existing).
        let max_id = registry
            .game_vars
            .values()
            .filter_map(|sym| {
                if let crate::symbol::SymbolKind::GameVar {
                    var_id, category, ..
                } = &sym.kind
                {
                    if category == *ext {
                        Some(*var_id)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .max()
            .unwrap_or(-1);
        next_auto_id.insert(ext.to_string(), max_id + 1);
    }

    for ext in &["varp", "varn", "vars", "varbit"] {
        let mut files = Vec::new();
        collect_files_by_ext(scripts_dir, ext, &mut files);
        for path in &files {
            let Ok(text) = fs::read_to_string(path) else {
                continue;
            };
            let mut current_name: Option<String> = None;
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with("//") {
                    continue;
                }
                if let Some(inner) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                    let name = inner.trim().to_string();
                    // Register the var if it doesn't exist yet (config-only var)
                    if !registry.game_vars.contains_key(&name) {
                        let id = next_auto_id.get_mut(*ext).unwrap();
                        let auto_id = *id;
                        *id += 1;
                        registry.register_game_var(
                            name.clone(),
                            Type::Int,
                            auto_id,
                            ext.to_string(),
                        );
                        registry.register_game_var(
                            format!(".{}", name),
                            Type::Int,
                            auto_id,
                            ext.to_string(),
                        );
                    }
                    current_name = Some(name);
                    continue;
                }
                let Some(name) = current_name.as_ref() else {
                    continue;
                };
                if let Some((k, v)) = line.split_once('=') {
                    if k.trim() == "type" {
                        let type_name = v.trim();
                        if let Some(ty) = Type::from_name(type_name) {
                            if let Some(sym) = registry.game_vars.get_mut(name) {
                                if let crate::symbol::SymbolKind::GameVar { var_type, .. } =
                                    &mut sym.kind
                                {
                                    *var_type = ty;
                                }
                            }
                            let dot_name = format!(".{}", name);
                            if let Some(sym) = registry.game_vars.get_mut(&dot_name) {
                                if let crate::symbol::SymbolKind::GameVar { var_type, .. } =
                                    &mut sym.kind
                                {
                                    *var_type = ty;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Recursively collect files with a given extension.
fn collect_files_by_ext(dir: &Path, ext: &str, out: &mut Vec<std::path::PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_by_ext(&path, ext, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some(ext) {
            out.push(path);
        }
    }
}

/// Parse a coord string in `z_bx_bz_lx_lz` format and return the packed integer.
/// Packing: x = bx*64 + lx, z = bz*64 + lz, packed = (level << 28) | (x << 14) | z
fn parse_coord_string(s: &str) -> Option<i32> {
    let parts: Vec<&str> = s.split('_').collect();
    if parts.len() != 5 {
        return None;
    }
    let y: i32 = parts[0].parse().ok()?;
    let bx: i32 = parts[1].parse().ok()?;
    let bz: i32 = parts[2].parse().ok()?;
    let lx: i32 = parts[3].parse().ok()?;
    let lz: i32 = parts[4].parse().ok()?;
    let x = bx * 64 + lx;
    let z = bz * 64 + lz;
    Some((y << 28) | (x << 14) | z)
}

/// Load an entity pack file. Two `interface.pack` dialects are accepted:
///
/// 1. **if3** (rev 647 / 2004scape): key encodes the IDs, value is the short name.
///    - `<iface_id>=<iface_name>`            → root interface
///    - `<iface_id>:<comp_id>=<comp_name>`   → component, packed idx = (iface<<16)|comp
///
/// 2. **if1** (rev 225 era): key is a flat global index, value carries the full path.
///    - `<flat_id>=<iface_name>`             → root interface
///    - `<flat_id>=<iface_name>:<comp_name>` → component, idx = flat_id
///
/// Detection is per-line, so a pack written entirely in either dialect parses cleanly,
/// and neither produces a misleading "subject did not resolve" for entries that are
/// actually present.
fn load_entity_ids(registry: &mut SymbolRegistry, path: &Path, entity_type: Type) {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return,
    };

    let is_interface_pack = entity_type == Type::Interface;

    // First pass: collect interface name→ID for component resolution.
    // Both dialects agree on the root form: `<id>=<name>` with no `:` on either side.
    let mut iface_id_to_name: std::collections::HashMap<i32, String> =
        std::collections::HashMap::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (id_str, name) = match line.split_once('=') {
            Some(pair) => pair,
            None => continue,
        };
        let name = name.trim();
        if name.is_empty() || name == "null" {
            continue;
        }

        if is_interface_pack && !id_str.contains(':') && !name.contains(':') {
            if let Ok(id) = id_str.parse::<i32>() {
                registry.interface_ids.insert(name.to_string(), id);
                iface_id_to_name.insert(id, name.to_string());
            }
        }
    }

    // Second pass: register entities + components
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (id_str, name) = match line.split_once('=') {
            Some(pair) => pair,
            None => continue,
        };
        let name = name.trim();
        if name.is_empty() || name == "null" {
            continue;
        }

        // Decide shape: native component (LHS has `:`), Jordan component
        // (RHS has `:` on an interface pack), or plain root.
        let (idx, component_key, register_name): (i32, Option<String>, String) =
            if let Some((iface_str, comp_str)) = id_str.split_once(':') {
                // Native component: `<iface_id>:<comp_id>=<comp_name>`
                let iface: i32 = match iface_str.parse() {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let comp: i32 = match comp_str.parse() {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let packed = (iface << 16) | comp;
                let key = iface_id_to_name
                    .get(&iface)
                    .map(|iface_name| format!("{}:{}", iface_name, name));
                (packed, key, name.to_string())
            } else if is_interface_pack {
                let flat_id: i32 = match id_str.parse() {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if name.contains(':') {
                    // Jordan component: `<flat_id>=<iface_name>:<comp_name>`
                    (flat_id, Some(name.to_string()), name.to_string())
                } else {
                    // Root interface in either dialect
                    (flat_id, None, name.to_string())
                }
            } else {
                // Non-interface pack
                let id: i32 = match id_str.parse() {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                (id, None, name.to_string())
            };

        // In combined interface.pack: component entries register as Type::Component,
        // interface roots register as Type::Interface.
        let actual_type = if is_interface_pack && component_key.is_some() {
            Type::Component
        } else {
            entity_type
        };

        // Normalize name matching TS SymbolTable.normalizeName for Basic symbols:
        // name.toLowerCase().replace(/\s+/g, '_')
        let normalized = register_name.to_lowercase().replace(' ', "_");
        registry.register_entity_id(normalized, actual_type, idx);

        if is_interface_pack {
            if let Some(key) = component_key {
                let normalized_key = if let Some((iface_part, comp_part)) = key.split_once(':') {
                    format!(
                        "{}:{}",
                        iface_part,
                        crate::symbol::normalize_comp_name(comp_part)
                    )
                } else {
                    key
                };
                registry.components.insert(normalized_key, idx);
            }
        }
    }
}

/// Register type-name identifiers as type-char constants.
/// These are stored in a separate map (registry.type_chars) checked LAST in identifier
/// resolution, so commands like `coord` take priority over the type name `coord`.
fn register_type_chars(registry: &mut SymbolRegistry) {
    let type_chars: &[(&str, i32)] = &[
        ("int", b'i' as i32),     // 105
        ("string", b's' as i32),  // 115
        ("boolean", b'1' as i32), // 49
        ("bool", b'1' as i32),
        ("coord", b'c' as i32),     // 99
        ("loc", b'l' as i32),       // 108
        ("npc", b'n' as i32),       // 110
        ("obj", b'o' as i32),       // 111
        ("namedobj", b'O' as i32),  // 79
        ("playeruid", b'p' as i32), // 112
        ("player_uid", b'p' as i32),
        ("npcuid", b'u' as i32), // 117
        ("npc_uid", b'u' as i32),
        ("seq", b'A' as i32),       // 65
        ("spotanim", b't' as i32),  // 116
        ("synth", b'P' as i32),     // 80
        ("stat", b'S' as i32),      // 83
        ("component", b'I' as i32), // 73
        ("interface", b'a' as i32), // 97
        ("toplevelinterface", b'a' as i32),
        ("overlayinterface", b'a' as i32),
        ("inv", b'v' as i32),     // 118
        ("enum", b'g' as i32),    // 103
        ("struct", b'J' as i32),  // 74
        ("dbrow", 0xD0),          // 208
        ("dbtable", b'i' as i32), // treat as int
        ("dbcolumn", b'i' as i32),
        ("category", b'y' as i32), // 121
        ("varp", b'V' as i32),     // 86
        ("npc_stat", 0xFE),        // 254
        ("npcstat", 0xFE),
        ("idkit", b'K' as i32), // 75
        ("idk", b'K' as i32),
    ];
    for (name, val) in type_chars {
        registry.type_chars.insert(name.to_string(), *val);
    }
}

/// Register engine-provided constants that the TS compiler passes via
/// CompilerTypeInfo objects built from engine enums/maps. These are
/// identical across all supported revisions (225-274+).
fn register_engine_constants(registry: &mut SymbolRegistry) {
    // NpcMode (from NpcModeMap in engine/src/engine/entity/NpcMode.ts)
    let npc_modes: &[(&str, i32)] = &[
        ("null", -1),
        ("none", 0),
        ("wander", 1),
        ("patrol", 2),
        ("playerescape", 3),
        ("playerfollow", 4),
        ("playerface", 5),
        ("playerfaceclose", 6),
        ("opplayer1", 7),
        ("opplayer2", 8),
        ("opplayer3", 9),
        ("opplayer4", 10),
        ("opplayer5", 11),
        ("applayer1", 12),
        ("applayer2", 13),
        ("applayer3", 14),
        ("applayer4", 15),
        ("applayer5", 16),
        ("oploc1", 17),
        ("oploc2", 18),
        ("oploc3", 19),
        ("oploc4", 20),
        ("oploc5", 21),
        ("aploc1", 22),
        ("aploc2", 23),
        ("aploc3", 24),
        ("aploc4", 25),
        ("aploc5", 26),
        ("opobj1", 27),
        ("opobj2", 28),
        ("opobj3", 29),
        ("opobj4", 30),
        ("opobj5", 31),
        ("apobj1", 32),
        ("apobj2", 33),
        ("apobj3", 34),
        ("apobj4", 35),
        ("apobj5", 36),
        ("opnpc1", 37),
        ("opnpc2", 38),
        ("opnpc3", 39),
        ("opnpc4", 40),
        ("opnpc5", 41),
        ("apnpc1", 42),
        ("apnpc2", 43),
        ("apnpc3", 44),
        ("apnpc4", 45),
        ("apnpc5", 46),
        ("queue1", 47),
        ("queue2", 48),
        ("queue3", 49),
        ("queue4", 50),
        ("queue5", 51),
        ("queue6", 52),
        ("queue7", 53),
        ("queue8", 54),
        ("queue9", 55),
        ("queue10", 56),
        ("queue11", 57),
        ("queue12", 58),
        ("queue13", 59),
        ("queue14", 60),
        ("queue15", 61),
        ("queue16", 62),
        ("queue17", 63),
        ("queue18", 64),
        ("queue19", 65),
        ("queue20", 66),
    ];
    for &(name, id) in npc_modes {
        registry.register_entity_id(name.to_string(), Type::NpcMode, id);
    }

    // NpcStat FIRST (lower priority — overlapping names will be overwritten by PlayerStat)
    let npc_stats: &[(&str, i32)] = &[
        ("attack", 0),
        ("defence", 1),
        ("strength", 2),
        ("hitpoints", 3),
        ("ranged", 4),
        ("magic", 5),
    ];
    for &(name, id) in npc_stats {
        registry.register_entity_id(name.to_string(), Type::NpcStat, id);
    }

    // PlayerStat SECOND (higher priority — "magic"=6 overwrites npc_stat "magic"=5
    // in the flat entity_ids map, matching load_packs ordering)
    let stats: &[(&str, i32)] = &[
        ("attack", 0),
        ("defence", 1),
        ("strength", 2),
        ("hitpoints", 3),
        ("ranged", 4),
        ("prayer", 5),
        ("magic", 6),
        ("cooking", 7),
        ("woodcutting", 8),
        ("fletching", 9),
        ("fishing", 10),
        ("firemaking", 11),
        ("crafting", 12),
        ("smithing", 13),
        ("mining", 14),
        ("herblore", 15),
        ("agility", 16),
        ("thieving", 17),
        ("slayer", 18),
        ("stat18", 18),
        ("farming", 19),
        ("stat19", 19),
        ("runecraft", 20),
        ("hunter", 21),
        ("construction", 22),
        ("summoning", 23),
        ("dungeoneering", 24),
    ];
    for &(name, id) in stats {
        registry.register_entity_id(name.to_string(), Type::Stat, id);
    }

    // Fontmetrics (both naming conventions: 225 uses short, 274 uses _full suffix)
    let fontmetrics: &[(&str, i32)] = &[
        ("p11", 0),
        ("p12", 1),
        ("b12", 2),
        ("q8", 3),
        ("p11_full", 0),
        ("p12_full", 1),
        ("b12_full", 2),
        ("q8_full", 3),
    ];
    for &(name, id) in fontmetrics {
        registry.register_entity_id(name.to_string(), Type::FontMetrics, id);
    }

    // LocShape (from locshapeInfo in engine/tools/pack/Compiler.ts)
    let locshapes: &[(&str, i32)] = &[
        ("wall_straight", 0),
        ("wall_diagonalcorner", 1),
        ("wall_l", 2),
        ("wall_squarecorner", 3),
        ("walldecor_straight_nooffset", 4),
        ("walldecor_straight_offset", 5),
        ("walldecor_diagonal_offset", 6),
        ("walldecor_diagonal_nooffset", 7),
        ("walldecor_diagonal_both", 8),
        ("wall_diagonal", 9),
        ("centrepiece_straight", 10),
        ("centrepiece_diagonal", 11),
        ("roof_straight", 12),
        ("roof_diagonal_with_roofedge", 13),
        ("roof_diagonal", 14),
        ("roof_l_concave", 15),
        ("roof_l_convex", 16),
        ("roof_flat", 17),
        ("roofedge_straight", 18),
        ("roofedge_diagonalcorner", 19),
        ("roofedge_l", 20),
        ("roofedge_squarecorner", 21),
        ("grounddecor", 22),
    ];
    for &(name, id) in locshapes {
        registry.register_entity_id(name.to_string(), Type::LocShape, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbol::SymbolRegistry;
    use crate::types::Type;
    use std::io::Write;
    use std::path::PathBuf;

    fn write_temp_pack(name: &str, content: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-symloader");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    // ── 647-style pack ─────────────────────────────────────────────────
    // Format: `<iface_id>=<name>` for roots, `<iface_id>:<comp_id>=<comp_name>`

    #[test]
    fn load_647_style_registers_components() {
        let pack = write_temp_pack(
            "interface_647.pack",
            "\
228=multi2\n\
228:0=com0\n\
228:1=com1\n\
228:2=com2\n\
228:3=com3\n\
228:6=thin_swords\n\
228:9=wide_swords\n",
        );
        let mut r = SymbolRegistry::new();
        load_entity_ids(&mut r, &pack, Type::Interface);

        // Root interface registered
        assert_eq!(r.interface_ids.get("multi2"), Some(&228));

        // Components use normalized keys (com0, not com_0)
        assert_eq!(r.lookup_component("multi2", "com0"), Some((228 << 16) | 0));
        assert_eq!(r.lookup_component("multi2", "com2"), Some((228 << 16) | 2));
        assert_eq!(r.lookup_component("multi2", "com3"), Some((228 << 16) | 3));

        // Named components (no digits) are unaffected by normalization
        assert_eq!(
            r.lookup_component("multi2", "thin_swords"),
            Some((228 << 16) | 6)
        );
        assert_eq!(
            r.lookup_component("multi2", "wide_swords"),
            Some((228 << 16) | 9)
        );
    }

    #[test]
    fn load_647_style_resolves_225_references() {
        let pack = write_temp_pack(
            "interface_647_cross.pack",
            "\
228=multi2\n\
228:0=com0\n\
228:1=com1\n\
228:2=com2\n",
        );
        let mut r = SymbolRegistry::new();
        load_entity_ids(&mut r, &pack, Type::Interface);

        // 225-style references (com_0, com_1, com_2) resolve against 647 pack
        assert_eq!(r.lookup_component("multi2", "com_0"), Some((228 << 16) | 0));
        assert_eq!(r.lookup_component("multi2", "com_1"), Some((228 << 16) | 1));
        assert_eq!(r.lookup_component("multi2", "com_2"), Some((228 << 16) | 2));
    }

    // ── 225-style pack ─────────────────────────────────────────────────
    // Format: `<flat_id>=<iface_name>` for roots, `<flat_id>=<iface_name>:<comp_name>`

    #[test]
    fn load_225_style_registers_components() {
        let pack = write_temp_pack(
            "interface_225.pack",
            "\
2459=multi2\n\
2460=multi2:com_0\n\
2461=multi2:com_1\n\
2462=multi2:com_2\n\
2463=multi2:com_4\n\
2464=multi2:com_5\n",
        );
        let mut r = SymbolRegistry::new();
        load_entity_ids(&mut r, &pack, Type::Interface);

        // Root interface registered
        assert_eq!(r.interface_ids.get("multi2"), Some(&2459));

        // 225 references resolve (com_0 normalized to com0 at registration)
        assert_eq!(r.lookup_component("multi2", "com_0"), Some(2460));
        assert_eq!(r.lookup_component("multi2", "com_2"), Some(2462));
        assert_eq!(r.lookup_component("multi2", "com_5"), Some(2464));
    }

    #[test]
    fn load_225_style_resolves_647_references() {
        let pack = write_temp_pack(
            "interface_225_cross.pack",
            "\
2459=multi2\n\
2460=multi2:com_0\n\
2461=multi2:com_1\n\
2462=multi2:com_2\n",
        );
        let mut r = SymbolRegistry::new();
        load_entity_ids(&mut r, &pack, Type::Interface);

        // 647-style references (com0, com1, com2) resolve against 225 pack
        assert_eq!(r.lookup_component("multi2", "com0"), Some(2460));
        assert_eq!(r.lookup_component("multi2", "com1"), Some(2461));
        assert_eq!(r.lookup_component("multi2", "com2"), Some(2462));
    }

    // ── Entity type correctness ────────────────────────────────────────

    #[test]
    fn components_register_as_component_type() {
        let pack = write_temp_pack(
            "interface_types.pack",
            "\
228=multi2\n\
228:0=com0\n",
        );
        let mut r = SymbolRegistry::new();
        load_entity_ids(&mut r, &pack, Type::Interface);

        // Root → Interface type, component → Component type
        assert!(
            r.lookup_entity_id_typed("multi2", Type::Interface)
                .is_some()
        );
        assert!(r.lookup_entity_id_typed("com0", Type::Component).is_some());
        assert!(r.lookup_entity_id_typed("com0", Type::Interface).is_none());
    }
}
