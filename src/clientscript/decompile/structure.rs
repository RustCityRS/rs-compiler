use super::Names;
use super::inference;
use crate::clientscript;
use crate::parser::{BinOp, Expr, LogicOp, ScriptDeclaration, Statement, StringPart, SwitchCase};
use crate::symbol::SymbolRegistry;
use crate::types::{BaseVarType, Type};
use anyhow::{Result, bail};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt::Write;
use std::mem::take;
use std::path::Path;

fn call(statement: &Statement, expected: &str) -> Option<Vec<Expr>> {
    if let Statement::Expression {
        expr: Expr::CommandCall { name, args, .. },
        ..
    } = statement
        && name == expected
    {
        return Some(args.clone());
    }
    None
}

fn label(statement: &Statement, name: &str) -> Option<String> {
    let args = call(statement, name)?;
    if let [Expr::Identifier(label)] = args.as_slice() {
        Some(label.clone())
    } else {
        None
    }
}

fn branch(statement: &Statement) -> Option<(Expr, String)> {
    let args = call(statement, "__branch")?;
    let [
        Expr::Identifier(opcode),
        Expr::Identifier(target),
        values @ ..,
    ] = args.as_slice()
    else {
        return None;
    };
    let condition = match (opcode.as_str(), values) {
        ("BranchIfTrue", [value]) => Expr::BinaryOp {
            op: BinOp::Eq,
            lhs: Box::new(value.clone()),
            rhs: Box::new(Expr::BoolLiteral(true)),
        },
        ("BranchIfFalse", [value]) => Expr::BinaryOp {
            op: BinOp::Eq,
            lhs: Box::new(value.clone()),
            rhs: Box::new(Expr::BoolLiteral(false)),
        },
        (name, [lhs, rhs]) => Expr::BinaryOp {
            op: match name.strip_prefix("Long").unwrap_or(name) {
                "BranchNot" => BinOp::NotEq,
                "BranchEquals" => BinOp::Eq,
                "BranchLessThan" => BinOp::Lt,
                "BranchGreaterThan" => BinOp::Gt,
                "BranchLessThanOrEquals" => BinOp::LtEq,
                "BranchGreaterThanOrEquals" => BinOp::GtEq,
                _ => return None,
            },
            lhs: Box::new(lhs.clone()),
            rhs: Box::new(rhs.clone()),
        },
        _ => return None,
    };
    Some((condition, target.clone()))
}

