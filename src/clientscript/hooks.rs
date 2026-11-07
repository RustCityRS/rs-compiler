use crate::arguments::{expression_types, stack_hints};
use crate::lexer::Lexer;
use crate::parser::{Expr, Parser, ScriptDeclaration};
use crate::symbol::{SymbolKind, SymbolRegistry, SymbolTable};
use crate::types::{BaseVarType, Type};
use std::path::PathBuf;

pub(crate) fn transmit_kind(command: &str) -> Option<&'static str> {
    let command = command.trim_start_matches('.');
    match command
        .strip_prefix("cc_")
        .or_else(|| command.strip_prefix("if_"))?
    {
        "setonvartransmit" => Some("varp"),
        "setonvarctransmit" => Some("varc_int"),
        "setonvarcstrtransmit" => Some("varc_string"),
        "setoninvtransmit" => Some("inv"),
        "setonstattransmit" => Some("stat"),
        _ => None,
    }
}

pub(crate) fn transmit_id(registry: &SymbolRegistry, kind: &str, name: &str) -> Option<i32> {
    if kind.starts_with("var") {
        match registry.lookup_game_var(name).map(|symbol| &symbol.kind) {
            Some(SymbolKind::GameVar {
                var_id, category, ..
            }) if category == kind => Some(*var_id),
            None => name
                .strip_prefix(kind)?
                .strip_prefix('_')?
                .parse::<u16>()
                .ok()
                .map(i32::from),
            _ => None,
        }
    } else {
        registry.lookup_reference(name, Type::from_name(kind)?)
    }
}

fn transmit_name(registry: &SymbolRegistry, kind: &str, id: i32) -> Option<String> {
    if kind.starts_with("var") {
        registry
            .game_variables()
            .filter_map(|(name, symbol)| {
                matches!(&symbol.kind, SymbolKind::GameVar { var_id, category, .. }
                if *var_id == id && category == kind)
                .then_some(name)
            })
            .min()
            .map(str::to_owned)
            .or_else(|| u16::try_from(id).ok().map(|_| format!("{kind}_{id}")))
    } else {
        let ty = Type::from_name(kind)?;
        registry
            .entity_ids
            .iter()
            .filter_map(|(name, entity)| {
                entity.variants.contains(&(ty, id)).then_some(name.as_str())
            })
            .min()
            .map(str::to_owned)
    }
}

fn resolve_transmits(
    values: &mut [Expr],
    command: &str,
    registry: &SymbolRegistry,
) -> Result<(), String> {
    if let Some(kind) = transmit_kind(command)
        && let Some(Expr::Tuple(transmits)) = values.get_mut(2)
    {
        for value in transmits {
            if let Expr::Identifier(name) = value {
                let id = transmit_id(registry, kind, name)
                    .ok_or_else(|| format!("Unknown {kind} transmit trigger '{name}'"))?;
                *value = Expr::IntLiteral(id);
            }
        }
    }
    Ok(())
}

pub(crate) fn lower(
    registry: &SymbolRegistry,
    name: &str,
    args: &[Expr],
    locals: &SymbolTable,
    line: usize,
) -> Result<Option<Vec<Expr>>, String> {
    let Some(&explicit) = registry.hook_commands.get(name.trim_start_matches('.')) else {
        return Ok(None);
    };
    if args.len() != 1 + usize::from(explicit) {
        return Ok(None);
    }
    let mut lowered = match &args[0] {
        Expr::NullLiteral => vec![Expr::NullLiteral, Expr::StringLiteral(String::new())],
        Expr::StringLiteral(text) => {
            let mut values = parse_at(text, line)?;
            resolve_transmits(&mut values, name, registry)?;
            if let (Expr::Identifier(target), Expr::Tuple(args)) = (&values[0], &values[1])
                && let Some(SymbolKind::Script { param_types, .. }) = registry
                    .lookup_script_by_trigger("clientscript", target)
                    .or_else(|| registry.lookup_script_by_trigger("proc", target))
                    .map(|symbol| &symbol.kind)
            {
                let expanded = expand(&values, registry, locals);
                let Expr::StringLiteral(signature) = expanded.last().unwrap() else {
                    unreachable!()
                };
                let mut expected = [0usize; 3];
                let mut actual = [0usize; 3];
                let stack = |ty: Type| match ty.base_type() {
                    BaseVarType::Integer => 0,
                    BaseVarType::String => 1,
                    BaseVarType::Long => 2,
                };
                for ty in param_types {
                    expected[stack(*ty)] += 1;
                }
                for ch in signature.trim_end_matches('Y').chars() {
                    actual[stack(Type::from_char(ch).unwrap_or(Type::Int))] += 1;
                }
                if expected != actual {
                    return Err(format!(
                        "Hook '{target}' expects {expected:?} int/string/long arguments, got {actual:?} ({} expressions)",
                        args.len()
                    ));
                }
            }
            expand(&values, registry, locals)
        }
        Expr::CommandCall {
            name: marker, args, ..
        } if marker == "__hook" => {
            let mut values = args.clone();
            resolve_transmits(&mut values, name, registry)?;
            expand(&values, registry, locals)
        }
        _ => return Ok(None),
    };
    if explicit {
        lowered.push(args[1].clone());
    }
    Ok(Some(lowered))
}

