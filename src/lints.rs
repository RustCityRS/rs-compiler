//! Static lint passes complementary to the pointer checker.
//!
//! Each lint:
//!   - consumes the fully-compiled `CompiledScript` objects (read-only),
//!   - emits `Diagnostic`s at `Severity::Warning`,
//!   - never feeds back into codegen or the script writer.
//!
//! Currently included:
//!   1. `check_unused_locals` — any local variable (including parameters)
//!      that is declared/written but never read.
//!   2. `check_unreachable_code` — any source statement whose emitted
//!      instructions are unreachable from node 0 of the CFG.
//!
//! Both lints attach help/suggestion blocks where the fix is mechanical
//! enough to print a before/after diff; otherwise they fall back to a
//! prose message.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use crate::bytecode::{CompiledScript, Instruction, Opcode, Operand};
use crate::diagnostics::{
    Applicability, Diagnostic, DiagnosticsCollector, Help, Phase, Severity, Suggestion,
};
use crate::symbol::LocalEntry;
use crate::types::BaseVarType;

pub fn run_lints(
    scripts: &[CompiledScript],
    source_cache: Option<&HashMap<String, Arc<String>>>,
) -> DiagnosticsCollector {
    let mut diags = DiagnosticsCollector::new();
    for script in scripts {
        check_unused_locals(script, source_cache, &mut diags);
        check_unreachable_code(script, source_cache, &mut diags);
    }
    diags
}

// ---------------------------------------------------------------------------
// Unused locals
// ---------------------------------------------------------------------------

/// Walk a script's instruction stream, collect per-type read sets for all
/// local opcodes, then cross-reference against `local_table` to find
/// locals that are written or declared but never read. Each unused local
/// gets a warning at its declaration/first-write line.
fn check_unused_locals(
    script: &CompiledScript,
    source_cache: Option<&HashMap<String, Arc<String>>>,
    diagnostics: &mut DiagnosticsCollector,
) {
    // Per-type slot id -> read? (PushXxxLocal / PushArrayInt).
    let mut int_reads: HashSet<i32> = HashSet::new();
    let mut string_reads: HashSet<i32> = HashSet::new();
    let mut long_reads: HashSet<i32> = HashSet::new();
    // Arrays share one id space; both reads AND writes count as "used".
    let mut array_touched: HashSet<i32> = HashSet::new();

    // Per-type slot id -> source line of the first write (for reporting).
    let mut int_first_write_line: HashMap<i32, usize> = HashMap::new();
    let mut string_first_write_line: HashMap<i32, usize> = HashMap::new();
    let mut long_first_write_line: HashMap<i32, usize> = HashMap::new();
    let mut array_first_def_line: HashMap<i32, usize> = HashMap::new();

    for (i, instr) in script.instructions.iter().enumerate() {
        let id = match &instr.operand {
            Operand::Int(v) => *v,
            _ => continue,
        };
        let line = resolve_line(&script.instructions, i);
        match instr.opcode {
            Opcode::PushIntLocal => {
                int_reads.insert(id);
            }
            Opcode::PopIntLocal => {
                int_first_write_line.entry(id).or_insert(line);
            }
            Opcode::PushStringLocal => {
                string_reads.insert(id);
            }
            Opcode::PopStringLocal => {
                string_first_write_line.entry(id).or_insert(line);
            }
            Opcode::PushLongLocal => {
                long_reads.insert(id);
            }
            Opcode::PopLongLocal => {
                long_first_write_line.entry(id).or_insert(line);
            }
            Opcode::PushArrayInt | Opcode::PopArrayInt => {
                array_touched.insert(id);
            }
            Opcode::DefineArray => {
                array_first_def_line.entry(id).or_insert(line);
            }
            _ => {}
        }
    }

    // For parameters, locate the actual `[trigger,name]` header line in
    // source so the warning points at the declaration rather than the
    // first body statement. Falls back to the first LineNumber (body
    // opener) if the header can't be located — still unambiguous.
    let short_name = script
        .name
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split_once(',')
        .map(|(_, n)| n.to_string())
        .unwrap_or_else(|| script.name.clone());
    let first_body_line = script
        .instructions
        .iter()
        .find_map(|i| match (i.opcode, &i.operand) {
            (Opcode::LineNumber, Operand::Int(n)) => Some(*n as usize),
            _ => None,
        })
        .unwrap_or(1);
    let header_line = source_cache
        .and_then(|c| c.get(&script.source_path))
        .and_then(|src| find_header_line(src, &script.trigger, &short_name))
        .unwrap_or_else(|| first_body_line.saturating_sub(1).max(1));

    // Walk the LocalTable, computing per-type slot ids on the fly using
    // the same counting strategy as `LocalTable::get_variable_id`.
    let mut int_slot_counter: i32 = 0;
    let mut string_slot_counter: i32 = 0;
    let mut long_slot_counter: i32 = 0;
    let mut array_slot_counter: i32 = 0;

    for entry in script.local_table.entries() {
        let LocalEntry {
            name,
            var_type,
            is_param,
            is_array,
        } = entry;
        let is_param = *is_param;
        let is_array = *is_array;
        let suppress = name.starts_with('_');
        let var_name = format!("${}", name);

        if is_array {
            let slot = array_slot_counter;
            array_slot_counter += 1;
            if suppress || array_touched.contains(&slot) {
                continue;
            }
            let line = array_first_def_line
                .get(&slot)
                .copied()
                .unwrap_or(header_line);
            emit_unused_local(
                script,
                source_cache,
                diagnostics,
                &var_name,
                "array",
                is_param,
                line,
            );
            continue;
        }

        // Parameters claim a slot per base type; locals too. Arrays of a
        // given type also occupy a type slot when they are parameters
        // (matches LocalTable::get_variable_id's `!e.is_array || e.is_param`
        // rule).
        match var_type.base_type() {
            BaseVarType::Integer => {
                let slot = int_slot_counter;
                int_slot_counter += 1;
                if suppress || int_reads.contains(&slot) {
                    continue;
                }
                let line = int_first_write_line
                    .get(&slot)
                    .copied()
                    .unwrap_or(header_line);
                emit_unused_local(
                    script,
                    source_cache,
                    diagnostics,
                    &var_name,
                    "int",
                    is_param,
                    line,
                );
            }
            BaseVarType::String => {
                let slot = string_slot_counter;
                string_slot_counter += 1;
                if suppress || string_reads.contains(&slot) {
                    continue;
                }
                let line = string_first_write_line
                    .get(&slot)
                    .copied()
                    .unwrap_or(header_line);
                emit_unused_local(
                    script,
                    source_cache,
                    diagnostics,
                    &var_name,
                    "string",
                    is_param,
                    line,
                );
            }
            BaseVarType::Long => {
                let slot = long_slot_counter;
                long_slot_counter += 1;
                if suppress || long_reads.contains(&slot) {
                    continue;
                }
                let line = long_first_write_line
                    .get(&slot)
                    .copied()
                    .unwrap_or(header_line);
                emit_unused_local(
                    script,
                    source_cache,
                    diagnostics,
                    &var_name,
                    "long",
                    is_param,
                    line,
                );
            }
        }
    }
}

