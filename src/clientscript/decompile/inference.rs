use super::Names;
use super::source::script_name;
use crate::arguments::{command_hints, command_returns, expression_types, proc_hints, stack_hints};
use crate::clientscript::hooks::{self, HookValue};
use crate::parser::{
    BinOp, Expr, ScriptDeclaration, ScriptFile, Statement, StringPart, array_name,
};
use crate::symbol::{SymbolKind, SymbolRegistry, SymbolTable};
use crate::types::{BaseVarType, Type};
use std::collections::HashMap;

struct Node {
    parent: usize,
    kind: BaseVarType,
    ty: Option<Type>,
    conflict: bool,
    /// Used as a plain number somewhere: an int kind it can't be.
    numeric: bool,
    /// A namedobj only by what takes it: holding one takes being given one.
    only_taken: bool,
}

#[derive(Clone)]
struct Scope {
    locals: HashMap<String, usize>,
    symbols: SymbolTable,
    params: Vec<usize>,
    returns: Vec<usize>,
    arrays: HashMap<i32, usize>,
    /// Arrays the script reads without defining: they come in as parameters.
    array_params: HashMap<i32, usize>,
}

pub struct Inferred {
    pub params: Vec<Type>,
    pub returns: Vec<Type>,
    pub locals: HashMap<String, Type>,
    /// The stacks of the parameters, in the order the first caller pushes them.
    pub order: Option<Vec<BaseVarType>>,
}

/// What a client event value holds.
fn event_type(name: &str) -> Option<Type> {
    Some(match name {
        "event_com" | "event_com2" => Type::Component,
        "event_keycode" => Type::Key,
        "event_keychar" => Type::Char,
        "event_opbase" => Type::String,
        "event_mousex" | "event_mousey" | "event_op" | "event_comsubid" | "event_comsubid2" => {
            Type::Int
        }
        _ => return None,
    })
}

struct Inference<'a> {
    registry: &'a SymbolRegistry,
    nodes: Vec<Node>,
    scopes: HashMap<String, Scope>,
    vars: HashMap<String, usize>,
    orders: HashMap<String, Vec<BaseVarType>>,
}

