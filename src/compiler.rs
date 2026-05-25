use crate::bytecode::*;
use crate::parser::*;
use crate::symbol::{SymbolKind, SymbolRegistry, SymbolTable};
use crate::types::{BaseVarType, Type};

/// Code generator that transforms parsed AST into bytecode instructions.
pub struct Compiler {
    pub registry: SymbolRegistry,
    /// Return types of the currently-compiling script (used for type hints in return statements).
    current_return_types: Vec<Type>,
    /// The line number of the currently-compiling statement.
    current_stmt_line: usize,
}

impl Compiler {
    pub fn new(registry: SymbolRegistry) -> Self {
        Compiler {
            registry,
            current_return_types: Vec::new(),
            current_stmt_line: 0,
        }
    }

    /// Compile a single script declaration into bytecode.
    pub fn compile_script(&mut self, script: &ScriptDeclaration) -> CompiledScript {
        let id = self
            .registry
            .script_id_for_trigger(&script.trigger, &script.name)
            .or_else(|| self.registry.script_id(&script.name))
            .unwrap_or(-1);
        let full_name = format!("[{},{}]", script.trigger, script.name);
        let mut compiled = CompiledScript::new(full_name, id);
        compiled.trigger = script.trigger.clone();
        compiled.lookup_key =
            Self::compute_lookup_key(&script.trigger, &script.name, &self.registry);
        compiled.param_types = script.params.iter().map(|p| p.param_type).collect();
        let mut locals = SymbolTable::new();

        // Register parameters — push to LocalTable AND define in SymbolTable
        for param in &script.params {
            let slot = compiled
                .local_table
                .push_param(param.name.clone(), param.param_type);
            locals.define(
                param.name.clone(),
                SymbolKind::ScriptParam {
                    param_type: param.param_type,
                    slot,
                },
            );
        }

        // Compile body statements
        self.current_return_types = script.return_types.clone();
        for stmt in &script.body {
            self.compile_statement(stmt, &mut compiled, &mut locals);
        }

        // Always emit a fallthrough return at the end
        for ret_type in &script.return_types {
            match ret_type.base_type() {
                BaseVarType::Integer => {
                    compiled.push(Instruction::push_int(ret_type.default_return_value()))
                }
                BaseVarType::String => compiled.push(Instruction::push_string(String::new())),
                BaseVarType::Long => compiled.push(Instruction::push_long(0)),
            }
        }
        compiled.push(Instruction::simple(Opcode::Return));

        // Set local variable counts from LocalTable (matching TS getLocalCount/getParameterCount)
        compiled.int_local_count = compiled.local_table.get_local_count(BaseVarType::Integer);
        compiled.string_local_count = compiled.local_table.get_local_count(BaseVarType::String);
        compiled.long_local_count = compiled.local_table.get_local_count(BaseVarType::Long);
        compiled.int_arg_count = compiled.local_table.get_param_count(BaseVarType::Integer);
        compiled.string_arg_count = compiled.local_table.get_param_count(BaseVarType::String);
        compiled.long_arg_count = compiled.local_table.get_param_count(BaseVarType::Long);

        compiled
    }

    fn emit_line(line: usize, out: &mut CompiledScript) {
        out.push(Instruction::new(
            Opcode::LineNumber,
            Operand::Int(line as i32),
        ));
    }

    fn emit_comparison_value(
        branch_opcode: Opcode,
        fallthrough_value: i32,
        branch_value: i32,
        out: &mut CompiledScript,
    ) {
        let branch_pos = out.len();
        out.push(Instruction::new(branch_opcode, Operand::JumpTarget(0)));
        out.push(Instruction::push_int(fallthrough_value));
        let jump_pos = out.len();
        out.push(Instruction::new(Opcode::Branch, Operand::JumpTarget(0)));
        let branch_target = out.len();
        out.push(Instruction::push_int(branch_value));
        let end_pos = out.len();
        out.patch_jump(branch_pos, branch_target);
        out.patch_jump(jump_pos, end_pos);
    }

    fn compile_statement(
        &mut self,
        stmt: &Statement,
        out: &mut CompiledScript,
        locals: &mut SymbolTable,
    ) {
        // Track the statement's line for re-emission after multi-line call arguments.
        match stmt {
            Statement::VarDeclaration { line, .. }
            | Statement::ArrayDeclaration { line, .. }
            | Statement::Assignment { line, .. }
            | Statement::If { line, .. }
            | Statement::While { line, .. }
            | Statement::Switch { line, .. }
            | Statement::Return { line, .. }
            | Statement::Expression { line, .. }
            | Statement::OrphanCase { line, .. } => {
                self.current_stmt_line = *line;
            }
            Statement::Empty => {}
        }
        match stmt {
            Statement::VarDeclaration {
                var_type,
                name,
                value,
                line,
            } => {
                // Push to LocalTable for slot assignment (matching TS locals.all.push)
                let slot = out.local_table.push_local(name.clone(), *var_type, false);
                // Also define in SymbolTable for scoped name resolution
                locals.define(
                    name.clone(),
                    SymbolKind::LocalVar {
                        var_type: *var_type,
                        slot,
                        is_array: false,
                    },
                );

                if let Some(init) = value {
                    // With initial value: LineNumber BEFORE the expression.
                    // The arg-based LineNumber emission in calls may override this via writer
                    // deduplication (same-PC rule keeps the last line number).
                    Self::emit_line(*line, out);
                    // Use the declared var type as a hint so identifiers resolve to the
                    // correct entity ID (e.g. `def_npc_stat $x = magic` → npc_stat id=5, not stat id=6).
                    self.compile_expr_hinted(init, out, locals, Some(*var_type));
                    // Emit LineNumber(stmt_line) again before the pop instruction.
                    // For multi-line init calls, this re-anchors the pop to the statement's line.
                    // Writer deduplication handles same-line cases (no extra entry emitted).
                    Self::emit_line(*line, out);
                } else {
                    // No initial value: Java pushes default BEFORE the LineNumber
                    match var_type.base_type() {
                        BaseVarType::Integer => {
                            out.push(Instruction::push_int(var_type.default_int_value()))
                        }
                        BaseVarType::String => out.push(Instruction::push_string(String::new())),
                        BaseVarType::Long => out.push(Instruction::push_long(0)),
                    }
                    Self::emit_line(*line, out);
                }

                // Pop into local variable slot
                let pop_opcode = match var_type.base_type() {
                    BaseVarType::Integer => Opcode::PopIntLocal,
                    BaseVarType::String => Opcode::PopStringLocal,
                    BaseVarType::Long => Opcode::PopLongLocal,
                };
                out.push(Instruction::new(pop_opcode, Operand::Int(slot)));
            }

            Statement::ArrayDeclaration {
                element_type,
                name,
                size,
                line,
            } => {
                Self::emit_line(*line, out);
                let slot = out
                    .local_table
                    .push_local(name.clone(), *element_type, true);
                locals.define(
                    name.clone(),
                    SymbolKind::LocalVar {
                        var_type: *element_type,
                        slot,
                        is_array: true,
                    },
                );

                // Push array size
                self.compile_expr(size, out, locals);
                out.push(Instruction::pop_int_local(slot));

                // Emit define array
                let type_char = match element_type.base_type() {
                    BaseVarType::Integer => b'i',
                    BaseVarType::String => b's',
                    BaseVarType::Long => b'l',
                };
                out.push(Instruction::new(
                    Opcode::DefineArray,
                    Operand::ArrayDef(slot, type_char),
                ));

                let _ = line;
            }

            Statement::Assignment {
                target,
                value,
                line,
            } => {
                Self::emit_line(*line, out);
                // Infer the target type to use as a hint when compiling the value.
                let target_type_hint = self.infer_target_type(target, locals);
                self.compile_expr_hinted(value, out, locals, target_type_hint);
                self.compile_store(target, out, locals);
            }

            Statement::If {
                condition,
                body,
                else_if,
                else_body,
                else_line,
                line,
            } => {
                Self::emit_line(*line, out);
                self.compile_if(condition, body, else_if, else_body, *else_line, out, locals);
            }

            Statement::While {
                condition,
                body,
                line,
            } => {
                Self::emit_line(*line, out);
                let loop_start = out.len();

                // Java pattern: BranchTrue(body) + Branch(exit) before body
                let (true_positions, false_positions) =
                    self.compile_condition(condition, out, locals);

                // Patch true branches to body_start
                let body_start = out.len();
                for pos in &true_positions {
                    out.patch_jump(*pos, body_start);
                }

                // Compile body
                for s in body {
                    self.compile_statement(s, out, locals);
                }

                // Jump back to loop start
                out.push(Instruction::new(
                    Opcode::Branch,
                    Operand::JumpTarget(loop_start),
                ));

                // Patch false branches to exit
                let exit_pos = out.len();
                for pos in &false_positions {
                    out.patch_jump(*pos, exit_pos);
                }
            }

            Statement::Switch {
                switch_type,
                expr,
                cases,
                default,
                default_index,
                line,
            } => {
                Self::emit_line(*line, out);
                self.compile_switch(
                    switch_type,
                    expr,
                    cases,
                    default,
                    *default_index,
                    out,
                    locals,
                );
            }

            Statement::Return { values, line } => {
                Self::emit_line(*line, out);
                let return_types = self.current_return_types.clone();
                for (i, val) in values.iter().enumerate() {
                    let hint = return_types.get(i).copied();
                    self.compile_expr_hinted(val, out, locals, hint);
                }
                out.push(Instruction::simple(Opcode::Return));
            }

            Statement::Expression { expr, line } => {
                // Java compiler behavior: for CommandCall expressions, leading ConstantVar
                // args that resolve to strings are pushed BEFORE the LineNumber instruction.
                let cmd_args_offset = if let Expr::CommandCall { name, args, .. } = expr {
                    let n = self.count_pre_stmt_string_args(name, args);
                    if n > 0 {
                        // The reference compiler emits LineNumber(stmt_line - 1) before the
                        // pre-stmt string args, but ONLY when the first pre-stmt arg is a
                        // string constant (^name). Null literals and string literals don't
                        // trigger the line-1 emission.
                        let first_is_const = matches!(&args[0], Expr::ConstantVar(..));
                        if first_is_const && *line > 1 {
                            Self::emit_line(*line - 1, out);
                        }
                        let lookup = name.strip_prefix('.').unwrap_or(name);
                        let ptypes: Vec<Type> = self
                            .registry
                            .command_param_types
                            .get(lookup)
                            .cloned()
                            .unwrap_or_default();
                        for (j, arg) in args[..n].iter().enumerate() {
                            let hint = ptypes.get(j).copied();
                            self.compile_expr_hinted(arg, out, locals, hint);
                        }
                    }
                    n
                } else {
                    0
                };
                Self::emit_line(*line, out);
                if cmd_args_offset > 0 {
                    if let Expr::CommandCall {
                        name,
                        args,
                        arg_lines,
                        call_line: _,
                    } = expr
                    {
                        self.compile_command_call_args_from(
                            name,
                            args,
                            arg_lines,
                            cmd_args_offset,
                            out,
                            locals,
                        );
                    }
                } else {
                    self.compile_expr(expr, out, locals);
                }
                // If the expression is a proc/label call that returns values in statement
                // context (unused), Java emits POP_INT/STRING_DISCARD for each return value.
                let return_types: Option<Vec<crate::types::Type>> = match expr {
                    Expr::ProcCall { name, .. } => {
                        self.registry.scripts.get(name.as_str()).and_then(|sym| {
                            if let SymbolKind::Script { return_types, .. } = &sym.kind {
                                Some(return_types.clone())
                            } else {
                                None
                            }
                        })
                    }
                    Expr::JumpCall { name, .. } => {
                        self.registry.scripts.get(name.as_str()).and_then(|sym| {
                            if let SymbolKind::Script { return_types, .. } = &sym.kind {
                                Some(return_types.clone())
                            } else {
                                None
                            }
                        })
                    }
                    Expr::CommandCall { name, .. } => {
                        // Strip leading dot for secondary entity commands
                        let lookup_name = name.strip_prefix('.').unwrap_or(name);
                        self.registry.commands.get(lookup_name).and_then(|sym| {
                            if let SymbolKind::Command { return_types, .. } = &sym.kind {
                                if return_types.is_empty() {
                                    None
                                } else {
                                    Some(return_types.clone())
                                }
                            } else {
                                None
                            }
                        })
                    }
                    _ => None,
                };
                if let Some(ret_types) = return_types {
                    for ret_type in &ret_types {
                        match ret_type.base_type() {
                            BaseVarType::Integer => {
                                out.push(Instruction::simple(Opcode::PopIntDiscard))
                            }
                            BaseVarType::String => {
                                out.push(Instruction::simple(Opcode::PopStringDiscard))
                            }
                            BaseVarType::Long => {
                                out.push(Instruction::simple(Opcode::PopLongDiscard))
                            }
                        }
                    }
                }
            }
            Statement::OrphanCase { .. } => {}
            Statement::Empty => {}
        }
    }