fn emit_unused_local(
    script: &CompiledScript,
    source_cache: Option<&HashMap<String, Arc<String>>>,
    diagnostics: &mut DiagnosticsCollector,
    name: &str,
    ty: &str,
    is_param: bool,
    line: usize,
) {
    let (message, help_text) = if is_param {
        (
            format!(
                "Parameter `{}` ({}) is declared but never read in this script body.",
                name, ty
            ),
            format!(
                "If `{0}` is intentionally part of the signature (e.g. to \
                 match a label family), prefix its name with an underscore \
                 or remove it from the header if no caller relies on the \
                 position.",
                name
            ),
        )
    } else {
        (
            format!("Local `{}` ({}) is assigned but never read.", name, ty),
            format!(
                "Remove the `def_{}` / assignment, or use `{}` before it \
                 gets overwritten.",
                ty, name
            ),
        )
    };

    // Prose-only help: deleting a local/param without understanding the
    // surrounding intent is rarely safe (callers may depend on
    // parameter position, adjacent statements may have side effects
    // captured through the var). The prose tells the reader what to do;
    // we deliberately don't auto-generate a diff.
    let _ = source_cache;
    let help = Help {
        message: help_text,
        suggestions: Vec::new(),
        applicability: Applicability::MaybeIncorrect,
    };

    let mut diag = Diagnostic {
        file: PathBuf::from(&script.source_path),
        line,
        column: 0,
        message,
        severity: Severity::Warning,
        phase: Phase::PointerCheck, // reused — see main.rs note on Phase
        help: Vec::new(),
    };
    diag.help.push(help);
    diagnostics.add(diag);
}

