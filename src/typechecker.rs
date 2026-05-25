use crate::diagnostic_messages as msg;
use crate::diagnostics::{DiagnosticsCollector, Phase};
use crate::parser::*;
use crate::symbol::{SymbolKind, SymbolRegistry, SymbolTable};
use crate::types::{BaseVarType, Type};
use std::path::{Path, PathBuf};

const LITERAL_TYPES: &[Type] = &[
    Type::Int,
    Type::Boolean,
    Type::Coord,
    Type::String,
    Type::Char,
    Type::Long,
];

fn is_literal_type(t: Type) -> bool {
    LITERAL_TYPES.contains(&t)
}

pub struct TypeChecker<'a> {
    pub diagnostics: DiagnosticsCollector,
    pub registry: &'a SymbolRegistry,
    file_path: PathBuf,
}

impl<'a> TypeChecker<'a> {
    pub fn new(registry: &'a SymbolRegistry) -> Self {
        TypeChecker {
            diagnostics: DiagnosticsCollector::new(),
            registry,
            file_path: PathBuf::new(),
        }
    }

    pub fn check_file(&mut self, file: &ScriptFile, file_path: &Path) {
        self.file_path = file_path.to_path_buf();
        for script in &file.scripts {
            self.check_script(script);
        }
    }

    fn check_script(&mut self, script: &ScriptDeclaration) {
        let mut locals = SymbolTable::new();

        // Check for duplicate parameter names and invalid parameter types
        let mut seen_params = std::collections::HashSet::new();
        for param in &script.params {
            if !seen_params.insert(param.name.clone()) {
                self.error(
                    script.line,
                    msg::fmt(msg::SCRIPT_LOCAL_REDECLARATION, &[&param.name]),
                );
            }
            // LOCAL_PARAMETER_INVALID_TYPE: certain types can't be used as parameters
            if !Self::type_allows_parameter(param.param_type) {
                self.error(
                    script.line,
                    msg::fmt(
                        msg::LOCAL_PARAMETER_INVALID_TYPE,
                        &[param.param_type.name()],
                    ),
                );
            }
            locals.define_param(param.name.clone(), param.param_type);
        }

        // Validate trigger constraints
        self.check_trigger_constraints(script);

        for stmt in &script.body {
            self.check_statement(stmt, &mut locals, Some(&script.return_types));
        }
    }

    // ──────────────────── Trigger validation ────────────────────
    // Matching TS ScriptRegistration: checkScriptParameters, checkScriptReturns,
    // checkScriptSubject

    fn check_trigger_constraints(&mut self, script: &ScriptDeclaration) {
        let trigger = script.trigger.as_str();

        // Triggers that don't allow parameters.
        // Most AP/OP/event triggers don't allow parameters (default in TS).
        // PROC, LABEL, DEBUGPROC, QUEUE, SOFTTIMER, TIMER all allow parameters.
        match trigger {
            "walktrigger" | "login" | "logout" if !script.params.is_empty() => {
                self.error(
                    script.line,
                    msg::fmt(msg::SCRIPT_TRIGGER_NO_PARAMETERS, &[trigger]),
                );
            }
            _ => {}
        }

        // Triggers with required parameter types
        if let Some(expected_params) = self.trigger_expected_params(trigger) {
            let actual: Vec<&str> = script.params.iter().map(|p| p.param_type.name()).collect();
            let expected: Vec<&str> = expected_params.iter().map(|t| t.name()).collect();
            if actual != expected && !expected.is_empty() {
                self.error(
                    script.line,
                    msg::fmt(
                        msg::SCRIPT_TRIGGER_EXPECTED_PARAMETERS,
                        &[trigger, &expected.join(", ")],
                    ),
                );
            }
        }

        // Triggers that don't allow return values (TS: allowReturns=false default).
        // Only PROC and LOGOUT have allowReturns=true.
        match trigger {
            "debugproc" | "label" | "queue" | "softtimer" | "timer" | "walktrigger" | "login"
                if !script.return_types.is_empty() =>
            {
                self.error(
                    script.line,
                    msg::fmt(msg::SCRIPT_TRIGGER_NO_RETURNS, &[trigger]),
                );
            }
            _ => {}
        }

        // Triggers with required return types
        if let Some(expected_returns) = self.trigger_expected_returns(trigger)
            && script.return_types != expected_returns
            && !expected_returns.is_empty()
        {
            let expected: Vec<&str> = expected_returns.iter().map(|t| t.name()).collect();
            self.error(
                script.line,
                msg::fmt(
                    msg::SCRIPT_TRIGGER_EXPECTED_RETURNS,
                    &[trigger, &expected.join(", ")],
                ),
            );
        }

        // Subject validation
        let subject = &script.name;
        match trigger {
            // Global-only triggers: subject must be the script name (no entity reference)
            "debugproc" | "command" => {}

            // Triggers that reference entity types — validate subject constraints
            "opnpc1" | "opnpc2" | "opnpc3" | "opnpc4" | "opnpc5" | "opobj1" | "opobj2"
            | "opobj3" | "opobj4" | "opobj5" | "oploc1" | "oploc2" | "oploc3" | "oploc4"
            | "oploc5" | "opplayer1" | "opplayer2" | "opplayer3" | "opplayer4" | "opplayer5"
            | "opnpct" | "opobjt" | "oploct" | "opplayert" | "opnpcu" | "opobju" | "oplocu"
            | "opplayeru" | "ai_opnpc1" | "ai_opnpc2" | "ai_opnpc3" | "ai_opnpc4" | "ai_opnpc5"
            | "ai_oploc1" | "ai_oploc2" | "ai_oploc3" | "ai_oploc4" | "ai_oploc5" | "ai_opobj1"
            | "ai_opobj2" | "ai_opobj3" | "ai_opobj4" | "ai_opobj5" | "ai_opplayer1"
            | "ai_opplayer2" | "ai_opplayer3" | "ai_opplayer4" | "ai_opplayer5" => {
                if subject == "_" {
                    // Global subject is allowed for entity triggers
                } else if subject.starts_with('_') {
                    // Category subject — allowed for entity triggers
                } else if subject.contains(' ') {
                    self.error(
                        script.line,
                        msg::fmt(msg::SCRIPT_SUBJECT_NO_SPACES, &[trigger]),
                    );
                }
            }

            // Triggers that only allow global subjects
            "login" | "logout" | "mapenter" | "mapleave" | "map_enter" | "worldinit"
            | "worldshutdown"
                if subject != "_" && !subject.is_empty() =>
            {
                self.error(
                    script.line,
                    msg::fmt(msg::SCRIPT_SUBJECT_ONLY_GLOBAL, &[trigger]),
                );
            }

            // Proc/label/queue: no spaces, no global-only constraint
            "proc" | "label" | "queue" | "softtimer" | "timer" => {
                if subject.contains(' ') {
                    self.error(
                        script.line,
                        msg::fmt(msg::SCRIPT_SUBJECT_NO_SPACES, &[trigger]),
                    );
                }
                if subject == "_" {
                    self.error(
                        script.line,
                        msg::fmt(msg::SCRIPT_SUBJECT_NO_GLOBAL, &[trigger]),
                    );
                }
                if subject.starts_with('_') && subject.len() > 1 {
                    self.error(
                        script.line,
                        msg::fmt(msg::SCRIPT_SUBJECT_NO_CATEGORY, &[trigger]),
                    );
                }
            }

            _ => {}
        }

        // SCRIPT_COMMAND_ONLY: '*' suffix is only for command triggers
        if script.name.ends_with('*') && trigger != "command" {
            self.error(script.line, msg::SCRIPT_COMMAND_ONLY.to_string());
        }
    }