    #[allow(clippy::too_many_arguments, clippy::only_used_in_recursion)]
    fn compile_if(
        &mut self,
        condition: &Expr,
        body: &[Statement],
        else_if: &[(Expr, Vec<Statement>, usize)],
        else_body: &Option<Vec<Statement>>,
        else_line: usize,
        out: &mut CompiledScript,
        locals: &mut SymbolTable,
    ) {
        let mut end_jumps = Vec::new();

        // Java pattern: BranchTrue(body) + Branch(else/end)
        let (true_positions, false_positions) = self.compile_condition(condition, out, locals);

        // Patch true branches to body_start
        let body_start = out.len();
        for pos in &true_positions {
            out.patch_jump(*pos, body_start);
        }

        // Compile if body
        for s in body {
            self.compile_statement(s, out, locals);
        }

        // Always emit Branch(end) after body (Java always does this)
        let end_jump_pos = out.len();
        out.push(Instruction::new(Opcode::Branch, Operand::JumpTarget(0)));
        end_jumps.push(end_jump_pos);

        // Patch false branches to else_start (right after body's Branch)
        let else_start = out.len();
        for pos in &false_positions {
            out.patch_jump(*pos, else_start);
        }

        if !else_if.is_empty() {
            // Java compiles else-if as nested: else { if (cond) { ... } else if ... }
            // Each nesting level adds one extra Branch wrapper at the end.
            let (first_cond, first_body, first_line) = &else_if[0];
            let remaining = &else_if[1..];
            // Emit line number for the else-if condition (Java does this)
            Self::emit_line(*first_line, out);
            self.compile_if(
                first_cond, first_body, remaining, else_body, else_line, out, locals,
            );

            // Wrapper end-jump for this "else { ... }" level
            let wrapper_jump = out.len();
            out.push(Instruction::new(Opcode::Branch, Operand::JumpTarget(0)));
            end_jumps.push(wrapper_jump);
        } else if let Some(else_stmts) = else_body {
            // Plain else body
            for s in else_stmts {
                self.compile_statement(s, out, locals);
            }
            // Always emit Branch(end) after else body
            let end_jump_pos = out.len();
            out.push(Instruction::new(Opcode::Branch, Operand::JumpTarget(0)));
            end_jumps.push(end_jump_pos);
        }

        // Patch all end-jumps to here
        let end_pos = out.len();
        for pos in end_jumps {
            out.patch_jump(pos, end_pos);
        }
    }

    /// Count how many leading args are string-type expressions that should be pushed
    /// BEFORE LineNumber in statement context (Neptune behavior).
    /// This includes: string constants (^name), string literals, and null literals
    /// when the command parameter type is string.
    fn count_pre_stmt_string_args(&self, name: &str, args: &[Expr]) -> usize {
        // Look up command parameter types to determine if null args are string-typed
        let lookup_name = name.strip_prefix('.').unwrap_or(name);
        let param_types: Vec<Type> = self
            .registry
            .command_param_types
            .get(lookup_name)
            .cloned()
            .unwrap_or_default();

        let mut count = 0;
        for (i, arg) in args.iter().enumerate() {
            let is_string_param = param_types
                .get(i)
                .map(|t| t.base_type() == crate::types::BaseVarType::String)
                .unwrap_or(false);

            match arg {
                Expr::ConstantVar(cname, _) => {
                    if let Some(sym) = self.registry.lookup_constant(cname)
                        && let SymbolKind::Constant {
                            string_value: Some(_),
                            ..
                        } = &sym.kind
                    {
                        count += 1;
                        continue;
                    }
                    break;
                }
                Expr::NullLiteral if is_string_param => {
                    count += 1;
                    continue;
                }
                _ => break,
            }
        }
        count
    }