pub(crate) fn parse(text: &str) -> Result<Vec<Expr>, String> {
    parse_at(text, 1)
}

fn parse_at(text: &str, line: usize) -> Result<Vec<Expr>, String> {
    let path = PathBuf::from("hook");
    let mut tokens = Lexer::new(text, &path)
        .tokenize()
        .map_err(|e| e.to_string())?;
    for token in &mut tokens {
        token.line += line.saturating_sub(1);
    }
    Parser::new(tokens, &path)
        .parse_hook()
        .map_err(|e| e.to_string())
}

pub(crate) fn register_callbacks<'a>(
    registry: &mut SymbolRegistry,
    scripts: impl Iterator<Item = &'a ScriptDeclaration>,
) {
    let mut scripts: Vec<_> = scripts
        .filter(|script| matches!(script.trigger.as_str(), "proc" | "clientscript"))
        .collect();
    scripts.sort_by_key(|script| script.trigger == "clientscript");
    for script in scripts {
        let id = registry
            .script_id_for_trigger("clientscript", &script.name)
            .or_else(|| {
                script
                    .name
                    .strip_prefix("clientscript_")
                    .and_then(|id| id.parse::<i32>().ok())
                    .filter(|id| *id >= 0)
            });
        if let Some(id) = id {
            registry.register_clientscript(
                script.name.clone(),
                id,
                script.params.iter().map(|param| param.param_type).collect(),
            );
        }
    }
}

pub(crate) fn server_call(
    registry: &SymbolRegistry,
    name: &str,
    args: &[Expr],
    locals: &SymbolTable,
    line: usize,
) -> Result<Option<Expr>, String> {
    let command = name.trim_start_matches('.');
    if !matches!(
        command,
        "runclientscript" | "runclientscript*" | "runclientscriptvararg"
    ) {
        return Ok(None);
    }
    let prefix = if name.starts_with('.') { "." } else { "" };
    let syntax_error = || {
        format!(
            "Use {prefix}runclientscript(\"script(args)\"); the callback must be one quoted expression"
        )
    };
    if command != "runclientscript" {
        return Err(syntax_error());
    }
    let [Expr::StringLiteral(text)] = args else {
        return Err(syntax_error());
    };
    let values = parse_at(text, line)?;
    if values.len() > 2 {
        return Err("runclientscript does not accept a transmit list".into());
    }
    let Expr::Identifier(target) = &values[0] else {
        return Err("Expected clientscript name".into());
    };
    let id = registry
        .script_id_for_trigger("clientscript", target)
        .ok_or_else(|| format!("Unknown clientscript '{target}'"))?;
    let params = registry
        .client_script_params
        .get(target)
        .or_else(|| {
            match registry
                .lookup_script_by_trigger("clientscript", target)
                .map(|symbol| &symbol.kind)
            {
                Some(SymbolKind::Script { param_types, .. }) => Some(param_types),
                _ => None,
            }
        })
        .ok_or_else(|| format!("Missing clientscript declaration for '{target}'"))?;
    let Expr::Tuple(args) = &values[1] else {
        unreachable!()
    };
    let hints = stack_hints(registry, params, args, locals);
    let mut actual = [0usize; 3];
    let mut expected = [0usize; 3];
    let stack = |ty: Type| match ty.base_type() {
        BaseVarType::Integer => 0,
        BaseVarType::String => 1,
        BaseVarType::Long => 2,
    };
    for ty in params {
        expected[stack(*ty)] += 1;
    }
    let mut lowered = vec![Expr::IntLiteral(id)];
    for (arg, hint) in args.iter().zip(hints) {
        let mut arg = arg.clone();
        if let (Expr::Identifier(value) | Expr::StringLiteral(value), Some(ty)) = (&arg, hint)
            && let Some(id) = registry.lookup_reference(value, ty)
        {
            arg = Expr::IntLiteral(id);
        }
        for ty in expression_types(&arg, registry, locals) {
            actual[stack(ty)] += 1;
        }
        lowered.push(arg);
    }
    if actual != expected {
        return Err(format!(
            "Clientscript '{target}' expects {expected:?} int/string/long arguments, got {actual:?}"
        ));
    }
    if actual[2] != 0 {
        return Err("runclientscript does not support long arguments".into());
    }
    let prefix = if name.starts_with('.') { "." } else { "" };
    let suffix = if args.is_empty() { "" } else { "*" };
    let count = lowered.len();
    Ok(Some(Expr::CommandCall {
        name: format!("{prefix}runclientscript{suffix}"),
        args: lowered,
        arg_lines: vec![line; count],
        call_line: line,
    }))
}