    fn trigger_expected_params(&self, _trigger: &str) -> Option<Vec<Type>> {
        None
    }

    fn trigger_expected_returns(&self, _trigger: &str) -> Option<Vec<Type>> {
        None
    }

    fn type_allows_declaration(t: Type) -> bool {
        !matches!(t, Type::Void | Type::Error | Type::Any)
    }

    fn type_allows_array(t: Type) -> bool {
        !matches!(t, Type::Void | Type::Error | Type::Any | Type::Long)
    }

    fn type_allows_parameter(t: Type) -> bool {
        !matches!(t, Type::Void | Type::Error)
    }

    fn check_block(
        &mut self,
        stmts: &[Statement],
        parent: &SymbolTable,
        return_types: Option<&[Type]>,
    ) {
        let mut child = parent.new_child();
        for s in stmts {
            self.check_statement(s, &mut child, return_types);
        }
    }

    fn check_statement(
        &mut self,
        stmt: &Statement,
        locals: &mut SymbolTable,
        return_types: Option<&[Type]>,
    ) {
        match stmt {
            Statement::VarDeclaration {
                var_type,
                name,
                value,
                line,
            } => {
                // SCRIPT_LOCAL_REDECLARATION
                if locals.lookup(name).is_some() {
                    self.error(*line, msg::fmt(msg::SCRIPT_LOCAL_REDECLARATION, &[name]));
                }
                // LOCAL_DECLARATION_INVALID_TYPE
                if !Self::type_allows_declaration(*var_type) {
                    self.error(
                        *line,
                        msg::fmt(msg::LOCAL_DECLARATION_INVALID_TYPE, &[var_type.name()]),
                    );
                }

                if let Some(init_expr) = value {
                    let expr_type = self.infer_expr_type(init_expr, locals, *line, Some(*var_type));
                    if let Some(et) = expr_type
                        && !self.types_compatible(et, *var_type)
                    {
                        self.emit_type_mismatch(*line, et, *var_type);
                    }
                }
                locals.define_local(name.clone(), *var_type, false);
            }

            Statement::ArrayDeclaration {
                element_type,
                name,
                size,
                line,
            } => {
                // SCRIPT_LOCAL_REDECLARATION
                if locals.lookup(name).is_some() {
                    self.error(*line, msg::fmt(msg::SCRIPT_LOCAL_REDECLARATION, &[name]));
                }
                // LOCAL_DECLARATION_INVALID_TYPE + LOCAL_ARRAY_INVALID_TYPE
                if !Self::type_allows_declaration(*element_type) {
                    self.error(
                        *line,
                        msg::fmt(msg::LOCAL_DECLARATION_INVALID_TYPE, &[element_type.name()]),
                    );
                }
                if !Self::type_allows_array(*element_type) {
                    self.error(
                        *line,
                        msg::fmt(msg::LOCAL_ARRAY_INVALID_TYPE, &[element_type.name()]),
                    );
                }

                let size_type = self.infer_expr_type(size, locals, *line, Some(Type::Int));
                if let Some(st) = size_type
                    && !self.types_compatible(st, Type::Int)
                {
                    self.emit_type_mismatch(*line, st, Type::Int);
                }
                locals.define_local(name.clone(), *element_type, true);
            }

            Statement::Assignment {
                target,
                value,
                line,
            } => {
                let target_type = self.infer_expr_type(target, locals, *line, None);
                let value_type = self.infer_expr_type(value, locals, *line, target_type);
                if let (Some(tt), Some(vt)) = (target_type, value_type)
                    && !self.types_compatible(vt, tt)
                {
                    self.emit_type_mismatch(*line, vt, tt);
                }
            }

            Statement::If {
                condition,
                body,
                else_if,
                else_body,
                line,
                ..
            } => {
                self.check_condition(condition, locals, *line);
                self.check_block(body, locals, return_types);
                for (cond, stmts, ei_line) in else_if {
                    self.check_condition(cond, locals, *ei_line);
                    self.check_block(stmts, locals, return_types);
                }
                if let Some(else_stmts) = else_body {
                    self.check_block(else_stmts, locals, return_types);
                }
            }

            Statement::While {
                condition,
                body,
                line,
            } => {
                self.check_condition(condition, locals, *line);
                self.check_block(body, locals, return_types);
            }

            Statement::Switch {
                switch_type,
                expr,
                cases,
                default,
                line,
                ..
            } => {
                let type_name = switch_type.strip_prefix("switch_").unwrap_or(switch_type);
                let resolved_type = Type::from_name(type_name);
                if resolved_type.is_none() {
                    self.error(*line, msg::fmt(msg::GENERIC_INVALID_TYPE, &[type_name]));
                }
                let switch_ty = resolved_type.unwrap_or(Type::Error);

                // SWITCH_INVALID_TYPE: check if the type supports switch
                if switch_ty != Type::Error && !switch_ty.allow_switch() {
                    self.error(
                        *line,
                        msg::fmt(msg::SWITCH_INVALID_TYPE, &[switch_ty.name()]),
                    );
                }

                let expr_type = self.infer_expr_type(expr, locals, *line, Some(switch_ty));
                if let Some(et) = expr_type
                    && switch_ty != Type::Error
                    && !self.types_compatible(et, switch_ty)
                {
                    self.emit_type_mismatch(*line, et, switch_ty);
                }

                // SWITCH_DUPLICATE_DEFAULT: the parser only stores one default block,
                // so duplicate default detection happens at parse time.

                for case in cases {
                    self.check_switch_case(case, Some(switch_ty), locals, return_types, *line);
                }
                if let Some(default_stmts) = default {
                    self.check_block(default_stmts, locals, return_types);
                }
            }

            Statement::Return { values, line } => {
                let script_returns = match return_types {
                    Some(rt) => rt,
                    None => {
                        self.error(*line, msg::RETURN_ORPHAN.to_string());
                        return;
                    }
                };
                let is_single_proc_call = values.len() == 1
                    && matches!(&values[0], Expr::ProcCall { .. } | Expr::CommandCall { .. });
                if values.len() != script_returns.len() && !is_single_proc_call {
                    self.error(
                        *line,
                        msg::fmt(
                            msg::GENERIC_TYPE_MISMATCH,
                            &[
                                &format!("{} value(s)", values.len()),
                                &format!("{} value(s)", script_returns.len()),
                            ],
                        ),
                    );
                }
                for (i, val) in values.iter().enumerate() {
                    let hint = script_returns.get(i).copied();
                    let val_type = self.infer_expr_type(val, locals, *line, hint);
                    if let Some(vt) = val_type
                        && let Some(expected) = hint
                        && !self.types_compatible(vt, expected)
                    {
                        self.emit_type_mismatch(*line, vt, expected);
                    }
                }
            }

            Statement::Expression { expr, line } => {
                let expr_type = self.infer_expr_type(expr, locals, *line, None);
                if let Some(et) = expr_type
                    && et != Type::Error
                    && !self.expression_has_side_effects(expr)
                {
                    self.warning(*line, msg::EXPRESSION_STATEMENT_NO_SIDE_EFFECT.to_string());
                }
            }
            Statement::OrphanCase { case, line } => {
                self.check_switch_case(case, None, locals, return_types, *line);
            }
            Statement::Empty => {}
        }
    }

