use super::Names;
use super::inference;
use super::structure;
use crate::bytecode::Opcode;
use crate::clientscript::decode::Script;
use crate::clientscript::decode::is_branch;
use crate::clientscript::hooks::{self, HookValue};
use crate::clientscript::{self, Command, CommandKind};
use crate::parser::Expr;
use crate::symbol::{SymbolKind, SymbolRegistry};
use crate::types::{BaseVarType, Type};
use anyhow::{Context, Result, bail, ensure};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt::Write;
use std::path::Path;

#[derive(Clone, Default)]
pub struct Signature {
    pub params: [usize; 3],
    pub returns: [usize; 3],
    pub param_types: Vec<Type>,
    pub return_types: Vec<Type>,
    pub locals: HashMap<String, Type>,
    /// The stack of each parameter in order, when a caller shows it.
    pub order: Option<Vec<BaseVarType>>,
    /// The stack of each return value in order, as the script's end pushes them.
    pub return_order: Vec<BaseVarType>,
}

fn types(counts: [usize; 3], inferred: &[Type]) -> Vec<Type> {
    if !inferred.is_empty() {
        return inferred.to_vec();
    }
    [Type::Int, Type::String, Type::Long]
        .into_iter()
        .zip(counts)
        .flat_map(|(ty, count)| vec![ty; count])
        .collect()
}

fn parameter_types(signature: &Signature) -> Vec<Type> {
    let types = types(signature.params, &signature.param_types);
    let Some(order) = &signature.order else {
        return types;
    };
    let mut pending: [VecDeque<Type>; 3] = Default::default();
    for ty in types {
        pending[stack(ty.base_type())].push_back(ty);
    }
    order
        .iter()
        .map(|&kind| pending[stack(kind)].pop_front().unwrap())
        .collect()
}

fn stack(kind: BaseVarType) -> usize {
    match kind {
        BaseVarType::Integer => 0,
        BaseVarType::String => 1,
        BaseVarType::Long => 2,
    }
}

pub fn infer_signatures(
    scripts: &HashMap<u32, Script>,
    component_hooks: &[Vec<HookValue>],
    names: &dyn Names,
    params: &HashMap<i32, bool>,
    commands: &[Command],
    registry: &mut SymbolRegistry,
) -> Result<HashMap<u32, Signature>> {
    let mut signatures = signatures(scripts);
    let ids = scripts
        .keys()
        .map(|&id| (script_key(names, id), id as i32))
        .collect();
    let mut files = Vec::new();
    let mut sorted: Vec<_> = scripts.keys().copied().collect();
    sorted.sort_unstable();
    for id in sorted {
        let text = decompile_source(
            &scripts[&id],
            id,
            names,
            &signatures,
            params,
            commands,
            registry,
            false,
        )
        .with_context(|| format!("Inferring clientscript {id}"))?;
        let file = clientscript::parse(&text, Path::new("inference.cs2"))
            .with_context(|| format!("Parsing inferred clientscript {id}"))?;
        clientscript::register(&file, &ids, registry)?;
        files.push((id, file));
    }
    let (mut inferred, vars) = inference::infer(&files, component_hooks, registry);
    for (name, ty) in vars {
        if let Some(SymbolKind::GameVar { var_type, .. }) = registry
            .game_vars
            .get_mut(&name)
            .map(|symbol| &mut symbol.kind)
        {
            *var_type = ty;
        }
    }
    for (id, mut file) in files {
        let info = inferred.remove(&id).unwrap();
        let signature = signatures.get_mut(&id).unwrap();
        signature.param_types = info.params;
        signature.return_types = info.returns;
        signature.locals = info.locals;
        signature.order = info.order.filter(|order| {
            let count = |kind: BaseVarType| order.iter().filter(|ty| **ty == kind).count();
            count(BaseVarType::Integer) == signature.params[0]
                && count(BaseVarType::String) == signature.params[1]
                && count(BaseVarType::Long) == signature.params[2]
        });
        for base in ["int", "string", "long"] {
            if let Some(ty) = signature.locals.remove(base) {
                signature.locals.insert(format!("{base}0"), ty);
            }
        }
        for (param, &ty) in file.scripts[0]
            .params
            .iter_mut()
            .zip(&signature.param_types)
        {
            param.param_type = ty;
        }
        file.scripts[0].return_types = signature.return_types.clone();
        clientscript::register(&file, &ids, registry)?;
        registry.register_script(
            file.scripts[0].name.clone(),
            file.scripts[0].trigger.clone(),
            parameter_types(signature),
            signature.return_types.clone(),
        );
    }
    Ok(signatures)
}

