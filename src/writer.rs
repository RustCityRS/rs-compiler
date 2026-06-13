use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;

use crate::bytecode::{CompiledScript, Opcode, Operand};
use crate::types::Type;

// ── CS2 Neptune binary format constants ────────────────────────────────────

/// Compiler version written into the script.dat header.
const COMPILER_VERSION: i32 = 27;

/// Returns true when the operand for `opcode` is 4 bytes (large).
/// Returns false when the operand is 1 byte (small).
fn is_large_operand(opcode: u16) -> bool {
    if opcode > 100 {
        return false;
    }
    match opcode {
        21 | 22 | 23 | 38 | 39 => false, // RETURN, GOSUB, JUMP, POP_INT_DISCARD, POP_STRING_DISCARD
        _ => true,
    }
}

/// Map a `Type` to its CS2 type-char byte (as used in ScriptVarType.ts).
fn type_to_char(t: Type) -> u8 {
    match t {
        Type::Int => b'i',
        Type::String => b's',
        Type::Boolean => b'1',
        Type::Coord => b'c',
        Type::Loc => b'l',
        Type::Npc => b'n',
        Type::Obj => b'o',
        Type::NamedObj => b'O',
        Type::PlayerUid => b'p',
        Type::NpcUid => b'u',
        Type::Seq => b'A',
        Type::Spotanim => b't',
        Type::Synth => b'P',
        Type::Stat => b'S',
        Type::Component => b'I',
        Type::Interface | Type::TopLevelInterface | Type::OverlayInterface => b'a',
        Type::Inv => b'v',
        Type::Enum => b'g',
        Type::Struct => b'J',
        Type::DbRow => 0xD0, // 'Ð' = 208
        Type::Category => b'y',
        Type::Varp => b'V',
        Type::NpcStat => 0xFE, // 'þ' = 254
        Type::IdKit => b'K',
        _ => b'i', // default to int for unknown/unrepresented types
    }
}

/// Encode a Rust string to RS2 single-byte encoding.
/// Maps Unicode characters to their RuneScript byte equivalents.
/// The RuneScript format uses the low byte of certain Unicode code points:
/// U+2019 (') → 0x19, U+2013 (–) → 0x13, etc.
fn encode_rs2_string(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for ch in s.chars() {
        let cp = ch as u32;
        if cp < 0x80 {
            out.push(cp as u8);
        } else if cp < 0x100 {
            // Latin-1 supplement: pass through as-is
            out.push(cp as u8);
        } else if (0x2010..=0x201F).contains(&cp) {
            // Typographic punctuation: use low byte
            // U+2013 (–) → 0x13, U+2014 (—) → 0x14
            // U+2018 (') → 0x18, U+2019 (') → 0x19
            // U+201C (") → 0x1C, U+201D (") → 0x1D
            out.push((cp & 0xFF) as u8);
        } else {
            // Other Unicode: use low byte as fallback
            out.push((cp & 0xFF) as u8);
        }
    }
    out
}

// ── Script binary encoder ───────────────────────────────────────────────────