    fn check_condition(&mut self, expr: &Expr, locals: &SymbolTable, line: usize) {
        match expr {
            Expr::BinaryOp { op, lhs, rhs } => {
                // CONDITION_NOT_VALID: non-logical operators can't have condition
                // sub-expressions (e.g. `if (($a < $b) = ($c < $d))` is invalid)
                match op {
                    BinOp::Eq
                    | BinOp::NotEq
                    | BinOp::Lt
                    | BinOp::Gt
                    | BinOp::LtEq
                    | BinOp::GtEq
                        if (self.is_condition_expr(lhs) || self.is_condition_expr(rhs)) =>
                    {
                        self.error(line, msg::CONDITION_NOT_VALID.to_string());
                        return;
                    }
                    _ => {}
                }

                let lt = self.infer_expr_type(lhs, locals, line, None);
                let rhs_hint = lt;
                let rt = self.infer_expr_type(rhs, locals, line, rhs_hint);

                match op {
                    BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq => {
                        if let Some(ref lt) = lt
                            && lt.base_type() == BaseVarType::String
                            && *lt != Type::Error
                            && *lt != Type::Any
                        {
                            self.error(line, msg::fmt(msg::ARITHMETIC_INVALID_TYPE, &[lt.name()]));
                        }
                        if let Some(ref rt) = rt
                            && rt.base_type() == BaseVarType::String
                            && *rt != Type::Error
                            && *rt != Type::Any
                        {
                            self.error(line, msg::fmt(msg::ARITHMETIC_INVALID_TYPE, &[rt.name()]));
                        }
                    }
                    BinOp::Eq | BinOp::NotEq => {
                        if let (Some(lt), Some(rt)) = (&lt, &rt)
                            && *lt == Type::String
                            && *rt == Type::String
                        {
                            let op_str = if *op == BinOp::Eq { "=" } else { "!" };
                            self.error(
                                line,
                                msg::fmt(msg::BINOP_INVALID_TYPES, &[op_str, lt.name(), rt.name()]),
                            );
                        }
                    }
                    _ => {}
                }
            }
            Expr::LogicalOp { lhs, rhs, .. } => {
                self.check_condition(lhs, locals, line);
                self.check_condition(rhs, locals, line);
            }
            _ => {
                self.error(line, msg::CONDITION_INVALID_NODE_TYPE.to_string());
            }
        }
    }

    fn is_condition_expr(&self, expr: &Expr) -> bool {
        match expr {
            Expr::BinaryOp { op, .. } => matches!(
                op,
                BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq
            ),
            Expr::LogicalOp { .. } => true,
            _ => false,
        }
    }