fn expand(values: &[Expr], registry: &SymbolRegistry, locals: &SymbolTable) -> Vec<Expr> {
    let Expr::Tuple(args) = &values[1] else {
        unreachable!()
    };
    let hints = if let Expr::Identifier(name) = &values[0] {
        match registry
            .lookup_script_by_trigger("clientscript", name)
            .or_else(|| registry.lookup_script_by_trigger("proc", name))
            .map(|symbol| &symbol.kind)
        {
            Some(SymbolKind::Script { param_types, .. }) => {
                stack_hints(registry, param_types, args, locals)
            }
            _ => vec![None; args.len()],
        }
    } else {
        vec![None; args.len()]
    };
    let mut signature = String::new();
    for (arg, hint) in args.iter().zip(hints) {
        let types = expression_types(arg, registry, locals);
        if types.len() == 1 {
            signature.push(hint.unwrap_or(types[0]).to_char());
        } else {
            signature.extend(types.iter().map(|ty| ty.to_char()));
        }
    }
    let mut lowered = vec![values[0].clone()];
    lowered.extend_from_slice(args);
    if let Some(Expr::Tuple(transmits)) = values.get(2) {
        lowered.extend_from_slice(transmits);
        lowered.push(Expr::IntLiteral(transmits.len() as i32));
        signature.push('Y');
    }
    lowered.push(Expr::StringLiteral(signature));
    lowered
}

pub(crate) fn compact(
    registry: &SymbolRegistry,
    name: &str,
    args: &mut Vec<Expr>,
    locals: &SymbolTable,
) {
    let Some(&explicit) = registry.hook_commands.get(name.trim_start_matches('.')) else {
        return;
    };
    let Some(signature_index) = args.len().checked_sub(1 + usize::from(explicit)) else {
        return;
    };
    let Expr::StringLiteral(signature) = &args[signature_index] else {
        return;
    };
    let mut end = signature_index;
    let transmits = if signature.ends_with('Y') {
        let Some(Expr::IntLiteral(count)) = end.checked_sub(1).and_then(|i| args.get(i)) else {
            return;
        };
        let Ok(count) = usize::try_from(*count) else {
            return;
        };
        let Some(start) = end.checked_sub(count + 1) else {
            return;
        };
        let mut values = args[start..end - 1].to_vec();
        if let Some(kind) = transmit_kind(name) {
            for value in &mut values {
                if let Expr::IntLiteral(id) = value
                    && let Some(name) = transmit_name(registry, kind, *id)
                {
                    *value = Expr::Identifier(name);
                }
            }
        }
        end = start;
        Some(Expr::Tuple(values))
    } else {
        None
    };
    if end == 0
        || !matches!(
            args[0],
            Expr::Identifier(_) | Expr::IntLiteral(_) | Expr::NullLiteral
        )
    {
        return;
    }
    let hook = if matches!(args[0], Expr::NullLiteral) && signature.is_empty() && end == 1 {
        Expr::NullLiteral
    } else {
        let mut values = vec![args[0].clone(), Expr::Tuple(args[1..end].to_vec())];
        values.extend(transmits);
        let expanded = expand(&values, registry, locals);
        if !matches!(expanded.last(), Some(Expr::StringLiteral(value)) if value == signature) {
            return;
        }
        Expr::CommandCall {
            name: "__hook".into(),
            args: values,
            arg_lines: Vec::new(),
            call_line: 0,
        }
    };
    let mut compact = vec![hook];
    if explicit {
        compact.push(args[signature_index + 1].clone());
    }
    *args = compact;
}
