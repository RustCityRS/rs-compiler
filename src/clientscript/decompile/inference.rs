use super::Names;
use super::source::script_name;
use crate::arguments::{command_hints, command_returns, expression_types, proc_hints, stack_hints};
use crate::clientscript::hooks;
use crate::parser::{BinOp, Expr, ScriptDeclaration, ScriptFile, Statement, StringPart};
use crate::symbol::{SymbolKind, SymbolRegistry, SymbolTable};
use crate::types::{BaseVarType, Type};
use std::collections::HashMap;

struct Node {
    parent: usize,
    kind: BaseVarType,
    ty: Option<Type>,
    conflict: bool,
}

#[derive(Clone)]
struct Scope {
    locals: HashMap<String, usize>,
    symbols: SymbolTable,
    params: Vec<usize>,
    returns: Vec<usize>,
}

pub struct Inferred {
    pub params: Vec<Type>,
    pub returns: Vec<Type>,
    pub locals: HashMap<String, Type>,
}

struct Inference<'a> {
    registry: &'a SymbolRegistry,
    nodes: Vec<Node>,
    scopes: HashMap<String, Scope>,
    hook_params: HashMap<usize, Option<Type>>,
}

impl Inference<'_> {
    fn node(&mut self, ty: Type) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node {
            parent: id,
            kind: ty.base_type(),
            ty: (!matches!(ty, Type::Int | Type::Any)).then_some(ty),
            conflict: false,
        });
        id
    }

    fn root(&mut self, id: usize) -> usize {
        let parent = self.nodes[id].parent;
        if parent != id {
            self.nodes[id].parent = self.root(parent);
        }
        self.nodes[id].parent
    }

    fn join(&mut self, a: usize, b: usize) {
        let a = self.root(a);
        let b = self.root(b);
        if a == b || self.nodes[a].kind != self.nodes[b].kind {
            return;
        }
        let (a, b) = (a.min(b), a.max(b));
        let left = self.nodes[a].ty;
        let right = self.nodes[b].ty;
        self.nodes[b].parent = a;
        self.nodes[a].conflict |=
            self.nodes[b].conflict || left.zip(right).is_some_and(|(a, b)| a != b);
        self.nodes[a].ty = left.or(right);
    }

    fn bind(&mut self, values: &[usize], targets: &[usize]) {
        for kind in [BaseVarType::Integer, BaseVarType::String, BaseVarType::Long] {
            let values: Vec<_> = values
                .iter()
                .copied()
                .filter(|&id| self.nodes[id].kind == kind)
                .collect();
            let targets: Vec<_> = targets
                .iter()
                .copied()
                .filter(|&id| self.nodes[id].kind == kind)
                .collect();
            for (a, b) in values.into_iter().zip(targets) {
                self.join(a, b);
            }
        }
    }

    fn ty(&mut self, id: usize) -> Type {
        let id = self.root(id);
        let node = &self.nodes[id];
        if !node.conflict
            && let Some(ty) = node.ty
        {
            return ty;
        }
        match node.kind {
            BaseVarType::Integer => Type::Int,
            BaseVarType::String => Type::String,
            BaseVarType::Long => Type::Long,
        }
    }

    fn declarations(&mut self, body: &[Statement], scope: &mut Scope) {
        for statement in body {
            match statement {
                Statement::VarDeclaration { name, var_type, .. } => {
                    let id = self.node(*var_type);
                    scope.locals.insert(name.clone(), id);
                    scope.symbols.define(
                        name.clone(),
                        SymbolKind::LocalVar {
                            var_type: *var_type,
                            slot: 0,
                            is_array: false,
                        },
                    );
                }
                Statement::If {
                    body,
                    else_if,
                    else_body,
                    ..
                } => {
                    self.declarations(body, scope);
                    for (_, body, _) in else_if {
                        self.declarations(body, scope);
                    }
                    if let Some(body) = else_body {
                        self.declarations(body, scope);
                    }
                }
                Statement::While { body, .. } => self.declarations(body, scope),
                Statement::Switch { cases, default, .. } => {
                    for case in cases {
                        self.declarations(&case.body, scope);
                    }
                    if let Some(body) = default {
                        self.declarations(body, scope);
                    }
                }
                _ => {}
            }
        }
    }

    fn expr(&mut self, expr: &Expr, scope: &Scope, hint: Option<Type>) -> Vec<usize> {
        let values = match expr {
            Expr::LocalVar(name, _) => scope.locals.get(name).copied().into_iter().collect(),
            Expr::StringLiteral(_) => vec![self.node(hint.unwrap_or(Type::String))],
            Expr::LongLiteral(_) => vec![self.node(Type::Long)],
            Expr::BoolLiteral(_) => vec![self.node(Type::Boolean)],
            Expr::CoordLiteral(_) => vec![self.node(Type::Coord)],
            Expr::GameVar(name, _) => {
                let ty = self
                    .registry
                    .lookup_game_var(name)
                    .and_then(|symbol| {
                        if let SymbolKind::GameVar { var_type, .. } = symbol.kind {
                            Some(var_type)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(Type::Int);
                vec![self.node(ty)]
            }
            Expr::Calc(value) => self.expr(value, scope, None),
            Expr::BinaryOp { lhs, rhs, .. } | Expr::LogicalOp { lhs, rhs, .. } => {
                self.expr(lhs, scope, None);
                self.expr(rhs, scope, None);
                vec![self.node(Type::Int)]
            }
            Expr::JoinedString { parts } => {
                for part in parts {
                    if let StringPart::Expr(value) = part {
                        self.expr(value, scope, None);
                    }
                }
                vec![self.node(Type::String)]
            }
            Expr::CommandCall { name, args, .. } => {
                let name = name.trim_start_matches('.');
                let lowered = hooks::lower(self.registry, name, args, &scope.symbols, 1)
                    .ok()
                    .flatten();
                let args = lowered.as_deref().unwrap_or(args);
                if let Some(&explicit) = self.registry.hook_commands.get(name)
                    && let Some(Expr::Identifier(target)) = args.first()
                    && let Some(signature) = args
                        .len()
                        .checked_sub(1 + usize::from(explicit))
                        .and_then(|index| args.get(index))
                    && let Expr::StringLiteral(signature) = signature
                    && let Some(callee) = self
                        .scopes
                        .get(&format!("clientscript:{target}"))
                        .or_else(|| self.scopes.get(&format!("proc:{target}")))
                        .cloned()
                {
                    let types: Vec<_> = signature
                        .trim_end_matches('Y')
                        .chars()
                        .map(|ch| Type::from_char(ch).unwrap_or(Type::Int))
                        .collect();
                    for base in [BaseVarType::Integer, BaseVarType::String, BaseVarType::Long] {
                        for (param, ty) in callee
                            .params
                            .iter()
                            .filter(|&&param| self.nodes[param].kind == base)
                            .zip(types.iter().filter(|ty| ty.base_type() == base))
                        {
                            self.hook_params
                                .entry(*param)
                                .and_modify(|previous| {
                                    if *previous != Some(*ty) {
                                        *previous = None;
                                    }
                                })
                                .or_insert(Some(*ty));
                        }
                    }
                }
                let hints = command_hints(self.registry, name, args, &scope.symbols);
                for (i, arg) in args.iter().enumerate() {
                    self.expr(arg, scope, hints.get(i).copied().flatten());
                }
                command_returns(self.registry, name, args)
                    .into_iter()
                    .map(|ty| self.node(ty))
                    .collect()
            }
            Expr::Identifier(name)
                if self
                    .registry
                    .lookup_command(name.trim_start_matches('.'))
                    .is_some() =>
            {
                command_returns(self.registry, name, &[])
                    .into_iter()
                    .map(|ty| self.node(ty))
                    .collect()
            }
            Expr::ProcCall { name, args, .. } => {
                let callee = self
                    .scopes
                    .get(&format!("proc:{name}"))
                    .or_else(|| self.scopes.get(&format!("clientscript:{name}")))
                    .cloned();
                let hints = proc_hints(self.registry, name, args, &scope.symbols);
                let values: Vec<_> = args
                    .iter()
                    .enumerate()
                    .flat_map(|(i, arg)| self.expr(arg, scope, hints.get(i).copied().flatten()))
                    .collect();
                if let Some(callee) = callee {
                    self.bind(&values, &callee.params);
                    callee.returns
                } else {
                    Vec::new()
                }
            }
            Expr::MultiAssign(targets, _) | Expr::Tuple(targets) => targets
                .iter()
                .flat_map(|expr| self.expr(expr, scope, None))
                .collect(),
            Expr::ArrayAccess { index, .. } => {
                self.expr(index, scope, None);
                vec![self.node(Type::Int)]
            }
            _ => vec![self.node(hint.unwrap_or(Type::Int))],
        };
        if let Some(ty) = hint
            && !matches!(ty, Type::Int | Type::Any)
            && values.len() == 1
        {
            let target = self.node(ty);
            self.join(values[0], target);
        }
        values
    }

    fn body(&mut self, body: &[Statement], scope: &Scope) {
        for statement in body {
            match statement {
                Statement::VarDeclaration {
                    name,
                    value: Some(value),
                    ..
                } => {
                    let values = self.expr(value, scope, None);
                    if let Some(&target) = scope.locals.get(name) {
                        self.bind(&values, &[target]);
                    }
                }
                Statement::Assignment { target, value, .. } => {
                    let values = self.expr(value, scope, None);
                    let targets = self.expr(target, scope, None);
                    self.bind(&values, &targets);
                }
                Statement::Expression { expr, .. }
                | Statement::ArrayDeclaration { size: expr, .. } => {
                    self.expr(expr, scope, None);
                }
                Statement::Return { values, .. } => {
                    let values: Vec<_> = values
                        .iter()
                        .flat_map(|value| self.expr(value, scope, None))
                        .collect();
                    self.bind(&values, &scope.returns);
                }
                Statement::If {
                    condition,
                    body,
                    else_if,
                    else_body,
                    ..
                } => {
                    self.expr(condition, scope, None);
                    self.body(body, scope);
                    for (condition, body, _) in else_if {
                        self.expr(condition, scope, None);
                        self.body(body, scope);
                    }
                    if let Some(body) = else_body {
                        self.body(body, scope);
                    }
                }
                Statement::While {
                    condition, body, ..
                } => {
                    self.expr(condition, scope, None);
                    self.body(body, scope);
                }
                Statement::Switch {
                    expr,
                    cases,
                    default,
                    ..
                } => {
                    self.expr(expr, scope, None);
                    for case in cases {
                        self.body(&case.body, scope);
                    }
                    if let Some(body) = default {
                        self.body(body, scope);
                    }
                }
                _ => {}
            }
        }
    }
}

pub fn infer(files: &[(u32, ScriptFile)], registry: &SymbolRegistry) -> HashMap<u32, Inferred> {
    let mut inference = Inference {
        registry,
        nodes: Vec::new(),
        scopes: HashMap::new(),
        hook_params: HashMap::new(),
    };
    for (_, file) in files {
        let script = &file.scripts[0];
        let mut scope = Scope {
            locals: HashMap::new(),
            symbols: SymbolTable::new(),
            params: Vec::new(),
            returns: Vec::new(),
        };
        for param in &script.params {
            let id = inference.node(param.param_type);
            scope.params.push(id);
            scope.locals.insert(param.name.clone(), id);
            scope.symbols.define(
                param.name.clone(),
                SymbolKind::ScriptParam {
                    param_type: param.param_type,
                    slot: 0,
                },
            );
        }
        scope.returns = script
            .return_types
            .iter()
            .map(|&ty| inference.node(ty))
            .collect();
        inference.declarations(&script.body, &mut scope);
        inference
            .scopes
            .insert(format!("{}:{}", script.trigger, script.name), scope);
    }
    for (_, file) in files {
        let script = &file.scripts[0];
        let scope = inference.scopes[&format!("{}:{}", script.trigger, script.name)].clone();
        inference.body(&script.body, &scope);
    }
    files
        .iter()
        .map(|(id, file)| {
            let script = &file.scripts[0];
            let scope = inference.scopes[&format!("{}:{}", script.trigger, script.name)].clone();
            (
                *id,
                Inferred {
                    params: scope
                        .params
                        .into_iter()
                        .map(|id| {
                            inference
                                .hook_params
                                .get(&id)
                                .copied()
                                .flatten()
                                .unwrap_or_else(|| inference.ty(id))
                        })
                        .collect(),
                    returns: scope
                        .returns
                        .into_iter()
                        .map(|id| inference.ty(id))
                        .collect(),
                    locals: scope
                        .locals
                        .into_iter()
                        .map(|(name, id)| (name, inference.ty(id)))
                        .collect(),
                },
            )
        })
        .collect()
}

pub fn named_literal(
    value: i32,
    ty: Type,
    names: &dyn Names,
    registry: &SymbolRegistry,
) -> Option<Expr> {
    if ty == Type::Boolean && matches!(value, 0 | 1) {
        return Some(Expr::BoolLiteral(value == 1));
    }
    if matches!(
        ty,
        Type::Int
            | Type::String
            | Type::Long
            | Type::Boolean
            | Type::Char
            | Type::Coord
            | Type::Colour
            | Type::IfType
            | Type::SetPosH
            | Type::SetPosV
            | Type::SetSize
            | Type::SetTextAlignH
            | Type::SetTextAlignV
    ) {
        return None;
    }
    if value == -1 {
        return Some(Expr::NullLiteral);
    }
    let id = u32::try_from(value).ok()?;
    let name = match ty {
        Type::Component => format!(
            "{}:{}",
            names.name("interface", id >> 16),
            names.component_name(id >> 16, id & 0xffff)
        ),
        Type::ClientScript => script_name(names, id),
        Type::Graphic => names.name("sprites", id),
        Type::NamedObj => names.name("obj", id),
        Type::IdKit => names.name("idk", id),
        _ => names.name(ty.name(), id),
    };
    if registry.lookup_reference(&name, ty) != Some(value) {
        return None;
    }
    Some(
        if name.split(':').all(|part| {
            part.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_')
                && part
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                && !matches!(
                    part,
                    "true" | "false" | "null" | "if" | "else" | "while" | "return"
                )
        }) {
            Expr::Identifier(name)
        } else {
            Expr::StringLiteral(name)
        },
    )
}

fn format_expr(
    expr: &mut Expr,
    hint: Option<Type>,
    names: &dyn Names,
    registry: &SymbolRegistry,
    locals: &SymbolTable,
) {
    if let Expr::BoolLiteral(value) = expr
        && hint != Some(Type::Boolean)
    {
        *expr = Expr::IntLiteral(i32::from(*value));
    }
    if let Expr::IntLiteral(value) = expr {
        if hint == Some(Type::Coord) && (0..=0x3fffffff).contains(value) {
            *expr = Expr::CoordLiteral(*value);
            return;
        }
        if let Some(named) = hint.and_then(|ty| named_literal(*value, ty, names, registry)) {
            *expr = named;
            return;
        }
        let names: &[&str] = match hint {
            Some(Type::IfType) => &[
                "iftype_layer",
                "iftype_inputbox",
                "iftype_inv",
                "iftype_rectangle",
                "iftype_text",
                "iftype_graphic",
                "iftype_model",
                "iftype_invtext",
                "iftype_tooltip",
                "iftype_line",
            ],
            Some(Type::SetPosH) => &[
                "setpos_abs_left",
                "setpos_abs_centre",
                "setpos_abs_right",
                "setpos_rel_left",
                "setpos_rel_centre",
                "setpos_rel_right",
            ],
            Some(Type::SetPosV) => &[
                "setpos_abs_top",
                "setpos_abs_centre",
                "setpos_abs_bottom",
                "setpos_rel_top",
                "setpos_rel_centre",
                "setpos_rel_bottom",
            ],
            Some(Type::SetSize) => &[
                "setsize_abs",
                "setsize_minus",
                "setsize_rel",
                "",
                "setsize_aspect",
            ],
            Some(Type::SetTextAlignH) => &[
                "settextalign_left",
                "settextalign_centre",
                "settextalign_right",
            ],
            Some(Type::SetTextAlignV) => &[
                "settextalign_top",
                "settextalign_centre",
                "settextalign_bottom",
            ],
            _ => &[],
        };
        let name = if hint == Some(Type::Colour) {
            match *value {
                0 => Some("black"),
                0xff => Some("blue"),
                0xff00 => Some("green"),
                0xffff => Some("cyan"),
                0xff0000 => Some("red"),
                0xff00ff => Some("magenta"),
                0xffff00 => Some("yellow"),
                0xffffff => Some("white"),
                _ => None,
            }
        } else {
            usize::try_from(*value)
                .ok()
                .and_then(|index| names.get(index).copied())
        };
        if let Some(name) = name && registry.lookup_constant(name).is_some_and(|symbol| matches!(symbol.kind, SymbolKind::Constant { int_value: Some(v), .. } if v == *value)) {
            *expr = Expr::ConstantVar(name.to_owned(), 0);
            return;
        }
        if hint == Some(Type::Colour) {
            *expr = Expr::Identifier(format!("0x{:06x}", *value as u32));
            return;
        }
    }
    match expr {
        Expr::CommandCall { name, args, .. } => {
            let hints = command_hints(registry, name.trim_start_matches('.'), args, locals);
            for (i, arg) in args.iter_mut().enumerate() {
                format_expr(
                    arg,
                    hints.get(i).copied().flatten(),
                    names,
                    registry,
                    locals,
                );
            }
            hooks::compact(registry, name, args, locals);
            if args.is_empty()
                && !name.starts_with("__")
                && registry
                    .lookup_entity_id(name.trim_start_matches('.'))
                    .is_none()
                && registry
                    .lookup_constant(name.trim_start_matches('.'))
                    .is_none()
            {
                *expr = Expr::Identifier(name.clone());
            }
        }
        Expr::ProcCall { name, args, .. } => {
            let hints = proc_hints(registry, name, args, locals);
            for (i, arg) in args.iter_mut().enumerate() {
                format_expr(
                    arg,
                    hints.get(i).copied().flatten(),
                    names,
                    registry,
                    locals,
                );
            }
        }
        Expr::Tuple(values) => {
            for value in values {
                format_expr(value, hint, names, registry, locals);
            }
        }
        Expr::BinaryOp {
            op: BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq,
            lhs,
            rhs,
        } => {
            let left = expression_types(lhs, registry, locals).first().copied();
            let right = expression_types(rhs, registry, locals).first().copied();
            let literal = matches!(rhs.as_ref(), Expr::BoolLiteral(_) | Expr::IntLiteral(0 | 1))
                .then(|| rhs.clone());
            format_expr(lhs, right, names, registry, locals);
            format_expr(rhs, left, names, registry, locals);
            if let Some(literal) = literal {
                *rhs = literal;
            }
        }
        Expr::BinaryOp { lhs, rhs, .. } | Expr::LogicalOp { lhs, rhs, .. } => {
            format_expr(lhs, None, names, registry, locals);
            format_expr(rhs, None, names, registry, locals);
        }
        Expr::Calc(value) => format_expr(value, None, names, registry, locals),
        Expr::ArrayAccess { index, .. } => format_expr(index, None, names, registry, locals),
        Expr::JoinedString { parts } => {
            for part in parts {
                if let StringPart::Expr(value) = part {
                    format_expr(value, None, names, registry, locals);
                }
            }
        }
        _ => {}
    }
}

fn format_body(
    body: &mut [Statement],
    names: &dyn Names,
    registry: &SymbolRegistry,
    locals: &mut SymbolTable,
    returns: &[Type],
) {
    for statement in body {
        match statement {
            Statement::VarDeclaration {
                name,
                var_type,
                value,
                ..
            } => {
                locals.define(
                    name.clone(),
                    SymbolKind::LocalVar {
                        var_type: *var_type,
                        slot: 0,
                        is_array: false,
                    },
                );
                if let Some(value) = value {
                    format_expr(value, Some(*var_type), names, registry, locals);
                }
            }
            Statement::Assignment { target, value, .. } => {
                let hints = expression_types(target, registry, locals);
                if let Expr::Tuple(values) = value {
                    for (i, value) in values.iter_mut().enumerate() {
                        format_expr(value, hints.get(i).copied(), names, registry, locals);
                    }
                } else {
                    format_expr(value, hints.first().copied(), names, registry, locals);
                }
            }
            Statement::ArrayDeclaration {
                name,
                element_type,
                size,
                ..
            } => {
                locals.define(
                    name.clone(),
                    SymbolKind::LocalVar {
                        var_type: *element_type,
                        slot: 0,
                        is_array: true,
                    },
                );
                format_expr(size, None, names, registry, locals);
            }
            Statement::Expression { expr, .. } => format_expr(expr, None, names, registry, locals),
            Statement::Return { values, .. } => {
                let hints = stack_hints(registry, returns, values, locals);
                for (value, hint) in values.iter_mut().zip(hints) {
                    format_expr(value, hint, names, registry, locals);
                }
            }
            Statement::If {
                condition,
                body,
                else_if,
                else_body,
                ..
            } => {
                format_expr(condition, None, names, registry, locals);
                format_body(body, names, registry, locals, returns);
                for (condition, body, _) in else_if {
                    format_expr(condition, None, names, registry, locals);
                    format_body(body, names, registry, locals, returns);
                }
                if let Some(body) = else_body {
                    format_body(body, names, registry, locals, returns);
                }
            }
            Statement::While {
                condition, body, ..
            } => {
                format_expr(condition, None, names, registry, locals);
                format_body(body, names, registry, locals, returns);
            }
            Statement::Switch {
                switch_type,
                expr,
                cases,
                default,
                ..
            } => {
                let hint = expression_types(expr, registry, locals).first().copied();
                if let Some(ty) = hint
                    && ty.allow_switch()
                {
                    *switch_type = format!("switch_{}", ty.name());
                }
                format_expr(expr, None, names, registry, locals);
                for case in cases {
                    for value in &mut case.values {
                        format_expr(value, hint, names, registry, locals);
                    }
                    format_body(&mut case.body, names, registry, locals, returns);
                }
                if let Some(body) = default {
                    format_body(body, names, registry, locals, returns);
                }
            }
            _ => {}
        }
    }
}

pub fn format_literals(
    script: &mut ScriptDeclaration,
    names: &dyn Names,
    registry: &SymbolRegistry,
) {
    let mut locals = SymbolTable::new();
    for param in &script.params {
        locals.define(
            param.name.clone(),
            SymbolKind::ScriptParam {
                param_type: param.param_type,
                slot: 0,
            },
        );
    }
    format_body(
        &mut script.body,
        names,
        registry,
        &mut locals,
        &script.return_types,
    );
}