    /// Mirrors TS `visitSwitchCase`: type-checks a single switch case entry.
    /// `parent_switch_type` is `Some` when the case is inside a switch statement,
    /// `None` if the case was somehow orphaned (CASE_WITHOUT_SWITCH).
    fn check_switch_case(
        &mut self,
        case: &SwitchCase,
        parent_switch_type: Option<Type>,
        locals: &SymbolTable,
        return_types: Option<&[Type]>,
        line: usize,
    ) {
        let switch_ty = match parent_switch_type {
            Some(t) => t,
            None => {
                self.error(line, msg::CASE_WITHOUT_SWITCH.to_string());
                return;
            }
        };

        for val in &case.values {
            if !self.is_constant_expression(val) {
                self.warning(line, msg::SWITCH_CASE_NOT_CONSTANT.to_string());
            }
            let val_type = self.infer_expr_type(val, locals, line, Some(switch_ty));
            if let Some(vt) = val_type
                && switch_ty != Type::Error
                && !self.types_compatible(vt, switch_ty)
            {
                self.emit_type_mismatch(line, vt, switch_ty);
            }
        }
        self.check_block(&case.body, locals, return_types);
    }

    /// Infer the type of an expression with an optional type hint.
    /// The hint flows top-down from context (command params, variable type, etc.)
    /// and is used for context-aware identifier/literal resolution — the same
    /// approach codegen uses via `compile_expr_hinted`.
    fn infer_expr_type(
        &mut self,
        expr: &Expr,
        locals: &SymbolTable,
        line: usize,
        hint: Option<Type>,
    ) -> Option<Type> {
        match expr {
            Expr::IntLiteral(_) => {
                // Matching TS visitIntegerLiteral: if hint is a non-literal type,
                // the integer is treated as a symbol reference of that type.
                if let Some(h) = hint {
                    if !is_literal_type(h) && h != Type::Error && h != Type::Any {
                        return Some(h);
                    }
                    if h == Type::Boolean {
                        return Some(Type::Boolean);
                    }
                }
                Some(Type::Int)
            }
            Expr::LongLiteral(_) => Some(Type::Long),
            Expr::StringLiteral(s) => {
                // Matching TS visitStringLiteral: if hint is a non-literal type,
                // the string is treated as a symbol reference (e.g. "sailing journey"
                // with hint=midi resolves to a midi entity).
                if let Some(h) = hint
                    && !is_literal_type(h)
                    && h != Type::Error
                    && h != Type::Any
                {
                    // Verify the name resolves against the registry.
                    // If it doesn't, surface a warning so silent
                    // fallthrough to `push_string` at codegen doesn't
                    // produce a runtime bug.
                    //
                    // Mirror the identifier-path resolution cascade:
                    //   1. typed entity table (strict)
                    //   2. untyped entity table (subtype tolerance,
                    //      e.g. obj ⊂ namedobj)
                    //   3. proc/script registries (for proc/label/
                    //      queue/timer/walktrigger hints)
                    let typed_ok = self.registry.lookup_entity_id_typed(s, h).is_some();
                    let untyped_ok = self.registry.lookup_entity_id(s).is_some();
                    let script_ok = self.registry.proc_script_id(s).is_some()
                        || self.registry.script_id(s).is_some();
                    if !typed_ok && !untyped_ok && !script_ok {
                        self.emit_unresolved_entity_warning(line, s, h);
                    } else if is_valid_bare_ident(s) {
                        // The string literal resolves AND its name is a
                        // legal bare identifier — recommend the bare form
                        // for clarity.
                        self.emit_prefer_bare_ident_warning(line, s, h);
                    }
                    return Some(h);
                }
                Some(Type::String)
            }
            Expr::BoolLiteral(_) => Some(Type::Boolean),
            Expr::NullLiteral => {
                // Matching TS visitNullLiteral: use hint if provided
                if let Some(h) = hint
                    && h != Type::Error
                {
                    return Some(h);
                }
                Some(Type::Int)
            }
            Expr::CharLiteral(_) => Some(Type::Char),
            Expr::CoordLiteral(_) => Some(Type::Coord),

            Expr::LocalVar(name, src_line) => {
                if let Some(sym) = locals.lookup(name) {
                    match &sym.kind {
                        SymbolKind::LocalVar {
                            var_type, is_array, ..
                        } => {
                            // LOCAL_ARRAY_REFERENCE_NOINDEX: array var used without index
                            if *is_array {
                                self.error(
                                    *src_line,
                                    msg::fmt(msg::LOCAL_ARRAY_REFERENCE_NOINDEX, &[name]),
                                );
                                return Some(Type::Error);
                            }
                            Some(*var_type)
                        }
                        SymbolKind::ScriptParam { param_type, .. } => Some(*param_type),
                        _ => None,
                    }
                } else {
                    self.error(
                        *src_line,
                        msg::fmt(msg::LOCAL_REFERENCE_UNRESOLVED, &[name]),
                    );
                    Some(Type::Error)
                }
            }

            Expr::ArrayAccess { name, index } => {
                if let Some(sym) = locals.lookup(name) {
                    match &sym.kind {
                        SymbolKind::LocalVar {
                            var_type, is_array, ..
                        } => {
                            if !is_array {
                                // LOCAL_REFERENCE_NOT_ARRAY: indexing a non-array
                                self.error(line, msg::fmt(msg::LOCAL_REFERENCE_NOT_ARRAY, &[name]));
                            }
                            let idx_type =
                                self.infer_expr_type(index, locals, line, Some(Type::Int));
                            if let Some(it) = idx_type
                                && !self.types_compatible(it, Type::Int)
                            {
                                self.emit_type_mismatch(line, it, Type::Int);
                            }
                            Some(*var_type)
                        }
                        SymbolKind::ScriptParam { param_type, .. } => {
                            self.error(line, msg::fmt(msg::LOCAL_REFERENCE_NOT_ARRAY, &[name]));
                            Some(*param_type)
                        }
                        _ => None,
                    }
                } else {
                    self.error(line, msg::fmt(msg::LOCAL_REFERENCE_UNRESOLVED, &[name]));
                    Some(Type::Error)
                }
            }

            Expr::GameVar(name, src_line) => {
                if let Some(sym) = self.registry.lookup_game_var(name) {
                    match &sym.kind {
                        SymbolKind::GameVar { var_type, .. } => Some(*var_type),
                        _ => None,
                    }
                } else {
                    self.error(*src_line, msg::fmt(msg::GAME_REFERENCE_UNRESOLVED, &[name]));
                    Some(Type::Error)
                }
            }

            Expr::ConstantVar(name, src_line) => {
                if let Some(sym) = self.registry.lookup_constant(name) {
                    match &sym.kind {
                        SymbolKind::Constant { const_type, .. } => Some(*const_type),
                        _ => None,
                    }
                } else {
                    self.error(
                        *src_line,
                        msg::fmt(msg::CONSTANT_REFERENCE_UNRESOLVED, &[name]),
                    );
                    Some(Type::Error)
                }
            }

            Expr::Identifier(name) => {
                // Type-aware resolution: if hint is provided and there's a typed entity match, use it.
                // This mirrors codegen's compile_expr_hinted (compiler.rs:1079).
                if let Some(h) = hint
                    && h != Type::Error
                    && h != Type::Any
                    && self.registry.lookup_entity_id_typed(name, h).is_some()
                {
                    return Some(h);
                }

                // Fallback: constants → entity IDs → commands → type chars
                if let Some(sym) = self.registry.lookup_constant(name) {
                    match &sym.kind {
                        SymbolKind::Constant { const_type, .. } => Some(*const_type),
                        other => {
                            let kind_name = other.kind_name();
                            self.error(
                                line,
                                msg::fmt(msg::UNSUPPORTED_SYMBOLTYPE_TO_TYPE, &[kind_name]),
                            );
                            Some(Type::Error)
                        }
                    }
                } else if let Some(sym) = self.registry.lookup_entity_id(name) {
                    match &sym.kind {
                        SymbolKind::Constant { const_type, .. } => Some(*const_type),
                        other => {
                            let kind_name = other.kind_name();
                            self.error(
                                line,
                                msg::fmt(msg::UNSUPPORTED_SYMBOLTYPE_TO_TYPE, &[kind_name]),
                            );
                            Some(Type::Error)
                        }
                    }
                } else if let Some(sym) = self.registry.lookup_command(name).cloned() {
                    if let SymbolKind::Command { return_types, .. } = &sym.kind {
                        return return_types.first().copied();
                    }
                    self.error(
                        line,
                        msg::fmt(msg::UNSUPPORTED_SYMBOLTYPE_TO_TYPE, &[sym.kind.kind_name()]),
                    );
                    Some(Type::Error)
                } else if self.registry.type_chars.contains_key(name) {
                    Some(Type::Int)
                } else if name.contains(':') {
                    // Component reference: "interface:component"
                    let parts: Vec<&str> = name.splitn(2, ':').collect();
                    if parts.len() == 2
                        && self.registry.lookup_component(parts[0], parts[1]).is_some()
                    {
                        Some(hint.unwrap_or(Type::Component))
                    } else if let Some(h) = hint {
                        if h != Type::Error {
                            Some(h)
                        } else {
                            Some(Type::Error)
                        }
                    } else {
                        self.error(line, msg::fmt(msg::GENERIC_UNRESOLVED_SYMBOL, &[name]));
                        Some(Type::Error)
                    }
                } else {
                    // Matching TS resolveSymbol:
                    // - If hint is String, treat identifier as string (allowToString)
                    // - If hint exists, trust it (codegen resolves via entity_ids_typed)
                    // - If no hint and no match, emit error
                    if let Some(h) = hint {
                        if h == Type::String {
                            return Some(Type::String);
                        }
                        if h != Type::Error {
                            // Before "trusting the hint", verify the name
                            // actually resolves somewhere. If not, codegen
                            // will silently emit push_int(-1) for unknown
                            // identifiers — surface a warning so the typo is
                            // visible at compile time.
                            //
                            // Resolution sources (matching codegen's cascade
                            // in compile_expr for Ident):
                            //   1. entity_ids_typed (typed entities)
                            //   2. entity_ids (untyped entity fallback)
                            //   3. proc_script_id / script_id (script refs
                            //      for proc/label/queue/timer/walktrigger
                            //      params)
                            let resolved = self.registry.lookup_entity_id_typed(name, h).is_some()
                                || self.registry.lookup_entity_id(name).is_some()
                                || self.registry.proc_script_id(name).is_some()
                                || self.registry.script_id(name).is_some();
                            if !resolved {
                                self.error(line, msg::fmt(msg::GENERIC_UNRESOLVED_SYMBOL, &[name]));
                                return Some(Type::Error);
                            }
                            return Some(h);
                        }
                    }
                    self.error(line, msg::fmt(msg::GENERIC_UNRESOLVED_SYMBOL, &[name]));
                    Some(Type::Error)
                }
            }

            Expr::BinaryOp { op, lhs, rhs } => match op {
                BinOp::Add
                | BinOp::Sub
                | BinOp::Mul
                | BinOp::Div
                | BinOp::Mod
                | BinOp::BitAnd
                | BinOp::BitOr => {
                    // Entity names may contain `+` (e.g. `cheese+tom_batta`,
                    // `premade_cheese+tom_batta`). The parser represents these
                    // as BinaryOp::Add with two Identifier children. Before
                    // descending (which would fire spurious "unresolved"
                    // warnings on each half), check if the combined name
                    // resolves as a single entity and short-circuit.
                    if matches!(op, BinOp::Add)
                        && let (Expr::Identifier(lname), Expr::Identifier(rname)) =
                            (lhs.as_ref(), rhs.as_ref())
                    {
                        let combined = format!("{}+{}", lname, rname);
                        if let Some(h) = hint {
                            if self.registry.lookup_entity_id_typed(&combined, h).is_some()
                                || self.registry.lookup_entity_id(&combined).is_some()
                            {
                                return Some(h);
                            }
                        } else if self.registry.lookup_entity_id(&combined).is_some()
                            && let Some(sym) = self.registry.lookup_entity_id(&combined)
                            && let SymbolKind::Constant { const_type, .. } = &sym.kind
                        {
                            return Some(*const_type);
                        }
                    }

                    let expected = hint.unwrap_or(Type::Int);
                    let lt = self.infer_expr_type(lhs, locals, line, Some(expected));
                    let rt = self.infer_expr_type(rhs, locals, line, Some(expected));
                    if let Some(ref lt) = lt
                        && lt.base_type() == BaseVarType::String
                        && *lt != Type::Error
                        && *lt != Type::Any
                    {
                        self.error(line, msg::fmt(msg::ARITHMETIC_INVALID_TYPE, &[lt.name()]));
                    }
                    if let Some(ref rt) = rt
                        && rt.base_type() == BaseVarType::String
                        && *rt != Type::Error
                        && *rt != Type::Any
                    {
                        self.error(line, msg::fmt(msg::ARITHMETIC_INVALID_TYPE, &[rt.name()]));
                    }
                    if matches!(lt, Some(Type::Long)) || matches!(rt, Some(Type::Long)) {
                        Some(Type::Long)
                    } else {
                        Some(Type::Int)
                    }
                }
                BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq => {
                    let lt = self.infer_expr_type(lhs, locals, line, None);
                    let _ = self.infer_expr_type(rhs, locals, line, lt);
                    Some(Type::Boolean)
                }
            },

            Expr::LogicalOp { lhs, rhs, .. } => {
                let _ = self.infer_expr_type(lhs, locals, line, None);
                let _ = self.infer_expr_type(rhs, locals, line, None);
                Some(Type::Boolean)
            }

            Expr::Calc(inner) => {
                let expected = hint.unwrap_or(Type::Int);
                let inner_type = self.infer_expr_type(inner, locals, line, Some(expected));
                if let Some(ref it) = inner_type
                    && it.base_type() == BaseVarType::String
                    && *it != Type::Error
                    && *it != Type::Any
                {
                    self.error(line, msg::fmt(msg::ARITHMETIC_INVALID_TYPE, &[it.name()]));
                }
                inner_type
            }

            Expr::CommandCall {
                name,
                args,
                call_line,
                ..
            } => {
                let lookup_name = name.strip_prefix('.').unwrap_or(name);

                // Look up command param types for hinting (same as compiler.rs:1345-1399)
                let param_types: Vec<Type> = self
                    .registry
                    .command_param_types
                    .get(lookup_name)
                    .cloned()
                    .unwrap_or_default();

                let arg_types: Vec<Option<Type>> = args
                    .iter()
                    .enumerate()
                    .map(|(i, a)| {
                        let arg_hint = param_types.get(i).copied();
                        self.infer_expr_type(a, locals, *call_line, arg_hint)
                    })
                    .collect();

                let is_variadic = lookup_name.ends_with('*');
                let cmd_sym = self.registry.lookup_command(lookup_name).cloned().or_else(|| {
                    lookup_name.strip_suffix('*').and_then(|base| {
                        let vararg_name = format!("{}vararg", base);
                        self.registry.lookup_command(&vararg_name).cloned()
                    })
                });
                if let Some(sym) = cmd_sym {
                    if let SymbolKind::Command {
                        param_types: cmd_params,
                        return_types,
                        ..
                    } = &sym.kind
                    {
                        if !is_variadic {
                            self.check_call_arguments(
                                name,
                                cmd_params,
                                &arg_types,
                                args,
                                *call_line,
                                CallKind::Command,
                            );
                        }
                        return return_types.first().copied();
                    }
                } else {
                    self.error(
                        *call_line,
                        msg::fmt(msg::COMMAND_REFERENCE_UNRESOLVED, &[name]),
                    );
                    return Some(Type::Error);
                }
                None
            }

            Expr::ProcCall {
                name,
                args,
                call_line,
                ..
            } => {
                // Use trigger-scoped lookup to avoid name collisions
                let script_sym = self
                    .registry
                    .lookup_script_by_trigger("proc", name)
                    .cloned();

                if self.registry.proc_script_id(name).is_none() {
                    self.error(
                        *call_line,
                        msg::fmt(msg::PROC_REFERENCE_UNRESOLVED, &[name]),
                    );
                    // Still infer arg types even on error
                    for arg in args {
                        let _ = self.infer_expr_type(arg, locals, *call_line, None);
                    }
                    return Some(Type::Error);
                }

                let (p_types, r_types) = if let Some(ref sym) = script_sym {
                    if let SymbolKind::Script {
                        param_types,
                        return_types,
                        ..
                    } = &sym.kind
                    {
                        (param_types.clone(), return_types.clone())
                    } else {
                        (vec![], vec![])
                    }
                } else {
                    (vec![], vec![])
                };

                let arg_types: Vec<Option<Type>> = args
                    .iter()
                    .enumerate()
                    .map(|(i, a)| {
                        let arg_hint = p_types.get(i).copied();
                        self.infer_expr_type(a, locals, *call_line, arg_hint)
                    })
                    .collect();

                if script_sym.is_some() {
                    self.check_call_arguments(
                        name,
                        &p_types,
                        &arg_types,
                        args,
                        *call_line,
                        CallKind::Proc,
                    );
                }
                r_types.first().copied()
            }

            Expr::JumpCall {
                name,
                args,
                call_line,
                ..
            } => {
                let script_sym = self
                    .registry
                    .lookup_script_by_trigger("label", name)
                    .cloned();

                if self.registry.label_script_id(name).is_none() {
                    self.error(
                        *call_line,
                        msg::fmt(msg::JUMP_REFERENCE_UNRESOLVED, &[name]),
                    );
                    for arg in args {
                        let _ = self.infer_expr_type(arg, locals, *call_line, None);
                    }
                    return Some(Type::Error);
                }

                let p_types = if let Some(ref sym) = script_sym {
                    if let SymbolKind::Script { param_types, .. } = &sym.kind {
                        param_types.clone()
                    } else {
                        vec![]
                    }
                } else {
                    vec![]
                };

                let arg_types: Vec<Option<Type>> = args
                    .iter()
                    .enumerate()
                    .map(|(i, a)| {
                        let arg_hint = p_types.get(i).copied();
                        self.infer_expr_type(a, locals, *call_line, arg_hint)
                    })
                    .collect();

                if script_sym.is_some() {
                    self.check_call_arguments(
                        name,
                        &p_types,
                        &arg_types,
                        args,
                        *call_line,
                        CallKind::Jump,
                    );
                }
                None
            }

            Expr::JoinedString { parts } => {
                for part in parts {
                    if let StringPart::Expr(e) = part {
                        let _ = self.infer_expr_type(e, locals, line, Some(Type::String));
                    }
                }
                Some(Type::String)
            }

            Expr::MultiAssign(targets, _) => {
                // ASSIGN_MULTI_ARRAY: arrays are not allowed in multi-assignment
                for target in targets {
                    if let Expr::ArrayAccess { name, .. } = target {
                        self.error(line, msg::fmt(msg::ASSIGN_MULTI_ARRAY, &[name]));
                    }
                }
                None
            }
        }
    }