// ---------------------------------------------------------------------------
// Unreachable code
// ---------------------------------------------------------------------------

/// Build a minimal CFG (reachability-only) and flag any LineNumber whose
/// following instruction is not reachable from the script entry. Compiler-
/// emitted fallthroughs (e.g. the always-emitted `Branch(end)` after a
/// Jump-terminated if body) have no preceding LineNumber and so never
/// trigger a false positive.
fn check_unreachable_code(
    script: &CompiledScript,
    source_cache: Option<&HashMap<String, Arc<String>>>,
    diagnostics: &mut DiagnosticsCollector,
) {
    let instrs = &script.instructions;
    if instrs.is_empty() {
        return;
    }

    // Skip LineNumbers when assigning node ids — they are diagnostic
    // markers, not control-flow points.
    let mut instr_to_node: HashMap<usize, usize> = HashMap::new();
    let mut node_instr: Vec<usize> = Vec::new();
    for (i, instr) in instrs.iter().enumerate() {
        if instr.opcode == Opcode::LineNumber {
            continue;
        }
        instr_to_node.insert(i, node_instr.len());
        node_instr.push(i);
    }
    if node_instr.is_empty() {
        return;
    }

    // Build forward adjacency. Fallthrough is the default; branch/jump
    // opcodes add target edges and suppress fallthrough when they are
    // unconditional terminals.
    let mut next: Vec<Vec<usize>> = vec![Vec::new(); node_instr.len()];
    for (order_idx, &instr_idx) in node_instr.iter().enumerate() {
        let instr = &instrs[instr_idx];

        let is_terminal = matches!(
            instr.opcode,
            Opcode::Branch | Opcode::Return | Opcode::Jump | Opcode::JumpWithParams
        );

        // Jump-target edge.
        match instr.opcode {
            Opcode::Branch
            | Opcode::BranchNot
            | Opcode::BranchEquals
            | Opcode::BranchLessThan
            | Opcode::BranchGreaterThan
            | Opcode::BranchLessThanOrEquals
            | Opcode::BranchGreaterThanOrEquals
            | Opcode::LongBranchNot
            | Opcode::LongBranchEquals
            | Opcode::LongBranchLessThan
            | Opcode::LongBranchGreaterThan
            | Opcode::LongBranchLessThanOrEquals
            | Opcode::LongBranchGreaterThanOrEquals
            | Opcode::ObjBranchEquals
            | Opcode::ObjBranchNot => {
                if let Operand::JumpTarget(target) = &instr.operand
                    && let Some(tnode) = resolve_target(*target, instrs, &instr_to_node)
                {
                    next[order_idx].push(tnode);
                }
            }
            Opcode::Switch => {
                if let Operand::SwitchTable(cases) = &instr.operand {
                    for &(_, target) in cases {
                        if let Some(tnode) = resolve_target(target, instrs, &instr_to_node) {
                            next[order_idx].push(tnode);
                        }
                    }
                }
            }
            _ => {}
        }

        // Fallthrough edge for non-terminals.
        if !is_terminal && order_idx + 1 < node_instr.len() {
            next[order_idx].push(order_idx + 1);
        }
    }

    // Forward BFS from node 0.
    let mut reachable = vec![false; node_instr.len()];
    reachable[0] = true;
    let mut queue: VecDeque<usize> = VecDeque::from([0]);
    while let Some(n) = queue.pop_front() {
        for &m in &next[n] {
            if !reachable[m] {
                reachable[m] = true;
                queue.push_back(m);
            }
        }
    }

    // Walk LineNumbers; for each, find the first following non-LineNumber
    // instruction and check reachability. Dedupe by source line.
    let mut reported_lines: HashSet<usize> = HashSet::new();
    for (i, instr) in instrs.iter().enumerate() {
        let line = match (instr.opcode, &instr.operand) {
            (Opcode::LineNumber, Operand::Int(n)) => *n as usize,
            _ => continue,
        };
        let mut j = i + 1;
        let tnode = loop {
            if j >= instrs.len() {
                break None;
            }
            if instrs[j].opcode != Opcode::LineNumber {
                break instr_to_node.get(&j).copied();
            }
            j += 1;
        };
        let tnode = match tnode {
            Some(n) => n,
            None => continue,
        };
        if reachable[tnode] {
            continue;
        }
        if !reported_lines.insert(line) {
            continue;
        }
        emit_unreachable(script, source_cache, diagnostics, line);
    }
}