/// Encode a single compiled script to the CS2 Neptune binary format.
///
/// Layout:
///   [header]  scriptName\0 + sourceFilePath\0 + lookupKey(i32) +
///             paramTypeCount(u8) + paramTypes(u8[]) +
///             lineTableLen(u16) + lineTable(pc+line pairs)
///   [instrs]  per-instruction: opcode(u16) + operand (variable)
///   [trailer] _instructions(i32) + intLocals(u16) + strLocals(u16) +
///             intArgs(u16) + strArgs(u16) + switches(u8) + tables...
///   [last2]   trailerLen(u16)
pub fn encode_script(script: &CompiledScript) -> Vec<u8> {
    // ── collect switch tables and assign indices ────────────────────────────
    // We scan instructions for SwitchTable operands, extract them, and
    // replace their operands with the table index.
    let mut switch_tables: Vec<Vec<(i32, i32)>> = Vec::new(); // (key, relative_offset)
    // We'll build the final instruction list with resolved operands.
    // Each entry: (effective_opcode: u16, resolved_operand)
    enum ResolvedOp {
        None,
        Small(u8),
        Large(i32),
        Str(String),
    }
    struct FlatInstr {
        opcode: u16,
        op: ResolvedOp,
    }

    // First pass: build flat instruction list, collect switch tables.
    // Jump targets are stored as absolute indices in Operand::JumpTarget;
    // we convert them to relative offsets later.
    let n = script.instructions.len();
    let mut flat: Vec<FlatInstr> = Vec::with_capacity(n);
    let mut switch_tbl_idxs: Vec<Option<usize>> = Vec::new(); // per-instruction switch table idx
    // Line number table: (flat_idx_of_sentinel, line_number)
    // After orig_to_final is built, flat_idx resolves to the PC of the next real instruction.
    let mut raw_line_entries: Vec<(usize, i32)> = Vec::new();

    for instr in &script.instructions {
        // For Command instructions, the "real" opcode is stored in the operand;
        // we extract it here and will write 0 as the operand value.
        let is_command = instr.opcode == Opcode::Command;

        // For Command instructions, the actual engine opcode is stored in the
        // lower 16 bits of the Int operand, and the small operand byte (0 or 1
        // for primary/secondary active-entity selection) is stored in bits 16-23.
        let mut cmd_small_operand: u8 = 0;
        let eff_opcode: u16 = match instr.opcode {
            Opcode::Command => {
                match &instr.operand {
                    Operand::Int(v) => {
                        cmd_small_operand = ((*v >> 16) & 0xFF) as u8;
                        (*v & 0xFFFF) as u16
                    }
                    Operand::Str(s) => {
                        // Unknown command referenced by name – skip with 0
                        let _ = s;
                        0
                    }
                    _ => 0,
                }
            }
            Opcode::LineNumber => {
                // Metadata – record line number and skip (add sentinel to filter).
                let line = match &instr.operand {
                    Operand::Int(v) => *v,
                    _ => 0,
                };
                raw_line_entries.push((flat.len(), line));
                switch_tbl_idxs.push(None);
                flat.push(FlatInstr {
                    opcode: 0xFFFF,
                    op: ResolvedOp::None,
                }); // sentinel to remove
                continue;
            }
            op => op as u16,
        };

        // Command instructions: the operand byte encodes which active entity
        // to use (0 = primary, 1 = secondary/dot-prefix form).
        if is_command {
            switch_tbl_idxs.push(None);
            flat.push(FlatInstr {
                opcode: eff_opcode,
                op: ResolvedOp::Small(cmd_small_operand),
            });
            continue;
        }

        let resolved = match &instr.operand {
            Operand::None => {
                // No-operand instructions still need a byte written (0) if small.
                if is_large_operand(eff_opcode) {
                    ResolvedOp::Large(0)
                } else {
                    ResolvedOp::Small(0)
                }
            }
            Operand::Int(v) => {
                if eff_opcode == 3 {
                    // PUSH_CONSTANT_STRING uses string operand; Int is a fallback
                    ResolvedOp::Large(*v)
                } else if is_large_operand(eff_opcode) {
                    ResolvedOp::Large(*v)
                } else {
                    ResolvedOp::Small(*v as u8)
                }
            }
            Operand::Long(v) => {
                // Long literal: 4-byte high + 4-byte low encoded as two push instructions.
                // For now, emit high 32-bits (this is rarely used in 2004scape).
                ResolvedOp::Large(*v as i32)
            }
            Operand::Str(s) => ResolvedOp::Str(s.clone()),
            Operand::JumpTarget(target) => {
                // Will be resolved to relative offset in second pass.
                ResolvedOp::Large(*target as i32) // placeholder – absolute index
            }
            Operand::SwitchTable(table) => {
                // Assign a table index and store the table.
                let tbl_idx = switch_tables.len();
                // We'll fill in relative offsets in the second pass.
                // For now, store the table with absolute targets.
                let raw: Vec<(i32, i32)> = table
                    .iter()
                    .map(|(k, abs_target)| (*k, *abs_target as i32))
                    .collect();
                switch_tables.push(raw);
                switch_tbl_idxs.push(Some(tbl_idx));
                flat.push(FlatInstr {
                    opcode: eff_opcode,
                    op: ResolvedOp::Large(tbl_idx as i32),
                });
                continue;
            }
            Operand::StringCount(c) => {
                if is_large_operand(eff_opcode) {
                    ResolvedOp::Large(*c as i32)
                } else {
                    ResolvedOp::Small(*c as u8)
                }
            }
            Operand::ArrayDef(var_id, type_char) => {
                let packed = (*var_id << 8) | (*type_char as i32);
                if is_large_operand(eff_opcode) {
                    ResolvedOp::Large(packed)
                } else {
                    ResolvedOp::Small(packed as u8)
                }
            }
        };

        switch_tbl_idxs.push(None);
        flat.push(FlatInstr {
            opcode: eff_opcode,
            op: resolved,
        });
    }

    // Remove LineNumber sentinel instructions and keep track of index mapping.
    // Build final instruction vector (without LineNumber instructions).
    let mut final_instrs: Vec<FlatInstr> = Vec::new();
    // Map from original index to final index (for resolving jump targets).
    let mut orig_to_final: Vec<usize> = Vec::new(); // orig_to_final[orig] = final
    for (i, fi) in flat.into_iter().enumerate() {
        let _ = i;
        if fi.opcode == 0xFFFF {
            // LineNumber sentinel
            orig_to_final.push(final_instrs.len()); // point to next valid instr
        } else {
            orig_to_final.push(final_instrs.len());
            final_instrs.push(fi);
        }
    }
    // Add sentinel for "one past end"
    let total = final_instrs.len();

    // Resolve raw_line_entries to (pc, line) pairs using orig_to_final.
    // Deduplicate: if multiple LineNumber entries map to the same PC, keep the last one.
    let mut line_table: Vec<(i32, i32)> = Vec::new();
    for (flat_idx, line) in &raw_line_entries {
        let pc = if *flat_idx < orig_to_final.len() {
            orig_to_final[*flat_idx] as i32
        } else {
            total as i32
        };
        // Deduplicate: skip if same PC (keep last line for that PC) or same line as previous.
        if let Some(last) = line_table.last_mut() {
            if last.0 == pc {
                last.1 = *line;
                continue;
            }
            if last.1 == *line {
                // Same line number as previous entry — Java compiler skips this.
                continue;
            }
        }
        line_table.push((pc, *line));
    }

    // Second pass: convert absolute jump targets to relative offsets.
    // Relative offset = target_final_idx - (current_final_idx + 1).
    for (i, fi) in final_instrs.iter_mut().enumerate() {
        if let ResolvedOp::Large(v) = &fi.op {
            // Check if this is a branch opcode (jump target stored as absolute idx)
            let is_branch = matches!(
                fi.opcode,
                6 | 7 | 8 | 9 | 10 | 31 | 32 | 68 | 69 | 70 | 71 | 72 | 73 | 86 | 87
            );
            if is_branch {
                let abs_target = *v as usize;
                let final_target = if abs_target < orig_to_final.len() {
                    orig_to_final[abs_target]
                } else {
                    total // out of bounds → point to end
                };
                let relative = final_target as i32 - (i as i32 + 1);
                fi.op = ResolvedOp::Large(relative);
            }
        }
    }

    // Convert switch table targets to relative offsets.
    // The switch instruction is at some index `sw_idx`; each case body has an
    // absolute target. offset = target - (sw_idx + 1).
    // We need to find where each switch instruction is in the final list to
    // compute relative offsets.
    // Re-scan original instructions to find switch instruction positions.
    let mut sw_instr_positions: HashMap<usize, usize> = HashMap::new(); // tbl_idx -> final_pos
    {
        for (orig_idx, orig_instr) in script.instructions.iter().enumerate() {
            if orig_idx >= orig_to_final.len() {
                break;
            }
            if orig_instr.opcode == Opcode::LineNumber {
                continue;
            }
            let final_pos = orig_to_final[orig_idx];
            if let Operand::SwitchTable(_) = &orig_instr.operand {
                // Use orig_idx to correctly index switch_tbl_idxs (which is populated
                // in parallel with script.instructions, including LineNumber entries).
                if let Some(tbl_idx) = switch_tbl_idxs.get(orig_idx).and_then(|x| *x) {
                    sw_instr_positions.insert(tbl_idx, final_pos);
                }
            }
        }
    }

    // Build relative-offset switch tables.
    let switch_tables_relative: Vec<Vec<(i32, i32)>> = switch_tables
        .iter()
        .enumerate()
        .map(|(tbl_idx, table)| {
            let sw_pos = *sw_instr_positions.get(&tbl_idx).unwrap_or(&0);
            table
                .iter()
                .map(|(key, abs_target)| {
                    let final_target = if (*abs_target as usize) < orig_to_final.len() {
                        orig_to_final[*abs_target as usize]
                    } else {
                        total
                    };
                    let rel = final_target as i32 - (sw_pos as i32 + 1);
                    (*key, rel)
                })
                .collect()
        })
        .collect();

    // ── build the binary buffer ────────────────────────────────────────────
    let mut buf: Vec<u8> = Vec::new();

    // Header: scriptName\0
    buf.extend_from_slice(script.name.as_bytes());
    buf.push(0);

    // sourceFilePath\0
    buf.extend_from_slice(script.source_path.as_bytes());
    buf.push(0);

    // lookupKey: -1 for non-entity scripts, or hash-based key for entity scripts
    buf.extend_from_slice(&script.lookup_key.to_be_bytes());

    // parameterTypeCount + parameterTypes
    // The Java compiler only writes param types for debugproc scripts;
    // all other triggers write 0 here.
    if script.trigger == "debugproc" {
        buf.push(script.param_types.len() as u8);
        for &pt in &script.param_types {
            buf.push(type_to_char(pt));
        }
    } else {
        buf.push(0);
    }

    // lineNumberTableLength = count of (pc, line) entries
    buf.extend_from_slice(&(line_table.len() as u16).to_be_bytes());
    for (pc, line) in &line_table {
        buf.extend_from_slice(&pc.to_be_bytes());
        buf.extend_from_slice(&line.to_be_bytes());
    }

    // ── instructions ──────────────────────────────────────────────────────
    for fi in &final_instrs {
        buf.extend_from_slice(&fi.opcode.to_be_bytes());
        match &fi.op {
            ResolvedOp::None => {}
            ResolvedOp::Small(v) => {
                buf.push(*v);
            }
            ResolvedOp::Large(v) => {
                buf.extend_from_slice(&v.to_be_bytes());
            }
            ResolvedOp::Str(s) => {
                buf.extend(encode_rs2_string(s));
                buf.push(0); // null terminator
            }
        }
    }

    // ── trailer ───────────────────────────────────────────────────────────
    let trailer_start = buf.len();

    // _instructions (instruction count)
    buf.extend_from_slice(&(final_instrs.len() as i32).to_be_bytes());

    // intLocalCount, stringLocalCount
    buf.extend_from_slice(&script.int_local_count.to_be_bytes());
    buf.extend_from_slice(&script.string_local_count.to_be_bytes());

    // intArgCount, stringArgCount
    buf.extend_from_slice(&script.int_arg_count.to_be_bytes());
    buf.extend_from_slice(&script.string_arg_count.to_be_bytes());

    // switches byte + switch tables
    buf.push(switch_tables_relative.len() as u8);
    for table in &switch_tables_relative {
        buf.extend_from_slice(&(table.len() as u16).to_be_bytes());
        for (key, offset) in table {
            buf.extend_from_slice(&key.to_be_bytes());
            buf.extend_from_slice(&offset.to_be_bytes());
        }
    }

    // trailerLen = switch count byte + switch table data (matching TS: switchOffset + 1)
    let switch_section_len = buf.len() - trailer_start - 12; // 12 = instrs(4) + locals/params(8)
    buf.extend_from_slice(&(switch_section_len as u16).to_be_bytes());

    buf
}