    // ──────────────────── Call argument validation ────────────────────

    fn check_call_arguments(
        &mut self,
        name: &str,
        expected_types: &[Type],
        actual_types: &[Option<Type>],
        args: &[Expr],
        line: usize,
        kind: CallKind,
    ) {
        // For commands, empty param_types means "signature unknown"
        if expected_types.is_empty() {
            if kind == CallKind::Command {
                return;
            }
            // A single null arg to a no-param proc is valid (null = unit)
            let is_single_null = args.len() == 1 && matches!(&args[0], Expr::NullLiteral);
            if !actual_types.is_empty() && !is_single_null {
                let actual_desc = actual_types
                    .iter()
                    .map(|t| t.map_or("unknown".to_string(), |t| t.name().to_string()))
                    .collect::<Vec<_>>()
                    .join(", ");
                let template = match kind {
                    CallKind::Proc => msg::PROC_NOARGS_EXPECTED,
                    CallKind::Jump => msg::JUMP_NOARGS_EXPECTED,
                    CallKind::Command => msg::COMMAND_NOARGS_EXPECTED,
                };
                self.error(line, msg::fmt(template, &[name, &actual_desc]));
            }
            return;
        }

        // Proc/command call args can produce multiple return values,
        // making the actual arg count less than the number of values produced.
        // Skip count validation when any arg is a multi-return call.
        let has_call_arg = args
            .iter()
            .any(|a| matches!(a, Expr::ProcCall { .. } | Expr::CommandCall { .. }));

        if expected_types.len() != actual_types.len() {
            if has_call_arg {
                return;
            }
            // Commands may have variadic or partially-known signatures;
            // only enforce strict arg count for procs and jumps.
            if kind != CallKind::Command {
                self.error(
                    line,
                    msg::fmt(
                        msg::GENERIC_TYPE_MISMATCH,
                        &[
                            &format!("{} arg(s)", actual_types.len()),
                            &format!("{} arg(s)", expected_types.len()),
                        ],
                    ),
                );
            }
            return;
        }

        for (i, (expected, actual)) in expected_types.iter().zip(actual_types.iter()).enumerate() {
            if let Some(actual_ty) = actual
                && !self.types_compatible(*actual_ty, *expected)
            {
                self.error(
                    line,
                    msg::fmt(
                        msg::GENERIC_TYPE_MISMATCH,
                        &[actual_ty.name(), expected.name()],
                    ),
                );
                let _ = i;
            }
        }
    }