fn condition(
    body: &[Statement],
    start: usize,
    labels: &HashMap<String, usize>,
    references: &HashMap<String, usize>,
) -> Option<(Expr, String, usize)> {
    branch(body.get(start)?)?;
    let mut end = start;
    while end < body.len()
        && (branch(&body[end]).is_some()
            || label(&body[end], "__jump").is_some()
            || label(&body[end], "__label").is_some())
    {
        end += 1;
    }
    let mut memo = HashMap::new();
    for yes in (start + 2..end).rev() {
        let Some(_) = label(&body[yes], "__label") else {
            continue;
        };
        let Some(no) = label(&body[yes - 1], "__jump") else {
            continue;
        };
        let Some(&no_pos) = labels.get(&no) else {
            continue;
        };
        let mut internal = HashMap::new();
        for statement in &body[start..yes] {
            if let Some(target) = branch(statement)
                .map(|(_, target)| target)
                .or_else(|| label(statement, "__jump"))
            {
                *internal.entry(target).or_insert(0) += 1;
            }
        }
        if body[start..yes]
            .iter()
            .filter_map(|statement| label(statement, "__label"))
            .any(|name| internal.get(&name) != references.get(&name))
        {
            continue;
        }
        if let Some(expr) = condition_range(body, start, yes, yes, no_pos, false, labels, &mut memo)
        {
            return Some((expr, no, yes - 1));
        }
    }
    None
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn condition_range(
    body: &[Statement],
    start: usize,
    end: usize,
    yes: usize,
    no: usize,
    fallthrough: bool,
    labels: &HashMap<String, usize>,
    memo: &mut HashMap<(usize, usize, usize, usize, bool), Option<Expr>>,
) -> Option<Expr> {
    let key = (start, end, yes, no, fallthrough);
    if let Some(result) = memo.get(&key) {
        return result.clone();
    }
    let indices: Vec<_> = (start..end)
        .filter(|&i| label(&body[i], "__label").is_none())
        .collect();
    let result = (|| {
        if let Some((&first, rest)) = indices.split_first()
            && let Some((expr, target)) = branch(&body[first])
            && labels.get(&target) == Some(&yes)
            && ((rest.is_empty() && fallthrough && no == end)
                || (rest.len() == 1
                    && label(&body[rest[0]], "__jump")
                        .as_ref()
                        .and_then(|name| labels.get(name))
                        == Some(&no)))
        {
            return Some(expr);
        }
        for split in start + 1..end {
            if branch(&body[split]).is_none() && label(&body[split], "__label").is_none() {
                continue;
            }
            if let Some(lhs) = condition_range(body, start, split, split, no, false, labels, memo)
                && let Some(rhs) =
                    condition_range(body, split, end, yes, no, fallthrough, labels, memo)
            {
                return Some(Expr::LogicalOp {
                    op: LogicOp::And,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                    rhs_line: 0,
                });
            }
            if let Some(lhs) = condition_range(body, start, split, yes, split, true, labels, memo)
                && let Some(rhs) =
                    condition_range(body, split, end, yes, no, fallthrough, labels, memo)
            {
                return Some(Expr::LogicalOp {
                    op: LogicOp::Or,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                    rhs_line: 0,
                });
            }
        }
        None
    })();
    memo.insert(key, result.clone());
    result
}

fn reconstruct(
    body: &[Statement],
    references: &HashMap<String, usize>,
    end: Option<&str>,
) -> Vec<Statement> {
    let mut labels: HashMap<_, _> = body
        .iter()
        .enumerate()
        .filter_map(|(i, statement)| label(statement, "__label").map(|name| (name, i)))
        .collect();
    if let Some(end) = end {
        labels.insert(end.to_owned(), body.len());
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < body.len() {
        if let Some((condition, target)) = branch(&body[i])
            && body
                .get(i + 1)
                .and_then(|statement| label(statement, "__jump"))
                .as_ref()
                == Some(&target)
            && labels.get(&target) == Some(&(i + 2))
        {
            out.push(Statement::If {
                condition,
                body: Vec::new(),
                else_if: Vec::new(),
                else_body: None,
                else_line: 0,
                line: 0,
            });
            i += 2;
            continue;
        }
        if let Some((statement, next)) = switch(body, i, &labels, references) {
            out.push(statement);
            i = next;
            continue;
        }
        if let Some((condition, _no, cursor)) = condition(body, i, &labels, references) {
            let yes = label(&body[cursor + 1], "__label").unwrap();
            if let Some(no) = body.get(cursor).and_then(|s| label(s, "__jump"))
                && labels.get(&yes) == Some(&(cursor + 1))
                && let Some(&otherwise) = labels.get(&no)
            {
                if otherwise == cursor + 1 {
                    out.push(Statement::If {
                        condition,
                        body: vec![],
                        else_if: vec![],
                        else_body: None,
                        else_line: 0,
                        line: 0,
                    });
                    i = otherwise;
                    continue;
                }
                if otherwise > cursor + 2
                    && let Some(end) = label(&body[otherwise - 1], "__jump")
                {
                    if i > 0 && label(&body[i - 1], "__label").as_ref() == Some(&end) {
                        out.push(Statement::While {
                            condition,
                            body: reconstruct(&body[cursor + 1..otherwise - 1], references, None),
                            line: 0,
                        });
                        i = otherwise;
                        continue;
                    }
                    if let Some(&end_index) = labels.get(&end) {
                        if end_index == otherwise {
                            let nested = body[cursor + 1..otherwise - 1].iter().any(|statement| {
                                label(statement, "__jump").as_ref() == Some(&no)
                                    || branch(statement).is_some_and(|(_, target)| target == no)
                            });
                            out.push(Statement::If {
                                condition,
                                body: reconstruct(
                                    &body[cursor + 1..otherwise - usize::from(!nested)],
                                    references,
                                    Some(&no),
                                ),
                                else_if: vec![],
                                else_body: (!nested).then(Vec::new),
                                else_line: 0,
                                line: 0,
                            });
                            i = end_index;
                            continue;
                        }
                        if end_index > otherwise {
                            out.push(Statement::If {
                                condition,
                                body: reconstruct(
                                    &body[cursor + 1..otherwise - 1],
                                    references,
                                    None,
                                ),
                                else_if: vec![],
                                else_body: Some(reconstruct(
                                    &body[otherwise..end_index],
                                    references,
                                    Some(&end),
                                )),
                                else_line: 0,
                                line: 0,
                            });
                            i = end_index;
                            continue;
                        }
                    }
                }
                if otherwise >= cursor + 2 {
                    out.push(Statement::If {
                        condition,
                        body: reconstruct(&body[cursor + 1..otherwise], references, Some(&no)),
                        else_if: vec![],
                        else_body: None,
                        else_line: 0,
                        line: 0,
                    });
                    i = otherwise;
                    continue;
                }
            }
        }
        out.push(body[i].clone());
        i += 1;
    }
    out
}

fn switch(
    body: &[Statement],
    i: usize,
    labels: &HashMap<String, usize>,
    references: &HashMap<String, usize>,
) -> Option<(Statement, usize)> {
    let args = call(&body[i], "__switch")?;
    let (expr, pairs) = args.split_first()?;
    let default_pos = if let Some(default_label) = label(body.get(i + 1)?, "__jump") {
        *labels.get(&default_label)?
    } else {
        i + 1
    };
    let mut cases: Vec<(usize, Vec<Expr>)> = Vec::new();
    for pair in pairs.as_chunks::<2>().0 {
        let [value, Expr::Identifier(target)] = pair else {
            return None;
        };
        let position = *labels.get(target)?;
        if position <= i {
            return None;
        }
        if let Some((_, values)) = cases.iter_mut().find(|(pos, _)| *pos == position) {
            values.push(value.clone());
        } else {
            cases.push((position, vec![value.clone()]));
        }
    }
    cases.sort_by_key(|(position, _)| *position);
    if default_pos != i + 1 && cases.first()?.0 != i + 2 {
        return None;
    }
    let first_end = if default_pos == i + 1 {
        cases.first()?.0
    } else {
        cases
            .get(1)
            .map_or(default_pos, |(pos, _)| (*pos).min(default_pos))
    };
    let end_label = label(body.get(first_end.checked_sub(1)?)?, "__jump").or_else(|| {
        (cases.len() == 1 && default_pos > cases[0].0)
            .then(|| label(&body[i + 1], "__jump"))
            .flatten()
    })?;
    let end = *labels.get(&end_label)?;
    if end <= i || default_pos > end || cases.iter().any(|(pos, _)| *pos >= end) {
        return None;
    }
    let trailing_jump = default_pos == end
        && label(body.get(end.checked_sub(1)?)?, "__jump").as_ref() == Some(&end_label);
    let mut result = Vec::new();
    for (index, (pos, values)) in cases.iter().enumerate() {
        let mut next = cases.get(index + 1).map_or(end, |(pos, _)| *pos);
        if default_pos > *pos {
            next = next.min(default_pos);
        }
        let stop = if next == end {
            next - usize::from(trailing_jump)
        } else {
            if label(body.get(next.checked_sub(1)?)?, "__jump").as_ref() != Some(&end_label) {
                return None;
            }
            next - 1
        };
        result.push(SwitchCase {
            values: values.clone(),
            body: reconstruct(&body[*pos..stop], references, Some(&end_label)),
        });
    }
    let default_index = cases.iter().filter(|(pos, _)| *pos < default_pos).count();
    let default = if default_pos == end {
        trailing_jump.then(Vec::new)
    } else {
        let next = cases
            .iter()
            .find(|(pos, _)| *pos > default_pos)
            .map_or(end, |(pos, _)| *pos);
        let stop = if next != end {
            if label(body.get(next - 1)?, "__jump").as_ref() != Some(&end_label) {
                return None;
            }
            next - 1
        } else {
            next
        };
        Some(reconstruct(
            &body[default_pos..stop],
            references,
            Some(&end_label),
        ))
    };
    Some((
        Statement::Switch {
            switch_type: "switch_int".into(),
            expr: expr.clone(),
            cases: result,
            default,
            default_index,
            line: 0,
        },
        end,
    ))
}

pub fn generate(
    source: &str,
    locals: &HashMap<String, Type>,
    keep_end: bool,
    returns: &HashMap<String, usize>,
    names: &dyn Names,
    registry: &SymbolRegistry,
) -> Result<String> {
    let mut file = clientscript::parse(source, Path::new("clientscript.cs2"))?;
    let script = &mut file.scripts[0];
    let mut references = HashMap::new();
    for statement in &script.body {
        let args = call(statement, "__jump")
            .or_else(|| call(statement, "__branch"))
            .or_else(|| call(statement, "__switch"))
            .unwrap_or_default();
        for arg in args {
            if let Expr::Identifier(name) = arg {
                *references.entry(name).or_insert(0) += 1;
            }
        }
    }
    fold_branch_values(&mut script.body, returns);
    script.body = reconstruct(&script.body, &references, None);
    let mut referenced = HashMap::new();
    collect_references(&script.body, &mut referenced);
    remove_labels(&mut script.body, &referenced);
    fold_assignments(&mut script.body, returns);
    let mut arrays = BTreeMap::new();
    restore_arrays(&mut script.body, &mut arrays);
    fold_stack(&mut script.body, returns);
    // An array read before anything defines it came in as a parameter.
    for (&id, &element) in arrays.iter().rev() {
        let name = format!("array{id}");
        let defined = script.body.iter().find(|statement| {
            statement_uses(statement, &name)
                || matches!(statement, Statement::ArrayDeclaration { name: declared, .. } if *declared == name)
        });
        if !defined.is_some_and(|statement| matches!(statement, Statement::ArrayDeclaration { .. }))
        {
            script.body.insert(
                0,
                Statement::ArrayDeclaration {
                    element_type: locals.get(&name).copied().unwrap_or(element),
                    name,
                    size: Expr::NullLiteral,
                    line: 0,
                },
            );
        }
    }
    merge_declarations(&mut script.body);
    if let Some(Statement::Return { values, .. }) = script.body.last()
        && !keep_end
        && values.len() == script.return_types.len()
        && values
            .iter()
            .zip(&script.return_types)
            .all(|(value, ty)| match value {
                Expr::IntLiteral(value) => {
                    ty.base_type() == BaseVarType::Integer && *value == ty.default_return_value()
                }
                Expr::StringLiteral(value) => {
                    ty.base_type() == BaseVarType::String && value.is_empty()
                }
                Expr::LongLiteral(value) => ty.base_type() == BaseVarType::Long && *value == 0,
                _ => false,
            })
    {
        script.body.pop();
    }
    inference::format_literals(script, names, registry);
    declare_array_params(script);
    rename_locals(script);
    let params = script
        .params
        .iter()
        .map(|param| {
            let ty = if param.param_type == Type::IntArray {
                param.name.trim_end_matches(|ch: char| ch.is_ascii_digit())
            } else {
                param.param_type.declared_name()
            };
            format!("{ty} ${}", param.name)
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut out = format!("[{},{}]", script.trigger, script.name);
    if !script.params.is_empty() || !script.return_types.is_empty() {
        write!(out, "({params})")?;
    }
    if !script.return_types.is_empty() {
        write!(
            out,
            "({})",
            script
                .return_types
                .iter()
                .map(Type::declared_name)
                .collect::<Vec<_>>()
                .join(", ")
        )?;
    }
    out.push('\n');
    emit(&mut out, &script.body, 0)?;
    Ok(out)
}

/// An array with no size of its own is the int parameter of its number.
fn declare_array_params(script: &mut ScriptDeclaration) {
    let mut declared = Vec::new();
    script.body.retain(|statement| {
        if let Statement::ArrayDeclaration {
            name,
            element_type,
            size: Expr::NullLiteral,
            ..
        } = statement
        {
            declared.push((name.clone(), *element_type));
            false
        } else {
            true
        }
    });
    for (name, element) in declared {
        let id = name.strip_prefix("array").unwrap_or_default();
        if let Some(param) = script
            .params
            .iter_mut()
            .find(|param| param.name == format!("int{id}"))
        {
            param.param_type = Type::IntArray;
            param.name = name;
            param.value_type = Some(element);
        }
    }
}

fn rename_locals(script: &mut ScriptDeclaration) {
    fn array_name(name: &str, element: Type) -> String {
        format!("{}{name}", element.name())
    }
    fn renamed(name: &str, ty: Type) -> String {
        for base in ["int", "string", "long"] {
            if let Some(slot) = name
                .strip_prefix(base)
                .filter(|slot| !slot.is_empty() && slot.bytes().all(|ch| ch.is_ascii_digit()))
            {
                return format!("{}{slot}", ty.name());
            }
        }
        name.to_owned()
    }
    fn collect(body: &[Statement], names: &mut HashMap<String, String>) {
        for statement in body {
            match statement {
                Statement::VarDeclaration { name, var_type, .. } => {
                    names.insert(name.clone(), renamed(name, *var_type));
                }
                Statement::ArrayDeclaration {
                    name, element_type, ..
                } => {
                    names.insert(name.clone(), array_name(name, *element_type));
                }
                Statement::If {
                    body,
                    else_if,
                    else_body,
                    ..
                } => {
                    collect(body, names);
                    for (_, body, _) in else_if {
                        collect(body, names);
                    }
                    if let Some(body) = else_body {
                        collect(body, names);
                    }
                }
                Statement::While { body, .. } => collect(body, names),
                Statement::Switch { cases, default, .. } => {
                    for case in cases {
                        collect(&case.body, names);
                    }
                    if let Some(body) = default {
                        collect(body, names);
                    }
                }
                _ => {}
            }
        }
    }
    fn expr(value: &mut Expr, names: &HashMap<String, String>) {
        match value {
            Expr::LocalVar(name, _) => {
                if let Some(new) = names.get(name) {
                    *name = new.clone();
                }
            }
            Expr::ArrayAccess { name, index } => {
                if let Some(new) = names.get(name) {
                    *name = new.clone();
                }
                expr(index, names);
            }
            Expr::Calc(value) => expr(value, names),
            Expr::BinaryOp { lhs, rhs, .. } | Expr::LogicalOp { lhs, rhs, .. } => {
                expr(lhs, names);
                expr(rhs, names);
            }
            Expr::CommandCall { args, .. }
            | Expr::ProcCall { args, .. }
            | Expr::MultiAssign(args, _)
            | Expr::Tuple(args) => {
                for value in args {
                    expr(value, names);
                }
            }
            Expr::JoinedString { parts } => {
                for part in parts {
                    if let StringPart::Expr(value) = part {
                        expr(value, names);
                    }
                }
            }
            _ => {}
        }
    }
    fn body(statements: &mut [Statement], names: &HashMap<String, String>) {
        for statement in statements {
            match statement {
                Statement::VarDeclaration { name, value, .. } => {
                    if let Some(new) = names.get(name) {
                        *name = new.clone();
                    }
                    if let Some(value) = value {
                        expr(value, names);
                    }
                }
                Statement::Assignment { target, value, .. } => {
                    expr(target, names);
                    expr(value, names);
                }
                Statement::ArrayDeclaration { name, size, .. } => {
                    if let Some(new) = names.get(name) {
                        *name = new.clone();
                    }
                    expr(size, names);
                }
                Statement::Expression { expr: value, .. } => expr(value, names),
                Statement::Return { values, .. } => {
                    for value in values {
                        expr(value, names);
                    }
                }
                Statement::If {
                    condition,
                    body: nested,
                    else_if,
                    else_body,
                    ..
                } => {
                    expr(condition, names);
                    body(nested, names);
                    for (condition, nested, _) in else_if {
                        expr(condition, names);
                        body(nested, names);
                    }
                    if let Some(nested) = else_body {
                        body(nested, names);
                    }
                }
                Statement::While {
                    condition,
                    body: nested,
                    ..
                } => {
                    expr(condition, names);
                    body(nested, names);
                }
                Statement::Switch {
                    expr: value,
                    cases,
                    default,
                    ..
                } => {
                    expr(value, names);
                    for case in cases {
                        body(&mut case.body, names);
                    }
                    if let Some(nested) = default {
                        body(nested, names);
                    }
                }
                _ => {}
            }
        }
    }
    let mut names = HashMap::new();
    for param in &script.params {
        let name = match param.value_type {
            Some(element) => array_name(&param.name, element),
            None => renamed(&param.name, param.param_type),
        };
        names.insert(param.name.clone(), name);
    }
    collect(&script.body, &mut names);
    for param in &mut script.params {
        param.name = names[&param.name].clone();
    }
    body(&mut script.body, &names);
}

fn uses(expr: &Expr, name: &str) -> bool {
    match expr {
        Expr::LocalVar(var, _) => var == name,
        Expr::ArrayAccess { name: var, index } => var == name || uses(index, name),
        Expr::Calc(expr) => uses(expr, name),
        Expr::BinaryOp { lhs, rhs, .. } | Expr::LogicalOp { lhs, rhs, .. } => {
            uses(lhs, name) || uses(rhs, name)
        }
        Expr::CommandCall { args, .. }
        | Expr::ProcCall { args, .. }
        | Expr::JumpCall { args, .. }
        | Expr::MultiAssign(args, _)
        | Expr::Tuple(args) => args.iter().any(|expr| uses(expr, name)),
        Expr::JoinedString { parts } => parts
            .iter()
            .any(|part| matches!(part, StringPart::Expr(expr) if uses(expr, name))),
        _ => false,
    }
}

fn array_expr(expr: &mut Expr, arrays: &mut BTreeMap<i32, Type>) {
    match expr {
        Expr::CommandCall { args, .. }
        | Expr::ProcCall { args, .. }
        | Expr::MultiAssign(args, _)
        | Expr::Tuple(args) => {
            for arg in args {
                array_expr(arg, arrays);
            }
        }
        Expr::BinaryOp { lhs, rhs, .. } | Expr::LogicalOp { lhs, rhs, .. } => {
            array_expr(lhs, arrays);
            array_expr(rhs, arrays);
        }
        Expr::Calc(value) | Expr::ArrayAccess { index: value, .. } => array_expr(value, arrays),
        Expr::JoinedString { parts } => {
            for part in parts {
                if let StringPart::Expr(value) = part {
                    array_expr(value, arrays);
                }
            }
        }
        _ => {}
    }
    if let Expr::CommandCall { name, args, .. } = expr
        && name == "__get_array"
        && let [Expr::IntLiteral(id), index] = args.as_slice()
    {
        arrays.entry(*id).or_insert(Type::Int);
        *expr = Expr::ArrayAccess {
            name: format!("array{id}"),
            index: Box::new(index.clone()),
        };
    }
}

fn restore_arrays(body: &mut [Statement], arrays: &mut BTreeMap<i32, Type>) {
    for statement in body {
        if let Some(args) = call(statement, "__define_array")
            && let [Expr::IntLiteral(id), Expr::IntLiteral(kind), size] = args.as_slice()
        {
            let ty = char::from_u32(*kind as u32)
                .and_then(Type::from_char)
                .unwrap_or(Type::Int);
            arrays.insert(*id, ty);
            *statement = Statement::ArrayDeclaration {
                element_type: ty,
                name: format!("array{id}"),
                size: size.clone(),
                line: 0,
            };
        } else if let Some(args) = call(statement, "__set_array")
            && let [Expr::IntLiteral(id), index, value] = args.as_slice()
        {
            arrays.entry(*id).or_insert(Type::Int);
            *statement = Statement::Assignment {
                target: Expr::ArrayAccess {
                    name: format!("array{id}"),
                    index: Box::new(index.clone()),
                },
                value: value.clone(),
                line: 0,
            };
        }
        match statement {
            Statement::VarDeclaration {
                value: Some(value), ..
            }
            | Statement::ArrayDeclaration { size: value, .. }
            | Statement::Expression { expr: value, .. } => array_expr(value, arrays),
            Statement::Assignment { target, value, .. } => {
                array_expr(target, arrays);
                array_expr(value, arrays);
            }
            Statement::Return { values, .. } => {
                for value in values {
                    array_expr(value, arrays);
                }
            }
            Statement::If {
                condition,
                body,
                else_body,
                ..
            } => {
                array_expr(condition, arrays);
                restore_arrays(body, arrays);
                if let Some(body) = else_body {
                    restore_arrays(body, arrays);
                }
            }
            Statement::While {
                condition, body, ..
            } => {
                array_expr(condition, arrays);
                restore_arrays(body, arrays);
            }
            Statement::Switch {
                expr,
                cases,
                default,
                ..
            } => {
                array_expr(expr, arrays);
                for case in cases {
                    restore_arrays(&mut case.body, arrays);
                }
                if let Some(body) = default {
                    restore_arrays(body, arrays);
                }
            }
            _ => {}
        }
    }
}

fn statement_uses(statement: &Statement, name: &str) -> bool {
    let block = |body: &[Statement]| body.iter().any(|statement| statement_uses(statement, name));
    match statement {
        Statement::VarDeclaration { value, .. } => {
            value.as_ref().is_some_and(|expr| uses(expr, name))
        }
        Statement::ArrayDeclaration { size, .. } => uses(size, name),
        Statement::Assignment { target, value, .. } => uses(target, name) || uses(value, name),
        Statement::Expression { expr, .. } => uses(expr, name),
        Statement::Return { values, .. } => values.iter().any(|expr| uses(expr, name)),
        Statement::If {
            condition,
            body,
            else_if,
            else_body,
            ..
        } => {
            uses(condition, name)
                || block(body)
                || else_if
                    .iter()
                    .any(|(condition, body, _)| uses(condition, name) || block(body))
                || else_body.as_deref().is_some_and(block)
        }
        Statement::While {
            condition, body, ..
        } => uses(condition, name) || block(body),
        Statement::Switch {
            expr,
            cases,
            default,
            ..
        } => {
            uses(expr, name)
                || cases.iter().any(|case| block(&case.body))
                || default.as_deref().is_some_and(block)
        }
        _ => false,
    }
}

fn merge_declarations(body: &mut Vec<Statement>) {
    let mut declarations: [VecDeque<Statement>; 3] = Default::default();
    let mut statements = Vec::new();
    for statement in take(body) {
        if let Statement::VarDeclaration {
            var_type,
            value: None,
            ..
        } = &statement
        {
            let kind = match var_type.base_type() {
                BaseVarType::Integer => 0,
                BaseVarType::String => 1,
                BaseVarType::Long => 2,
            };
            declarations[kind].push_back(statement);
        } else {
            statements.push(statement);
        }
    }
    for mut statement in statements {
        for declarations in &mut declarations {
            let last = declarations.iter().rposition(|declaration| {
                let Statement::VarDeclaration { name, .. } = declaration else {
                    unreachable!()
                };
                statement_uses(&statement, name)
            });
            if let Some(last) = last {
                for mut declaration in declarations.drain(..=last) {
                    let Statement::VarDeclaration {
                        name,
                        value: initial,
                        ..
                    } = &mut declaration
                    else {
                        unreachable!()
                    };
                    if let Statement::Assignment {
                        target: Expr::LocalVar(target, _),
                        value,
                        ..
                    } = &statement
                        && target == name
                    {
                        *initial = Some(value.clone());
                        statement = Statement::Empty;
                    }
                    body.push(declaration);
                }
            }
        }
        if !matches!(statement, Statement::Empty) {
            body.push(statement);
        }
    }
    body.extend(declarations.into_iter().flatten());
}

fn fold_assignments(body: &mut Vec<Statement>, returns: &HashMap<String, usize>) {
    let mut index = 0;
    while index < body.len() {
        if let Some(args) = call(&body[index], "__push")
            && let [value] = args.as_slice()
        {
            let key = match value {
                Expr::ProcCall { name, .. } => format!("~{name}"),
                Expr::CommandCall { name, .. } => name.trim_start_matches('.').to_owned(),
                _ => String::new(),
            };
            if let Some(&count) = returns.get(&key)
                && count > 0
                && index + count < body.len()
            {
                let targets: Option<Vec<_>> = body[index + 1..index + 1 + count]
                    .iter()
                    .map(|statement| {
                        if let Statement::Assignment {
                            target,
                            value: Expr::CommandCall { name, args, .. },
                            ..
                        } = statement
                            && args.is_empty()
                            && matches!(name.as_str(), "__pop_int" | "__pop_string" | "__pop_long")
                        {
                            return Some(target.clone());
                        }
                        None
                    })
                    .collect();
                if let Some(mut targets) = targets {
                    targets.reverse();
                    body[index] = Statement::Assignment {
                        target: Expr::MultiAssign(targets, vec![]),
                        value: value.clone(),
                        line: 0,
                    };
                    body.drain(index + 1..index + 1 + count);
                } else if body[index + 1..index + 1 + count].iter().all(|statement| {
                    matches!(
                        call(statement, "__opcode").as_deref(),
                        Some([Expr::Identifier(name), Expr::IntLiteral(0)]) if matches!(name.as_str(), "PopIntDiscard" | "PopStringDiscard" | "PopLongDiscard")
                    )
                }) {
                    body[index] = Statement::Expression {
                        expr: value.clone(),
                        line: 0,
                    };
                    body.drain(index + 1..index + 1 + count);
                }
            }
        }
        match &mut body[index] {
            Statement::If {
                body, else_body, ..
            } => {
                fold_assignments(body, returns);
                if let Some(body) = else_body {
                    fold_assignments(body, returns);
                }
            }
            Statement::While { body, .. } => fold_assignments(body, returns),
            Statement::Switch { cases, default, .. } => {
                for case in cases {
                    fold_assignments(&mut case.body, returns);
                }
                if let Some(body) = default {
                    fold_assignments(body, returns);
                }
            }
            _ => {}
        }
        index += 1;
    }
}

fn pop(expr: &Expr) -> bool {
    matches!(expr, Expr::CommandCall { name, args, .. } if args.is_empty() && matches!(name.as_str(),"__pop_int" | "__pop_string" | "__pop_long"))
}

fn has_pop(expr: &Expr) -> bool {
    if pop(expr) {
        return true;
    }
    match expr {
        Expr::CommandCall { args, .. } | Expr::ProcCall { args, .. } | Expr::Tuple(args) => {
            args.iter().any(has_pop)
        }
        Expr::BinaryOp { lhs, rhs, .. } | Expr::LogicalOp { lhs, rhs, .. } => {
            has_pop(lhs) || has_pop(rhs)
        }
        Expr::Calc(value) | Expr::ArrayAccess { index: value, .. } => has_pop(value),
        Expr::JoinedString { parts } => parts
            .iter()
            .any(|part| matches!(part,StringPart::Expr(value) if has_pop(value))),
        _ => false,
    }
}

fn value_count(expr: &Expr, returns: &HashMap<String, usize>) -> usize {
    match expr {
        Expr::ProcCall { name, .. } => returns.get(&format!("~{name}")).copied().unwrap_or(1),
        Expr::CommandCall { name, .. } => returns
            .get(name.trim_start_matches('.'))
            .copied()
            .unwrap_or(1),
        Expr::Tuple(values) => values.iter().map(|value| value_count(value, returns)).sum(),
        _ => 1,
    }
}

fn inline_values(args: &mut Vec<Expr>, values: &[Expr], count: usize) -> bool {
    if count <= args.len() && args[..count].iter().all(pop) {
        args.splice(..count, values.iter().cloned());
        true
    } else {
        args.first_mut()
            .is_some_and(|arg| inline_prefix(arg, values, count))
    }
}

fn inline_prefix(expr: &mut Expr, values: &[Expr], count: usize) -> bool {
    if count == 1 && values.len() == 1 && pop(expr) {
        *expr = values[0].clone();
        return true;
    }
    match expr {
        Expr::CommandCall { args, .. } | Expr::ProcCall { args, .. } | Expr::Tuple(args) => {
            inline_values(args, values, count)
        }
        Expr::BinaryOp { lhs, .. } | Expr::LogicalOp { lhs, .. } => {
            inline_prefix(lhs, values, count)
        }
        Expr::Calc(value) | Expr::ArrayAccess { index: value, .. } => {
            inline_prefix(value, values, count)
        }
        _ => false,
    }
}

/// What a branch compares can take several values at once, like a call's. In
/// the branch by now, conditions read as the single expression they are.
fn fold_branch_values(body: &mut Vec<Statement>, returns: &HashMap<String, usize>) {
    let mut i = 0;
    while i + 1 < body.len() {
        if let Some(values) = call(&body[i], "__push")
            && let Statement::Expression {
                expr: Expr::CommandCall { name, args, .. },
                ..
            } = &mut body[i + 1]
            && name == "__branch"
            && args.len() > 2
        {
            let count = values.iter().map(|value| value_count(value, returns)).sum();
            let mut compared = args.split_off(2);
            let inlined = inline_values(&mut compared, &values, count);
            args.extend(compared);
            if inlined {
                body.remove(i);
                continue;
            }
        }
        i += 1;
    }
}

fn fold_stack(body: &mut Vec<Statement>, returns: &HashMap<String, usize>) {
    for statement in body.iter_mut() {
        match statement {
            Statement::If {
                body, else_body, ..
            } => {
                fold_stack(body, returns);
                if let Some(body) = else_body {
                    fold_stack(body, returns);
                }
            }
            Statement::While { body, .. } => fold_stack(body, returns),
            Statement::Switch { cases, default, .. } => {
                for case in cases {
                    fold_stack(&mut case.body, returns);
                }
                if let Some(body) = default {
                    fold_stack(body, returns);
                }
            }
            _ => {}
        }
    }
    let mut i = 0;
    while i < body.len() {
        let mut values = Vec::new();
        let mut next = i;
        while let Some(args) = body
            .get(next)
            .and_then(|statement| call(statement, "__push"))
        {
            values.extend(args);
            next += 1;
        }
        if values.is_empty() || next == body.len() {
            i += 1;
            continue;
        }
        let count: usize = values.iter().map(|value| value_count(value, returns)).sum();
        let inlined = match &mut body[next] {
            Statement::Expression { expr, .. }
            | Statement::VarDeclaration {
                value: Some(expr), ..
            }
            | Statement::If {
                condition: expr, ..
            } => inline_prefix(expr, &values, count),
            Statement::Assignment { target, value, .. } => {
                if let Expr::ArrayAccess { index, .. } = target {
                    inline_prefix(index, &values, count)
                } else {
                    inline_prefix(value, &values, count)
                }
            }
            Statement::Return { values: args, .. } => inline_values(args, &values, count),
            _ => false,
        };
        if inlined {
            body.drain(i..next);
            continue;
        }
        let mut count = count;
        let mut targets = Vec::new();
        let mut end = next;
        while let Some(Statement::Assignment { target, value, .. }) = body.get(end) {
            if matches!(target, Expr::ArrayAccess { .. }) {
                break;
            }
            if !pop(value) {
                if !targets.is_empty() || has_pop(value) {
                    break;
                }
                values.push(value.clone());
                count += value_count(value, returns);
            }
            if let Expr::MultiAssign(multi, _) = target {
                targets.extend(multi.iter().rev().cloned());
            } else {
                targets.push(target.clone());
            }
            end += 1;
            if targets.len() == count {
                targets.reverse();
                let value = if values.len() == 1 {
                    values.pop().unwrap()
                } else {
                    Expr::Tuple(values)
                };
                body.splice(
                    i..end,
                    [Statement::Assignment {
                        target: Expr::MultiAssign(targets, vec![]),
                        value,
                        line: 0,
                    }],
                );
                break;
            }
            if targets.len() > count {
                break;
            }
        }
        i += 1;
    }
}

fn collect_references(body: &[Statement], references: &mut HashMap<String, usize>) {
    for statement in body {
        for arg in call(statement, "__jump")
            .or_else(|| call(statement, "__branch"))
            .or_else(|| call(statement, "__switch"))
            .unwrap_or_default()
        {
            if let Expr::Identifier(name) = arg {
                *references.entry(name).or_default() += 1;
            }
        }
        match statement {
            Statement::If {
                body, else_body, ..
            } => {
                collect_references(body, references);
                if let Some(body) = else_body {
                    collect_references(body, references);
                }
            }
            Statement::While { body, .. } => collect_references(body, references),
            Statement::Switch { cases, default, .. } => {
                for case in cases {
                    collect_references(&case.body, references);
                }
                if let Some(body) = default {
                    collect_references(body, references);
                }
            }
            _ => {}
        }
    }
}

fn remove_labels(body: &mut Vec<Statement>, references: &HashMap<String, usize>) {
    body.retain(|statement| {
        label(statement, "__label").is_none_or(|label| references.contains_key(&label))
    });
    for statement in body {
        match statement {
            Statement::If {
                body, else_body, ..
            } => {
                remove_labels(body, references);
                if let Some(body) = else_body {
                    remove_labels(body, references);
                }
            }
            Statement::While { body, .. } => remove_labels(body, references),
            Statement::Switch { cases, default, .. } => {
                for case in cases {
                    remove_labels(&mut case.body, references);
                }
                if let Some(body) = default {
                    remove_labels(body, references);
                }
            }
            _ => {}
        }
    }
}

fn emit(out: &mut String, statements: &[Statement], depth: usize) -> Result<()> {
    let indent = "    ".repeat(depth);
    for statement in statements {
        out.push_str(&indent);
        match statement {
            Statement::ArrayDeclaration {
                element_type,
                name,
                size,
                ..
            } => {
                let size = if matches!(size, Expr::NullLiteral) {
                    String::new()
                } else {
                    expression(size)?
                };
                writeln!(out, "def_{} ${name}({size});", element_type.name())?;
            }
            Statement::VarDeclaration {
                var_type,
                name,
                value,
                ..
            } => {
                write!(out, "def_{} ${name}", var_type.declared_name())?;
                if let Some(value) = value {
                    write!(out, " = {}", expression(value)?)?;
                }
                out.push_str(";\n");
            }
            Statement::Assignment { target, value, .. } => {
                writeln!(out, "{} = {};", expression(target)?, expression(value)?)?
            }
            Statement::Expression { expr, .. } => writeln!(out, "{};", expression(expr)?)?,
            Statement::Return { values, .. } => {
                if values.is_empty() {
                    out.push_str("return;\n");
                } else {
                    writeln!(out, "return({});", expressions(values)?)?;
                }
            }
            Statement::If {
                condition,
                body,
                else_body,
                ..
            } => {
                writeln!(out, "if ({}) {{", expression(condition)?)?;
                emit(out, body, depth + 1)?;
                write!(out, "{indent}}}")?;
                if let Some(body) = else_body {
                    if matches!(body.as_slice(), [Statement::If { .. }]) {
                        out.push_str(" else ");
                        let mut nested = String::new();
                        emit(&mut nested, body, depth)?;
                        out.push_str(nested.trim_start().trim_end());
                    } else {
                        out.push_str(" else {\n");
                        emit(out, body, depth + 1)?;
                        write!(out, "{indent}}}")?;
                    }
                }
                out.push('\n');
            }
            Statement::While {
                condition, body, ..
            } => {
                writeln!(out, "while ({}) {{", expression(condition)?)?;
                emit(out, body, depth + 1)?;
                writeln!(out, "{indent}}}")?;
            }
            Statement::Switch {
                switch_type,
                expr,
                cases,
                default,
                default_index,
                ..
            } => {
                writeln!(out, "{switch_type} ({}) {{", expression(expr)?)?;
                for index in 0..=cases.len() {
                    if index == *default_index
                        && let Some(body) = default
                    {
                        writeln!(out, "{indent}    case default :")?;
                        emit(out, body, depth + 2)?;
                    }
                    if let Some(case) = cases.get(index) {
                        writeln!(out, "{indent}    case {} :", expressions(&case.values)?)?;
                        emit(out, &case.body, depth + 2)?;
                    }
                }
                writeln!(out, "{indent}}}")?;
            }
            Statement::Empty => out.push_str(";\n"),
            _ => bail!("Unexpected decompiler statement"),
        }
    }
    Ok(())
}

pub(super) fn expressions(values: &[Expr]) -> Result<String> {
    Ok(values
        .iter()
        .map(expression)
        .collect::<Result<Vec<_>>>()?
        .join(", "))
}

fn escape_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn parse_string(value: &str) -> Option<Expr> {
    let mut file = clientscript::parse(
        &format!("[proc,string_literal]() return({value});"),
        Path::new("string.cs2"),
    )
    .ok()?;
    let Statement::Return { mut values, .. } = file.scripts.pop()?.body.pop()? else {
        return None;
    };
    values.pop()
}

fn literal_string(value: &str) -> String {
    let text = format!("\"{}\"", escape_string(value));
    if !value.contains('<')
        || matches!(parse_string(&text), Some(Expr::StringLiteral(parsed)) if parsed == value)
    {
        text
    } else {
        format!("\"{}\"", escape_string(value).replace('<', "\\<"))
    }
}

/// A string a component's hook holds whole: its tags stay as they are written.
pub(super) fn hook_string(value: &str) -> String {
    if value == "null" || value == "event_opbase" {
        return value.to_owned();
    }
    let text = format!("\"{}\"", escape_string(value));
    let literal = match parse_string(&text) {
        Some(Expr::StringLiteral(parsed)) => parsed == value,
        Some(Expr::JoinedString { parts }) => {
            let mut parsed = String::new();
            parts.iter().all(|part| match part {
                StringPart::Literal(part) => {
                    parsed.push_str(part);
                    true
                }
                StringPart::Expr(_) => false,
            }) && parsed == value
        }
        _ => false,
    };
    if literal {
        text
    } else {
        format!("\"{}\"", escape_string(value).replace('<', "\\<"))
    }
}

fn joined_string(args: &[Expr]) -> Result<String> {
    let mut text = String::from("\"");
    let mut previous: Option<&str> = None;
    for arg in args {
        if let Expr::StringLiteral(value) = arg {
            // A tag ends a piece of text by itself: a number only needs marking without one.
            if previous.is_some_and(|text| !text.ends_with('>'))
                && !value.is_empty()
                && value.bytes().all(|byte| byte.is_ascii_digit())
            {
                write!(text, "<{value}>")?;
            } else {
                text.push_str(&escape_string(value));
            }
            previous = Some(value);
        } else {
            write!(text, "<{}>", expression(arg)?)?;
            previous = None;
        }
    }
    text.push('"');
    if let Some(Expr::JoinedString { parts }) = parse_string(&text)
        && parts.len() == args.len()
        && parts.iter().zip(args).all(|(part, arg)| match (part, arg) {
            (StringPart::Literal(a), Expr::StringLiteral(b)) => a == b,
            (StringPart::Expr(_), arg) => !matches!(arg, Expr::StringLiteral(_)),
            _ => false,
        })
    {
        return Ok(text);
    }
    text = String::from("\"");
    for arg in args {
        write!(text, "<{}>", escape_string(&expression(arg)?))?;
    }
    text.push('"');
    Ok(text)
}

fn arithmetic(expr: &Expr, parent: u8) -> Result<String> {
    if let Expr::Calc(expr) = expr {
        return arithmetic(expr, parent);
    }
    let node = match expr {
        Expr::LogicalOp { op, lhs, rhs, .. } => Some(if *op == LogicOp::And {
            ("&", 2, lhs.as_ref(), rhs.as_ref())
        } else {
            ("|", 1, lhs.as_ref(), rhs.as_ref())
        }),
        Expr::BinaryOp { op, lhs, rhs } => match op {
            BinOp::Add => Some(("+", 3, lhs.as_ref(), rhs.as_ref())),
            BinOp::Sub => Some(("-", 3, lhs.as_ref(), rhs.as_ref())),
            BinOp::Mul => Some(("*", 4, lhs.as_ref(), rhs.as_ref())),
            BinOp::Div => Some(("/", 4, lhs.as_ref(), rhs.as_ref())),
            BinOp::Mod => Some(("%", 4, lhs.as_ref(), rhs.as_ref())),
            _ => None,
        },
        Expr::CommandCall { name, args, .. } => {
            let op = match name.as_str() {
                "or" => Some(("|", 1)),
                "and" => Some(("&", 2)),
                "add" => Some(("+", 3)),
                "sub" => Some(("-", 3)),
                "multiply" => Some(("*", 4)),
                "divide" => Some(("/", 4)),
                "modulo" => Some(("%", 4)),
                _ => None,
            };
            if let (Some((op, prec)), [lhs, rhs]) = (op, args.as_slice()) {
                Some((op, prec, lhs, rhs))
            } else {
                None
            }
        }
        _ => None,
    };
    let Some((op, prec, lhs, rhs)) = node else {
        return expression(expr);
    };
    let text = format!(
        "{} {op} {}",
        arithmetic(lhs, prec)?,
        arithmetic(rhs, prec + 1)?
    );
    Ok(if prec < parent {
        format!("({text})")
    } else {
        text
    })
}

fn condition_text(expr: &Expr, parent: u8) -> Result<String> {
    let Expr::LogicalOp { op, lhs, rhs, .. } = expr else {
        return expression(expr);
    };
    let (op, prec) = if *op == LogicOp::And {
        ("&", 2)
    } else {
        ("|", 1)
    };
    let text = format!(
        "{} {op} {}",
        condition_text(lhs, prec)?,
        condition_text(rhs, prec)?
    );
    Ok(if prec < parent {
        format!("({text})")
    } else {
        text
    })
}

fn expression(expr: &Expr) -> Result<String> {
    Ok(match expr {
        Expr::IntLiteral(value) => value.to_string(),
        Expr::CoordLiteral(value) => format!(
            "{}_{}_{}_{}_{}",
            (*value as u32) >> 28,
            (value >> 20) & 0xff,
            (value >> 6) & 0xff,
            (value >> 14) & 0x3f,
            value & 0x3f
        ),
        Expr::LongLiteral(value) => format!("{value}L"),
        Expr::StringLiteral(value) if value == "null" || value == "event_opbase" => value.clone(),
        Expr::StringLiteral(value) => literal_string(value),
        Expr::BoolLiteral(value) => value.to_string(),
        Expr::NullLiteral => "null".into(),
        Expr::LocalVar(name, _) => format!("${name}"),
        Expr::GameVar(name, _) => format!("%{name}"),
        Expr::ConstantVar(name, _) => format!("^{name}"),
        Expr::Identifier(name) => name.clone(),
        Expr::ArrayAccess { name, index } => format!("${name}({})", expression(index)?),
        Expr::Calc(expr) => format!("calc({})", arithmetic(expr, 0)?),
        Expr::BinaryOp { op, lhs, rhs } => format!(
            "{} {} {}",
            expression(lhs)?,
            match op {
                BinOp::Eq => "=",
                BinOp::NotEq => "!",
                BinOp::Lt => "<",
                BinOp::Gt => ">",
                BinOp::LtEq => "<=",
                BinOp::GtEq => ">=",
                BinOp::Add => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
                BinOp::Mod => "%",
                BinOp::BitAnd => "&",
                BinOp::BitOr => "|",
            },
            expression(rhs)?
        ),
        Expr::LogicalOp { .. } => condition_text(expr, 0)?,
        Expr::CommandCall { name, args, .. } => {
            if name == "__join" {
                return joined_string(args);
            }
            if name == "__hook" {
                let mut text = expression(&args[0])?;
                if !matches!(&args[1], Expr::Tuple(values) if values.is_empty()) {
                    write!(text, "({})", expression(&args[1])?)?;
                }
                if let Some(transmits) = args.get(2) {
                    write!(text, "{{{}}}", expression(transmits)?)?;
                }
                return Ok(literal_string(&text));
            }
            let op = match name.as_str() {
                "add" => Some("+"),
                "sub" => Some("-"),
                "multiply" => Some("*"),
                "divide" => Some("/"),
                "modulo" => Some("%"),
                "and" => Some("&"),
                "or" => Some("|"),
                _ => None,
            };
            if op.is_some() && args.len() == 2 {
                format!("calc({})", arithmetic(expr, 0)?)
            } else {
                format!("{name}({})", expressions(args)?)
            }
        }
        Expr::ProcCall { name, args, .. } => {
            if args.is_empty() {
                format!("~{name}")
            } else {
                format!("~{name}({})", expressions(args)?)
            }
        }
        Expr::MultiAssign(targets, _) | Expr::Tuple(targets) => expressions(targets)?,
        _ => bail!("Unexpected decompiler expression"),
    })
}
