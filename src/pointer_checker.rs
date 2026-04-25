use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use crate::bytecode::{CompiledScript, Instruction, Opcode, Operand};
use crate::diagnostic_messages as msg;
use crate::diagnostics::{
    Applicability, Diagnostic, DiagnosticsCollector, Help, Phase, Severity, Suggestion,
};
use crate::pointer::{PointerHolder, PointerSet, PointerType, command_pointers, trigger_pointers};
use crate::symbol::{SymbolKind, SymbolRegistry};

// ---------------------------------------------------------------------------
// Control flow graph node
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct CfgNode {
    /// Index into `CompiledScript::instructions`, or `None` for synthetic nodes
    /// (start node, pointer-instruction nodes).
    instruction_index: Option<usize>,
    /// For pointer-instruction nodes: the set of pointers this node establishes.
    pointer_set: PointerSet,
    /// Successor node indices.
    next: Vec<usize>,
    /// Predecessor node indices.
    previous: Vec<usize>,
}

impl CfgNode {
    fn new(instruction_index: Option<usize>) -> Self {
        CfgNode {
            instruction_index,
            pointer_set: PointerSet::new(),
            next: Vec::new(),
            previous: Vec::new(),
        }
    }

    fn pointer_node(pointer_set: PointerSet) -> Self {
        CfgNode {
            instruction_index: None,
            pointer_set,
            next: Vec::new(),
            previous: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Per-script analysis result
// ---------------------------------------------------------------------------

struct ScriptAnalysis {
    nodes: Vec<CfgNode>,
    /// `required[ptr_index]` = list of node indices whose instruction requires this pointer.
    required: Vec<Vec<usize>>,
    /// `set_list[ptr_index]` = list of node indices that establish this pointer.
    set_list: Vec<Vec<usize>>,
    /// `corrupted_list[ptr_index]` = list of node indices that corrupt this pointer.
    corrupted_list: Vec<Vec<usize>>,
    /// Quick lookup sets for BFS blocking.
    set_nodes: Vec<HashSet<usize>>,
    corrupted_nodes: Vec<HashSet<usize>>,
    /// Node indices of Return instructions.
    returns: Vec<usize>,
}

impl ScriptAnalysis {
    fn new() -> Self {
        let n = PointerType::COUNT;
        ScriptAnalysis {
            nodes: Vec::new(),
            required: vec![Vec::new(); n],
            set_list: vec![Vec::new(); n],
            corrupted_list: vec![Vec::new(); n],
            set_nodes: vec![HashSet::new(); n],
            corrupted_nodes: vec![HashSet::new(); n],
            returns: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// PointerChecker
// ---------------------------------------------------------------------------

pub struct PointerChecker<'a> {
    scripts: &'a [CompiledScript],
    cmd_pointers: HashMap<String, PointerHolder>,
    #[allow(dead_code)]
    registry: &'a SymbolRegistry,
    /// Reverse map: command opcode -> command name (lowercase).
    opcode_to_name: HashMap<i32, String>,
    /// Reverse map: script id -> index into `scripts`.
    script_id_to_index: HashMap<i32, usize>,
    /// Overlay interface names (normalized lowercase). If_button/inv_button triggers
    /// for non-overlay interfaces grant p_active_player, matching ServerPointerChecker.
    overlay_interfaces: HashSet<String>,
    /// Optional source-text cache, keyed by `CompiledScript::source_path`.
    /// When present, the checker attaches rustc-style Help/Suggestion
    /// blocks to its warnings. When `None`, warnings still fire but carry
    /// no concrete fix text. Read-only; never feeds codegen.
    source_cache: Option<&'a HashMap<String, std::rc::Rc<String>>>,
    // Caches
    script_analyses: HashMap<String, ScriptAnalysis>,
    script_pointers: HashMap<String, PointerHolder>,
    pending_analyses: HashSet<String>,
    pending_scripts: HashSet<String>,
}

impl<'a> PointerChecker<'a> {
    pub fn new(scripts: &'a [CompiledScript], registry: &'a SymbolRegistry) -> Self {
        // Build reverse opcode -> name map from registry.commands
        let mut opcode_to_name = HashMap::new();
        for (name, sym) in &registry.commands {
            if let SymbolKind::Command { opcode, .. } = &sym.kind {
                opcode_to_name.insert(*opcode, name.to_lowercase());
            }
        }

        // Build reverse script_id -> index map
        let mut script_id_to_index = HashMap::new();
        for (i, script) in scripts.iter().enumerate() {
            script_id_to_index.insert(script.id, i);
        }

        // Build overlay interface set from entity_ids (OverlayInterface type).
        // Any if_button/inv_button trigger whose interface is NOT in this set
        // gets p_active_player granted, matching ServerPointerChecker.
        let mut overlay_interfaces = HashSet::new();
        for (name, sym) in &registry.entity_ids {
            if let SymbolKind::Constant { const_type, .. } = &sym.kind {
                if *const_type == crate::types::Type::OverlayInterface {
                    overlay_interfaces.insert(name.to_lowercase().replace(' ', "_"));
                }
            }
        }

        PointerChecker {
            scripts,
            cmd_pointers: command_pointers(),
            registry,
            opcode_to_name,
            script_id_to_index,
            overlay_interfaces,
            source_cache: None,
            script_analyses: HashMap::new(),
            script_pointers: HashMap::new(),
            pending_analyses: HashSet::new(),
            pending_scripts: HashSet::new(),
        }
    }

    /// Attach a source-text cache so emitted warnings can carry concrete
    /// before/after suggestion text. Purely diagnostic — never touches
    /// bytecode, script metadata, or output ordering.
    pub fn set_source_cache(&mut self, cache: &'a HashMap<String, std::rc::Rc<String>>) {
        self.source_cache = Some(cache);
    }

    /// Check if an if_button/inv_button trigger on a non-overlay interface
    /// grants p_active_player. Matches ServerPointerChecker.setsPointerTrigger().
    fn interface_trigger_grants_protected(&self, trigger: &str, script_name: &str) -> bool {
        if !crate::trigger_table::is_button(trigger) {
            return false;
        }
        // Extract interface name from script name: "[trigger,interface_name:component]"
        let subject = script_name
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split_once(',')
            .map(|(_, rest)| rest)
            .unwrap_or("");
        let iface_name = subject
            .split(':')
            .next()
            .unwrap_or("")
            .to_lowercase()
            .replace(' ', "_");
        if iface_name.is_empty() {
            return false;
        }
        // Non-overlay interfaces grant p_active_player
        !self.overlay_interfaces.contains(&iface_name)
    }

    /// Run pointer checking on all scripts and return collected diagnostics.
    pub fn run(&mut self) -> DiagnosticsCollector {
        let mut diagnostics = DiagnosticsCollector::new();

        // Collect script names first to avoid borrow issues
        let script_names: Vec<String> = self.scripts.iter().map(|s| s.name.clone()).collect();

        for script_name in &script_names {
            self.validate_script(script_name, &mut diagnostics);
        }

        diagnostics
    }

    fn validate_script(&mut self, script_name: &str, diagnostics: &mut DiagnosticsCollector) {
        // Find the script
        let script_idx = match self.scripts.iter().position(|s| s.name == script_name) {
            Some(i) => i,
            None => return,
        };
        let trigger = self.scripts[script_idx].trigger.clone();
        let source_path = self.scripts[script_idx].source_path.clone();
        let trigger_set = trigger_pointers(&trigger);

        // Ensure analysis exists
        self.ensure_analysis(script_name);

        for &ptr in &PointerType::ALL {
            let ptr_idx = ptr.index();

            // Get analysis data — we need to work with references carefully
            let analysis = match self.script_analyses.get(script_name) {
                Some(a) => a,
                None => continue,
            };

            let required_nodes: Vec<usize> = analysis.required[ptr_idx].clone();
            if required_nodes.is_empty() {
                continue;
            }

            // Build corrupted set: start with the analysis corrupted nodes
            let mut corrupted: HashSet<usize> = analysis.corrupted_nodes[ptr_idx].clone();

            // If the trigger doesn't set this pointer, add the start node (node 0) as corrupted.
            // Special case: if_button/inv_button triggers on non-overlay interfaces
            // grant p_active_player, matching ServerPointerChecker.
            let trigger_provides = if trigger_set.contains(ptr) {
                true
            } else if ptr == PointerType::PActivePlayer {
                self.interface_trigger_grants_protected(&trigger, script_name)
            } else {
                false
            };
            if !trigger_provides {
                corrupted.insert(0);
            }

            if corrupted.is_empty() {
                continue;
            }

            let set_blocked: HashSet<usize> = analysis.set_nodes[ptr_idx].clone();

            // Try to find a path from a required node backwards to a corrupted node
            let path = self.find_edge_path(
                script_name,
                &required_nodes,
                |node_idx| corrupted.contains(&node_idx),
                &set_blocked,
            );

            let mut reported_via_main_bfs = false;
            if let Some(path) = path {
                if path.is_empty() {
                    continue;
                }
                reported_via_main_bfs = true;

                let end_node_idx = path[path.len() - 1];
                let start_node_idx = path[0];

                let is_start_node = end_node_idx == 0;
                let repr = ptr.representation();
                let static_only = is_static_only_pointer(ptr);

                // Use softer "stale reference" wording for last_* / find_*
                // pointers — those are not tracked by the runtime and the
                // scripts compile and run; "corrupted" overstates it. The
                // engine-tracked family keeps the strong wording because a
                // missing pointer there is a real pointerCheck crash.
                let error_msg = if is_start_node {
                    if static_only {
                        msg::fmt(msg::POINTER_STALE_UNINIT, &[repr])
                    } else {
                        msg::fmt(msg::POINTER_UNINITIALIZED, &[repr])
                    }
                } else if static_only {
                    msg::fmt(msg::POINTER_STALE, &[repr])
                } else {
                    msg::fmt(msg::POINTER_CORRUPTED, &[repr])
                };

                let analysis = self.script_analyses.get(script_name).unwrap();
                let instructions = &self.scripts[script_idx].instructions;
                let req_line = analysis.nodes[start_node_idx]
                    .instruction_index
                    .map(|idx| resolve_line(instructions, idx))
                    .unwrap_or(0);

                // Pointer diagnostics are reported as warnings: they flag
                // real static-analysis findings but should not fail the
                // compile. Runtime pointer enforcement (via engine
                // `pointerCheck`) still catches genuine misuse.
                diagnostics.add(Diagnostic {
                    file: PathBuf::from(&source_path),
                    line: req_line,
                    column: 0,
                    message: error_msg,
                    severity: Severity::Warning,
                    phase: Phase::PointerCheck,
                    help: Vec::new(),
                });

                let read_hint = if static_only {
                    msg::fmt(msg::POINTER_READ_HERE, &[repr])
                } else {
                    msg::fmt(msg::POINTER_REQUIRED_LOC, &[repr])
                };
                diagnostics.add(Diagnostic {
                    file: PathBuf::from(&source_path),
                    line: req_line,
                    column: 0,
                    message: read_hint,
                    severity: Severity::Info,
                    phase: Phase::PointerCheck,
                    help: Vec::new(),
                });

                if !is_start_node {
                    let corrupt_line = analysis.nodes[end_node_idx]
                        .instruction_index
                        .map(|idx| resolve_line(instructions, idx))
                        .unwrap_or(0);

                    let cause_hint = if static_only {
                        msg::fmt(msg::POINTER_SUPERSEDED_HERE, &[repr])
                    } else {
                        msg::fmt(msg::POINTER_CORRUPTED_LOC, &[repr])
                    };
                    diagnostics.add(Diagnostic {
                        file: PathBuf::from(&source_path),
                        line: corrupt_line,
                        column: 0,
                        message: cause_hint,
                        severity: Severity::Info,
                        phase: Phase::PointerCheck,
                        help: Vec::new(),
                    });
                }
            }

            // Secondary check: flag nodes that simultaneously require AND
            // corrupt this pointer. The 2004scape TS PointerChecker does not
            // catch these (its BFS only inspects predecessors of required
            // nodes, never the required node itself), but they correspond to
            // real "@jump to a label that consumes then invalidates the
            // pointer" patterns that are worth surfacing.
            //
            // Only emit for static-only pointers (last_*, find_*). The
            // active_player / p_active_player / active_npc / active_loc /
            // active_obj family is runtime-tracked by the engine and, in
            // practice, the callers in these patterns do not access the
            // pointer again after the gosub, so flagging them is noise.
            if !reported_via_main_bfs && is_static_only_pointer(ptr) {
                let analysis = self.script_analyses.get(script_name).unwrap();
                let instructions = &self.scripts[script_idx].instructions;
                let corrupted_nodes = &analysis.corrupted_nodes[ptr_idx];
                let mut seen_lines: HashSet<usize> = HashSet::new();
                let repr = ptr.representation();
                for &node_idx in &required_nodes {
                    if !corrupted_nodes.contains(&node_idx) {
                        continue;
                    }
                    let line = analysis.nodes[node_idx]
                        .instruction_index
                        .map(|idx| resolve_line(instructions, idx))
                        .unwrap_or(0);
                    if !seen_lines.insert(line) {
                        continue;
                    }

                    // Build a rustc-style Help pointing at the concrete fix:
                    // thread the pointer through the callee as a parameter.
                    // Falls back to prose-only if source cache is absent or
                    // the callee can't be identified.
                    let help =
                        self.build_rule_a_help(ptr, &source_path, line, node_idx, script_idx);

                    // Secondary-heuristic warning: both "read here" and
                    // "superseded here" collapse to the same Jump/Gosub
                    // node, so the info hints from the main-BFS path would
                    // just triple-print the same source line. The Help
                    // block already pinpoints the call site and the label
                    // header with a concrete fix — skip the duplicate
                    // hints and use the softer stale wording.
                    let mut diag = Diagnostic {
                        file: PathBuf::from(&source_path),
                        line,
                        column: 0,
                        message: msg::fmt(msg::POINTER_STALE, &[repr]),
                        severity: Severity::Warning,
                        phase: Phase::PointerCheck,
                        help: Vec::new(),
                    };
                    if let Some(h) = help {
                        diag.help.push(h);
                    }
                    diagnostics.add(diag);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // BFS path finding
    // -----------------------------------------------------------------------

    /// Find a path from any start node backwards through predecessor edges to
    /// a node satisfying `end_pred`, avoiding nodes in `blocked`.
    ///
    /// Returns the path from start to end (inclusive) if found.
    fn find_edge_path(
        &self,
        script_name: &str,
        starts: &[usize],
        end_pred: impl Fn(usize) -> bool,
        blocked: &HashSet<usize>,
    ) -> Option<Vec<usize>> {
        let analysis = self.script_analyses.get(script_name)?;

        let mut queue: VecDeque<usize> = VecDeque::new();
        let mut visited: HashSet<usize> = HashSet::new();
        // Track which node led us to each node, plus which start node originated this path
        let mut sources: HashMap<usize, (usize, usize)> = HashMap::new(); // node -> (from_node, start_node)

        // Seed BFS: for each start node, add its predecessors.
        // Matches TS: we do NOT check start nodes themselves against the end predicate.
        // This prevents false positives when a Gosub node is both required and corrupted
        // for the same pointer (the requirement is satisfied first within the proc).
        for &start in starts {
            for &prev in &analysis.nodes[start].previous {
                if blocked.contains(&prev) || visited.contains(&prev) {
                    continue;
                }
                visited.insert(prev);
                queue.push_back(prev);
                sources.insert(prev, (start, start));
            }
        }

        while let Some(current) = queue.pop_front() {
            if end_pred(current) {
                // Reconstruct path from current back to start
                let mut path = vec![current];
                let mut node = current;
                loop {
                    if let Some(&(from, start)) = sources.get(&node) {
                        if from == start && starts.contains(&from) {
                            path.push(from);
                            break;
                        }
                        path.push(from);
                        node = from;
                    } else {
                        break;
                    }
                }
                path.reverse();
                return Some(path);
            }

            let (_, start_node) = sources[&current];

            for &prev in &analysis.nodes[current].previous {
                if blocked.contains(&prev) || visited.contains(&prev) {
                    continue;
                }
                visited.insert(prev);
                queue.push_back(prev);
                sources.insert(prev, (current, start_node));
            }
        }

        None
    }

    /// Static version of find_edge_path that operates on nodes directly,
    /// used by get_script_pointers where we can't borrow &self.
    fn find_edge_path_static(
        nodes: &[CfgNode],
        starts: &[usize],
        end_pred: impl Fn(usize) -> bool,
        blocked: &HashSet<usize>,
    ) -> Option<Vec<usize>> {
        let mut queue: VecDeque<usize> = VecDeque::new();
        let mut visited: HashSet<usize> = HashSet::new();
        let mut sources: HashMap<usize, (usize, usize)> = HashMap::new();

        for &start in starts {
            for &prev in &nodes[start].previous {
                if blocked.contains(&prev) || visited.contains(&prev) {
                    continue;
                }
                visited.insert(prev);
                queue.push_back(prev);
                sources.insert(prev, (start, start));
            }
        }

        while let Some(current) = queue.pop_front() {
            if end_pred(current) {
                let mut path = vec![current];
                let mut node = current;
                loop {
                    if let Some(&(from, start)) = sources.get(&node) {
                        if from == start && starts.contains(&from) {
                            path.push(from);
                            break;
                        }
                        path.push(from);
                        node = from;
                    } else {
                        break;
                    }
                }
                path.reverse();
                return Some(path);
            }

            for &prev in &nodes[current].previous {
                if blocked.contains(&prev) || visited.contains(&prev) {
                    continue;
                }
                visited.insert(prev);
                queue.push_back(prev);
                sources.insert(prev, (current, sources[&current].1));
            }
        }

        None
    }

    // -----------------------------------------------------------------------
    // Analysis
    // -----------------------------------------------------------------------

    fn ensure_analysis(&mut self, script_name: &str) {
        if self.script_analyses.contains_key(script_name) {
            return;
        }
        if self.pending_analyses.contains(script_name) {
            return; // cycle detection
        }
        self.pending_analyses.insert(script_name.to_string());

        let script_idx = match self.scripts.iter().position(|s| s.name == script_name) {
            Some(i) => i,
            None => return,
        };

        let mut analysis = self.build_cfg(script_idx);
        self.categorize_nodes(script_idx, &mut analysis);

        self.script_analyses
            .insert(script_name.to_string(), analysis);
        self.pending_analyses.remove(script_name);
    }

    // -----------------------------------------------------------------------
    // CFG construction
    // -----------------------------------------------------------------------

    fn build_cfg(&self, script_idx: usize) -> ScriptAnalysis {
        let script = &self.scripts[script_idx];
        let instructions = &script.instructions;
        let mut analysis = ScriptAnalysis::new();

        // Node 0 = start node (no instruction)
        analysis.nodes.push(CfgNode::new(None));

        // Map instruction index -> CFG node index (skipping LineNumber instructions)
        let mut instr_to_node: HashMap<usize, usize> = HashMap::new();
        let mut node_order: Vec<usize> = Vec::new(); // instruction indices in order

        for (i, instr) in instructions.iter().enumerate() {
            if instr.opcode == Opcode::LineNumber {
                continue;
            }
            let node_idx = analysis.nodes.len();
            instr_to_node.insert(i, node_idx);
            node_order.push(i);
            analysis.nodes.push(CfgNode::new(Some(i)));
        }

        if node_order.is_empty() {
            return analysis;
        }

        // Connect start node to first instruction node
        let first_node = instr_to_node[&node_order[0]];
        add_edge(&mut analysis.nodes, 0, first_node);

        // Build edges based on instruction semantics
        for (order_idx, &instr_idx) in node_order.iter().enumerate() {
            let node_idx = instr_to_node[&instr_idx];
            let instr = &instructions[instr_idx];

            let is_terminal = matches!(
                instr.opcode,
                Opcode::Branch | Opcode::Return | Opcode::Jump | Opcode::JumpWithParams
            );

            // Handle jump targets for branches (conditional and unconditional).
            // Jump targets often land on a LineNumber instruction (emitted for the
            // body's first source line); LineNumbers are not in `instr_to_node`, so
            // walk forward to the first real instruction before resolving.
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
                    if let Operand::JumpTarget(target_instr_idx) = &instr.operand {
                        if let Some(target_node) =
                            resolve_target_node(*target_instr_idx, instructions, &instr_to_node)
                        {
                            add_edge(&mut analysis.nodes, node_idx, target_node);
                        }
                    }
                }

                Opcode::Switch => {
                    if let Operand::SwitchTable(cases) = &instr.operand {
                        for &(_, target_instr_idx) in cases {
                            if let Some(target_node) =
                                resolve_target_node(target_instr_idx, instructions, &instr_to_node)
                            {
                                add_edge(&mut analysis.nodes, node_idx, target_node);
                            }
                        }
                    }
                }

                _ => {}
            }

            // Add fallthrough edge (next non-LineNumber instruction) for non-terminal instructions
            if !is_terminal {
                if let Some(next_order_idx) = order_idx.checked_add(1) {
                    if next_order_idx < node_order.len() {
                        let next_node = instr_to_node[&node_order[next_order_idx]];
                        add_edge(&mut analysis.nodes, node_idx, next_node);
                    }
                }
            }

            // For conditional branches: also add fallthrough
            // (already handled above since conditional branches are not terminal)

            // Track Return nodes
            if instr.opcode == Opcode::Return {
                analysis.returns.push(node_idx);
            }
        }

        // Pointer inversion: detect Command -> PushConstantInt(0/1) -> BranchEquals patterns
        self.insert_pointer_nodes(script_idx, &mut analysis, &instr_to_node, &node_order);

        analysis
    }

    /// Detect conditional pointer setting patterns and insert synthetic
    /// PointerInstructionNode nodes.
    fn insert_pointer_nodes(
        &self,
        script_idx: usize,
        analysis: &mut ScriptAnalysis,
        instr_to_node: &HashMap<usize, usize>,
        node_order: &[usize],
    ) {
        let instructions = &self.scripts[script_idx].instructions;

        // Look for the pattern: [Command with conditional_set] ... [PushConstantInt(0 or 1)] [BranchEquals]
        // The Command may not be immediately before PushConstantInt because argument
        // pushes sit between them. So we scan backwards from PushConstantInt+BranchEquals
        // to find the nearest Command.
        for window_end in 2..node_order.len() {
            let push_instr_idx = node_order[window_end - 1];
            let branch_instr_idx = node_order[window_end];

            let push_instr = &instructions[push_instr_idx];
            let branch_instr = &instructions[branch_instr_idx];

            // Check: PushConstantInt(0 or 1) followed by BranchEquals
            let push_value = match &push_instr.operand {
                Operand::Int(v) if *v == 0 || *v == 1 => *v,
                _ => continue,
            };
            if push_instr.opcode != Opcode::PushConstantInt {
                continue;
            }
            if branch_instr.opcode != Opcode::BranchEquals {
                continue;
            }

            // Scan backwards to find the nearest Command instruction
            let mut cmd_instr_idx = None;
            for scan in (0..window_end - 1).rev() {
                let idx = node_order[scan];
                if instructions[idx].opcode == Opcode::Command {
                    cmd_instr_idx = Some(idx);
                    break;
                }
                // Stop scanning if we hit a branch/return/switch (control flow boundary)
                if matches!(
                    instructions[idx].opcode,
                    Opcode::Branch
                        | Opcode::Return
                        | Opcode::Switch
                        | Opcode::BranchEquals
                        | Opcode::BranchNot
                        | Opcode::BranchLessThan
                        | Opcode::BranchGreaterThan
                        | Opcode::BranchLessThanOrEquals
                        | Opcode::BranchGreaterThanOrEquals
                ) {
                    break;
                }
            }
            let cmd_instr_idx = match cmd_instr_idx {
                Some(idx) => idx,
                None => continue,
            };
            let cmd_instr = &instructions[cmd_instr_idx];

            // Look up command pointer info
            let cmd_name = self.resolve_command_name(cmd_instr);
            let cmd_name = match cmd_name {
                Some(n) => n,
                None => continue,
            };

            let holder = match self.cmd_pointers.get(&cmd_name) {
                Some(h) => h,
                None => continue,
            };

            if !holder.conditional_set {
                continue;
            }

            if holder.set.is_empty() {
                continue;
            }

            let branch_node_idx = instr_to_node[&branch_instr_idx];

            // Create a synthetic pointer node with the command's set pointers
            let ptr_node_idx = analysis.nodes.len();
            analysis.nodes.push(CfgNode::pointer_node(holder.set));

            if push_value == 0 {
                // Inverted conditional: pointer is set when the branch is NOT taken
                // (i.e., the fallthrough path after BranchEquals).
                // The BranchEquals jumps when equal (value == 0 means "not found"),
                // so the fallthrough means "found" -> pointer is set.
                //
                // Insert pointer node between BranchEquals and its fallthrough target.
                // BranchEquals fallthrough = the next node in sequence after BranchEquals.
                let fallthrough_targets: Vec<usize> = analysis.nodes[branch_node_idx]
                    .next
                    .iter()
                    .copied()
                    .filter(|&n| {
                        // fallthrough target = not the jump target
                        if let Operand::JumpTarget(target) = &branch_instr.operand {
                            if let Some(jump_node) =
                                resolve_target_node(*target, instructions, instr_to_node)
                            {
                                return n != jump_node;
                            }
                        }
                        true
                    })
                    .collect();

                for ft_target in fallthrough_targets {
                    // Remove edge: branch -> fallthrough
                    remove_edge(&mut analysis.nodes, branch_node_idx, ft_target);
                    // Add: branch -> ptr_node -> fallthrough
                    add_edge(&mut analysis.nodes, branch_node_idx, ptr_node_idx);
                    add_edge(&mut analysis.nodes, ptr_node_idx, ft_target);
                }
            } else {
                // push_value == 1: pointer is set when the branch IS taken
                // BranchEquals jumps when equal (value == 1 means "found"),
                // so the jump target means "found" -> pointer is set.
                if let Operand::JumpTarget(target) = &branch_instr.operand {
                    if let Some(jump_target_node) =
                        resolve_target_node(*target, instructions, instr_to_node)
                    {
                        // Remove edge: branch -> jump_target
                        remove_edge(&mut analysis.nodes, branch_node_idx, jump_target_node);
                        // Add: branch -> ptr_node -> jump_target
                        add_edge(&mut analysis.nodes, branch_node_idx, ptr_node_idx);
                        add_edge(&mut analysis.nodes, ptr_node_idx, jump_target_node);
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Node categorization
    // -----------------------------------------------------------------------

    fn categorize_nodes(&mut self, script_idx: usize, analysis: &mut ScriptAnalysis) {
        let instructions = &self.scripts[script_idx].instructions;

        for node_idx in 0..analysis.nodes.len() {
            let instr_idx = match analysis.nodes[node_idx].instruction_index {
                Some(i) => i,
                None => {
                    // Synthetic pointer node — categorize its set pointers
                    let ptr_set = analysis.nodes[node_idx].pointer_set;
                    if !ptr_set.is_empty() {
                        for ptr in ptr_set.iter() {
                            let pi = ptr.index();
                            analysis.set_list[pi].push(node_idx);
                            analysis.set_nodes[pi].insert(node_idx);
                        }
                    }
                    continue;
                }
            };

            let instr = &instructions[instr_idx];

            match instr.opcode {
                Opcode::Command => {
                    let cmd_name = self.resolve_command_name(instr);
                    if let Some(name) = cmd_name {
                        if let Some(holder) = self.cmd_pointers.get(&name).cloned() {
                            self.apply_holder(analysis, node_idx, &holder);
                        }
                    }
                }

                Opcode::Gosub | Opcode::GosubWithParams => {
                    if let Operand::Int(script_id) = &instr.operand {
                        let callee_holder = self.get_script_pointers(*script_id);
                        self.apply_holder(analysis, node_idx, &callee_holder);
                    }
                }

                Opcode::Jump | Opcode::JumpWithParams => {
                    if let Operand::Int(script_id) = &instr.operand {
                        let callee_holder = self.get_script_pointers(*script_id);
                        self.apply_holder(analysis, node_idx, &callee_holder);
                    }
                }

                // Var reads/writes: dot-prefixed sources (`.%v`) route to the
                // secondary active_* pointer. The compiler encodes this by
                // setting bit 16 of the operand (see compiler.rs, where
                // secondary game vars emit `(1 << 16) | var_id`). TS emits
                // distinct opcodes (PushVar/PushVar2); we flatten that into a
                // single opcode + operand flag and recover it here.
                Opcode::PushVarp | Opcode::PushVarbit => {
                    let pi = if is_secondary_var_operand(&instr.operand) {
                        PointerType::ActivePlayer2.index()
                    } else {
                        PointerType::ActivePlayer.index()
                    };
                    analysis.required[pi].push(node_idx);
                }

                Opcode::PopVarp | Opcode::PopVarbit => {
                    let pi = if is_secondary_var_operand(&instr.operand) {
                        PointerType::ActivePlayer2.index()
                    } else {
                        PointerType::ActivePlayer.index()
                    };
                    analysis.required[pi].push(node_idx);
                }

                Opcode::PushVarn | Opcode::PopVarn => {
                    let pi = if is_secondary_var_operand(&instr.operand) {
                        PointerType::ActiveNpc2.index()
                    } else {
                        PointerType::ActiveNpc.index()
                    };
                    analysis.required[pi].push(node_idx);
                }

                // PushVars/PopVars are global server vars — no pointer requirements
                _ => {}
            }
        }
    }

    fn apply_holder(&self, analysis: &mut ScriptAnalysis, node_idx: usize, holder: &PointerHolder) {
        for ptr in holder.required.iter() {
            let pi = ptr.index();
            analysis.required[pi].push(node_idx);
        }
        for ptr in holder.set.iter() {
            let pi = ptr.index();
            analysis.set_list[pi].push(node_idx);
            analysis.set_nodes[pi].insert(node_idx);
        }
        for ptr in holder.corrupted.iter() {
            let pi = ptr.index();
            analysis.corrupted_list[pi].push(node_idx);
            analysis.corrupted_nodes[pi].insert(node_idx);
        }
    }

    // -----------------------------------------------------------------------
    // Callee pointer resolution
    // -----------------------------------------------------------------------

    /// Get the merged pointer requirements/effects of a called script.
    ///
    /// Uses graph reachability (matching the TS `calculatePointers`):
    /// - **requires**: is there a path from a required-node backwards to graph[0]
    ///   without passing through a setter? If yes, the script needs this pointer
    ///   from its caller.
    /// - **sets**: is there any return-path that reaches graph[0] or a corruptor
    ///   without passing through a setter? If NO such path, the script guarantees
    ///   the pointer is set on all exit paths.
    /// - **corrupts**: is there a return-path that reaches a corruptor without
    ///   passing through a setter? If yes, the script may corrupt this pointer.
    fn get_script_pointers(&mut self, script_id: i32) -> PointerHolder {
        // Find script by ID
        let script_idx = match self.script_id_to_index.get(&script_id) {
            Some(&i) => i,
            None => {
                return PointerHolder::default();
            }
        };
        let script_name = self.scripts[script_idx].name.clone();

        // Check cache
        if let Some(holder) = self.script_pointers.get(&script_name) {
            return holder.clone();
        }

        // Cycle detection
        if self.pending_scripts.contains(&script_name) {
            return PointerHolder::default();
        }
        self.pending_scripts.insert(script_name.clone());

        // Ensure analysis exists
        self.ensure_analysis(&script_name);

        let mut holder = PointerHolder::default();

        if let Some(analysis) = self.script_analyses.get(&script_name) {
            for &ptr in &PointerType::ALL {
                let pi = ptr.index();

                // requiresPointerScript: BFS from required → graph[0], blocked by setters
                if !analysis.required[pi].is_empty() {
                    let path = Self::find_edge_path_static(
                        &analysis.nodes,
                        &analysis.required[pi],
                        |node_idx| node_idx == 0,
                        &analysis.set_nodes[pi],
                    );
                    if path.is_some() {
                        holder.required.insert(ptr);
                    }
                }

                // setsPointerScript: BFS from returns → (graph[0] OR corruptor), blocked by setters
                // If NO path found, script guarantees the pointer is set.
                if !analysis.returns.is_empty() {
                    let corrupted_nodes = &analysis.corrupted_nodes[pi];
                    let path = Self::find_edge_path_static(
                        &analysis.nodes,
                        &analysis.returns,
                        |node_idx| node_idx == 0 || corrupted_nodes.contains(&node_idx),
                        &analysis.set_nodes[pi],
                    );
                    if path.is_none() {
                        holder.set.insert(ptr);
                    }
                }

                // corruptsPointerScript: BFS from returns → corruptor, blocked by setters
                if !analysis.returns.is_empty() && !analysis.corrupted_nodes[pi].is_empty() {
                    let corrupted_nodes = &analysis.corrupted_nodes[pi];
                    let path = Self::find_edge_path_static(
                        &analysis.nodes,
                        &analysis.returns,
                        |node_idx| corrupted_nodes.contains(&node_idx),
                        &analysis.set_nodes[pi],
                    );
                    if path.is_some() {
                        holder.corrupted.insert(ptr);
                    }
                }
            }
        }

        self.pending_scripts.remove(&script_name);

        self.script_pointers
            .insert(script_name.clone(), holder.clone());
        holder
    }

    // -----------------------------------------------------------------------
    // Command name resolution
    // -----------------------------------------------------------------------

    /// Walk the callee's analyzed CFG and return every instruction that
    /// put `ptr` into `corrupted_list` — a direct `Command(p_delay)`,
    /// a `Gosub(~chatnpc)` whose callee transitively corrupts the
    /// pointer, etc. Each entry is `(display_name, source_line)`.
    /// Results are deduplicated by `(name, line)`.
    fn corruptors_in_callee(&self, callee_idx: usize, ptr: PointerType) -> Vec<(String, usize)> {
        let callee = &self.scripts[callee_idx];
        let analysis = match self.script_analyses.get(&callee.name) {
            Some(a) => a,
            None => return Vec::new(),
        };
        let ptr_idx = ptr.index();
        let mut seen: HashSet<(String, usize)> = HashSet::new();
        let mut result: Vec<(String, usize)> = Vec::new();

        for &node_idx in &analysis.corrupted_list[ptr_idx] {
            let instr_idx = match analysis.nodes[node_idx].instruction_index {
                Some(i) => i,
                None => continue,
            };
            let instr = &callee.instructions[instr_idx];
            let line = resolve_line(&callee.instructions, instr_idx);

            let name = match instr.opcode {
                Opcode::Command => match self.resolve_command_name(instr) {
                    Some(n) => n,
                    None => continue,
                },
                Opcode::Gosub | Opcode::GosubWithParams => match &instr.operand {
                    Operand::Int(id) => match self.script_id_to_index.get(id) {
                        Some(&target_idx) => short_script_name(&self.scripts[target_idx].name)
                            .map(|s| format!("~{}", s))
                            .unwrap_or_else(|| self.scripts[target_idx].name.clone()),
                        None => continue,
                    },
                    _ => continue,
                },
                // Jumps are terminal — they don't contribute to a
                // return-path corruption (the proc/label that @jumps out
                // is flagged via its own gosub/jump inheritance).
                _ => continue,
            };

            let key = (name.clone(), line);
            if seen.insert(key) {
                result.push((name, line));
            }
        }

        result
    }

    fn resolve_command_name(&self, instr: &Instruction) -> Option<String> {
        match &instr.operand {
            Operand::Int(encoded) => {
                let opcode = encoded & 0xFFFF;
                let secondary = (encoded >> 16) & 1;
                let base_name = self.opcode_to_name.get(&opcode)?;
                if secondary == 1 {
                    Some(format!(".{}", base_name))
                } else {
                    Some(base_name.clone())
                }
            }
            Operand::Str(name) => Some(name.to_lowercase()),
            _ => None,
        }
    }

    // -----------------------------------------------------------------------
    // Suggestion builder: Rule A — thread `last_*` through the callee
    // -----------------------------------------------------------------------
    //
    // Given a jump/gosub node that secondary-flagged for a static-only
    // pointer, produce a rustc-style Help with two Suggestions:
    //   1. rewrite the caller line to pass the captured pointer as an arg
    //   2. append a matching parameter to the callee's label/proc header
    //
    // Falls back to `None` when the source cache is absent or the callee
    // cannot be identified — the warning itself still fires.
    fn build_rule_a_help(
        &self,
        ptr: PointerType,
        caller_source_path: &str,
        caller_line: usize,
        node_idx: usize,
        caller_script_idx: usize,
    ) -> Option<Help> {
        let analysis = self
            .script_analyses
            .get(&self.scripts[caller_script_idx].name)?;
        let caller_instructions = &self.scripts[caller_script_idx].instructions;
        let instr_idx = analysis.nodes[node_idx].instruction_index?;
        let instr = &caller_instructions[instr_idx];

        // Only Jump/Gosub nodes drive Rule A.
        let (is_gosub, script_id) = match instr.opcode {
            Opcode::Jump | Opcode::JumpWithParams => (
                false,
                match &instr.operand {
                    Operand::Int(id) => *id,
                    _ => return None,
                },
            ),
            Opcode::Gosub | Opcode::GosubWithParams => (
                true,
                match &instr.operand {
                    Operand::Int(id) => *id,
                    _ => return None,
                },
            ),
            _ => return None,
        };

        let callee_idx = *self.script_id_to_index.get(&script_id)?;
        let callee = &self.scripts[callee_idx];
        let callee_source = callee.source_path.clone();

        // Derive the callee's `[trigger,name]` header line. The first
        // LineNumber instruction points at the *body's* first statement,
        // which is usually one line below the header — but not always
        // (blank lines, doc comments can shift it). Instead, scan the
        // source directly for the unambiguous `[<trigger>,<short>]`
        // pattern and fall back to the LineNumber estimate if the source
        // cache isn't attached.
        let first_body_line = callee
            .instructions
            .iter()
            .find_map(|i| match (i.opcode, &i.operand) {
                (Opcode::LineNumber, Operand::Int(n)) => Some(*n as usize),
                _ => None,
            })
            .unwrap_or(1);
        let callee_trigger = callee.trigger.clone();
        let callee_short_early = callee
            .name
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split_once(',')
            .map(|(_, n)| n.to_string())
            .unwrap_or_else(|| callee.name.clone());
        let callee_header_line = self
            .source_cache
            .and_then(|c| c.get(&callee_source))
            .and_then(|src| find_header_line(src, &callee_trigger, &callee_short_early))
            .unwrap_or_else(|| first_body_line.saturating_sub(1).max(1));

        let ptr_cmd = ptr.representation(); // "last_useitem", "last_item", …
        let param_type = pointer_param_type(ptr);
        let param_name = format!("${}", ptr_cmd);

        let callee_short = callee
            .name
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split_once(',')
            .map(|(_, n)| n.to_string())
            .unwrap_or_else(|| callee.name.clone());

        let arrow = if is_gosub { "~" } else { "@" };

        // Pull the specific corruptor(s) from the callee's own analysis
        // so the help message names what actually supersedes the pointer
        // (e.g. `p_delay` at line 21, or `~chatnpc` at line 11) rather
        // than saying "a delay or subroutine" generically.
        let corruptors = self.corruptors_in_callee(callee_idx, ptr);
        let corruptor_phrase = format_corruptors(&corruptors);

        let message = format!(
            "`{callee}` reads `{ptr}` then {phrase}; the script still \
             compiles and runs, but the reference is brittle to reorder. \
             Consider capturing `{ptr}` at the call site and threading it \
             through `{callee}` as an `{ty} {pname}` parameter so the \
             value is explicit and stable.",
            callee = callee_short,
            ptr = ptr_cmd,
            phrase = corruptor_phrase,
            ty = param_type,
            pname = param_name,
        );

        let cache = match self.source_cache {
            Some(c) => c,
            None => {
                // Prose-only fallback — still useful even without source text.
                return Some(Help {
                    message,
                    suggestions: Vec::new(),
                    applicability: Applicability::Unspecified,
                });
            }
        };

        let mut suggestions = Vec::new();

        // (1) Caller rewrite: insert the captured pointer as an argument
        // to the @label/~proc call on `caller_line`.
        if let Some(src) = cache.get(caller_source_path) {
            if let Some(line_text) = nth_source_line(src, caller_line) {
                if let Some(rewritten) =
                    rewrite_call_site_arg(line_text, arrow, &callee_short, ptr_cmd)
                {
                    suggestions.push(Suggestion {
                        file: PathBuf::from(caller_source_path),
                        line_range: (caller_line, caller_line),
                        replacement: rewritten,
                        label: Some(format!("at the call site (pass `{ptr_cmd}` as an arg)")),
                    });
                }
            }
        }

        // (2) Callee rewrite: append `<ty> <name>` to the header's param list.
        if let Some(src) = cache.get(&callee_source) {
            if let Some(header_text) = nth_source_line(src, callee_header_line) {
                if let Some(rewritten) =
                    rewrite_header_add_param(header_text, &callee_short, param_type, &param_name)
                {
                    suggestions.push(Suggestion {
                        file: PathBuf::from(&callee_source),
                        line_range: (callee_header_line, callee_header_line),
                        replacement: rewritten,
                        label: Some(format!(
                            "at the `{}` header (add `{} {}`)",
                            callee_short, param_type, param_name
                        )),
                    });
                }
            }
        }

        Some(Help {
            message,
            suggestions,
            // The rewrite may collide with an existing local named after the
            // pointer, or with an existing param list that already contains
            // the value under a different name — human review recommended.
            applicability: Applicability::MaybeIncorrect,
        })
    }
}

// ---------------------------------------------------------------------------
// Graph edge helpers
// ---------------------------------------------------------------------------

fn add_edge(nodes: &mut [CfgNode], from: usize, to: usize) {
    if !nodes[from].next.contains(&to) {
        nodes[from].next.push(to);
    }
    if !nodes[to].previous.contains(&from) {
        nodes[to].previous.push(from);
    }
}

/// The 2004scape engine tracks `active_player`, `active_npc`, `active_loc`,
/// `active_obj`, their `.`-prefixed secondary variants, and
/// `p_active_player`/`p_active_player2` via `ScriptState.pointerCheck` at
/// runtime. `last_*` and `find_*` pointers are *not* tracked at runtime —
/// they are a compile-time discipline only (see the `last_useitem` handler
/// in PlayerOps.ts, which gates on `state.trigger`, not a runtime pointer).
///
/// The "requires AND corrupts at the same node" heuristic (a Jump/Gosub
/// inheriting `required | corrupted` from its callee) produces real
/// findings for the static-only set but is typically noise for the
/// engine-tracked set — callers in that pattern rarely touch the pointer
/// after the jump, so the engine's runtime check never fires.
fn is_static_only_pointer(ptr: PointerType) -> bool {
    use PointerType::*;
    matches!(
        ptr,
        FindPlayer
            | FindNpc
            | FindLoc
            | FindObj
            | FindDb
            | LastCom
            | LastInt
            | LastItem
            | LastSlot
            | LastTargetslot
            | LastUseitem
            | LastUseslot
    )
}

/// Strip the `[trigger,name]` brackets from a script's full name and
/// return just the `name` half, for display in help messages.
fn short_script_name(full_name: &str) -> Option<String> {
    full_name
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split_once(',')
        .map(|(_, n)| n.to_string())
}

/// Render a list of `(name, line)` corruptors into the clause slot
/// in the help message. Empty → generic fallback; 1 → `calls `X` at line N`;
/// 2 → `calls `X` at line N and `Y` at line M`; 3+ → list with commas
/// and a trailing "and", truncated after 3 with "…".
fn format_corruptors(corruptors: &[(String, usize)]) -> String {
    if corruptors.is_empty() {
        return "calls a delay or subroutine that may supersede it".to_string();
    }

    const MAX: usize = 3;
    let shown: Vec<String> = corruptors
        .iter()
        .take(MAX)
        .map(|(name, line)| {
            if *line > 0 {
                format!("`{}` at line {}", name, line)
            } else {
                format!("`{}`", name)
            }
        })
        .collect();

    let suffix = if corruptors.len() > MAX {
        format!(" (and {} more)", corruptors.len() - MAX)
    } else {
        String::new()
    };

    let joined = match shown.len() {
        1 => shown[0].clone(),
        2 => format!("{} and {}", shown[0], shown[1]),
        _ => {
            let head = shown[..shown.len() - 1].join(", ");
            format!("{}, and {}", head, shown[shown.len() - 1])
        }
    };

    format!("calls {}{} which may supersede it", joined, suffix)
}

/// The RuneScript parameter type to use when threading a `last_*` pointer
/// through as a label argument. Mirrors the return types declared on the
/// corresponding engine commands in `content/scripts/engine.rs2`:
///   last_useitem/last_item -> obj
///   last_com               -> component
///   last_useslot/last_slot/last_targetslot/last_int -> int
/// Every other static-only pointer falls back to `int` (safe default —
/// the suggestion lands in `Applicability::MaybeIncorrect` anyway).
fn pointer_param_type(ptr: PointerType) -> &'static str {
    use PointerType::*;
    match ptr {
        LastUseitem | LastItem => "obj",
        LastCom => "component",
        _ => "int",
    }
}

/// Extract 1-indexed line `line_no` from `text`. `None` if out of range.
fn nth_source_line(text: &str, line_no: usize) -> Option<&str> {
    if line_no == 0 {
        return None;
    }
    text.lines().nth(line_no - 1)
}

/// Rewrite a caller line like `[oplocu,...] @label;` or
/// `@label(1);` to pass `last_useitem` (or whichever pointer) as an
/// additional argument. Returns `None` if the pattern isn't recognized.
///
/// Handles:
///   `@name;`                -> `@name(last_useitem);`
///   `@name(a, b);`          -> `@name(a, b, last_useitem);`
///   `~name;` / `~name(...)` (gosub) — same shape with `~`.
fn rewrite_call_site_arg(
    line: &str,
    arrow: &str,
    callee_short: &str,
    ptr_cmd: &str,
) -> Option<String> {
    // Locate `<arrow><callee_short>` as a call reference. The label may
    // appear multiple times on one line (rare) — we rewrite the first.
    let needle = format!("{}{}", arrow, callee_short);
    let start = line.find(&needle)?;
    let after_name = start + needle.len();
    let tail = &line[after_name..];

    // Case A: immediately followed by `;` or end-of-line — no existing args.
    let first_non_ws = tail.chars().next();
    if matches!(first_non_ws, None | Some(';') | Some('\r') | Some('\n'))
        || tail.trim_start().is_empty()
    {
        let mut out = String::with_capacity(line.len() + ptr_cmd.len() + 2);
        out.push_str(&line[..after_name]);
        out.push('(');
        out.push_str(ptr_cmd);
        out.push(')');
        out.push_str(tail);
        return Some(out);
    }

    // Case B: followed by `(` — locate the matching `)` and insert the arg
    // just before it.
    if tail.trim_start().starts_with('(') {
        let open_rel = tail.find('(')?;
        let abs_open = after_name + open_rel;
        let close_rel = find_matching_close_paren(&line[abs_open..])?;
        let abs_close = abs_open + close_rel;

        // Empty arg list: `@name()` -> `@name(last_useitem)`.
        let inside = line[abs_open + 1..abs_close].trim();
        let mut out = String::with_capacity(line.len() + ptr_cmd.len() + 2);
        out.push_str(&line[..abs_close]);
        if inside.is_empty() {
            out.push_str(ptr_cmd);
        } else {
            out.push_str(", ");
            out.push_str(ptr_cmd);
        }
        out.push_str(&line[abs_close..]);
        return Some(out);
    }

    None
}

/// Rewrite a label/proc header line to append `<ty> <name>` to its
/// parameter list. Handles all three forms:
///   `[label,foo]`              -> `[label,foo](obj $used)`
///   `[label,foo](int $x)`      -> `[label,foo](int $x, obj $used)`
///   `[proc,foo]()(int)`        -> `[proc,foo](obj $used)(int)`
fn rewrite_header_add_param(
    line: &str,
    callee_short: &str,
    param_type: &str,
    param_name: &str,
) -> Option<String> {
    // Locate the `,<callee_short>]` subject closer so we know where the
    // header ends and any parameter list begins.
    let needle = format!(",{}]", callee_short);
    let subject_end = line.find(&needle)?;
    let after_bracket = subject_end + needle.len();

    let remainder = &line[after_bracket..];
    let added = format!("{} {}", param_type, param_name);

    // No param list at all — append `(<ty> <name>)`.
    if !remainder.trim_start().starts_with('(') {
        let mut out = String::with_capacity(line.len() + added.len() + 2);
        out.push_str(&line[..after_bracket]);
        out.push('(');
        out.push_str(&added);
        out.push(')');
        out.push_str(remainder);
        return Some(out);
    }

    // Has at least one `(...)` — find its close and insert before it.
    let open_rel = remainder.find('(')?;
    let abs_open = after_bracket + open_rel;
    let close_rel = find_matching_close_paren(&line[abs_open..])?;
    let abs_close = abs_open + close_rel;
    let inside = line[abs_open + 1..abs_close].trim();

    let mut out = String::with_capacity(line.len() + added.len() + 2);
    out.push_str(&line[..abs_close]);
    if inside.is_empty() {
        out.push_str(&added);
    } else {
        out.push_str(", ");
        out.push_str(&added);
    }
    out.push_str(&line[abs_close..]);
    Some(out)
}

/// Locate the 1-indexed line that declares `[<trigger>,<name>]` in `src`.
/// Ignores leading whitespace; matches on the exact `[trigger,name]`
/// prefix (before any `(...)` parameter list).
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

/// Given a string whose first char is `(`, return the byte offset of the
/// matching `)`. Respects nesting but is deliberately naive about strings
/// (RuneScript headers don't contain string literals).
fn find_matching_close_paren(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.first().copied() != Some(b'(') {
        return None;
    }
    let mut depth = 0i32;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Game-var Push/Pop instructions encode the secondary (dot-prefixed,
/// `.%v`) vs primary (`%v`) distinction in bit 16 of the integer operand
/// (see `compiler.rs` Expr::GameVar handling: `(1 << 16) | var_id`).
/// TS uses distinct opcodes for this; we collapse to a single opcode and
/// recover the flag here when attributing the pointer requirement.
fn is_secondary_var_operand(operand: &Operand) -> bool {
    match operand {
        Operand::Int(encoded) => ((*encoded >> 16) & 1) == 1,
        _ => false,
    }
}

/// Resolve a jump target (an instruction index) to the CFG node index, walking
/// past any `LineNumber` instructions that are stripped from the CFG. Without
/// this, an `if` body's BranchEquals target — which usually lands on the body's
/// leading LineNumber — would fail the `instr_to_node.get()` lookup and the
/// edge would silently drop, disconnecting the body from its predecessors.
fn resolve_target_node(
    mut target_idx: usize,
    instructions: &[Instruction],
    instr_to_node: &HashMap<usize, usize>,
) -> Option<usize> {
    while target_idx < instructions.len() && instructions[target_idx].opcode == Opcode::LineNumber {
        target_idx += 1;
    }
    instr_to_node.get(&target_idx).copied()
}

fn remove_edge(nodes: &mut [CfgNode], from: usize, to: usize) {
    nodes[from].next.retain(|&n| n != to);
    nodes[to].previous.retain(|&n| n != from);
}

/// Find the source line number for an instruction by scanning backwards for
/// the nearest `LineNumber` instruction. Returns 0 if none found.
fn resolve_line(instructions: &[Instruction], instr_idx: usize) -> usize {
    for i in (0..=instr_idx).rev() {
        if instructions[i].opcode == Opcode::LineNumber {
            if let Operand::Int(line) = instructions[i].operand {
                return line as usize;
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{CompiledScript, Instruction, Opcode, Operand};
    use crate::symbol::SymbolRegistry;

    /// Build a minimal registry with a command registered at a given opcode.
    fn registry_with_commands(commands: &[(&str, i32)]) -> SymbolRegistry {
        let mut reg = SymbolRegistry::new();
        for &(name, opcode) in commands {
            reg.register_command(name.to_string(), opcode, vec![], vec![]);
        }
        reg
    }

    /// Build a compiled script with the given trigger, name, and instructions.
    fn make_script(
        name: &str,
        trigger: &str,
        id: i32,
        instructions: Vec<Instruction>,
    ) -> CompiledScript {
        let mut script = CompiledScript::new(format!("[{trigger},{name}]"), id);
        script.trigger = trigger.to_string();
        script.source_path = "test.rs2".to_string();
        script.instructions = instructions;
        script
    }

    /// Helper: Command instruction with encoded opcode (no secondary entity).
    fn cmd(opcode: i32) -> Instruction {
        Instruction::new(Opcode::Command, Operand::Int(opcode))
    }

    fn line(n: i32) -> Instruction {
        Instruction::new(Opcode::LineNumber, Operand::Int(n))
    }

    fn push_int(v: i32) -> Instruction {
        Instruction::push_int(v)
    }

    fn branch_eq(target: usize) -> Instruction {
        Instruction::new(Opcode::BranchEquals, Operand::JumpTarget(target))
    }

    fn branch(target: usize) -> Instruction {
        Instruction::new(Opcode::Branch, Operand::JumpTarget(target))
    }

    fn ret() -> Instruction {
        Instruction::simple(Opcode::Return)
    }

    fn gosub(script_id: i32) -> Instruction {
        Instruction::new(Opcode::GosubWithParams, Operand::Int(script_id))
    }

    // -----------------------------------------------------------------------
    // Test: conditional setter with argument push between Command and Branch
    // -----------------------------------------------------------------------
    // Simulates: if (p_finduid(uid) = true) { p_stopaction; }
    // Bytecode: PushVar, Command(p_finduid), PushConstantInt(1), BranchEquals, Branch, ...
    // The conditional setter should be detected despite the PushVar before Command.
    #[test]
    fn conditional_setter_with_arg_push() {
        // p_finduid opcode = 4267 (arbitrary, just needs to be in command_pointers)
        // p_stopaction opcode = 4268
        let p_finduid_op = 4267;
        let p_stopaction_op = 4268;

        let registry = registry_with_commands(&[
            ("p_finduid", p_finduid_op),
            ("p_stopaction", p_stopaction_op),
        ]);

        let instructions = vec![
            line(1),              // 0: LineNumber
            push_int(0),          // 1: PushVar(uid) - simulated as push_int
            cmd(p_finduid_op),    // 2: Command(p_finduid)
            push_int(1),          // 3: PushConstantInt(1) = true
            branch_eq(7),         // 4: BranchEquals -> body
            branch(8),            // 5: Branch -> end
            line(2),              // 6: LineNumber
            cmd(p_stopaction_op), // 7: Command(p_stopaction) - requires p_active_player
            ret(),                // 8: Return
        ];

        // Script with "label" trigger (gets all pointers initially? No - we want
        // to test that the conditional setter provides the pointer)
        // Use "ai_timer" which only provides active_npc — no p_active_player.
        let script = make_script("test", "ai_timer", 0, instructions);
        let scripts = vec![script];

        let mut checker = PointerChecker::new(&scripts, &registry);
        let diags = checker.run();

        // p_finduid conditionally sets p_active_player on the true branch.
        // p_stopaction requires p_active_player and is only on the true branch.
        // So this should produce NO diagnostics.
        assert_eq!(
            diags.warning_count(),
            0,
            "conditional setter should provide p_active_player on true branch, got {} warnings",
            diags.warning_count()
        );
    }

    // -----------------------------------------------------------------------
    // Test: proc that requires AND corrupts the same pointer (gnomeball pattern)
    // -----------------------------------------------------------------------
    // Simulates a label calling a proc that:
    //   - requires p_active_player (via p_delay)
    //   - then corrupts p_active_player (via npc_delay)
    // The Gosub node is simultaneously in required and corrupted lists.
    // This should NOT be flagged — the requirement is satisfied before corruption.
    #[test]
    fn gosub_requires_then_corrupts_same_pointer() {
        let p_delay_op = 5001;
        let npc_delay_op = 5002;

        let registry =
            registry_with_commands(&[("p_delay", p_delay_op), ("npc_delay", npc_delay_op)]);

        // Proc: p_delay(0); npc_delay(0); return;
        // This proc requires p_active_player (p_delay) then corrupts it (npc_delay).
        let proc_script = make_script(
            "tackle_success",
            "proc",
            100,
            vec![
                line(1),
                push_int(0),
                cmd(p_delay_op), // requires p_active_player, does NOT corrupt it
                line(2),
                push_int(0),
                cmd(npc_delay_op), // corrupts p_active_player
                ret(),
            ],
        );

        // Label: calls the proc via gosub
        // The label trigger provides all pointers including p_active_player.
        let label_script = make_script(
            "do_tackle",
            "label",
            200,
            vec![
                line(10),
                gosub(100), // Gosub to proc — requires AND corrupts p_active_player
                ret(),
            ],
        );

        let scripts = vec![proc_script, label_script];
        let mut checker = PointerChecker::new(&scripts, &registry);
        let diags = checker.run();

        // The label has p_active_player from trigger.
        // The gosub requires it (satisfied) then corrupts it.
        // No diagnostic — the main BFS starts from predecessors of the
        // required node (not the node itself), and the secondary require+
        // corrupt heuristic skips engine-tracked pointers like PActivePlayer.
        assert_eq!(
            diags.warning_count(),
            0,
            "gosub that requires-then-corrupts p_active_player should not \
             be flagged (engine-tracked pointer), got {} warnings",
            diags.warning_count()
        );
    }

    // -----------------------------------------------------------------------
    // Test: legitimate corruption SHOULD be caught
    // -----------------------------------------------------------------------
    // A command corrupts p_active_player, then a later command requires it.
    // This should produce an error.
    #[test]
    fn corruption_before_requirement_is_caught() {
        let npc_delay_op = 5002;
        let p_delay_op = 5001;

        let registry =
            registry_with_commands(&[("npc_delay", npc_delay_op), ("p_delay", p_delay_op)]);

        // Label: npc_delay(0); p_delay(0);
        // npc_delay corrupts p_active_player, then p_delay requires it.
        let script = make_script(
            "bad_order",
            "label",
            0,
            vec![
                line(1),
                push_int(0),
                cmd(npc_delay_op), // corrupts p_active_player
                line(2),
                push_int(0),
                cmd(p_delay_op), // requires p_active_player — should fail!
                ret(),
            ],
        );

        let scripts = vec![script];
        let mut checker = PointerChecker::new(&scripts, &registry);
        let diags = checker.run();

        assert!(
            diags.warning_count() > 0,
            "corruption before requirement should be caught as a warning"
        );
    }

    // -----------------------------------------------------------------------
    // Test: uninitialized pointer access
    // -----------------------------------------------------------------------
    // A script with a trigger that doesn't provide p_active_player uses p_delay.
    #[test]
    fn uninitialized_pointer_caught() {
        let p_delay_op = 5001;

        let registry = registry_with_commands(&[("p_delay", p_delay_op)]);

        // ai_timer trigger only provides active_npc, not p_active_player.
        let script = make_script(
            "bad_access",
            "ai_timer",
            0,
            vec![
                line(1),
                push_int(0),
                cmd(p_delay_op), // requires p_active_player — not provided by trigger
                ret(),
            ],
        );

        let scripts = vec![script];
        let mut checker = PointerChecker::new(&scripts, &registry);
        let diags = checker.run();

        assert!(
            diags.warning_count() > 0,
            "uninitialized pointer access should be caught"
        );
    }

    // -----------------------------------------------------------------------
    // Test: jump target landing on a LineNumber instruction
    // -----------------------------------------------------------------------
    // Regression: BranchEquals/Branch targets patched to body_start often
    // land on a LineNumber instruction (emitted for the body's first source
    // line). LineNumbers are stripped from the CFG, so without
    // resolve_target_node() the edge would be silently dropped and the body
    // node would have no predecessors — hiding genuine uninitialized-pointer
    // errors for callers that wrap the @jump in an `if` block.
    #[test]
    fn branch_target_on_line_number_still_connects() {
        let p_delay_op = 5001;
        let registry = registry_with_commands(&[("p_delay", p_delay_op)]);

        // ai_timer trigger provides active_npc but NOT p_active_player.
        // Layout: `if (1 = 1) { p_delay(0); }` — BranchEquals jumps to the
        // body's leading LineNumber. If the CFG drops that edge the body
        // node is disconnected and the uninitialized p_active_player read
        // is never flagged.
        //
        //   0 LineNumber(1)
        //   1 PushConstantInt(1)
        //   2 PushConstantInt(1)
        //   3 BranchEquals -> 5 (LineNumber at body start)
        //   4 Branch       -> 8 (end-of-if marker; here, Return)
        //   5 LineNumber(2)
        //   6 PushConstantInt(0)
        //   7 Command(p_delay)
        //   8 Return
        let script = make_script(
            "target_on_line_number",
            "ai_timer",
            0,
            vec![
                line(1),
                push_int(1),
                push_int(1),
                branch_eq(5), // jump target is the LineNumber at index 5
                Instruction::new(Opcode::Branch, Operand::JumpTarget(8)),
                line(2),
                push_int(0),
                cmd(p_delay_op),
                ret(),
            ],
        );

        let scripts = vec![script];
        let mut checker = PointerChecker::new(&scripts, &registry);
        let diags = checker.run();

        assert!(
            diags.warning_count() > 0,
            "jump target on LineNumber must still flag uninitialized pointer \
             access (got {} warnings)",
            diags.warning_count()
        );
    }

    // -----------------------------------------------------------------------
    // Test: secondary require+corrupt warning for static-only pointers
    // -----------------------------------------------------------------------
    // A label that requires AND corrupts LastUseitem (via last_useitem then
    // p_delay) is called via @jump from an oplocu trigger that provides
    // LastUseitem. The main BFS correctly finds no error (predecessors are
    // clean), but the secondary heuristic flags the Jump as "requires and
    // corrupts last_useitem on the same node" — this is the 23-warning
    // pattern from the legacy lost-city 225 output.
    #[test]
    fn secondary_requires_and_corrupts_warns_for_static_pointer() {
        // last_useitem and p_delay opcodes (arbitrary, just need to be in
        // command_pointers()).
        let last_useitem_op = 4270;
        let p_delay_op = 5001;
        let registry =
            registry_with_commands(&[("last_useitem", last_useitem_op), ("p_delay", p_delay_op)]);

        // Callee label: reads last_useitem, then corrupts it via p_delay.
        let callee = make_script(
            "consume_useitem",
            "label",
            100,
            vec![
                line(1),
                cmd(last_useitem_op), // requires LastUseitem
                line(2),
                push_int(0),
                cmd(p_delay_op), // corrupts LastUseitem
                ret(),
            ],
        );

        // Caller: oplocu trigger provides LastUseitem. Jumps to the label.
        // The Jump node inherits BOTH required[LastUseitem] AND
        // corrupted[LastUseitem]. Main BFS: predecessors of Jump trace back
        // to node 0, which is NOT corrupted (trigger provides) — no main
        // diagnostic. Secondary heuristic: Jump appears in both sets, emit
        // a warning.
        let caller = make_script(
            "use_on_loc",
            "oplocu",
            200,
            vec![
                line(10),
                Instruction::new(Opcode::JumpWithParams, Operand::Int(100)),
            ],
        );

        let scripts = vec![callee, caller];
        let mut checker = PointerChecker::new(&scripts, &registry);
        let diags = checker.run();

        let last_useitem_warnings = diags
            .diagnostics()
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .filter(|d| d.message.contains("last_useitem"))
            .count();
        assert!(
            last_useitem_warnings > 0,
            "static-only pointer (last_useitem) should trigger the \
             require+corrupt warning at the jump site; got diags: {:?}",
            diags
                .diagnostics()
                .iter()
                .map(|d| &d.message)
                .collect::<Vec<_>>()
        );

        assert_eq!(
            diags.error_count(),
            0,
            "pointer-check diagnostics must never raise error severity; got {} errors",
            diags.error_count()
        );
    }

    // -----------------------------------------------------------------------
    // Test: secondary heuristic SUPPRESSES warnings for engine-tracked pointers
    // -----------------------------------------------------------------------
    // Same pattern as above, but the pointer is PActivePlayer (engine-
    // tracked). The secondary heuristic must NOT emit — the user's legacy
    // 23-warning output specifically excludes the gnome_baller
    // p_active_player findings because the 2004scape engine manages that
    // pointer at runtime.
    #[test]
    fn secondary_requires_and_corrupts_silent_for_engine_pointer() {
        let p_delay_op = 5001;
        let npc_delay_op = 5002;
        let registry =
            registry_with_commands(&[("p_delay", p_delay_op), ("npc_delay", npc_delay_op)]);

        let callee = make_script(
            "consume_protected",
            "proc",
            100,
            vec![
                line(1),
                push_int(0),
                cmd(p_delay_op), // requires PActivePlayer
                line(2),
                push_int(0),
                cmd(npc_delay_op), // corrupts PActivePlayer
                ret(),
            ],
        );

        // opnpc1 provides ActivePlayer, PActivePlayer, ActiveNpc.
        let caller = make_script(
            "do_opnpc",
            "opnpc1",
            200,
            vec![
                line(10),
                Instruction::new(Opcode::GosubWithParams, Operand::Int(100)),
                ret(),
            ],
        );

        let scripts = vec![callee, caller];
        let mut checker = PointerChecker::new(&scripts, &registry);
        let diags = checker.run();

        let p_active_player_warnings = diags
            .diagnostics()
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .filter(|d| d.message.contains("p_active_player"))
            .count();
        assert_eq!(
            p_active_player_warnings, 0,
            "engine-tracked pointer (p_active_player) must not trigger the \
             require+corrupt warning; got {} such warnings",
            p_active_player_warnings
        );
    }

    // -----------------------------------------------------------------------
    // Test: secondary (dot-prefixed) var reads require the `*2` pointer
    // -----------------------------------------------------------------------
    // Regression: categorize_nodes used to map every PushVarn/PopVarn (and
    // PushVarp/PopVarp/PushVarbit/PopVarbit) to the primary active_npc /
    // active_player pointer, ignoring the bit-16 "secondary" flag that
    // compiler.rs sets on `.%var` operands. That miscategorization caused
    // the 4 spurious `uninitialized active_npc` warnings at
    // gnomeball_pass.rs2:1,2 and duel_arena.rs2:3,10.
    //
    // A proc reading `.%npc_var` should only require active_npc2 — no
    // active_npc warning should propagate to its callers.
    #[test]
    fn secondary_varn_requires_active_npc2_not_primary() {
        // PushVarn is Opcode::PushVarn (=4). Operand encodes var_id in low
        // 16 bits; bit 16 marks secondary. We bypass the varn registry by
        // emitting the raw opcode — the pointer checker only inspects the
        // opcode + operand.
        let secondary_varn = Instruction::new(
            Opcode::PushVarn,
            Operand::Int((1 << 16) | 0), // .%var_id 0, secondary
        );

        // Proc reads a secondary NPC var and returns.
        let proc_script = make_script(
            "reads_secondary_varn",
            "proc",
            100,
            vec![
                line(1),
                secondary_varn,
                Instruction::simple(Opcode::PopIntDiscard),
                ret(),
            ],
        );

        // Caller: opplayer1 trigger provides active_player/p_active_player/
        // active_player2, but does NOT provide active_npc or active_npc2.
        // Before the fix, the gosub inherited active_npc (primary) from the
        // proc and the caller would be flagged "uninitialized active_npc".
        // After the fix, the proc only requires active_npc2, which also
        // isn't provided — but crucially, `active_npc` is not flagged.
        let caller = make_script(
            "gosub_into_proc",
            "opplayer1",
            200,
            vec![line(10), gosub(100), ret()],
        );

        let scripts = vec![proc_script, caller];
        let registry = registry_with_commands(&[]);
        let mut checker = PointerChecker::new(&scripts, &registry);
        let diags = checker.run();

        // Diagnostic messages use the pointer's `representation()`:
        //   PointerType::ActiveNpc  -> "active_npc"
        //   PointerType::ActiveNpc2 -> ".active_npc"
        // Match "pointer active_npc." to count ONLY primary hits — the
        // secondary form appears as "pointer .active_npc.".
        let active_npc_primary_warnings = diags
            .diagnostics()
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .filter(|d| d.message.contains("pointer active_npc."))
            .count();
        assert_eq!(
            active_npc_primary_warnings,
            0,
            "secondary .%var read must not flag primary active_npc at the \
             caller; got diags: {:?}",
            diags
                .diagnostics()
                .iter()
                .filter(|d| d.severity == Severity::Warning)
                .map(|d| &d.message)
                .collect::<Vec<_>>()
        );

        // Sanity: the secondary requirement DID propagate — opplayer1 doesn't
        // provide active_npc2 either, so `.active_npc` should be flagged.
        // This proves the attribution went to the `*2` pointer as intended.
        let active_npc2_warnings = diags
            .diagnostics()
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .filter(|d| d.message.contains("pointer .active_npc."))
            .count();
        assert!(
            active_npc2_warnings > 0,
            "secondary .%var should route the requirement to active_npc2; \
             got no such warning"
        );
    }

    // -----------------------------------------------------------------------
    // Unit tests for the Rule A text rewriters
    // -----------------------------------------------------------------------

    #[test]
    fn rewrite_call_site_no_args() {
        let got = rewrite_call_site_arg(
            "[opnpcu,wantcat1] @civilian_give_runes;",
            "@",
            "civilian_give_runes",
            "last_useitem",
        )
        .unwrap();
        assert_eq!(got, "[opnpcu,wantcat1] @civilian_give_runes(last_useitem);");
    }

    #[test]
    fn rewrite_call_site_with_args() {
        let got = rewrite_call_site_arg(
            "[opheld4,amulet_of_glory_4] @amulet_of_glory_interface(\"msg\");",
            "@",
            "amulet_of_glory_interface",
            "last_item",
        )
        .unwrap();
        assert_eq!(
            got,
            "[opheld4,amulet_of_glory_4] @amulet_of_glory_interface(\"msg\", last_item);"
        );
    }

    #[test]
    fn rewrite_call_site_gosub_no_args() {
        let got =
            rewrite_call_site_arg("    ~some_proc;", "~", "some_proc", "last_useitem").unwrap();
        assert_eq!(got, "    ~some_proc(last_useitem);");
    }

    #[test]
    fn rewrite_call_site_empty_arg_list() {
        let got = rewrite_call_site_arg("@foo();", "@", "foo", "last_useitem").unwrap();
        assert_eq!(got, "@foo(last_useitem);");
    }

    #[test]
    fn rewrite_header_no_params() {
        let got = rewrite_header_add_param(
            "[label,civilian_give_runes]",
            "civilian_give_runes",
            "obj",
            "$last_useitem",
        )
        .unwrap();
        assert_eq!(got, "[label,civilian_give_runes](obj $last_useitem)");
    }

    #[test]
    fn rewrite_header_with_existing_params() {
        let got = rewrite_header_add_param(
            "[label,amulet_of_glory_interface](string $message)",
            "amulet_of_glory_interface",
            "obj",
            "$last_item",
        )
        .unwrap();
        assert_eq!(
            got,
            "[label,amulet_of_glory_interface](string $message, obj $last_item)"
        );
    }

    #[test]
    fn rewrite_header_proc_with_return_type() {
        // `[proc,foo]()(int)` — parameter list comes before the return
        // list. We must insert into the FIRST `(...)` group.
        let got = rewrite_header_add_param("[proc,foo]()(int)", "foo", "obj", "$x").unwrap();
        assert_eq!(got, "[proc,foo](obj $x)(int)");
    }

    #[test]
    fn find_header_line_matches_trigger_and_name() {
        let src = "// comment\n[label,foo]\nbody line\n[label,bar](int $x)\n";
        assert_eq!(find_header_line(src, "label", "foo"), Some(2));
        assert_eq!(find_header_line(src, "label", "bar"), Some(4));
        assert_eq!(find_header_line(src, "label", "missing"), None);
    }

    #[test]
    fn format_corruptors_shapes() {
        assert_eq!(
            format_corruptors(&[]),
            "calls a delay or subroutine that may supersede it"
        );
        assert_eq!(
            format_corruptors(&[("p_delay".to_string(), 21)]),
            "calls `p_delay` at line 21 which may supersede it"
        );
        assert_eq!(
            format_corruptors(&[("p_delay".to_string(), 21), ("~chatnpc".to_string(), 11),]),
            "calls `p_delay` at line 21 and `~chatnpc` at line 11 which may supersede it"
        );
        assert_eq!(
            format_corruptors(&[
                ("p_delay".to_string(), 21),
                ("~chatnpc".to_string(), 11),
                ("~objbox".to_string(), 14),
            ]),
            "calls `p_delay` at line 21, `~chatnpc` at line 11, and `~objbox` at line 14 which may supersede it"
        );
        // Truncates after 3 with a summary suffix.
        assert_eq!(
            format_corruptors(&[
                ("p_delay".to_string(), 1),
                ("~a".to_string(), 2),
                ("~b".to_string(), 3),
                ("~c".to_string(), 4),
                ("~d".to_string(), 5),
            ]),
            "calls `p_delay` at line 1, `~a` at line 2, and `~b` at line 3 (and 2 more) which may supersede it"
        );
        // Unknown line falls back to bare name.
        assert_eq!(
            format_corruptors(&[("p_delay".to_string(), 0)]),
            "calls `p_delay` which may supersede it"
        );
    }

    #[test]
    fn find_matching_close_paren_nested() {
        assert_eq!(find_matching_close_paren("()"), Some(1));
        assert_eq!(find_matching_close_paren("(a, b)"), Some(5));
        assert_eq!(find_matching_close_paren("(f(x), y)"), Some(8));
        assert_eq!(find_matching_close_paren("("), None);
        assert_eq!(find_matching_close_paren("abc"), None);
    }

    // -----------------------------------------------------------------------
    // Test: secondary require+corrupt warning carries a populated Help
    //       with concrete before/after suggestions when source is available
    // -----------------------------------------------------------------------
    #[test]
    fn secondary_warning_attaches_help_with_rewrite_suggestions() {
        use std::collections::HashMap;
        use std::rc::Rc;

        let last_useitem_op = 4270;
        let p_delay_op = 5001;
        let registry =
            registry_with_commands(&[("last_useitem", last_useitem_op), ("p_delay", p_delay_op)]);

        // Callee label — reads last_useitem, then p_delay corrupts it.
        let mut callee = make_script(
            "consume_useitem",
            "label",
            100,
            vec![
                line(2), // body starts on source line 2
                cmd(last_useitem_op),
                line(3),
                push_int(0),
                cmd(p_delay_op),
                ret(),
            ],
        );
        callee.source_path = "/virt/consume.rs2".to_string();

        // Caller — oplocu provides last_useitem; @jumps into callee.
        let mut caller = make_script(
            "use_on_loc",
            "oplocu",
            200,
            vec![
                line(1),
                Instruction::new(Opcode::JumpWithParams, Operand::Int(100)),
            ],
        );
        caller.source_path = "/virt/caller.rs2".to_string();

        // Virtual source text. Caller on line 1, callee header on line 1
        // of its own file.
        let mut source_cache: HashMap<String, Rc<String>> = HashMap::new();
        source_cache.insert(
            "/virt/caller.rs2".to_string(),
            Rc::new("[oplocu,target] @consume_useitem;\n".to_string()),
        );
        source_cache.insert(
            "/virt/consume.rs2".to_string(),
            Rc::new(
                "[label,consume_useitem]\n\
                 last_useitem;\n\
                 p_delay(0);\n"
                    .to_string(),
            ),
        );

        let scripts = vec![callee, caller];
        let mut checker = PointerChecker::new(&scripts, &registry);
        checker.set_source_cache(&source_cache);
        let diags = checker.run();

        // Find the corrupted-last_useitem warning and verify help is
        // attached with both call-site and header rewrites.
        let warning = diags
            .diagnostics()
            .iter()
            .find(|d| d.severity == Severity::Warning && d.message.contains("stale `last_useitem`"))
            .expect("expected a stale-last_useitem warning");

        assert_eq!(
            warning.help.len(),
            1,
            "warning should have exactly one Help block, got {}",
            warning.help.len()
        );
        let help = &warning.help[0];
        assert!(
            help.message.contains("consume_useitem"),
            "help message should name the callee, got: {}",
            help.message
        );
        assert!(
            help.message.contains("last_useitem"),
            "help message should name the pointer, got: {}",
            help.message
        );

        // Two suggestions: (1) call site, (2) label header.
        assert_eq!(
            help.suggestions.len(),
            2,
            "help should include both call-site and header suggestions, \
             got {}: {:?}",
            help.suggestions.len(),
            help.suggestions
                .iter()
                .map(|s| &s.label)
                .collect::<Vec<_>>()
        );

        // Caller rewrite lands on the call file.
        let caller_sug = help
            .suggestions
            .iter()
            .find(|s| s.file.to_string_lossy() == "/virt/caller.rs2")
            .expect("expected a call-site suggestion on the caller file");
        assert!(
            caller_sug
                .replacement
                .contains("@consume_useitem(last_useitem)"),
            "call-site replacement must pass last_useitem as an arg, got: {}",
            caller_sug.replacement
        );

        // Header rewrite lands on the callee file.
        let header_sug = help
            .suggestions
            .iter()
            .find(|s| s.file.to_string_lossy() == "/virt/consume.rs2")
            .expect("expected a header suggestion on the callee file");
        assert!(
            header_sug.replacement.contains("(obj $last_useitem)"),
            "header replacement must add `obj $last_useitem` param, got: {}",
            header_sug.replacement
        );

        assert_eq!(help.applicability, Applicability::MaybeIncorrect);
    }

    // -----------------------------------------------------------------------
    // Test: help falls back to prose-only when source cache is absent
    // -----------------------------------------------------------------------
    #[test]
    fn secondary_warning_falls_back_to_prose_help_without_source() {
        let last_useitem_op = 4270;
        let p_delay_op = 5001;
        let registry =
            registry_with_commands(&[("last_useitem", last_useitem_op), ("p_delay", p_delay_op)]);

        let callee = make_script(
            "consume_useitem",
            "label",
            100,
            vec![
                line(1),
                cmd(last_useitem_op),
                line(2),
                push_int(0),
                cmd(p_delay_op),
                ret(),
            ],
        );
        let caller = make_script(
            "use_on_loc",
            "oplocu",
            200,
            vec![
                line(1),
                Instruction::new(Opcode::JumpWithParams, Operand::Int(100)),
            ],
        );

        let scripts = vec![callee, caller];
        let mut checker = PointerChecker::new(&scripts, &registry);
        // Deliberately NO set_source_cache call.
        let diags = checker.run();

        let warning = diags
            .diagnostics()
            .iter()
            .find(|d| d.severity == Severity::Warning && d.message.contains("stale `last_useitem`"))
            .expect("warning still fires without source cache");

        assert_eq!(warning.help.len(), 1);
        let help = &warning.help[0];
        assert_eq!(
            help.suggestions.len(),
            0,
            "prose-only help should have no concrete suggestions"
        );
        assert_eq!(help.applicability, Applicability::Unspecified);
    }
}