    // ──────────────────── Constant expression check ────────────────────

    fn is_constant_expression(&self, expr: &Expr) -> bool {
        match expr {
            Expr::IntLiteral(_)
            | Expr::LongLiteral(_)
            | Expr::StringLiteral(_)
            | Expr::BoolLiteral(_)
            | Expr::NullLiteral
            | Expr::CharLiteral(_)
            | Expr::CoordLiteral(_) => true,
            Expr::ConstantVar(..) => true,
            Expr::Identifier(name) => {
                self.registry.lookup_constant(name).is_some()
                    || self.registry.lookup_entity_id(name).is_some()
                    || self.registry.type_chars.contains_key(name)
                    // Component references: "interface:component"
                    || name.contains(':') && {
                        let parts: Vec<&str> = name.splitn(2, ':').collect();
                        parts.len() == 2
                            && self
                                .registry
                                .lookup_component(parts[0], parts[1])
                                .is_some()
                    }
            }
            // Entity names containing '+' (e.g. cheese+tom_batta): the parser
            // splits these into BinaryOp::Add but codegen recombines them.
            Expr::BinaryOp {
                op: BinOp::Add,
                lhs,
                rhs,
            } => {
                if let (Expr::Identifier(l), Expr::Identifier(r)) = (lhs.as_ref(), rhs.as_ref()) {
                    let combined = format!("{}+{}", l, r);
                    self.registry.lookup_entity_id(&combined).is_some()
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    // ──────────────────── Side-effect analysis ────────────────────

    fn expression_has_side_effects(&self, expr: &Expr) -> bool {
        match expr {
            Expr::CommandCall { .. } | Expr::ProcCall { .. } | Expr::JumpCall { .. } => true,
            Expr::BinaryOp { lhs, rhs, .. } | Expr::LogicalOp { lhs, rhs, .. } => {
                self.expression_has_side_effects(lhs) || self.expression_has_side_effects(rhs)
            }
            Expr::Calc(inner) => self.expression_has_side_effects(inner),
            Expr::JoinedString { parts } => parts.iter().any(|p| {
                if let StringPart::Expr(e) = p {
                    self.expression_has_side_effects(e)
                } else {
                    false
                }
            }),
            _ => false,
        }
    }

    // ──────────────────── Type compatibility ────────────────────
    // Matching TS compiler's registered TypeCheckers (ScriptCompiler.ts:129-189)

    fn types_compatible(&self, actual: Type, expected: Type) -> bool {
        // Rule 1: Any accepts anything
        if expected == Type::Any {
            return true;
        }
        // Rule 2: Error on either side prevents cascading
        if expected == Type::Error || actual == Type::Error {
            return true;
        }
        // Rule 3: Exact match
        if actual == expected {
            return true;
        }
        // Rule 4 (server): namedobj assignable to obj
        if expected == Type::Obj && actual == Type::NamedObj {
            return true;
        }
        // Rule 5: Any actual also matches (symmetric with rule 1)
        if actual == Type::Any {
            return true;
        }
        // Temporary fallback: integer-base interchangeability.
        // The TS compiler doesn't need this because type hints resolve
        // identifiers/literals to the correct type before comparison.
        // Keep until hint coverage is proven complete.
        if actual.base_type() == BaseVarType::Integer
            && expected.base_type() == BaseVarType::Integer
        {
            return true;
        }
        false
    }

    // ──────────────────── Diagnostic helpers ────────────────────

    fn emit_type_mismatch(&mut self, line: usize, actual: Type, expected: Type) {
        self.error(
            line,
            msg::fmt(
                msg::GENERIC_TYPE_MISMATCH,
                &[actual.name(), expected.name()],
            ),
        );
    }

    fn error(&mut self, line: usize, message: String) {
        self.diagnostics.error(
            self.file_path.clone(),
            line,
            0,
            message,
            Phase::TypeChecking,
        );
    }

    fn warning(&mut self, line: usize, message: String) {
        self.diagnostics.warning(
            self.file_path.clone(),
            line,
            0,
            message,
            Phase::TypeChecking,
        );
    }

    /// Emit a warning with attached Help (and optional Suggestion) in one
    /// call — mirrors `warning` but threads a `Help` block through.
    fn warning_with_help(&mut self, line: usize, message: String, help: crate::diagnostics::Help) {
        use crate::diagnostics::{Diagnostic, Severity};
        self.diagnostics.add(Diagnostic {
            file: self.file_path.clone(),
            line,
            column: 0,
            message,
            severity: Severity::Warning,
            phase: Phase::TypeChecking,
            help: vec![help],
        });
    }

    /// Emit UNRESOLVED_ENTITY_REF with a "did you mean?" suggestion list
    /// pulled from the nearest names in the expected type's entity table.
    fn emit_unresolved_entity_warning(&mut self, line: usize, name: &str, expected: Type) {
        use crate::diagnostics::{Applicability, Help};

        let message = msg::fmt(msg::UNRESOLVED_ENTITY_REF, &[name, expected.name()]);

        let near = self.near_entity_names(name, expected, 2, 3);
        let help_message = if near.is_empty() {
            format!(
                "Check the `{}` pack/registry for the canonical name, or remove \
                 the reference if it's stale.",
                expected.name()
            )
        } else {
            let suggestions = near
                .iter()
                .map(|n| format!("`{}`", n))
                .collect::<Vec<_>>()
                .join(", ");
            format!("Did you mean one of: {}?", suggestions)
        };

        self.warning_with_help(
            line,
            message,
            Help {
                message: help_message,
                suggestions: Vec::new(),
                applicability: Applicability::MaybeIncorrect,
            },
        );
    }

    /// Emit PREFER_BARE_IDENT with a one-line rewrite suggestion.
    fn emit_prefer_bare_ident_warning(&mut self, line: usize, name: &str, expected: Type) {
        use crate::diagnostics::{Applicability, Help};

        let message = msg::fmt(msg::PREFER_BARE_IDENT, &[name, expected.name()]);
        self.warning_with_help(
            line,
            message,
            Help {
                message: format!(
                    "Replace `\"{}\"` with the bare identifier `{}` — same \
                     resolution, one fewer layer of indirection.",
                    name, name
                ),
                suggestions: Vec::new(),
                applicability: Applicability::MachineApplicable,
            },
        );
    }

    /// Search the registry for up to `limit` entity names whose edit distance
    /// from `name` is within `max_edits`.
    ///
    /// Optimized for large registries (100k+ entries) via:
    ///   1. Length pre-filter — skip candidates whose byte length differs by
    ///      more than `max_edits` (eliminates 90%+ without any computation).
    ///   2. Bounded Levenshtein — early-terminates rows whose minimum exceeds
    ///      the threshold.
    fn near_entity_names(
        &self,
        name: &str,
        expected: Type,
        max_edits: usize,
        limit: usize,
    ) -> Vec<String> {
        use std::collections::HashSet;
        let target = name.to_lowercase();
        let target_len = target.len();
        let mut seen: HashSet<String> = HashSet::new();
        let mut scored: Vec<(usize, String)> = Vec::new();

        let consider =
            |cand: &str, scored: &mut Vec<(usize, String)>, seen: &mut HashSet<String>| {
                if cand.len().abs_diff(target_len) > max_edits {
                    return;
                }
                let key = cand.to_lowercase();
                if !seen.insert(key.clone()) {
                    return;
                }
                let d = levenshtein_bounded(&target, &key, max_edits);
                if d == 0 || d > max_edits {
                    return;
                }
                scored.push((d, cand.to_string()));
            };

        for (cand, by_type) in self.registry.entity_ids_typed.iter() {
            if by_type.contains_key(&expected) {
                consider(cand, &mut scored, &mut seen);
            }
        }
        for cand in self.registry.entity_ids.keys() {
            consider(cand, &mut scored, &mut seen);
        }

        scored.sort_by_key(|(d, _)| *d);
        scored.into_iter().take(limit).map(|(_, n)| n).collect()
    }
}

/// Levenshtein edit distance with early termination. Returns `max + 1`
/// if the true distance exceeds `max`, avoiding full-matrix computation
/// for distant strings.
fn levenshtein_bounded(a: &str, b: &str, max: usize) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    if n.abs_diff(m) > max {
        return max + 1;
    }
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr = vec![0usize; m + 1];
    for i in 1..=n {
        curr[0] = i;
        let mut row_min = curr[0];
        for j in 1..=m {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
            row_min = row_min.min(curr[j]);
        }
        if row_min > max {
            return max + 1;
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

/// An RS2 bare identifier must start with a letter / underscore and
/// consist only of `[A-Za-z0-9_+]` (the `+` variant is supported via the
/// parser's `cheese+tom_batta` handling). Names with spaces or other
/// characters must use the quoted string form.
fn is_valid_bare_ident(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '+')
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CallKind {
    Command,
    Proc,
    Jump,
}