// ── PackedWriter: writes script.dat + script.idx ───────────────────────────

/// Writes compiled scripts to script.dat and script.idx in the output directory.
pub struct ScriptWriter {
    output_dir: String,
}

impl ScriptWriter {
    pub fn new(output_dir: String) -> Self {
        ScriptWriter { output_dir }
    }

    /// Write all compiled scripts to script.dat and script.idx.
    ///
    /// File layout is **sparse by script.id**: the idx has one entry per
    /// slot from 0..max_id; slots without a compiled script write length
    /// 0 and contribute no bytes to the dat. The loader (`rs-engine`)
    /// mirrors this — empty-length slots become no-op Script stubs so
    /// the script at array-index N matches compile-time id N.
    ///
    /// Packing contiguously (old behaviour) misaligned runtime array
    /// indices with compile-time ids, so bytecode `GOSUB 2405` (intended
    /// `chatnpc_page`) resolved to `chatnpc` at runtime — infinite
    /// recursion.
    pub fn write_all(&self, scripts: &[CompiledScript]) -> io::Result<()> {
        fs::create_dir_all(&self.output_dir)?;

        // Max id determines the sparse slot count. Scripts with id < 0
        // are stray (unresolved) and would collide at slot 0 — drop them
        // explicitly rather than let them mask a valid id.
        let max_id = scripts.iter().map(|s| s.id).max().unwrap_or(-1);
        let slot_count = if max_id < 0 { 0 } else { (max_id + 1) as usize };

        // Index scripts by id. Later writes at the same id overwrite
        // earlier ones (matches Java compiler's last-wins behaviour when
        // two declarations share an id — typically a compile-time bug,
        // but we preserve the shape).
        let mut by_id: Vec<Option<&CompiledScript>> = vec![None; slot_count];
        for s in scripts {
            if s.id >= 0 && (s.id as usize) < slot_count {
                by_id[s.id as usize] = Some(s);
            }
        }

        // Encode each occupied slot once. Empty slots emit zero bytes.
        // Encoding is independent per slot, so fan it out across threads.
        let encoded: Vec<Vec<u8>> =
            crate::parallel_map(&by_id, |opt| opt.map(encode_script).unwrap_or_default());

        // Build script.dat
        let dat_path = Path::new(&self.output_dir).join("script.dat");
        {
            let mut dat: Vec<u8> = Vec::new();
            // Header: count (u32) + version (i32)
            dat.extend_from_slice(&(slot_count as u32).to_be_bytes());
            dat.extend_from_slice(&COMPILER_VERSION.to_be_bytes());
            for bytes in &encoded {
                dat.extend_from_slice(bytes);
            }
            fs::write(&dat_path, dat)?;
        }

        // Build script.idx
        let idx_path = Path::new(&self.output_dir).join("script.idx");
        {
            let mut idx: Vec<u8> = Vec::new();
            idx.extend_from_slice(&(slot_count as u32).to_be_bytes());
            for bytes in &encoded {
                idx.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            }
            fs::write(&idx_path, idx)?;
        }

        Ok(())
    }