fn emit_unreachable(
    script: &CompiledScript,
    source_cache: Option<&HashMap<String, Arc<String>>>,
    diagnostics: &mut DiagnosticsCollector,
    line: usize,
) {
    let message = "Unreachable code: this line cannot be executed because control \
         flow never reaches it (a preceding `return`, `@jump`, or \
         unconditional branch prevents fallthrough)."
        .to_string();

    let mut suggestions: Vec<Suggestion> = Vec::new();
    if let Some(cache) = source_cache
        && let Some(src) = cache.get(&script.source_path)
        && nth_source_line(src, line).is_some()
    {
        suggestions.push(Suggestion {
            file: PathBuf::from(&script.source_path),
            line_range: (line, line),
            replacement: String::new(), // recommend removal
            label: Some("remove the unreachable statement".to_string()),
        });
    }

    let help = Help {
        message: "Remove the dead statement, or move it before the \
                  preceding terminal instruction if it was meant to run."
            .to_string(),
        suggestions,
        applicability: Applicability::MaybeIncorrect,
    };

    let mut diag = Diagnostic {
        file: PathBuf::from(&script.source_path),
        line,
        column: 0,
        message,
        severity: Severity::Warning,
        phase: Phase::PointerCheck,
        help: Vec::new(),
    };
    diag.help.push(help);
    diagnostics.add(diag);
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn resolve_line(instructions: &[Instruction], instr_idx: usize) -> usize {
    for i in (0..=instr_idx).rev() {
        if instructions[i].opcode == Opcode::LineNumber
            && let Operand::Int(line) = instructions[i].operand
        {
            return line as usize;
        }
    }
    0
}

fn nth_source_line(src: &str, line_no: usize) -> Option<&str> {
    if line_no == 0 {
        return None;
    }
    src.lines().nth(line_no - 1)
}

fn resolve_target(
    mut target: usize,
    instrs: &[Instruction],
    instr_to_node: &HashMap<usize, usize>,
) -> Option<usize> {
    while target < instrs.len() && instrs[target].opcode == Opcode::LineNumber {
        target += 1;
    }
    instr_to_node.get(&target).copied()
}

/// Locate the 1-indexed line that declares `[<trigger>,<name>]` in `src`.
/// Mirrors the pointer_checker helper so this module stays self-contained.
fn find_header_line(src: &str, trigger: &str, name: &str) -> Option<usize> {
    let prefix = format!("[{},{}]", trigger, name);
    let prefix_with_params = format!("[{},{}](", trigger, name);
    for (i, line) in src.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with(&prefix) || trimmed.starts_with(&prefix_with_params) {
            return Some(i + 1);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{Instruction, Opcode, Operand};
    use crate::symbol::LocalEntry;
    use crate::types::Type;

    fn empty_script(name: &str) -> CompiledScript {
        let mut s = CompiledScript::new(format!("[label,{}]", name), 0);
        s.trigger = "label".to_string();
        s.source_path = "/virt/test.rs2".to_string();
        s
    }

    fn push_local_entry(
        s: &mut CompiledScript,
        name: &str,
        var_type: Type,
        is_param: bool,
        is_array: bool,
    ) {
        s.local_table.all.push(LocalEntry {
            name: name.to_string(),
            var_type,
            is_param,
            is_array,
        });
    }

    // -----------------------------------------------------------------------
    // Unused locals
    // -----------------------------------------------------------------------

    #[test]
    fn unused_int_local_is_reported() {
        // Body: def_int $x = 5;   (written, never read)
        let mut script = empty_script("foo");
        push_local_entry(&mut script, "x", Type::Int, false, false);
        script.instructions = vec![
            Instruction::new(Opcode::LineNumber, Operand::Int(2)),
            Instruction::push_int(5),
            Instruction::pop_int_local(0),
            Instruction::simple(Opcode::Return),
        ];

        let scripts = [script];
        let mut diags = DiagnosticsCollector::new();
        check_unused_locals(&scripts[0], None, &mut diags);

        let warnings: Vec<_> = diags
            .diagnostics()
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .collect();
        assert_eq!(warnings.len(), 1, "expected one unused-local warning");
        assert!(
            warnings[0].message.contains("$x"),
            "message should name $x: {}",
            warnings[0].message
        );
        assert_eq!(warnings[0].line, 2);
    }

    #[test]
    fn read_then_written_int_local_is_ok() {
        // def_int $x = 5; def_int $y = $x;   (x is read, y is not — y warns)
        let mut script = empty_script("foo");
        push_local_entry(&mut script, "x", Type::Int, false, false);
        push_local_entry(&mut script, "y", Type::Int, false, false);
        script.instructions = vec![
            Instruction::new(Opcode::LineNumber, Operand::Int(2)),
            Instruction::push_int(5),
            Instruction::pop_int_local(0),
            Instruction::new(Opcode::LineNumber, Operand::Int(3)),
            Instruction::push_int_local(0), // read of $x
            Instruction::pop_int_local(1),  // write to $y
            Instruction::simple(Opcode::Return),
        ];

        let mut diags = DiagnosticsCollector::new();
        check_unused_locals(&script, None, &mut diags);
        let warnings: Vec<_> = diags
            .diagnostics()
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .collect();
        assert_eq!(warnings.len(), 1, "only $y should warn");
        assert!(warnings[0].message.contains("$y"));
    }

    #[test]
    fn unused_parameter_is_reported_with_param_wording() {
        let mut script = empty_script("foo");
        script.trigger = "label".to_string();
        push_local_entry(&mut script, "unused", Type::Int, true, false);
        script.int_arg_count = 1;
        script.int_local_count = 1;
        script.instructions = vec![
            Instruction::new(Opcode::LineNumber, Operand::Int(1)),
            Instruction::simple(Opcode::Return),
        ];

        let mut diags = DiagnosticsCollector::new();
        check_unused_locals(&script, None, &mut diags);
        let warnings: Vec<_> = diags
            .diagnostics()
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .collect();
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].message.contains("Parameter"),
            "message should use parameter wording: {}",
            warnings[0].message
        );
    }

    #[test]
    fn string_and_long_locals_tracked_independently() {
        let mut script = empty_script("foo");
        push_local_entry(&mut script, "s", Type::String, false, false);
        push_local_entry(&mut script, "l", Type::Long, false, false);
        script.instructions = vec![
            Instruction::new(Opcode::LineNumber, Operand::Int(1)),
            Instruction::push_string("hi".to_string()),
            Instruction::pop_string_local(0),
            Instruction::push_long(42),
            Instruction::pop_long_local(0),
            Instruction::simple(Opcode::Return),
        ];

        let mut diags = DiagnosticsCollector::new();
        check_unused_locals(&script, None, &mut diags);
        let warnings: Vec<_> = diags
            .diagnostics()
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .collect();
        assert_eq!(warnings.len(), 2, "both $s and $l should warn");
        let names: Vec<&str> = warnings.iter().map(|d| d.message.as_str()).collect();
        assert!(names.iter().any(|m| m.contains("$s")));
        assert!(names.iter().any(|m| m.contains("$l")));
    }

    #[test]
    fn unused_array_is_reported() {
        let mut script = empty_script("foo");
        push_local_entry(&mut script, "arr", Type::Int, false, true);
        script.instructions = vec![
            Instruction::new(Opcode::LineNumber, Operand::Int(2)),
            Instruction::push_int(5),
            Instruction::new(Opcode::DefineArray, Operand::Int(0)),
            Instruction::simple(Opcode::Return),
        ];

        let mut diags = DiagnosticsCollector::new();
        check_unused_locals(&script, None, &mut diags);
        let warnings: Vec<_> = diags
            .diagnostics()
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .collect();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("$arr"));
        assert!(warnings[0].message.contains("array"));
    }

    #[test]
    fn touched_array_does_not_warn() {
        // def_int $arr(3); $arr(0) = 1; read $arr(0) — array IS used.
        let mut script = empty_script("foo");
        push_local_entry(&mut script, "arr", Type::Int, false, true);
        script.instructions = vec![
            Instruction::new(Opcode::LineNumber, Operand::Int(1)),
            Instruction::push_int(3),
            Instruction::new(Opcode::DefineArray, Operand::Int(0)),
            Instruction::push_int(1),
            Instruction::push_int(0),
            Instruction::new(Opcode::PopArrayInt, Operand::Int(0)),
            Instruction::push_int(0),
            Instruction::new(Opcode::PushArrayInt, Operand::Int(0)),
            Instruction::new(Opcode::PopIntDiscard, Operand::Int(0)),
            Instruction::simple(Opcode::Return),
        ];

        let mut diags = DiagnosticsCollector::new();
        check_unused_locals(&script, None, &mut diags);
        let warnings: Vec<_> = diags
            .diagnostics()
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .collect();
        assert_eq!(warnings.len(), 0, "touched array should not warn");
    }

    // -----------------------------------------------------------------------
    // Unreachable code
    // -----------------------------------------------------------------------

    #[test]
    fn dead_code_after_return_is_reported() {
        // mes("hi"); return; mes("bye");  <-- line 4 unreachable
        let mut script = empty_script("foo");
        script.instructions = vec![
            Instruction::new(Opcode::LineNumber, Operand::Int(2)),
            Instruction::push_string("hi".to_string()),
            Instruction::new(Opcode::Command, Operand::Int(0)), // mes
            Instruction::new(Opcode::LineNumber, Operand::Int(3)),
            Instruction::simple(Opcode::Return),
            Instruction::new(Opcode::LineNumber, Operand::Int(4)),
            Instruction::push_string("bye".to_string()),
            Instruction::new(Opcode::Command, Operand::Int(0)),
            Instruction::simple(Opcode::Return),
        ];

        let mut diags = DiagnosticsCollector::new();
        check_unreachable_code(&script, None, &mut diags);
        let warnings: Vec<_> = diags
            .diagnostics()
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .collect();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].line, 4);
        assert!(warnings[0].message.contains("Unreachable"));
    }

    #[test]
    fn dead_code_after_jump_is_reported() {
        // @some_label; mes("never");
        let mut script = empty_script("foo");
        script.instructions = vec![
            Instruction::new(Opcode::LineNumber, Operand::Int(2)),
            Instruction::new(Opcode::JumpWithParams, Operand::Int(999)),
            Instruction::new(Opcode::LineNumber, Operand::Int(3)),
            Instruction::push_string("never".to_string()),
            Instruction::new(Opcode::Command, Operand::Int(0)),
            Instruction::simple(Opcode::Return),
        ];

        let mut diags = DiagnosticsCollector::new();
        check_unreachable_code(&script, None, &mut diags);
        let warnings: Vec<_> = diags
            .diagnostics()
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .collect();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].line, 3);
    }

    #[test]
    fn reachable_code_after_conditional_branch_is_ok() {
        // if (cond) { body } — body IS reachable via BranchEquals target.
        let mut script = empty_script("foo");
        script.instructions = vec![
            Instruction::new(Opcode::LineNumber, Operand::Int(1)),
            Instruction::push_int(1),
            Instruction::push_int(1),
            Instruction::new(Opcode::BranchEquals, Operand::JumpTarget(6)),
            Instruction::new(Opcode::Branch, Operand::JumpTarget(9)),
            Instruction::new(Opcode::LineNumber, Operand::Int(2)),
            Instruction::push_string("yes".to_string()),
            Instruction::new(Opcode::Command, Operand::Int(0)),
            Instruction::new(Opcode::LineNumber, Operand::Int(3)),
            Instruction::simple(Opcode::Return),
        ];

        let mut diags = DiagnosticsCollector::new();
        check_unreachable_code(&script, None, &mut diags);
        let warnings: Vec<_> = diags
            .diagnostics()
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .collect();
        assert_eq!(
            warnings.len(),
            0,
            "conditional-branch body is reachable; got: {:?}",
            warnings.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn compiler_emitted_branch_without_linenumber_does_not_warn() {
        // Simulates compile_if's always-emitted Branch(end) after a body
        // that ends in Return. The Branch has NO leading LineNumber, so
        // our lint should skip it.
        let mut script = empty_script("foo");
        script.instructions = vec![
            Instruction::new(Opcode::LineNumber, Operand::Int(1)),
            Instruction::push_int(1),
            Instruction::push_int(1),
            Instruction::new(Opcode::BranchEquals, Operand::JumpTarget(5)),
            Instruction::new(Opcode::Branch, Operand::JumpTarget(8)),
            Instruction::new(Opcode::LineNumber, Operand::Int(2)),
            Instruction::simple(Opcode::Return),
            Instruction::new(Opcode::Branch, Operand::JumpTarget(8)), // compiler-emitted, unreachable
            Instruction::simple(Opcode::Return),
        ];

        let mut diags = DiagnosticsCollector::new();
        check_unreachable_code(&script, None, &mut diags);
        let warnings: Vec<_> = diags
            .diagnostics()
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .collect();
        assert_eq!(
            warnings.len(),
            0,
            "compiler-emitted unreachable Branch must not warn; got: {:?}",
            warnings.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }
}