    /// Compile a CommandCall expression starting at `args_start` (skipping already-pushed args).
    fn compile_command_call_args_from(
        &mut self,
        name: &str,
        args: &[Expr],
        arg_lines: &[usize],
        args_start: usize,
        out: &mut CompiledScript,
        locals: &mut SymbolTable,
    ) {
        let (lookup_name, cmd_index) = if let Some(stripped) = name.strip_prefix('.') {
            (stripped, 1u8)
        } else {
            (name, 0u8)
        };
        let param_types: Vec<Type> = self
            .registry
            .command_param_types
            .get(lookup_name)
            .cloned()
            .unwrap_or_default();
        let script_trigger = Self::command_script_trigger(lookup_name);
        let variadic_info = Self::command_variadic_info(lookup_name);

        for (i, arg) in args.iter().enumerate() {
            if i < args_start {
                continue;
            }
            if let Some(&al) = arg_lines.get(i)
                && al > 0
            {
                Self::emit_line(al, out);
            }
            if i == 0
                && let Some(trigger) = script_trigger
                && let Expr::Identifier(script_name) = arg
            {
                let script_id = self
                    .registry
                    .script_id_for_trigger(trigger, script_name)
                    .or_else(|| self.registry.proc_script_id(script_name))
                    .or_else(|| self.registry.script_id(script_name))
                    .unwrap_or(-1);
                out.push(Instruction::push_int(script_id));
                continue;
            }
            let hint = param_types.get(i).copied();
            self.compile_expr_hinted(arg, out, locals, hint);
        }

        if let Some(fixed_arg_count) = variadic_info {
            let variadic_args = if args.len() > fixed_arg_count {
                &args[fixed_arg_count..]
            } else {
                &[]
            };
            let type_desc: String = variadic_args
                .iter()
                .map(|arg| self.infer_type_char(arg, locals))
                .collect();
            out.push(Instruction::push_string(type_desc));
        }

        if let Some(sym) = self.registry.lookup_command(lookup_name).cloned() {
            if let SymbolKind::Command { opcode, .. } = &sym.kind {
                let encoded = opcode | ((cmd_index as i32) << 16);
                out.push(Instruction::new(Opcode::Command, Operand::Int(encoded)));
            }
        } else {
            out.push(Instruction::new(
                Opcode::Command,
                Operand::Str(name.to_string()),
            ));
        }
    }

