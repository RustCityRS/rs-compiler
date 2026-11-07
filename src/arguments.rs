use crate::clientscript::hook_types;
use crate::parser::Expr;
use crate::symbol::{ReturnType, SymbolKind, SymbolRegistry, SymbolTable};
use crate::types::{BaseVarType, Type};
use std::collections::VecDeque;

fn stack(ty: Type) -> usize {
    match ty.base_type() {
        BaseVarType::Integer => 0,
        BaseVarType::String => 1,
        BaseVarType::Long => 2,
    }
}

pub fn expression_types(expr: &Expr, registry: &SymbolRegistry, locals: &SymbolTable) -> Vec<Type> {
    let symbol = match expr {
        Expr::Tuple(values) | Expr::MultiAssign(values, _) => {
            return values
                .iter()
                .flat_map(|value| expression_types(value, registry, locals))
                .collect();
        }
        Expr::StringLiteral(_) | Expr::JoinedString { .. } => return vec![Type::String],
        Expr::LongLiteral(_) => return vec![Type::Long],
        Expr::LocalVar(name, _) | Expr::ArrayAccess { name, .. } => locals.lookup(name),
        Expr::GameVar(name, _) => registry.lookup_game_var(name.trim_start_matches('.')),
        Expr::ConstantVar(name, _) => registry.lookup_constant(name),
        Expr::CommandCall { name, args, .. } => return command_returns(registry, name, args),
        Expr::Identifier(name) => registry.lookup_command(name.trim_start_matches('.')),
        Expr::ProcCall { name, .. } => registry
            .lookup_script_by_trigger("proc", name)
            .or_else(|| registry.lookup_script_by_trigger("clientscript", name)),
        Expr::Calc(value) | Expr::BinaryOp { lhs: value, .. } => {
            return expression_types(value, registry, locals);
        }
        _ => None,
    };
    match symbol.map(|symbol| &symbol.kind) {
        Some(SymbolKind::LocalVar { var_type, .. } | SymbolKind::GameVar { var_type, .. }) => {
            vec![*var_type]
        }
        Some(SymbolKind::ScriptParam { param_type, .. }) => vec![*param_type],
        Some(SymbolKind::Constant { const_type, .. }) => vec![*const_type],
        Some(
            SymbolKind::Command { return_types, .. } | SymbolKind::Script { return_types, .. },
        ) => return_types.clone(),
        _ => vec![Type::Int],
    }
}

pub fn command_returns(registry: &SymbolRegistry, name: &str, args: &[Expr]) -> Vec<Type> {
    let name = name.trim_start_matches('.');
    let value = |index: usize, hint: Type| {
        args.get(index).and_then(|arg| match arg {
            Expr::IntLiteral(value) => Some(*value),
            Expr::CharLiteral(value) => Some(*value as i32),
            Expr::Identifier(name) => registry
                .lookup_reference(name, hint)
                .or_else(|| registry.type_chars.get(name).copied()),
            Expr::ConstantVar(name, _) => registry.lookup_constant(name).and_then(|symbol| {
                if let SymbolKind::Constant { int_value, .. } = symbol.kind {
                    int_value
                } else {
                    None
                }
            }),
            _ => None,
        })
    };
    let dynamic = match registry.dynamic_returns.get(name) {
        Some(ReturnType::TypeArgument(index)) => value(*index, Type::Int)
            .and_then(|value| char::from_u32(value as u32))
            .and_then(Type::from_char),
        Some(ReturnType::ParamArgument(index)) => value(*index, Type::Param)
            .map(|id| registry.param_types.get(&id).copied().unwrap_or(Type::Int)),
        None => None,
    };
    if let Some(ty) = dynamic {
        return vec![ty];
    }
    match registry.lookup_command(name).map(|symbol| &symbol.kind) {
        Some(SymbolKind::Command { return_types, .. }) => return_types.clone(),
        _ => Vec::new(),
    }
}

pub fn command_hints(
    registry: &SymbolRegistry,
    name: &str,
    args: &[Expr],
    locals: &SymbolTable,
) -> Vec<Option<Type>> {
    let mut params = registry
        .command_param_types
        .get(name)
        .cloned()
        .unwrap_or_default();
    if !registry.stack_commands.contains(name) {
        return (0..args.len()).map(|i| params.get(i).copied()).collect();
    }
    if let Some(&explicit) = registry.hook_commands.get(name)
        && let Some(signature) = args.iter().rev().find_map(|arg| match arg {
            Expr::StringLiteral(value) => Some(value.as_str()),
            _ => None,
        })
    {
        let triggers = if signature.ends_with('Y') {
            args.iter()
                .rev()
                .filter(|arg| {
                    expression_types(arg, registry, locals)
                        .first()
                        .is_some_and(|ty| stack(*ty) == 0)
                })
                .nth(usize::from(explicit))
                .and_then(|arg| match arg {
                    Expr::IntLiteral(n) => usize::try_from(*n).ok(),
                    _ => None,
                })
                .unwrap_or(0)
        } else {
            0
        };
        params = hook_types(signature, triggers, explicit);
    }
    stack_hints(registry, &params, args, locals)
}

pub fn proc_hints(
    registry: &SymbolRegistry,
    name: &str,
    args: &[Expr],
    locals: &SymbolTable,
) -> Vec<Option<Type>> {
    let symbol = registry
        .lookup_script_by_trigger("proc", name)
        .or_else(|| registry.lookup_script_by_trigger("clientscript", name));
    let Some(SymbolKind::Script {
        trigger,
        param_types,
        ..
    }) = symbol.map(|symbol| &symbol.kind)
    else {
        return vec![None; args.len()];
    };
    if registry
        .stack_scripts
        .contains(&format!("{trigger}:{name}"))
    {
        stack_hints(registry, param_types, args, locals)
    } else {
        (0..args.len())
            .map(|i| param_types.get(i).copied())
            .collect()
    }
}

pub fn stack_hints(
    registry: &SymbolRegistry,
    params: &[Type],
    args: &[Expr],
    locals: &SymbolTable,
) -> Vec<Option<Type>> {
    let mut pending: [VecDeque<Type>; 3] = Default::default();
    for &ty in params {
        pending[stack(ty)].push_back(ty);
    }
    args.iter()
        .enumerate()
        .map(|(i, arg)| {
            let mut types = expression_types(arg, registry, locals);
            if let Expr::StringLiteral(name) | Expr::Identifier(name) = arg
                && let Some(&ty) = pending[0].front()
                && registry.lookup_reference(name, ty).is_some()
                && (matches!(arg, Expr::Identifier(_))
                    || args[i..]
                        .iter()
                        .flat_map(|arg| expression_types(arg, registry, locals))
                        .filter(|ty| stack(*ty) == 1)
                        .count()
                        > pending[1].len())
            {
                types = vec![ty];
            }
            let hint = types
                .first()
                .and_then(|ty| pending[stack(*ty)].front().copied());
            for ty in types {
                pending[stack(ty)].pop_front();
            }
            hint
        })
        .collect()
}