/// How an interface writes a component's hook: `script(args){transmits}`.
pub fn component_hook_text(
    values: &[HookValue],
    transmits: Option<(&str, &[i32])>,
    signatures: &HashMap<u32, Signature>,
    names: &dyn Names,
    registry: &SymbolRegistry,
) -> Result<String> {
    let Some(HookValue::Int(script)) = values.first() else {
        bail!("A hook must start with its script");
    };
    let script = u32::try_from(*script).context("Hook with a negative script")?;
    let mut text = script_name(names, script);
    if values.len() > 1 {
        let mut ints = signatures
            .get(&script)
            .map_or(&[][..], |signature| signature.param_types.as_slice())
            .iter()
            .filter(|ty| ty.base_type() == BaseVarType::Integer);
        let args = values[1..]
            .iter()
            .map(|value| {
                Ok(match value {
                    HookValue::Int(value) => structure::expressions(&[inference::hook_literal(
                        *value,
                        ints.next().copied(),
                        names,
                        registry,
                    )])?,
                    HookValue::String(value) => structure::hook_string(value),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        write!(text, "({})", args.join(", "))?;
    }
    if let Some((kind, list)) = transmits {
        let list: Vec<_> = list
            .iter()
            .map(|&id| hooks::transmit_name(registry, kind, id).unwrap_or_else(|| id.to_string()))
            .collect();
        write!(text, "{{{}}}", list.join(", "))?;
    }
    Ok(text)
}

pub fn script_name(names: &dyn Names, id: u32) -> String {
    let name = names.name("clientscript", id);
    let name = name
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .and_then(|s| s.split_once(','))
        .map_or(name.as_str(), |(_, name)| name);
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

pub fn script_key(names: &dyn Names, id: u32) -> String {
    clientscript::script_key(&names.name("clientscript", id))
}

pub fn signatures(scripts: &HashMap<u32, Script>) -> HashMap<u32, Signature> {
    scripts
        .iter()
        .map(|(&id, script)| {
            let mut returns = [0; 3];
            let mut return_order = Vec::new();
            for &opcode in script.operations.iter().rev().skip(1) {
                let kind = match opcode {
                    Opcode::PushConstantInt | Opcode::PushIntLocal => 0,
                    Opcode::PushConstantString | Opcode::PushStringLocal => 1,
                    Opcode::PushConstantLong | Opcode::PushLongLocal => 2,
                    _ => break,
                };
                returns[kind] += 1;
                return_order.insert(
                    0,
                    [BaseVarType::Integer, BaseVarType::String, BaseVarType::Long][kind],
                );
            }
            (
                id,
                Signature {
                    params: [
                        script.int_param_count as usize,
                        script.string_param_count as usize,
                        script.long_param_count as usize,
                    ],
                    returns,
                    return_order,
                    ..Signature::default()
                },
            )
        })
        .collect()
}

#[derive(Clone)]
struct Value {
    text: String,
    kind: usize,
    start: usize,
    end: usize,
    int: Option<i32>,
    string: Option<String>,
}

impl Value {
    fn stack(kind: usize, pc: usize) -> Self {
        Self {
            text: format!("__pop_{}()", ["int", "string", "long"][kind]),
            kind,
            start: pc,
            end: pc,
            int: None,
            string: None,
        }
    }
}

struct Decompiler<'a> {
    names: &'a dyn Names,
    registry: &'a SymbolRegistry,
    signatures: &'a HashMap<u32, Signature>,
    param_types: &'a HashMap<i32, bool>,
    commands: HashMap<u16, &'a Command>,
    returns: [usize; 3],
    return_types: Vec<Type>,
    stack: [Vec<Value>; 3],
    body: String,
    canonical: bool,
}

pub fn decompile(
    script: &Script,
    id: u32,
    names: &dyn Names,
    signatures: &HashMap<u32, Signature>,
    param_types: &HashMap<i32, bool>,
    commands: &[Command],
    registry: &SymbolRegistry,
) -> Result<String> {
    decompile_source(
        script,
        id,
        names,
        signatures,
        param_types,
        commands,
        registry,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn decompile_source(
    script: &Script,
    id: u32,
    names: &dyn Names,
    signatures: &HashMap<u32, Signature>,
    param_types: &HashMap<i32, bool>,
    commands: &[Command],
    registry: &SymbolRegistry,
    canonical: bool,
) -> Result<String> {
    let sig = signatures
        .get(&id)
        .with_context(|| format!("Missing signature for script {id}"))?;
    if !sig.param_types.is_empty() && sig.param_types.len() != sig.params.iter().sum::<usize>() {
        bail!("Parameter types do not match stack counts for script {id}");
    }
    let mut dc = Decompiler {
        names,
        registry,
        signatures,
        param_types,
        commands: commands
            .iter()
            .map(|command| (command.opcode, command))
            .collect(),
        returns: sig.returns,
        return_types: if sig.return_types.is_empty() {
            sig.return_order
                .iter()
                .map(|kind| match kind {
                    BaseVarType::Integer => Type::Int,
                    BaseVarType::String => Type::String,
                    BaseVarType::Long => Type::Long,
                })
                .collect()
        } else {
            sig.return_types.clone()
        },
        stack: Default::default(),
        body: String::new(),
        canonical,
    };
    let mut labels = BTreeSet::new();
    for (pc, &opcode) in script.operations.iter().enumerate() {
        if is_branch(opcode) {
            labels.insert((pc as i64 + 1 + script.int_operands[pc] as i64) as usize);
        }
        if opcode == Opcode::Switch {
            for (_, offset) in &script.switches[script.int_operands[pc] as usize] {
                labels.insert((pc as i64 + 1 + *offset as i64) as usize);
            }
        }
    }
    for (pc, &opcode) in script.operations.iter().enumerate() {
        if labels.contains(&pc) {
            dc.flush();
            dc.stack.iter_mut().for_each(Vec::clear);
            writeln!(dc.body, "__label(L{pc});")?;
        }
        dc.instruction(script, pc, opcode)?;
    }
    dc.flush();
    if labels.contains(&script.operations.len()) {
        writeln!(dc.body, "__label(L{});", script.operations.len())?;
    }
    ensure!(
        script.name.is_empty(),
        "Named bytecode requires an explicit source name"
    );
    let mut out = format!("{}(", script_key(names, id));
    let base_names = ["int", "string", "long"];
    let mut params = Vec::new();
    let mut slots = [0; 3];
    for ty in parameter_types(sig) {
        let kind = stack(ty.base_type());
        let slot = slots[kind];
        slots[kind] += 1;
        // An array parameter is declared by generate: here it's an int.
        let ty = match ty {
            Type::IntArray => "int",
            ty => ty.name(),
        };
        params.push(format!("{ty} ${}{slot}", base_names[kind]));
    }
    let returns: Vec<_> = if sig.return_types.is_empty() {
        sig.return_order
            .iter()
            .map(|kind| match kind {
                BaseVarType::Integer => "int",
                BaseVarType::String => "string",
                BaseVarType::Long => "long",
            })
            .collect()
    } else {
        sig.return_types.iter().map(Type::name).collect()
    };
    out.push_str(&params.join(", "));
    out.push(')');
    if !returns.is_empty() {
        write!(out, "({})", returns.join(", "))?;
    }
    out.push('\n');
    for (kind, count) in [
        script.local_int_count,
        script.local_string_count,
        script.local_long_count,
    ]
    .into_iter()
    .enumerate()
    {
        for i in sig.params[kind]..count as usize {
            let name = format!("{}{i}", base_names[kind]);
            let ty = sig
                .locals
                .get(&name)
                .map_or(base_names[kind], |ty| ty.name());
            writeln!(out, "def_{ty} ${name};")?;
        }
    }
    out.push_str(&dc.body);
    let mut returns = HashMap::new();
    for (pc, (&opcode, &operand)) in script
        .operations
        .iter()
        .zip(&script.int_operands)
        .enumerate()
    {
        if opcode == Opcode::GosubWithParams {
            if let Some(signature) = signatures.get(&(operand as u32)) {
                returns.insert(
                    format!("~{}", script_name(names, operand as u32)),
                    signature.returns.iter().sum(),
                );
            }
        } else if let Some(command) = dc.commands.get(&script.opcodes[pc]) {
            returns.insert(command.name.clone(), command.returns.len());
        }
    }
    // Until its types are known, a script's end still shows what they are.
    structure::generate(
        &out,
        &sig.locals,
        sig.return_types.is_empty() && !sig.return_order.is_empty(),
        &returns,
        names,
        registry,
        canonical,
    )
    .with_context(|| format!("Structuring clientscript {id}"))
}

fn quote(text: &str) -> String {
    format!("r\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

fn counts(types: &[Type]) -> [usize; 3] {
    let mut counts = [0; 3];
    for ty in types {
        counts[match ty.base_type() {
            BaseVarType::Integer => 0,
            BaseVarType::String => 1,
            BaseVarType::Long => 2,
        }] += 1;
    }
    counts
}

impl Decompiler<'_> {
    fn ordered_arguments(
        &mut self,
        mut args: Vec<Value>,
        types: &[Type],
        component_first: bool,
        pc: usize,
    ) -> Vec<Value> {
        if !self.canonical {
            return args;
        }
        let mut pending: [VecDeque<usize>; 3] = Default::default();
        for (index, value) in args.iter().enumerate() {
            pending[value.kind].push_back(index);
        }
        let order = types
            .iter()
            .enumerate()
            .map(|(slot, ty)| {
                let values = &mut pending[stack(ty.base_type())];
                if component_first && slot == 0 {
                    values.pop_back().unwrap()
                } else {
                    values.pop_front().unwrap()
                }
            })
            .collect::<Vec<_>>();
        let emitted = if component_first {
            order[1..]
                .iter()
                .chain(&order[..1])
                .copied()
                .collect::<Vec<_>>()
        } else {
            order.clone()
        };
        let mut positions = vec![0; args.len()];
        for (position, &index) in emitted.iter().enumerate() {
            positions[index] = position;
        }
        let mut next = usize::MAX;
        let prefix = args
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, value)| {
                if value.start == value.end {
                    return None;
                }
                if positions[index] >= next {
                    Some(index + 1)
                } else {
                    next = positions[index];
                    None
                }
            })
            .unwrap_or(0);
        if prefix > 0 {
            self.flush();
            for value in &args[..prefix] {
                if value.start != value.end {
                    writeln!(self.body, "__push({});", value.text).unwrap();
                }
            }
            for value in &mut args[..prefix] {
                *value = Value::stack(value.kind, pc);
            }
        }
        order.into_iter().map(|index| args[index].clone()).collect()
    }

    fn arguments(&self, args: &mut [Value], types: &[Type], hook: bool) {
        let mut by_kind: [Vec<Type>; 3] = Default::default();
        for &ty in types {
            by_kind[match ty.base_type() {
                BaseVarType::Integer => 0,
                BaseVarType::String => 1,
                BaseVarType::Long => 2,
            }]
            .push(ty);
        }
        let mut slots = [0; 3];
        for value in args {
            if let Some(&ty) = by_kind[value.kind].get(slots[value.kind]) {
                self.typed(value, ty, hook);
            }
            slots[value.kind] += 1;
        }
    }
    fn typed(&self, value: &mut Value, ty: Type, hook: bool) {
        let Some(id) = value.int else {
            return;
        };
        if hook && let Some(name) = inference::event_name(id, self.registry) {
            value.text = name.to_owned();
            return;
        }
        if ty == Type::ClientScript && id >= 0 && !self.signatures.contains_key(&(id as u32)) {
            return;
        }
        if let Some(literal) = inference::named_literal(id, ty, self.names, self.registry) {
            value.text = match literal {
                Expr::BoolLiteral(value) => value.to_string(),
                Expr::NullLiteral => "null".to_owned(),
                Expr::Identifier(name) => name,
                Expr::StringLiteral(name) => quote(&name),
                _ => unreachable!(),
            };
        }
    }

    fn flush(&mut self) {
        let mut values: Vec<_> = self
            .stack
            .iter()
            .flatten()
            .filter(|v| v.start != v.end)
            .cloned()
            .collect();
        values.sort_by_key(|v| v.start);
        for value in values {
            writeln!(self.body, "__push({});", value.text).unwrap();
        }
        for stack in &mut self.stack {
            for value in stack {
                *value = Value::stack(value.kind, value.end);
            }
        }
    }

    fn args(&mut self, counts: [usize; 3], pc: usize) -> Vec<Value> {
        let mut values = Vec::new();
        for (kind, count) in counts.into_iter().enumerate() {
            let mut args = Vec::new();
            for _ in 0..count {
                args.push(
                    self.stack[kind]
                        .pop()
                        .unwrap_or_else(|| Value::stack(kind, pc)),
                );
            }
            args.reverse();
            values.extend(args);
        }
        values.sort_by_key(|v| v.start);
        let start = values
            .iter()
            .filter(|v| v.start != v.end)
            .map(|v| v.start)
            .min()
            .unwrap_or(pc);
        if self
            .stack
            .iter()
            .flatten()
            .any(|v| v.start != v.end && v.start >= start)
        {
            let mut pending: Vec<_> = values
                .iter()
                .chain(self.stack.iter().flatten())
                .filter(|v| v.start != v.end)
                .cloned()
                .collect();
            pending.sort_by_key(|v| v.start);
            for value in pending {
                writeln!(self.body, "__push({});", value.text).unwrap();
            }
            for stack in &mut self.stack {
                for value in stack {
                    *value = Value::stack(value.kind, pc);
                }
            }
            for value in &mut values {
                *value = Value::stack(value.kind, pc);
            }
        }
        values
    }

    fn expression(&mut self, text: String, args: &[Value], returns: [usize; 3], pc: usize) {
        let start = args
            .iter()
            .filter(|v| v.start != v.end)
            .map(|v| v.start)
            .min()
            .unwrap_or(pc);
        if returns.iter().sum::<usize>() == 1 {
            let kind = returns.iter().position(|&n| n == 1).unwrap();
            self.stack[kind].push(Value {
                text,
                kind,
                start,
                end: pc + 1,
                int: None,
                string: None,
            });
        } else {
            self.flush();
            if returns == [0; 3] {
                writeln!(self.body, "{text};").unwrap();
            } else {
                writeln!(self.body, "__push({text});").unwrap();
            }
            for (kind, count) in returns.into_iter().enumerate() {
                for _ in 0..count {
                    self.stack[kind].push(Value::stack(kind, pc + 1));
                }
            }
        }
    }

    fn instruction(&mut self, script: &Script, pc: usize, opcode: Opcode) -> Result<()> {
        let operand = script.int_operands[pc];
        let local = |kind: usize| format!("${}{}", ["int", "string", "long"][kind], operand);
        let variable = |domain: &str| format!("%{}", self.names.name(domain, operand as u32));
        let text = match opcode {
            Opcode::PushConstantInt => Some((operand.to_string(), 0)),
            Opcode::PushConstantString => {
                Some((quote(script.string_operands[pc].as_deref().unwrap()), 1))
            }
            Opcode::PushConstantLong => {
                Some((format!("{}L", script.long_operands[pc].unwrap()), 2))
            }
            Opcode::PushVarp => Some((variable("varp"), 0)),
            Opcode::PushVarbit => Some((variable("varpbit"), 0)),
            Opcode::PushVarcInt => Some((variable("varc"), 0)),
            Opcode::PushVarcString => Some((variable("varcstr"), 1)),
            Opcode::PushClanSettingInt => Some((variable("varclansetting"), 0)),
            Opcode::PushClanSettingVarbit => Some((variable("varclansetting"), 0)),
            Opcode::PushClanSettingLong => Some((variable("varclansetting"), 2)),
            Opcode::PushClanSettingString => Some((variable("varclansetting"), 1)),
            Opcode::PushIntLocal => Some((local(0), 0)),
            Opcode::PushStringLocal => Some((local(1), 1)),
            Opcode::PushLongLocal => Some((local(2), 2)),
            _ => None,
        };
        if let Some((text, kind)) = text {
            self.stack[kind].push(Value {
                text,
                kind,
                start: pc,
                end: pc + 1,
                int: (opcode == Opcode::PushConstantInt).then_some(operand),
                string: script.string_operands[pc].clone(),
            });
            return Ok(());
        }
        match opcode {
            Opcode::PopVarp
            | Opcode::PopVarbit
            | Opcode::PopVarcInt
            | Opcode::PopVarcString
            | Opcode::PopIntLocal
            | Opcode::PopStringLocal
            | Opcode::PopLongLocal => {
                let (target, kind) = match opcode {
                    Opcode::PopVarp => (variable("varp"), 0),
                    Opcode::PopVarbit => (variable("varpbit"), 0),
                    Opcode::PopVarcInt => (variable("varc"), 0),
                    Opcode::PopVarcString => (variable("varcstr"), 1),
                    Opcode::PopIntLocal => (local(0), 0),
                    Opcode::PopStringLocal => (local(1), 1),
                    _ => (local(2), 2),
                };
                let mut count = [0; 3];
                count[kind] = 1;
                let args = self.args(count, pc);
                self.expression(format!("{target} = {}", args[0].text), &args, [0; 3], pc);
            }
            Opcode::Branch => {
                self.flush();
                writeln!(self.body, "__jump(L{});", pc as i64 + 1 + operand as i64)?;
                self.stack.iter_mut().for_each(Vec::clear);
            }
            op if is_branch(op) => {
                let count = if matches!(op, Opcode::BranchIfTrue | Opcode::BranchIfFalse) {
                    [1, 0, 0]
                } else if matches!(
                    op,
                    Opcode::LongBranchNot
                        | Opcode::LongBranchEquals
                        | Opcode::LongBranchLessThan
                        | Opcode::LongBranchGreaterThan
                        | Opcode::LongBranchLessThanOrEquals
                        | Opcode::LongBranchGreaterThanOrEquals
                ) {
                    [0, 0, 2]
                } else {
                    [2, 0, 0]
                };
                let args = self.args(count, pc);
                let target = pc as i64 + 1 + operand as i64;
                let values = args
                    .iter()
                    .map(|v| v.text.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let name = match op {
                    Opcode::BranchNot => "BranchNot",
                    Opcode::BranchEquals => "BranchEquals",
                    Opcode::BranchLessThan => "BranchLessThan",
                    Opcode::BranchGreaterThan => "BranchGreaterThan",
                    Opcode::BranchLessThanOrEquals => "BranchLessThanOrEquals",
                    Opcode::BranchGreaterThanOrEquals => "BranchGreaterThanOrEquals",
                    Opcode::LongBranchNot => "LongBranchNot",
                    Opcode::LongBranchEquals => "LongBranchEquals",
                    Opcode::LongBranchLessThan => "LongBranchLessThan",
                    Opcode::LongBranchGreaterThan => "LongBranchGreaterThan",
                    Opcode::LongBranchLessThanOrEquals => "LongBranchLessThanOrEquals",
                    Opcode::LongBranchGreaterThanOrEquals => "LongBranchGreaterThanOrEquals",
                    Opcode::BranchIfTrue => "BranchIfTrue",
                    Opcode::BranchIfFalse => "BranchIfFalse",
                    _ => unreachable!(),
                };
                self.expression(
                    format!("__branch({name}, L{target}, {values})"),
                    &args,
                    [0; 3],
                    pc,
                );
            }
            Opcode::Switch => {
                let args = self.args([1, 0, 0], pc);
                let mut text = format!("__switch({}", args[0].text);
                for (key, offset) in &script.switches[operand as usize] {
                    write!(text, ", {key}, L{}", pc as i64 + 1 + *offset as i64)?;
                }
                text.push(')');
                self.expression(text, &args, [0; 3], pc);
            }
            Opcode::Return => {
                let args = self.args(self.returns, pc);
                let types = self.return_types.clone();
                let args = self.ordered_arguments(args, &types, false, pc);
                self.expression(
                    format!(
                        "return({})",
                        args.iter()
                            .map(|v| v.text.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    &args,
                    [0; 3],
                    pc,
                );
                self.stack.iter_mut().for_each(Vec::clear);
            }
            Opcode::JoinString => {
                let args = self.args([0, operand as usize, 0], pc);
                self.expression(
                    format!(
                        "__join({})",
                        args.iter()
                            .map(|v| v.text.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    &args,
                    [0, 1, 0],
                    pc,
                );
            }
            Opcode::GosubWithParams => {
                let signature = self
                    .signatures
                    .get(&(operand as u32))
                    .context("Missing called clientscript")?;
                let mut args = self.args(signature.params, pc);
                let parameters = parameter_types(signature);
                self.arguments(&mut args, &parameters, false);
                let args = self.ordered_arguments(args, &parameters, false, pc);
                self.expression(
                    format!(
                        "~{}({})",
                        script_name(self.names, operand as u32),
                        args.iter()
                            .map(|v| v.text.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    &args,
                    signature.returns,
                    pc,
                );
            }
            Opcode::PopIntDiscard | Opcode::PopStringDiscard | Opcode::PopLongDiscard => {
                self.flush();
                let kind = match opcode {
                    Opcode::PopStringDiscard => 1,
                    Opcode::PopLongDiscard => 2,
                    _ => 0,
                };
                self.stack[kind].pop();
                writeln!(self.body, "__opcode({opcode:?}, {operand});")?;
            }
            Opcode::DefineArray | Opcode::PushArrayInt | Opcode::PopArrayInt => {
                let args = self.args(
                    [if opcode == Opcode::PopArrayInt { 2 } else { 1 }, 0, 0],
                    pc,
                );
                let text = match opcode {
                    Opcode::DefineArray => format!(
                        "__define_array({}, {}, {})",
                        operand >> 16,
                        operand & 65535,
                        args[0].text
                    ),
                    Opcode::PushArrayInt => format!("__get_array({operand}, {})", args[0].text),
                    _ => format!("__set_array({operand}, {}, {})", args[0].text, args[1].text),
                };
                self.expression(
                    text,
                    &args,
                    [usize::from(opcode == Opcode::PushArrayInt), 0, 0],
                    pc,
                );
            }
            _ => {
                let command = self
                    .commands
                    .get(&script.opcodes[pc])
                    .copied()
                    .with_context(|| format!("Unknown operation {opcode:?}"))?;
                let mut types = command.params.clone();
                let mut returns = counts(&command.returns);
                let mut name = command.name.to_owned();
                let component_first = self.registry.component_first_command(&name);
                if component_first {
                    let component = types.remove(0);
                    types.push(component);
                }
                if matches!(command.kind, CommandKind::Hook | CommandKind::HookComponent) {
                    let explicit = command.kind == CommandKind::HookComponent;
                    let signature = self.stack[1]
                        .last()
                        .and_then(|v| v.string.as_deref())
                        .context("Nonconstant hook signature")?;
                    let triggers = if signature.ends_with('Y') {
                        let index = self.stack[0]
                            .len()
                            .checked_sub(1 + usize::from(explicit))
                            .context("Missing hook count")?;
                        self.stack[0][index].int.context("Nonconstant hook count")? as usize
                    } else {
                        0
                    };
                    types = clientscript::hook_types(signature, triggers, explicit);
                }
                if command.kind == CommandKind::Enum {
                    let n = self.stack[0].len();
                    let string = n >= 3 && self.stack[0][n - 3].int == Some(115);
                    returns = if string { [0, 1, 0] } else { [1, 0, 0] };
                } else if command.kind == CommandKind::Param {
                    let id = self.stack[0]
                        .last()
                        .and_then(|v| v.int)
                        .context("Nonconstant param")?;
                    if self.param_types.get(&id) == Some(&true) {
                        returns = [0, 1, 0];
                    }
                }
                if operand == 1 {
                    name.insert(0, '.');
                }
                let mut args = self.args(counts(&types), pc);
                self.arguments(
                    &mut args,
                    &types,
                    matches!(command.kind, CommandKind::Hook | CommandKind::HookComponent),
                );
                if component_first {
                    let component = types.pop().unwrap();
                    types.insert(0, component);
                }
                let mut args = self.ordered_arguments(args, &types, component_first, pc);
                let type_args = match command.kind {
                    CommandKind::Enum => 2,
                    CommandKind::EnumOutput => 1,
                    _ => 0,
                };
                if type_args > 0 {
                    // These say what the enum maps between, as types.
                    for arg in args.iter_mut().take(type_args) {
                        if let Some(ty) = arg
                            .int
                            .and_then(|value| char::from_u32(value as u32))
                            .and_then(Type::from_char)
                        {
                            arg.text = ty.name().to_owned();
                        }
                    }
                }
                self.expression(
                    format!(
                        "{name}({})",
                        args.iter()
                            .map(|v| v.text.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    &args,
                    returns,
                    pc,
                );
            }
        }
        Ok(())
    }
}