    /// Compile a condition expression emitting Java-style BranchTrue + Branch pair.
    /// Returns (true_branch_positions, false_branch_positions):
    ///   true_branch_positions  = indices of BranchXxx instructions that jump to the body
    ///   false_branch_positions = indices of Branch instructions that jump to the else/end
    fn compile_condition(
        &mut self,
        condition: &Expr,
        out: &mut CompiledScript,
        locals: &mut SymbolTable,
    ) -> (Vec<usize>, Vec<usize>) {
        match condition {
            Expr::BinaryOp { op, lhs, rhs } => {
                // Infer lhs type to use as hint for rhs (type-aware entity ID resolution).
                let lhs_type_hint = self.infer_type(lhs, locals);
                self.compile_expr(lhs, out, locals);
                self.compile_expr_hinted(rhs, out, locals, lhs_type_hint);

                // Emit positive branch (branch-if-true to body)
                let opcode = match op {
                    BinOp::Eq => Opcode::BranchEquals,
                    BinOp::NotEq => Opcode::BranchNot,
                    BinOp::Lt => Opcode::BranchLessThan,
                    BinOp::Gt => Opcode::BranchGreaterThan,
                    BinOp::LtEq => Opcode::BranchLessThanOrEquals,
                    BinOp::GtEq => Opcode::BranchGreaterThanOrEquals,
                    _ => Opcode::BranchNot,
                };
                let true_pos = out.len();
                out.push(Instruction::new(opcode, Operand::JumpTarget(0)));

                // Emit unconditional branch (fallthrough to else/end)
                let false_pos = out.len();
                out.push(Instruction::new(Opcode::Branch, Operand::JumpTarget(0)));

                (vec![true_pos], vec![false_pos])
            }

            Expr::LogicalOp {
                op,
                lhs,
                rhs,
                rhs_line,
            } => {
                match op {
                    LogicOp::And => {
                        // Short-circuit AND: if lhs is true continue to rhs, else → else/end
                        let (lhs_true, lhs_false) = self.compile_condition(lhs, out, locals);

                        // Patch lhs_true branches to rhs_start
                        let rhs_start = out.len();
                        for pos in &lhs_true {
                            out.patch_jump(*pos, rhs_start);
                        }

                        // Emit LineNumber for RHS if it starts on a different line.
                        // Writer deduplication handles same-line cases.
                        Self::emit_line(*rhs_line, out);
                        let (rhs_true, rhs_false) = self.compile_condition(rhs, out, locals);

                        // Combined: body reached only if rhs also true
                        // false paths: either lhs false OR rhs false
                        let mut false_branches = lhs_false;
                        false_branches.extend(rhs_false);
                        (rhs_true, false_branches)
                    }
                    LogicOp::Or => {
                        // Short-circuit OR: if lhs is true → body, else check rhs
                        let (lhs_true, lhs_false) = self.compile_condition(lhs, out, locals);

                        // Patch lhs_false branches to rhs_start
                        let rhs_start = out.len();
                        for pos in &lhs_false {
                            out.patch_jump(*pos, rhs_start);
                        }

                        // Emit LineNumber for RHS if it starts on a different line.
                        Self::emit_line(*rhs_line, out);
                        let (rhs_true, rhs_false) = self.compile_condition(rhs, out, locals);

                        // Combined: body reached if lhs true OR rhs true
                        let mut true_branches = lhs_true;
                        true_branches.extend(rhs_true);
                        (true_branches, rhs_false)
                    }
                }
            }

            // For any other expression, push value and 0, branch if value != 0 (true)
            _ => {
                self.compile_expr(condition, out, locals);
                out.push(Instruction::push_int(0));

                let true_pos = out.len();
                out.push(Instruction::new(Opcode::BranchNot, Operand::JumpTarget(0)));

                let false_pos = out.len();
                out.push(Instruction::new(Opcode::Branch, Operand::JumpTarget(0)));

                (vec![true_pos], vec![false_pos])
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn compile_switch(
        &mut self,
        switch_type: &str,
        expr: &Expr,
        cases: &[SwitchCase],
        default: &Option<Vec<Statement>>,
        default_index: usize,
        out: &mut CompiledScript,
        locals: &mut SymbolTable,
    ) {
        self.compile_expr(expr, out, locals);
        // Parse switch type to get type hint for case value resolution.
        let case_type_hint =
            Type::from_name(switch_type.strip_prefix("switch_").unwrap_or(switch_type));

        // Emit SWITCH instruction (table to be filled in later)
        let switch_idx = out.len();
        out.push(Instruction::new(
            Opcode::Switch,
            Operand::SwitchTable(Vec::new()),
        ));

        // Layout (matches reference compiler):
        //
        //   default_index == 0 (default appeared FIRST):
        //     [Switch]          ← on no-match, falls through to sw_pos+1 = default body
        //     [default body]    ← at sw_pos+1
        //     [Branch(end)]
        //     [case_0 body]     ← switch table points here
        //     [Branch(end)]
        //     ...
        //     [end_pos]
        //
        //   default_index > 0 (default appeared after some/all cases, or absent):
        //     [Switch]          ← on no-match, falls through to sw_pos+1 = Branch
        //     [Branch(to_default_or_end)]  ← at sw_pos+1
        //     [case_0 body]
        //     [Branch(end)]
        //     ...
        //     [default body]    ← emitted at source position among cases
        //     [Branch(end)]
        //     ...
        //     [end_pos]

        let mut end_jumps = Vec::new();
        // position of the no-match Branch (when default is not first)
        let default_first = default_index == 0 && default.is_some();

        let default_branch_pos = if default_first {
            // Default body at sw_pos+1 (direct fall-through, no Branch needed at sw_pos+1)
            if let Some(default_stmts) = default {
                for s in default_stmts {
                    self.compile_statement(s, out, locals);
                }
                let jump_pos = out.len();
                out.push(Instruction::new(Opcode::Branch, Operand::JumpTarget(0)));
                end_jumps.push(jump_pos);
            }
            None
        } else {
            // Branch at sw_pos+1 (to skip cases and reach default, or to end if no default)
            let pos = out.len();
            out.push(Instruction::new(Opcode::Branch, Operand::JumpTarget(0)));
            Some(pos)
        };

        // Compile case bodies in source order, inserting default at its original position
        let mut case_positions: Vec<(Vec<i32>, usize)> = Vec::new();
        let mut default_body_pos = 0usize;

        for (i, case) in cases.iter().enumerate() {
            // If default appeared at this position (and wasn't first), emit it now
            if !default_first && i == default_index {
                default_body_pos = out.len();
                if let Some(default_stmts) = default {
                    for s in default_stmts {
                        self.compile_statement(s, out, locals);
                    }
                    let jump_pos = out.len();
                    out.push(Instruction::new(Opcode::Branch, Operand::JumpTarget(0)));
                    end_jumps.push(jump_pos);
                }
            }

            let body_start = out.len();
            let mut values = Vec::new();
            for val in &case.values {
                if let Expr::IntLiteral(n) = val {
                    values.push(*n);
                } else if let Expr::ConstantVar(cname, _) = val {
                    // Resolve constant to integer for switch case value
                    if let Some(sym) = self.registry.constants.get(cname)
                        && let SymbolKind::Constant {
                            int_value: Some(value),
                            ..
                        } = &sym.kind
                    {
                        values.push(*value);
                    }
                } else if let Expr::Identifier(ident) = val {
                    // Component reference: interface:component → packed hash
                    let mut resolved = false;
                    if let Some((iface_name, comp_name)) = ident.split_once(':')
                        && let Some(packed) = self.registry.lookup_component(iface_name, comp_name)
                    {
                        values.push(packed);
                        resolved = true;
                    }
                    // Use switch type hint for type-aware resolution
                    if !resolved
                        && let Some(hint) = case_type_hint
                        && let Some(id) = self.registry.lookup_entity_id_typed(ident, hint)
                    {
                        values.push(id);
                        resolved = true;
                    }
                    if !resolved {
                        if let Some(sym) = self.registry.entity_ids.get(ident.as_str()) {
                            if let SymbolKind::Constant {
                                int_value: Some(value),
                                ..
                            } = &sym.kind
                            {
                                values.push(*value);
                            }
                        } else if let Some(sym) = self.registry.constants.get(ident.as_str())
                            && let SymbolKind::Constant {
                                int_value: Some(value),
                                ..
                            } = &sym.kind
                        {
                            values.push(*value);
                        }
                    }
                } else if let Expr::BinaryOp {
                    op: BinOp::Add,
                    lhs,
                    rhs,
                } = val
                {
                    // Handle entity names containing '+' (e.g. cheese+tom_batta)
                    if let (Expr::Identifier(lhs_name), Expr::Identifier(rhs_name)) =
                        (lhs.as_ref(), rhs.as_ref())
                    {
                        let combined = format!("{}+{}", lhs_name, rhs_name);
                        if let Some(hint) = case_type_hint {
                            if let Some(id) = self.registry.lookup_entity_id_typed(&combined, hint)
                            {
                                values.push(id);
                            }
                        } else if let Some(sym) = self.registry.lookup_entity_id(&combined).cloned()
                            && let SymbolKind::Constant {
                                int_value: Some(id),
                                ..
                            } = &sym.kind
                        {
                            values.push(*id);
                        }
                    }
                } else if let Expr::CoordLiteral(v) = val {
                    values.push(*v);
                }
            }
            case_positions.push((values, body_start));

            for s in &case.body {
                self.compile_statement(s, out, locals);
            }

            // Jump to end of switch (after all cases)
            let jump_pos = out.len();
            out.push(Instruction::new(Opcode::Branch, Operand::JumpTarget(0)));
            end_jumps.push(jump_pos);
        }

        // If default appeared after ALL explicit cases (default_index == cases.len()), emit it now
        if !default_first && default_index >= cases.len() {
            default_body_pos = out.len();
            if let Some(default_stmts) = default {
                for s in default_stmts {
                    self.compile_statement(s, out, locals);
                }
                let jump_pos = out.len();
                out.push(Instruction::new(Opcode::Branch, Operand::JumpTarget(0)));
                end_jumps.push(jump_pos);
            }
        }

        let end_pos = out.len();

        // Patch the no-match Branch (when default is not first)
        if let Some(dbp) = default_branch_pos {
            let default_target = if default.is_some() {
                default_body_pos
            } else {
                end_pos
            };
            out.patch_jump(dbp, default_target);
        }

        // Build the switch table with body positions
        let mut table = Vec::new();
        for (values, pos) in &case_positions {
            for &v in values {
                table.push((v, *pos));
            }
        }

        // Update the SWITCH instruction with the complete table
        out.instructions[switch_idx].operand = Operand::SwitchTable(table);

        // Patch end-of-case jumps to end_pos
        for pos in end_jumps {
            out.patch_jump(pos, end_pos);
        }
    }

    fn compile_expr(&mut self, expr: &Expr, out: &mut CompiledScript, locals: &mut SymbolTable) {
        self.compile_expr_hinted(expr, out, locals, None);
    }

    fn compile_expr_hinted(
        &mut self,
        expr: &Expr,
        out: &mut CompiledScript,
        locals: &mut SymbolTable,
        type_hint: Option<Type>,
    ) {
        match expr {
            Expr::IntLiteral(n) => {
                out.push(Instruction::push_int(*n));
            }

            Expr::LongLiteral(n) => {
                out.push(Instruction::push_long(*n));
            }

            Expr::StringLiteral(s) => {
                if let Some(h) = type_hint
                    && h.base_type() == BaseVarType::Integer
                    && h != Type::Int
                    && h != Type::Boolean
                {
                    if let Some(id) = self.registry.lookup_entity_id_typed(s, h) {
                        out.push(Instruction::push_int(id));
                        return;
                    }
                    if let Some(sym) = self.registry.lookup_entity_id(s)
                        && let SymbolKind::Constant {
                            int_value: Some(id),
                            ..
                        } = &sym.kind
                    {
                        out.push(Instruction::push_int(*id));
                        return;
                    }
                }
                out.push(Instruction::push_string(s.clone()));
            }

            Expr::BoolLiteral(b) => {
                out.push(Instruction::push_int(if *b { 1 } else { 0 }));
            }

            Expr::NullLiteral => {
                // For string-typed parameters, null is the string "null".
                // For all other types, null is integer -1.
                if matches!(type_hint, Some(Type::String)) {
                    out.push(Instruction::push_string("null".to_string()));
                } else {
                    out.push(Instruction::push_int(-1));
                }
            }

            Expr::CharLiteral(c) => {
                out.push(Instruction::push_int(*c as i32));
            }

            Expr::CoordLiteral(c) => {
                out.push(Instruction::push_int(*c));
            }

            Expr::LocalVar(name, _var_line) => {
                if let Some(sym) = locals.lookup(name) {
                    let (slot, base) = match &sym.kind {
                        SymbolKind::LocalVar { var_type, slot, .. } => {
                            (*slot, var_type.base_type())
                        }
                        SymbolKind::ScriptParam { param_type, slot } => {
                            (*slot, param_type.base_type())
                        }
                        _ => (0, BaseVarType::Integer),
                    };
                    let opcode = match base {
                        BaseVarType::Integer => Opcode::PushIntLocal,
                        BaseVarType::String => Opcode::PushStringLocal,
                        BaseVarType::Long => Opcode::PushLongLocal,
                    };
                    out.push(Instruction::new(opcode, Operand::Int(slot)));
                } else {
                    // Variable not found - push 0 as fallback
                    out.push(Instruction::push_int(0));
                }
            }

            Expr::GameVar(name, _var_line) => {
                // Dot-prefixed game vars (e.g. .%tradepartner) address secondary entity.
                // Encoding: (1 << 16) | var_id in the operand.
                let (lookup_name, secondary) = if let Some(stripped) = name.strip_prefix('.') {
                    (stripped, true)
                } else {
                    (name.as_str(), false)
                };
                if let Some(sym) = self.registry.lookup_game_var(lookup_name).cloned()
                    && let SymbolKind::GameVar {
                        var_id, category, ..
                    } = &sym.kind
                {
                    let opcode = match category.as_str() {
                        "varp" => Opcode::PushVarp,
                        "varn" => Opcode::PushVarn,
                        "vars" => Opcode::PushVars,
                        "varbit" => Opcode::PushVarbit,
                        _ => Opcode::PushVarp,
                    };
                    let encoded_id = if secondary {
                        (1 << 16) | *var_id
                    } else {
                        *var_id
                    };
                    out.push(Instruction::new(opcode, Operand::Int(encoded_id)));
                }
            }

            Expr::ConstantVar(name, const_line) => {
                if let Some(sym) = self.registry.lookup_constant(name).cloned()
                    && let SymbolKind::Constant {
                        int_value,
                        string_value,
                        ..
                    } = &sym.kind
                {
                    if let Some(v) = int_value {
                        out.push(Instruction::push_int(*v));
                    } else if let Some(s) = string_value {
                        // String constants: emit line - 1 to match Java compiler.
                        // The Java type checker resolves ^constant to a StringLiteral
                        // with NodeSourceLocation(line - 1, col - 1), converting from
                        // 1-based ANTLR lines to a 0-based offset. Integer constants
                        // are re-parsed via ANTLR with the offset, so they keep the
                        // original line (1 + (line-1) = line).
                        if *const_line > 0 {
                            Self::emit_line(*const_line - 1, out);
                        }
                        out.push(Instruction::push_string(s.clone()));
                    }
                }
            }

            Expr::Identifier(name) => {
                // Component reference: interface_name:component_name → (iface_id << 16) | comp_id
                if let Some((iface_name, comp_name)) = name.split_once(':')
                    && let Some(packed) = self.registry.lookup_component(iface_name, comp_name)
                {
                    out.push(Instruction::push_int(packed));
                    return;
                }

                // Resolution order: entity IDs (stat/npc/loc/etc.) > constants (.constant)
                // > commands > game vars > script IDs > fallback -1.
                // This matches Java: plain identifiers use entity IDs, not .constant overrides.
                //
                // When a type hint is available (propagated from the enclosing command call's
                // parameter types), prefer the type-specific entity ID. This handles cases where
                // the same name exists in multiple pack files (e.g. `smokepuff` in both synth.pack
                // and spotanim.pack) — the correct ID is chosen based on the expected parameter type.
                //
                // Script reference type hints (proc, label, queue, etc.) bypass entity IDs and
                // go directly to trigger-specific script ID lookup.
                if let Some(hint) = type_hint {
                    let trigger_opt: Option<&'static str> = match hint {
                        Type::Proc => Some("proc"),
                        Type::Label => Some("label"),
                        Type::Queue => Some("queue"),
                        Type::SoftTimer => Some("softtimer"),
                        Type::Timer => Some("timer"),
                        Type::Walktrigger => Some("walktrigger"),
                        _ => None,
                    };
                    if let Some(trigger) = trigger_opt {
                        let script_id = self
                            .registry
                            .script_id_for_trigger(trigger, name)
                            .or_else(|| self.registry.proc_script_id(name))
                            .or_else(|| self.registry.script_id(name))
                            .unwrap_or(-1);
                        out.push(Instruction::push_int(script_id));
                        return;
                    }
                    if let Some(id) = self.registry.lookup_entity_id_typed(name, hint) {
                        out.push(Instruction::push_int(id));
                        return;
                    }
                }
                if let Some(sym) = self.registry.lookup_entity_id(name).cloned()
                    && let SymbolKind::Constant {
                        const_type,
                        int_value,
                        string_value,
                    } = &sym.kind
                {
                    // Param entities should NOT shadow type name identifiers unless
                    // we have a Param-typed hint. This allows "namedobj" etc. to resolve
                    // to their type chars in enum(int, namedobj, ...) contexts.
                    let skip_for_type_char = *const_type == Type::Param
                        && type_hint != Some(Type::Param)
                        && self.registry.type_chars.contains_key(name);
                    if !skip_for_type_char {
                        if let Some(v) = int_value {
                            out.push(Instruction::push_int(*v));
                        } else if let Some(s) = string_value {
                            out.push(Instruction::push_string(s.clone()));
                        }
                        // If skip_for_type_char, fall through to commands/type_chars below
                        return;
                    }
                }
                // Note: we fall through here only if entity lookup found a Param-type entity
                // that should be overridden by a type_char, or if entity lookup returned None.
                if let Some(sym) = self.registry.lookup_constant(name).cloned() {
                    if let SymbolKind::Constant {
                        int_value,
                        string_value,
                        ..
                    } = &sym.kind
                    {
                        if let Some(v) = int_value {
                            out.push(Instruction::push_int(*v));
                        } else if let Some(s) = string_value {
                            out.push(Instruction::push_string(s.clone()));
                        }
                    }
                } else {
                    let (lookup, idx) = if let Some(stripped) = name.strip_prefix('.') {
                        (stripped, 1u8)
                    } else {
                        (name.as_str(), 0u8)
                    };
                    let resolved = if let Some(sym) = self.registry.lookup_command(lookup).cloned()
                    {
                        if let SymbolKind::Command { opcode, .. } = &sym.kind {
                            let encoded = opcode | ((idx as i32) << 16);
                            out.push(Instruction::new(Opcode::Command, Operand::Int(encoded)));
                        }
                        true
                    } else {
                        false
                    };
                    if !resolved {
                        if let Some(sym) = self.registry.lookup_game_var(name).cloned()
                            && let SymbolKind::GameVar {
                                var_id, category, ..
                            } = &sym.kind
                        {
                            let push_op = match category.as_str() {
                                "varp" => Opcode::PushVarp,
                                "varn" => Opcode::PushVarn,
                                "vars" => Opcode::PushVars,
                                "varbit" => Opcode::PushVarbit,
                                _ => Opcode::PushVarp,
                            };
                            out.push(Instruction::new(push_op, Operand::Int(*var_id)));
                        } else if let Some(&type_char) = self.registry.type_chars.get(name) {
                            out.push(Instruction::push_int(type_char));
                        } else if let Some(script_id) = self
                            .registry
                            .proc_script_id(name)
                            .or_else(|| self.registry.script_id(name))
                        {
                            out.push(Instruction::push_int(script_id));
                        } else {
                            out.push(Instruction::push_int(-1));
                        }
                    }
                }
            }

            Expr::ArrayAccess { name, index } => {
                // Push index
                self.compile_expr(index, out, locals);

                // Push array element
                if let Some(sym) = locals.lookup(name) {
                    let slot = match &sym.kind {
                        SymbolKind::LocalVar { slot, .. } => *slot,
                        _ => 0,
                    };
                    out.push(Instruction::new(Opcode::PushArrayInt, Operand::Int(slot)));
                }
            }

            Expr::BinaryOp { op, lhs, rhs } => {
                // Handle entity names that contain '+' (e.g. "cheese+tom_batta").
                // The parser splits these into BinaryOp::Add, but they should resolve
                // as a single entity identifier.
                if matches!(op, BinOp::Add)
                    && let (Expr::Identifier(lhs_name), Expr::Identifier(rhs_name)) =
                        (lhs.as_ref(), rhs.as_ref())
                {
                    let combined = format!("{}+{}", lhs_name, rhs_name);
                    if let Some(sym) = self.registry.lookup_entity_id(&combined).cloned()
                        && let SymbolKind::Constant {
                            int_value: Some(id),
                            ..
                        } = &sym.kind
                    {
                        out.push(Instruction::push_int(*id));
                        return;
                    }
                }

                // For comparisons, infer the type of lhs to use as a hint for rhs.
                // This allows e.g. `$struct = blamish_oil` to correctly resolve
                // `blamish_oil` from struct.pack rather than a higher-priority pack.
                let lhs_type_hint = match op {
                    BinOp::Eq
                    | BinOp::NotEq
                    | BinOp::Lt
                    | BinOp::Gt
                    | BinOp::LtEq
                    | BinOp::GtEq => self.infer_type(lhs, locals),
                    _ => None,
                };
                self.compile_expr(lhs, out, locals);
                self.compile_expr_hinted(rhs, out, locals, lhs_type_hint);

                // Determine if this is a long operation based on operand types
                // For now, use int operations; type checker can refine
                let opcode = match op {
                    BinOp::Add => Opcode::Add,
                    BinOp::Sub => Opcode::Sub,
                    BinOp::Mul => Opcode::Multiply,
                    BinOp::Div => Opcode::Divide,
                    BinOp::Mod => Opcode::Modulo,
                    BinOp::BitAnd => Opcode::And,
                    BinOp::BitOr => Opcode::Or,
                    BinOp::Eq => {
                        Self::emit_comparison_value(Opcode::BranchEquals, 0, 1, out);
                        return;
                    }
                    BinOp::NotEq => {
                        Self::emit_comparison_value(Opcode::BranchEquals, 1, 0, out);
                        return;
                    }
                    BinOp::Lt => {
                        Self::emit_comparison_value(Opcode::BranchLessThan, 0, 1, out);
                        return;
                    }
                    BinOp::Gt => {
                        Self::emit_comparison_value(Opcode::BranchGreaterThan, 0, 1, out);
                        return;
                    }
                    BinOp::LtEq => {
                        Self::emit_comparison_value(Opcode::BranchLessThanOrEquals, 0, 1, out);
                        return;
                    }
                    BinOp::GtEq => {
                        Self::emit_comparison_value(Opcode::BranchGreaterThanOrEquals, 0, 1, out);
                        return;
                    }
                };
                out.push(Instruction::simple(opcode));
            }

            Expr::LogicalOp { op, lhs, rhs, .. } => {
                self.compile_expr(lhs, out, locals);
                self.compile_expr(rhs, out, locals);
                match op {
                    LogicOp::And => out.push(Instruction::simple(Opcode::And)),
                    LogicOp::Or => out.push(Instruction::simple(Opcode::Or)),
                }
            }

            Expr::Calc(inner) => {
                self.compile_expr(inner, out, locals);
            }

            Expr::CommandCall {
                name,
                args,
                arg_lines,
                call_line,
            } => {
                // Dot-prefixed commands address the secondary active entity (operand = 1).
                let (lookup_name, cmd_index) = if let Some(stripped) = name.strip_prefix('.') {
                    (stripped, 1u8)
                } else {
                    (name.as_str(), 0u8)
                };

                // Fetch per-position type hints from the engine command signature.
                let param_types: Vec<Type> = self
                    .registry
                    .command_param_types
                    .get(lookup_name)
                    .cloned()
                    .unwrap_or_default();

                // Commands that take a script reference as their first argument and
                // variadic extra arguments (described by a type-string pushed after them).
                let script_trigger = Self::command_script_trigger(lookup_name);
                let variadic_info = Self::command_variadic_info(lookup_name);

                // Compile arguments, with special handling for the script-name first arg.
                for (i, arg) in args.iter().enumerate() {
                    // Emit LineNumber before each arg to track multi-line calls.
                    if let Some(&al) = arg_lines.get(i)
                        && al > 0
                    {
                        Self::emit_line(al, out);
                    }
                    if i == 0
                        && let Some(trigger) = script_trigger
                    {
                        // First arg is a script name identifier: resolve to trigger-specific ID.
                        if let Expr::Identifier(script_name) = arg {
                            let script_id = self
                                .registry
                                .script_id_for_trigger(trigger, script_name)
                                .or_else(|| self.registry.proc_script_id(script_name))
                                .or_else(|| self.registry.script_id(script_name))
                                .unwrap_or(-1);
                            out.push(Instruction::push_int(script_id));
                            continue;
                        }
                    }
                    // For 'enum' command, args 0 and 1 are input/output type chars.
                    // Resolve plain identifiers as type chars before falling through to commands.
                    if (lookup_name == "enum"
                        || lookup_name == "enum2"
                        || lookup_name == "db_getfield"
                        || lookup_name == "db_find"
                        || lookup_name == "db_find_refine"
                        || lookup_name == "db_findbyindex"
                        || lookup_name == "db_find_with_count"
                        || lookup_name == "param")
                        && (i == 0 || i == 1)
                        && let Expr::Identifier(n) = arg
                        && let Some(&tc) = self.registry.type_chars.get(n.as_str())
                    {
                        out.push(Instruction::push_int(tc));
                        continue;
                    }
                    let hint = param_types.get(i).copied();
                    self.compile_expr_hinted(arg, out, locals, hint);
                }

                // Push the variadic type-descriptor string for commands that support it.
                if let Some(fixed_arg_count) = variadic_info {
                    let variadic_args = if args.len() > fixed_arg_count {
                        &args[fixed_arg_count..]
                    } else {
                        &[]
                    };
                    let type_desc: String = variadic_args
                        .iter()
                        .map(|arg| self.infer_type_char(arg, locals))
                        .collect();
                    out.push(Instruction::push_string(type_desc));
                }

                // db_find needs an implicit extra push_int argument: the Java BaseVarType
                // ordinal of the column's first field type (INT=0, LONG=1, STRING=2).
                if lookup_name == "db_find" || lookup_name == "db_find_refine" {
                    let base_type_ordinal = if let Some(Expr::Identifier(col_name)) = args.first() {
                        self.registry
                            .dbcolumn_types
                            .get(col_name.as_str())
                            .map(|t| match t.base_type() {
                                crate::types::BaseVarType::Integer => 0,
                                crate::types::BaseVarType::Long => 1,
                                crate::types::BaseVarType::String => 2,
                            })
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    out.push(Instruction::push_int(base_type_ordinal));
                }

                // Re-emit the call expression's line before the final command instruction.
                // The reference compiler does lineInstruction(expr) after visiting args.
                if *call_line > 0 {
                    Self::emit_line(*call_line, out);
                }

                // Look up command opcode and emit.
                // For variadic variants (name*), also check the "namevararg" form
                // used in command.pack.
                let cmd_sym = self
                    .registry
                    .lookup_command(lookup_name)
                    .or_else(|| {
                        lookup_name.strip_suffix('*').and_then(|base| {
                            let vararg_name = format!("{}vararg", base);
                            self.registry.lookup_command(&vararg_name)
                        })
                    })
                    .cloned();
                if let Some(sym) = cmd_sym {
                    if let SymbolKind::Command { opcode, .. } = &sym.kind {
                        let encoded = (*opcode) | ((cmd_index as i32) << 16);
                        out.push(Instruction::new(Opcode::Command, Operand::Int(encoded)));
                    }
                } else {
                    out.push(Instruction::new(
                        Opcode::Command,
                        Operand::Str(name.clone()),
                    ));
                }

                // Note: db_find and db_find_refine do NOT push a count value.
                // Only db_find_with_count and db_find_refine_with_count push a count.
            }

            Expr::ProcCall {
                name,
                args,
                arg_lines,
                call_line,
            } => {
                // Get parameter types from target proc for type-aware arg compilation
                let param_types: Vec<Type> = self
                    .registry
                    .lookup_script(name)
                    .and_then(|sym| {
                        if let SymbolKind::Script { param_types, .. } = &sym.kind {
                            Some(param_types.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();
                for (i, arg) in args.iter().enumerate() {
                    if let Some(&al) = arg_lines.get(i)
                        && al > 0
                    {
                        Self::emit_line(al, out);
                    }
                    let hint = param_types.get(i).copied();
                    self.compile_expr_hinted(arg, out, locals, hint);
                }

                // Re-emit call expression's line before final gosub.
                if *call_line > 0 {
                    Self::emit_line(*call_line, out);
                }

                // `~name(...)` resolves ONLY against `[proc,name]` — matching
                // RuneScriptTS. A silent fallback to any script with this name
                // (debugproc / label / timer) lets `~foo` bind to an unrelated
                // trigger; if that trigger's body also calls `~foo`, you get
                // infinite recursion at runtime. The type checker emits an
                // error diagnostic for unresolved procs, so `-1` here only
                // fires in paths that already produced an error.
                let script_id = self.registry.proc_script_id(name).unwrap_or(-1);
                out.push(Instruction::gosub_with_params(script_id));
            }

            Expr::JumpCall {
                name,
                args,
                arg_lines,
                call_line,
            } => {
                // Get parameter types from target label for type-aware arg compilation
                let param_types: Vec<Type> = self
                    .registry
                    .lookup_script(name)
                    .and_then(|sym| {
                        if let SymbolKind::Script { param_types, .. } = &sym.kind {
                            Some(param_types.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();
                for (i, arg) in args.iter().enumerate() {
                    if let Some(&al) = arg_lines.get(i)
                        && al > 0
                    {
                        Self::emit_line(al, out);
                    }
                    let hint = param_types.get(i).copied();
                    self.compile_expr_hinted(arg, out, locals, hint);
                }
                // Re-emit call expression's line before final jump.
                if *call_line > 0 {
                    Self::emit_line(*call_line, out);
                }

                let script_id = self
                    .registry
                    .label_script_id(name)
                    .or_else(|| self.registry.script_id(name))
                    .unwrap_or(-1);
                out.push(Instruction::new(
                    Opcode::JumpWithParams,
                    Operand::Int(script_id),
                ));
            }

            Expr::JoinedString { parts } => {
                let mut count = 0u32;
                for part in parts {
                    match part {
                        StringPart::Literal(s) => {
                            out.push(Instruction::push_string(s.clone()));
                            count += 1;
                        }
                        StringPart::Expr(e) => {
                            self.compile_expr(e, out, locals);
                            count += 1;
                        }
                    }
                }
                out.push(Instruction::join_string(count));
            }

            Expr::MultiAssign(..) => {
                // Multi-assign targets handled at statement level; no-op as expression
            }
        }
    }

    fn compile_store(&mut self, target: &Expr, out: &mut CompiledScript, locals: &mut SymbolTable) {
        match target {
            Expr::LocalVar(name, _var_line) => {
                if let Some(sym) = locals.lookup(name) {
                    let (slot, base) = match &sym.kind {
                        SymbolKind::LocalVar { var_type, slot, .. } => {
                            (*slot, var_type.base_type())
                        }
                        SymbolKind::ScriptParam { param_type, slot } => {
                            (*slot, param_type.base_type())
                        }
                        _ => (0, BaseVarType::Integer),
                    };
                    let opcode = match base {
                        BaseVarType::Integer => Opcode::PopIntLocal,
                        BaseVarType::String => Opcode::PopStringLocal,
                        BaseVarType::Long => Opcode::PopLongLocal,
                    };
                    out.push(Instruction::new(opcode, Operand::Int(slot)));
                }
            }

            Expr::GameVar(name, _var_line) => {
                // Dot-prefixed game vars (e.g. .%tradepartner) address secondary entity.
                // Encoding: (1 << 16) | var_id in the operand.
                let (lookup_name, secondary) = if let Some(stripped) = name.strip_prefix('.') {
                    (stripped, true)
                } else {
                    (name.as_str(), false)
                };
                if let Some(sym) = self.registry.lookup_game_var(lookup_name).cloned()
                    && let SymbolKind::GameVar {
                        var_id, category, ..
                    } = &sym.kind
                {
                    let opcode = match category.as_str() {
                        "varp" => Opcode::PopVarp,
                        "varn" => Opcode::PopVarn,
                        "vars" => Opcode::PopVars,
                        "varbit" => Opcode::PopVarbit,
                        _ => Opcode::PopVarp,
                    };
                    let encoded_id = if secondary {
                        (1 << 16) | *var_id
                    } else {
                        *var_id
                    };
                    out.push(Instruction::new(opcode, Operand::Int(encoded_id)));
                }
            }

            Expr::ArrayAccess { name, index } => {
                // For array store: push index, then value is already on stack
                self.compile_expr(index, out, locals);
                if let Some(sym) = locals.lookup(name) {
                    let slot = match &sym.kind {
                        SymbolKind::LocalVar { slot, .. } => *slot,
                        _ => 0,
                    };
                    out.push(Instruction::new(Opcode::PopArrayInt, Operand::Int(slot)));
                }
            }

            Expr::MultiAssign(targets, target_lines) => {
                // Multi-return: pop return values in reverse order (last target = top of stack)
                for (i, target) in targets.iter().enumerate().rev() {
                    // Emit LineNumber for each target's source line
                    if let Some(&tl) = target_lines.get(i)
                        && tl > 0
                    {
                        Self::emit_line(tl, out);
                    }
                    self.compile_store(target, out, locals);
                }
            }

            _ => {
                // Invalid store target - should be caught by type checker
            }
        }
    }

    /// Returns the trigger type for commands whose first argument is a script name.
    fn command_script_trigger(cmd_name: &str) -> Option<&'static str> {
        match cmd_name {
            "queue" | "queue*" | "longqueue" | "longqueue*" | "strongqueue" | "strongqueue*"
            | "weakqueue" | "weakqueue*" | "clearqueue" => Some("queue"),
            "softtimer" | "softtimer*" => Some("softtimer"),
            "settimer" | "settimer*" | "cleartimer" => Some("timer"),
            "walktrigger" => Some("walktrigger"),
            _ => None,
        }
    }

    /// Number of fixed (non-variadic) arguments for commands that push a type descriptor.
    /// Returns None for commands that don't use a type descriptor at all.
    fn command_variadic_info(cmd_name: &str) -> Option<usize> {
        match cmd_name {
            // queue/strongqueue/weakqueue have separate * variants (different opcodes)
            "queue*" | "strongqueue*" | "weakqueue*" => Some(2),
            "longqueue*" => Some(3),
            // settimer/softtimer are always variadic (no separate * variant)
            "settimer" | "softtimer" => Some(2),
            // runclientscript* has 1 fixed arg (script_id) + variadic params
            "runclientscript*" => Some(1),
            _ => None,
        }
    }

    /// Infer the RS2 type character for an expression (used for variadic type descriptors).
    fn infer_type_char(&self, expr: &Expr, locals: &SymbolTable) -> char {
        match expr {
            Expr::StringLiteral(_) | Expr::JoinedString { .. } => 's',
            Expr::LongLiteral(_) => 'l',
            Expr::IntLiteral(_)
            | Expr::BoolLiteral(_)
            | Expr::CoordLiteral(_)
            | Expr::CharLiteral(_) => 'i',
            Expr::LocalVar(name, _) | Expr::ArrayAccess { name, .. } => {
                if let Some(sym) = locals.lookup(name) {
                    match &sym.kind {
                        SymbolKind::LocalVar { var_type, .. } => self.type_to_type_char(*var_type),
                        SymbolKind::ScriptParam { param_type, .. } => {
                            self.type_to_type_char(*param_type)
                        }
                        _ => 'i',
                    }
                } else {
                    'i'
                }
            }
            Expr::GameVar(name, _var_line) => {
                if let Some(sym) = self.registry.lookup_game_var(name)
                    && let SymbolKind::GameVar { var_type, .. } = &sym.kind
                {
                    return self.type_to_type_char(*var_type);
                }
                'i'
            }
            Expr::CommandCall { name, .. } => {
                let lookup = name.trim_start_matches('.');
                if let Some(sym) = self.registry.lookup_command(lookup)
                    && let SymbolKind::Command { return_types, .. } = &sym.kind
                    && let Some(rt) = return_types.first()
                {
                    return self.type_to_type_char(*rt);
                }
                // Unknown or void command - check known command types
                Self::known_command_type_char(lookup)
            }
            Expr::ProcCall { name, .. } => {
                if let Some(sym) = self.registry.lookup_script(name)
                    && let SymbolKind::Script { return_types, .. } = &sym.kind
                    && let Some(rt) = return_types.first()
                {
                    return self.type_to_type_char(*rt);
                }
                'i'
            }
            Expr::Identifier(name) => {
                // Bare identifier (no parens) - might be a zero-arg command like `uid`
                let lookup = name.trim_start_matches('.');
                if let Some(sym) = self.registry.lookup_command(lookup)
                    && let SymbolKind::Command { return_types, .. } = &sym.kind
                    && let Some(rt) = return_types.first()
                {
                    return self.type_to_type_char(*rt);
                }
                Self::known_command_type_char(lookup)
            }
            Expr::ConstantVar(name, _) => {
                if let Some(sym) = self.registry.lookup_constant(name)
                    && let SymbolKind::Constant { const_type, .. } = &sym.kind
                {
                    return self.type_to_type_char(*const_type);
                }
                'i'
            }
            Expr::BinaryOp { .. } | Expr::Calc(_) => 'i',
            Expr::NullLiteral => 'i',
            _ => 'i',
        }
    }

    /// Infer the type of an assignment target (lvalue) for type-hinted value compilation.
    fn infer_target_type(&self, target: &Expr, locals: &SymbolTable) -> Option<Type> {
        match target {
            Expr::LocalVar(name, _) | Expr::ArrayAccess { name, .. } => {
                if let Some(sym) = locals.lookup(name) {
                    match &sym.kind {
                        SymbolKind::LocalVar { var_type, .. } => Some(*var_type),
                        SymbolKind::ScriptParam { param_type, .. } => Some(*param_type),
                        _ => None,
                    }
                } else {
                    None
                }
            }
            Expr::GameVar(name, _var_line) => {
                let lookup_name = if let Some(stripped) = name.strip_prefix('.') {
                    stripped
                } else {
                    name.as_str()
                };
                if let Some(sym) = self.registry.lookup_game_var(lookup_name)
                    && let SymbolKind::GameVar { var_type, .. } = &sym.kind
                {
                    return Some(*var_type);
                }
                None
            }
            _ => None,
        }
    }

    /// Infer the `Type` of an expression for use as a comparison type hint.
    fn infer_type(&self, expr: &Expr, locals: &SymbolTable) -> Option<Type> {
        match expr {
            Expr::LocalVar(name, _) | Expr::ArrayAccess { name, .. } => {
                if let Some(sym) = locals.lookup(name) {
                    match &sym.kind {
                        SymbolKind::LocalVar { var_type, .. } => Some(*var_type),
                        SymbolKind::ScriptParam { param_type, .. } => Some(*param_type),
                        _ => None,
                    }
                } else {
                    None
                }
            }
            Expr::GameVar(name, _var_line) => {
                let lookup_name = if let Some(stripped) = name.strip_prefix('.') {
                    stripped
                } else {
                    name.as_str()
                };
                if let Some(sym) = self.registry.lookup_game_var(lookup_name)
                    && let SymbolKind::GameVar { var_type, .. } = &sym.kind
                {
                    return Some(*var_type);
                }
                None
            }
            Expr::CommandCall { name, .. } => {
                let lookup = name.trim_start_matches('.');
                if let Some(sym) = self.registry.lookup_command(lookup)
                    && let SymbolKind::Command { return_types, .. } = &sym.kind
                {
                    return return_types.first().copied();
                }
                None
            }
            Expr::Identifier(name) => {
                // Zero-arg commands used without parens (e.g. `loc_category`)
                let lookup = name.trim_start_matches('.');
                if let Some(sym) = self.registry.lookup_command(lookup)
                    && let SymbolKind::Command { return_types, .. } = &sym.kind
                {
                    return return_types.first().copied();
                }
                None
            }
            _ => None,
        }
    }

    fn type_to_type_char(&self, t: Type) -> char {
        match t.base_type() {
            BaseVarType::String => 's',
            BaseVarType::Long => 'l',
            BaseVarType::Integer => {
                // Use the type_chars map for more precise type chars
                // (e.g., namedobj='O', playeruid='p')
                for (name, &tc) in &self.registry.type_chars {
                    if let Some(from_type) = Type::from_name(name)
                        && from_type == t
                    {
                        return (tc as u8) as char;
                    }
                }
                'i'
            }
        }
    }

    fn known_command_type_char(name: &str) -> char {
        // Commands whose return type is not in the sym file but is known
        match name {
            "uid" => 'p',     // PlayerUid
            "npc_uid" => 'u', // NpcUid
            "displayname" | "name" | "npc_name" | "oc_name" | "oc_cert" => 's',
            _ => 'i',
        }
    }

    /// Returns `Some(error message)` if `[trigger,entity_name]` resolved a
    /// trigger byte but failed to resolve `entity_name` to a real subject,
    /// which would silently produce an unreachable script (lookup_key = -1).
    /// Returns `None` for legitimate cases:
    ///   - bare `_` wildcard (global trigger, no subject)
    ///   - `_category` form (category lookup — caller validates separately)
    ///   - coord-shaped triggers (`zone`, `zoneexit`, `mapzone`, `mapzoneexit`)
    ///   - name-keyed triggers (`queue`, `timer`, `walktrigger`, `proc`, `label`, …)
    pub fn validate_trigger_subject(
        trigger: &str,
        entity_name: &str,
        registry: &SymbolRegistry,
    ) -> Option<String> {
        if entity_name.is_empty() || entity_name == "_" {
            return None;
        }
        // Category subjects are validated by their own path; skip.
        if entity_name.starts_with('_') {
            return None;
        }
        // Triggers whose subject is a coord, has no subject at all, or is
        // historically not validated (ai_queue/ai_timer) — see trigger_table.
        if !crate::trigger_table::validates_subject(trigger) {
            return None;
        }
        let key = Self::compute_lookup_key(trigger, entity_name, registry);
        if key == -1 {
            let hint = if entity_name.contains(':') {
                let (iface, comp) = entity_name.split_once(':').unwrap();
                format!(
                    "interface:component subject — check interface.pack for either an if3 entry \
                     `<iface_id>:<comp_id>=<comp_name>` (with iface_id matching `{iface}`) \
                     or an if1 entry `<flat_id>={iface}:{comp}`"
                )
            } else {
                "Likely a typo or missing pack entry (check interface.pack / npc.pack / obj.pack / loc.pack)".to_string()
            };
            return Some(format!(
                "[{trigger},{entity_name}] — subject `{entity_name}` did not resolve to an entity. \
                 {hint}. Script will compile but never dispatch."
            ));
        }
        None
    }

    /// Compute the CS2 Neptune lookup key for a script.
    /// Formula: `entity_id * 1024 + 512 + trigger_byte`, or -1 if not applicable.
    /// For global scripts (entity="_"), returns just `trigger_byte`.
    pub fn compute_lookup_key(trigger: &str, entity_name: &str, registry: &SymbolRegistry) -> i32 {
        // Trigger byte from the central trigger_table. Unknown triggers
        // (queue, softtimer, timer, walktrigger, proc, label, command,
        // clientscript, debugproc) are name-keyed and have no byte.
        let trigger_byte = match crate::trigger_table::byte(trigger) {
            Some(b) => b as i64,
            None => return -1,
        };

        // Global scripts with no specific entity return just the trigger byte
        if entity_name == "_" {
            return trigger_byte as i32;
        }

        // Coordinate-based triggers: parse "level_bx_by" format
        if matches!(trigger, "mapzone" | "mapzoneexit") {
            let parts: Vec<&str> = entity_name.splitn(4, '_').collect();
            if parts.len() >= 3
                && let (Ok(bx), Ok(by)) = (parts[1].parse::<i64>(), parts[2].parse::<i64>())
            {
                let entity_id: i64 = ((bx & 3) << 20) | (by << 6);
                return (entity_id * 1024 + 512 + trigger_byte) as i32;
            }
            return -1;
        }

        // Coordinate-based triggers: parse "level_bx_by_lx_lz" format
        if matches!(trigger, "zone" | "zoneexit") {
            let parts: Vec<&str> = entity_name.splitn(6, '_').collect();
            if parts.len() >= 5
                && let (Ok(bx), Ok(by), Ok(lx), Ok(lz)) = (
                    parts[1].parse::<i64>(),
                    parts[2].parse::<i64>(),
                    parts[3].parse::<i64>(),
                    parts[4].parse::<i64>(),
                )
            {
                let entity_id: i64 = ((bx & 3) << 20) | (by << 6) | (lx << 14) | lz;
                return (entity_id * 1024 + 512 + trigger_byte) as i32;
            }
            return -1;
        }

        // Category-based entities: entity names starting with '_' reference a category
        // (e.g. [opheldu,_alcoholic_drinks] → category "alcoholic_drinks").
        // These use offset 256 instead of 512 in the lookupKey formula.
        //
        // NOTE: the bare `_` (pure wildcard) case is handled earlier in
        // this function (entity_name == "_" → returns just trigger_byte).
        if let Some(cat_name) = entity_name.strip_prefix('_') {
            if let Some(sym) = registry.lookup_entity_id_typed(cat_name, Type::Category) {
                return (sym as i64 * 1024 + 256 + trigger_byte) as i32;
            }
            return -1;
        }

        // Component subject in `interface:component` form (e.g. `options:com_14`).
        // The subject is the packed `(iface_id << 16) | comp_id` registered by
        // symloader's `registry.components` map. Used by if_button / inv_button
        // triggers — their dispatch hashes iface+comp into 32 bits.
        if let Some((iface_name, comp_name)) = entity_name.split_once(':') {
            if let Some(packed) = registry.lookup_component(iface_name, comp_name) {
                return (packed as i64 * 1024 + 512 + trigger_byte) as i32;
            }
            return -1;
        }

        // Type-aware entity lookup so that e.g. "potato" resolves to loc_id
        // for [oploc2,potato] rather than the obj_id that has higher flat
        // priority. Trigger → entity-type routing lives in trigger_table.
        if let Some(entity_type) = crate::trigger_table::subject_type(trigger)
            && let Some(entity_id) = registry.lookup_entity_id_typed(entity_name, entity_type)
        {
            return (entity_id as i64 * 1024 + 512 + trigger_byte) as i32;
        }

        // Fallback: flat entity_ids lookup
        if let Some(sym) = registry.lookup_entity_id(entity_name)
            && let crate::symbol::SymbolKind::Constant {
                int_value: Some(entity_id),
                ..
            } = &sym.kind
        {
            return (*entity_id as i64 * 1024 + 512 + trigger_byte) as i32;
        }
        -1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Type;

    /// Build a registry seeded with a few entities and one interface+component
    /// pair, mimicking the shape symloader produces from interface.pack /
    /// npc.pack / obj.pack.
    fn seed_registry() -> SymbolRegistry {
        let mut r = SymbolRegistry::new();
        // Interface 261 = "options", with component 14 = "com_14".
        // Key uses normalized form (com14) matching what symloader produces.
        r.register_entity_id("options".into(), Type::Interface, 261);
        r.register_entity_id("com14".into(), Type::Component, 14);
        r.components
            .insert("options:com14".into(), (261 << 16) | 14);
        // An npc + obj entity for non-interface trigger tests.
        r.register_entity_id("man".into(), Type::Npc, 1);
        r.register_entity_id("bronze_axe".into(), Type::Obj, 1351);
        r
    }

    #[test]
    fn if_button_resolves_interface_component_form() {
        let r = seed_registry();
        let key = Compiler::compute_lookup_key("if_button", "options:com_14", &r);
        let expected = ((261i64 << 16 | 14) * 1024 + 512 + 147) as i32;
        assert_eq!(key, expected);
        // And the lookup-key wraps to match the engine's
        // ServerTriggerType::IfButton::lookup_key_subject(com_hash) where
        // com_hash = iface*65536 + comp.
        let com_hash = (261i64 << 16 | 14) as i32;
        let engine_key = (com_hash as i64 * 1024 + 512 + 147) as i32;
        assert_eq!(key, engine_key);
    }

    #[test]
    fn validate_warns_on_unresolved_if_button_subject() {
        let r = seed_registry();
        // Unknown component name — old bug shape (would silently never dispatch).
        let warning =
            Compiler::validate_trigger_subject("if_button", "options:com_does_not_exist", &r);
        assert!(
            warning.is_some(),
            "expected a warning for unresolved subject"
        );
        assert!(warning.unwrap().contains("did not resolve"));
    }

    #[test]
    fn validate_silent_on_resolvable_subject() {
        let r = seed_registry();
        assert!(Compiler::validate_trigger_subject("if_button", "options:com_14", &r).is_none());
        assert!(Compiler::validate_trigger_subject("opnpc1", "man", &r).is_none());
        assert!(Compiler::validate_trigger_subject("opheld1", "bronze_axe", &r).is_none());
    }

    #[test]
    fn validate_silent_on_wildcards_and_categories() {
        let r = seed_registry();
        // Bare `_` — global trigger, no subject.
        assert!(Compiler::validate_trigger_subject("opheld1", "_", &r).is_none());
        // `_category` form — caller validates separately.
        assert!(Compiler::validate_trigger_subject("opheldu", "_alcoholic_drinks", &r).is_none());
    }

    #[test]
    fn validate_silent_on_name_keyed_triggers() {
        let r = seed_registry();
        // queue/timer/walktrigger/proc are name-keyed; `lookup_key = -1` is correct.
        assert!(Compiler::validate_trigger_subject("queue", "my_queue", &r).is_none());
        assert!(Compiler::validate_trigger_subject("timer", "my_timer", &r).is_none());
        assert!(Compiler::validate_trigger_subject("proc", "my_proc", &r).is_none());
    }

    #[test]
    fn validate_silent_on_coord_triggers() {
        let r = seed_registry();
        // Coord-shaped names are parsed from the entity string itself.
        assert!(Compiler::validate_trigger_subject("zone", "0_50_50_10_10", &r).is_none());
    }

    #[test]
    fn validate_warns_on_unknown_npc_subject() {
        let r = seed_registry();
        let warning = Compiler::validate_trigger_subject("opnpc1", "not_a_real_npc", &r);
        assert!(warning.is_some());
    }
}