    pub fn build_all(&self, scripts: &[CompiledScript]) -> io::Result<(Vec<u8>, Vec<u8>)> {
        // Max id determines the sparse slot count. Scripts with id < 0
        // are stray (unresolved) and would collide at slot 0 — drop them
        // explicitly rather than let them mask a valid id.
        let max_id = scripts.iter().map(|s| s.id).max().unwrap_or(-1);
        let slot_count = if max_id < 0 { 0 } else { (max_id + 1) as usize };

        // Index scripts by id. Later writes at the same id overwrite
        // earlier ones (matches Java compiler's last-wins behaviour when
        // two declarations share an id — typically a compile-time bug,
        // but we preserve the shape).
        let mut by_id: Vec<Option<&CompiledScript>> = vec![None; slot_count];
        for s in scripts {
            if s.id >= 0 && (s.id as usize) < slot_count {
                by_id[s.id as usize] = Some(s);
            }
        }

        // Encode each occupied slot once. Empty slots emit zero bytes.
        // Encoding is independent per slot, so fan it out across threads.
        let encoded: Vec<Vec<u8>> =
            crate::parallel_map(&by_id, |opt| opt.map(encode_script).unwrap_or_default());

        let mut dat: Vec<u8> = Vec::new();
        dat.extend_from_slice(&(slot_count as u32).to_be_bytes());
        dat.extend_from_slice(&COMPILER_VERSION.to_be_bytes());
        for bytes in &encoded {
            dat.extend_from_slice(bytes);
        }

        let mut idx: Vec<u8> = Vec::new();
        idx.extend_from_slice(&(slot_count as u32).to_be_bytes());
        for bytes in &encoded {
            idx.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        }

        // // Build script.dat
        // let dat_path = Path::new(&self.output_dir).join("script.dat");
        // {
        //     let mut dat: Vec<u8> = Vec::new();
        //     // Header: count (u32) + version (i32)
        //     dat.extend_from_slice(&(slot_count as u32).to_be_bytes());
        //     dat.extend_from_slice(&COMPILER_VERSION.to_be_bytes());
        //     for bytes in &encoded {
        //         dat.extend_from_slice(bytes);
        //     }
        //     fs::write(&dat_path, dat)?;
        // }
        //
        // // Build script.idx
        // let idx_path = Path::new(&self.output_dir).join("script.idx");
        // {
        //     let mut idx: Vec<u8> = Vec::new();
        //     idx.extend_from_slice(&(slot_count as u32).to_be_bytes());
        //     for bytes in &encoded {
        //         idx.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        //     }
        //     fs::write(&idx_path, idx)?;
        // }

        Ok((dat, idx))
    }
}

// ── PackBuffer (in-memory, for testing) ────────────────────────────────────

pub struct PackBuffer {
    pub scripts: HashMap<String, Vec<u8>>,
}

impl Default for PackBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl PackBuffer {
    pub fn new() -> Self {
        PackBuffer {
            scripts: HashMap::new(),
        }
    }

    pub fn add_script(&mut self, script: &CompiledScript) {
        let bytes = encode_script(script);
        self.scripts.insert(script.name.clone(), bytes);
    }
}