impl Inference<'_> {
    fn node(&mut self, ty: Type) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node {
            parent: id,
            kind: ty.base_type(),
            // Nothing holds a chat filter: only its literals show it.
            ty: (!matches!(ty, Type::Int | Type::Long | Type::Any | Type::ChatFilter))
                .then_some(ty),
            conflict: false,
            numeric: false,
            only_taken: false,
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
        let only_taken = match (left, right) {
            (Some(_), Some(_)) => self.nodes[a].only_taken && self.nodes[b].only_taken,
            (Some(_), None) => self.nodes[a].only_taken,
            _ => self.nodes[b].only_taken,
        };
        self.nodes[a].only_taken = only_taken;
        self.nodes[b].parent = a;
        let named = |ty: Type| matches!(ty, Type::Obj | Type::NamedObj);
        self.nodes[a].conflict |= self.nodes[b].conflict
            || left
                .zip(right)
                .is_some_and(|(a, b)| a != b && !(named(a) && named(b)));
        self.nodes[a].numeric |= self.nodes[b].numeric;
        self.nodes[a].ty = match (left, right) {
            (Some(Type::NamedObj), Some(Type::Obj)) => Some(Type::Obj),
            _ => left.or(right),
        };
    }

    fn numeric(&mut self, id: usize) {
        let id = self.root(id);
        self.nodes[id].numeric = true;
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
            && !(node.numeric && ty.is_int_kind())
        {
            return if ty == Type::NamedObj && node.only_taken {
                Type::Obj
            } else {
                ty
            };
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
                Statement::ArrayDeclaration {
                    name,
                    element_type,
                    size,
                    ..
                } => {
                    let id = self.node(*element_type);
                    scope.locals.insert(name.clone(), id);
                    scope.symbols.define(
                        name.clone(),
                        SymbolKind::LocalVar {
                            var_type: *element_type,
                            slot: 0,
                            is_array: true,
                        },
                    );
                    if let Some((_, array)) = array_name(name) {
                        scope.arrays.insert(array, id);
                        if matches!(size, Expr::NullLiteral) {
                            scope.array_params.insert(array, id);
                        }
                    }
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

    /// `&` and `|` in a calc work on numbers.
    fn calc(&mut self, expr: &Expr, scope: &Scope) -> Vec<usize> {
        if let Expr::LogicalOp { lhs, rhs, .. } = expr {
            for side in [lhs, rhs] {
                for id in self.calc(side, scope) {
                    self.numeric(id);
                }
            }
            return vec![self.node(Type::Int)];
        }
        self.expr(expr, scope, None)
    }

    fn expr(&mut self, expr: &Expr, scope: &Scope, hint: Option<Type>) -> Vec<usize> {
        let values = match expr {
            Expr::LocalVar(name, _) => scope.locals.get(name).copied().into_iter().collect(),
            Expr::StringLiteral(_) => vec![self.node(hint.unwrap_or(Type::String))],
            Expr::LongLiteral(_) => vec![self.node(Type::Long)],
            Expr::BoolLiteral(_) => vec![self.node(Type::Boolean)],
            Expr::CoordLiteral(_) => vec![self.node(Type::Coord)],
            Expr::GameVar(name, _) => {
                if let Some(&id) = self.vars.get(name) {
                    vec![id]
                } else {
                    // A variable is a boolean by being used as one, whatever it declares.
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
                        .filter(|ty| *ty != Type::Boolean)
                        .unwrap_or(Type::Int);
                    let id = self.node(ty);
                    self.vars.insert(name.clone(), id);
                    vec![id]
                }
            }
            Expr::Calc(value) => self.calc(value, scope),
            Expr::BinaryOp { op, lhs, rhs } => {
                let left = self.expr(lhs, scope, None);
                let right = self.expr(rhs, scope, None);
                if matches!(
                    op,
                    BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq
                ) {
                    if let ([a], [b]) = (left.as_slice(), right.as_slice()) {
                        self.join(*a, *b);
                    }
                } else {
                    for id in left.into_iter().chain(right) {
                        self.numeric(id);
                    }
                }
                vec![self.node(Type::Int)]
            }
            Expr::LogicalOp { lhs, rhs, .. } => {
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
                let hook = self.registry.hook_commands.get(name).copied();
                if let Some(explicit) = hook
                    && let Some(Expr::Identifier(target)) = args.first()
                    && let Some(signature) = args
                        .len()
                        .checked_sub(1 + usize::from(explicit))
                        .and_then(|index| args.get(index))
                    && let Expr::StringLiteral(signature) = signature
                {
                    let key = self
                        .scopes
                        .contains_key(&format!("clientscript:{target}"))
                        .then(|| format!("clientscript:{target}"))
                        .or_else(|| {
                            self.scopes
                                .contains_key(&format!("proc:{target}"))
                                .then(|| format!("proc:{target}"))
                        });
                    if let Some(key) = key {
                        let callee = self.scopes[&key].clone();
                        let types: Vec<_> = signature
                            .trim_end_matches('Y')
                            .chars()
                            .map(|ch| Type::from_char(ch).unwrap_or(Type::Int))
                            .collect();
                        self.orders
                            .entry(key)
                            .or_insert_with(|| types.iter().map(Type::base_type).collect());
                        let values: Vec<_> = args
                            .iter()
                            .skip(1)
                            .take(types.len())
                            .flat_map(|arg| self.expr(arg, scope, None))
                            .collect();
                        self.bind(&values, &callee.params);
                        for base in [BaseVarType::Integer, BaseVarType::String, BaseVarType::Long] {
                            let params: Vec<_> = callee
                                .params
                                .iter()
                                .copied()
                                .filter(|&param| self.nodes[param].kind == base)
                                .collect();
                            for (param, ty) in params
                                .into_iter()
                                .zip(types.iter().filter(|ty| ty.base_type() == base))
                            {
                                if !matches!(ty, Type::Int) {
                                    let target = self.node(*ty);
                                    self.join(param, target);
                                }
                            }
                        }
                    }
                }
                let hints = command_hints(self.registry, name, args, &scope.symbols);
                for (i, arg) in args.iter().enumerate() {
                    // A hook's own signature calls everything without a type an int.
                    let hint = hints
                        .get(i)
                        .copied()
                        .flatten()
                        .filter(|ty| hook.is_none() || *ty != Type::Int);
                    self.expr(arg, scope, hint);
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
                let key = if self.scopes.contains_key(&format!("proc:{name}")) {
                    format!("proc:{name}")
                } else {
                    format!("clientscript:{name}")
                };
                let callee = self.scopes.get(&key).cloned();
                let hints = proc_hints(self.registry, name, args, &scope.symbols);
                let values: Vec<_> = args
                    .iter()
                    .enumerate()
                    .flat_map(|(i, arg)| {
                        let hint = hints
                            .get(i)
                            .copied()
                            .flatten()
                            .filter(|ty| *ty != Type::Int);
                        self.expr(arg, scope, hint)
                    })
                    .collect();
                if let Some(callee) = callee {
                    let kinds = values.iter().map(|&id| self.nodes[id].kind).collect();
                    self.orders.entry(key).or_insert(kinds);
                    // An array goes by its number: both sides name the same one.
                    let mut slot = 0;
                    for arg in args {
                        if let Expr::IntLiteral(array) = arg
                            && let Some(&target) = callee.array_params.get(&slot)
                            && let Some(&source) = scope.arrays.get(array)
                        {
                            self.join(source, target);
                        }
                        if !matches!(
                            arg,
                            Expr::StringLiteral(_)
                                | Expr::JoinedString { .. }
                                | Expr::LongLiteral(_)
                        ) {
                            slot += 1;
                        }
                    }
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
            Expr::ArrayAccess { name, index } => {
                self.expr(index, scope, None);
                match scope.locals.get(name) {
                    Some(&id) => vec![id],
                    None => vec![self.node(Type::Int)],
                }
            }
            Expr::Identifier(name) if event_type(name).is_some() => {
                vec![self.node(event_type(name).unwrap())]
            }
            _ => vec![self.node(hint.unwrap_or(Type::Int))],
        };
        if let Some(ty) = hint
            && !matches!(ty, Type::Int | Type::Any)
            && values.len() == 1
        {
            let target = self.node(ty);
            self.nodes[target].only_taken = ty == Type::NamedObj;
            self.join(values[0], target);
        }
        if hint == Some(Type::Int) && values.len() == 1 {
            self.numeric(values[0]);
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

pub(crate) fn collect_returns(body: &[Statement], returned: &mut [Vec<Expr>]) {
    for statement in body {
        match statement {
            Statement::Return { values, .. } if values.len() == returned.len() => {
                for (index, value) in values.iter().enumerate() {
                    returned[index].push(value.clone());
                }
            }
            Statement::If {
                body,
                else_if,
                else_body,
                ..
            } => {
                collect_returns(body, returned);
                for (_, body, _) in else_if {
                    collect_returns(body, returned);
                }
                if let Some(body) = else_body {
                    collect_returns(body, returned);
                }
            }
            Statement::While { body, .. } => collect_returns(body, returned),
            Statement::Switch { cases, default, .. } => {
                for case in cases {
                    collect_returns(&case.body, returned);
                }
                if let Some(body) = default {
                    collect_returns(body, returned);
                }
            }
            _ => {}
        }
    }
}

/// The event a hook's value stands for.
pub fn event_name(value: i32, registry: &SymbolRegistry) -> Option<&str> {
    (i32::MIN + 1..=i32::MIN + 9)
        .contains(&value)
        .then(|| registry.integer_constant_name(value, "event_"))
        .flatten()
}

/// A hook's int as a literal of the type its script takes.
pub fn hook_literal(
    value: i32,
    ty: Option<Type>,
    names: &dyn Names,
    registry: &SymbolRegistry,
) -> Expr {
    if let Some(name) = event_name(value, registry) {
        return Expr::Identifier(name.to_owned());
    }
    let mut literal = Expr::IntLiteral(value);
    format_expr(&mut literal, ty, names, registry, &SymbolTable::new());
    literal
}

/// The scripts' types, and what each variable they use turned out to be.
pub fn infer(
    files: &[(u32, ScriptFile)],
    component_hooks: &[Vec<HookValue>],
    registry: &SymbolRegistry,
) -> (HashMap<u32, Inferred>, Vec<(String, Type)>) {
    let mut inference = Inference {
        registry,
        nodes: Vec::new(),
        scopes: HashMap::new(),
        vars: HashMap::new(),
        orders: HashMap::new(),
    };
    for (_, file) in files {
        let script = &file.scripts[0];
        let mut scope = Scope {
            locals: HashMap::new(),
            symbols: SymbolTable::new(),
            params: Vec::new(),
            returns: Vec::new(),
            arrays: HashMap::new(),
            array_params: HashMap::new(),
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
        // A script ends in its type's default value: -1 for all but an int.
        if let Some(Statement::Return { values, .. }) = script.body.last()
            && values.len() == scope.returns.len()
        {
            let mut returned = vec![Vec::new(); values.len()];
            collect_returns(&script.body, &mut returned);
            for (index, value) in values.iter().enumerate() {
                if matches!(value, Expr::IntLiteral(-1))
                    && returned[index]
                        .iter()
                        .all(|value| matches!(value, Expr::IntLiteral(-1..=1)))
                    && returned[index]
                        .iter()
                        .any(|value| matches!(value, Expr::IntLiteral(0 | 1)))
                {
                    let target = inference.node(Type::Boolean);
                    inference.join(scope.returns[index], target);
                }
            }
        }
        inference
            .scopes
            .insert(format!("{}:{}", script.trigger, script.name), scope);
    }
    for (_, file) in files {
        let script = &file.scripts[0];
        let key = format!("{}:{}", script.trigger, script.name);
        let scope = inference.scopes[&key].clone();
        inference.body(&script.body, &scope);
    }
    let keys: HashMap<i32, String> = files
        .iter()
        .map(|(id, file)| {
            let script = &file.scripts[0];
            (*id as i32, format!("{}:{}", script.trigger, script.name))
        })
        .collect();
    for hook in component_hooks {
        if let Some(HookValue::Int(script)) = hook.first()
            && let Some(key) = keys.get(script)
        {
            let callee = inference.scopes[key].clone();
            let values: Vec<_> = hook[1..]
                .iter()
                .map(|value| {
                    let ty = match value {
                        HookValue::String(_) => Type::String,
                        HookValue::Int(value) => event_name(*value, registry)
                            .and_then(event_type)
                            .unwrap_or(Type::Int),
                    };
                    inference.node(ty)
                })
                .collect();
            let kinds = values.iter().map(|&id| inference.nodes[id].kind).collect();
            inference.orders.entry(key.clone()).or_insert(kinds);
            inference.bind(&values, &callee.params);
        }
    }
    let vars: Vec<_> = inference
        .vars
        .iter()
        .map(|(name, &id)| (name.clone(), id))
        .collect();
    let vars = vars
        .into_iter()
        .map(|(name, id)| (name, inference.ty(id)))
        .collect();
    let scripts = files
        .iter()
        .map(|(id, file)| {
            let script = &file.scripts[0];
            let key = format!("{}:{}", script.trigger, script.name);
            let scope = inference.scopes[&key].clone();
            let order = inference.orders.get(&key).cloned();
            let array_params = scope.array_params.clone();
            let mut locals = scope.locals;
            for (array, id) in scope.arrays {
                locals.insert(format!("array{array}"), id);
            }
            (
                *id,
                Inferred {
                    params: scope
                        .params
                        .into_iter()
                        .enumerate()
                        .map(|(slot, id)| {
                            if array_params.contains_key(&(slot as i32)) {
                                Type::IntArray
                            } else {
                                inference.ty(id)
                            }
                        })
                        .collect(),
                    returns: scope
                        .returns
                        .into_iter()
                        .map(|id| inference.ty(id))
                        .collect(),
                    locals: locals
                        .into_iter()
                        .map(|(name, id)| (name, inference.ty(id)))
                        .collect(),
                    order,
                },
            )
        })
        .collect();
    (scripts, vars)
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
            | Type::Key
            | Type::ChatType
            | Type::ChatFilter
            | Type::WindowMode
            | Type::PlatformType
            | Type::IntBool
            | Type::IntArray
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
    if matches!(expr, Expr::LongLiteral(-1)) && hint == Some(Type::ClanForumQfc) {
        *expr = Expr::NullLiteral;
        return;
    }
    if let Expr::IntLiteral(value) = expr {
        if *value == -1
            && hint.is_some_and(|ty| {
                matches!(ty, Type::Coord | Type::Char | Type::Boolean) || ty.is_int_kind()
            })
        {
            *expr = Expr::NullLiteral;
            return;
        }
        if hint == Some(Type::IntArray) {
            let name = format!("array{value}");
            let element = match locals.lookup(&name).map(|symbol| &symbol.kind) {
                Some(SymbolKind::LocalVar { var_type, .. }) => *var_type,
                _ => Type::Int,
            };
            *expr = Expr::Identifier(format!("{}{name}", element.name()));
            return;
        }
        if hint == Some(Type::Coord) && (0..=0x3fffffff).contains(value) {
            *expr = Expr::CoordLiteral(*value);
            return;
        }
        let constant = match (*value, hint) {
            (i32::MAX, _) => Some("max_32bit_int"),
            (i32::MIN, _) => Some("min_32bit_int"),
            (0, Some(Type::IntBool)) => Some("false"),
            (1, Some(Type::IntBool)) => Some("true"),
            _ => None,
        };
        let prefixed = match hint {
            Some(Type::Key) => Some("key_"),
            Some(Type::ChatType) => Some("chattype_"),
            Some(Type::ChatFilter) => Some("chatfilter_"),
            Some(Type::WindowMode) => Some("windowmode_"),
            Some(Type::PlatformType) => Some("platformtype_"),
            _ => None,
        }
        .and_then(|prefix| registry.integer_constant_name(*value, prefix));
        if let Some(name) = constant.or(prefixed)
            && registry.lookup_constant(name).is_some()
        {
            *expr = Expr::ConstantVar(name.to_owned(), 0);
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
            *expr = Expr::Identifier(if (*value as u32) > 0xffffff {
                format!("0x{:08x}", *value as u32)
            } else {
                format!("0x{:06x}", *value as u32)
            });
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
            // What a hook's script takes says how its arguments print.
            if let Some(end) = hooks::arguments_end(registry, name, args)
                && let Some(Expr::Identifier(target)) = args.first().cloned()
            {
                let hints = hooks::argument_hints(registry, &target, &args[1..end], locals);
                for (arg, hint) in args[1..end].iter_mut().zip(hints) {
                    format_expr(arg, hint, names, registry, locals);
                }
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
            op: op @ (BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq),
            lhs,
            rhs,
        } => {
            // Only `=` and `!` read a value by the other side's type.
            let equality = matches!(op, BinOp::Eq | BinOp::NotEq);
            let left = expression_types(lhs, registry, locals)
                .first()
                .copied()
                .filter(|_| equality);
            let right = expression_types(rhs, registry, locals)
                .first()
                .copied()
                .filter(|_| equality);
            format_expr(lhs, right, names, registry, locals);
            format_expr(rhs, left, names, registry, locals);
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
                    *switch_type = format!("switch_{}", ty.declared_name());
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
