//! Read-only inspection of a source call or its containing script.
use crate::bytecode::{CompiledScript, Instruction, Opcode, Operand};
use crate::compiler::Compiler;
use crate::ide::highlighting::interpolations;
use crate::lexer::Lexer;
use crate::parser::{Expr, ScriptDeclaration, ScriptFile, Statement, StringPart};
use crate::symbol::{SymbolKind, SymbolRegistry};
use crate::text::LineIndex;
use crate::token::Kind;
use crate::typechecker::TypeChecker;
use crate::types::{BaseVarType, Type};
use std::collections::HashMap;
use std::fmt::Write;
use std::ops::Range;
use std::path::Path;

/// Counts are ordered int, string, long.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StackEffect {
    pub pops: [usize; 3],
    pub pushes: [usize; 3],
}

impl StackEffect {
    fn signature(params: &[Type], returns: &[Type]) -> Self {
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
        Self {
            pops: counts(params),
            pushes: counts(returns),
        }
    }
}

#[derive(Clone, Debug)]
pub struct InspectedInstruction {
    pub pc: usize,
    pub mnemonic: String,
    pub operand: String,
    pub effect: Option<StackEffect>,
}

#[derive(Clone, Debug)]
pub struct CallInspection {
    pub range: Range<usize>,
    pub call: String,
    pub script: String,
    pub instructions: Vec<InspectedInstruction>,
    pub net: Option<[i64; 3]>,
    /// True when a broad selection falls back to the containing script.
    pub whole_script: bool,
}

impl CallInspection {
    pub fn markdown(&self) -> String {
        let mut out = format!("# Bytecode: {}\n\nScript: `{}`\n\n", self.call, self.script);
        if self.whole_script {
            out.push_str("Showing the entire containing script.\n\n");
        }
        out.push_str(
            "Instruction order includes argument evaluation. Stack columns show pops → pushes.\n\n",
        );
        out.push_str("| PC | Instruction | Operand | int | string | long |\n| ---: | --- | --- | ---: | ---: | ---: |\n");
        for row in &self.instructions {
            let counts = row
                .effect
                .map(|effect| {
                    (0..3)
                        .map(|i| format!("{} → {}", effect.pops[i], effect.pushes[i]))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| vec!["unknown".into(); 3]);
            let operand = row.operand.replace('|', "\\|").replace('\n', "\\n");
            let _ = writeln!(
                out,
                "| {} | {} | {} | {} | {} | {} |",
                row.pc, row.mnemonic, operand, counts[0], counts[1], counts[2]
            );
        }
        match self.net {
            Some(net) => { let _ = writeln!(out, "\nNet stack change: int {:+}, string {:+}, long {:+}.", net[0], net[1], net[2]); }
            None => out.push_str("\nNet stack change: unknown (an instruction has an unknown effect or the sequence branches).\n"),
        }
        if self.instructions.is_empty() {
            out.push_str("\nThis selection emits no instructions.\n");
        }
        out
    }
}

/// Finds a call without compiling or reading any files.
pub fn call_range(
    source: &str,
    path: &Path,
    selection: Range<usize>,
    registry: &SymbolRegistry,
) -> Result<Option<Range<usize>>, String> {
    let file = crate::clientscript::parse(source, path).map_err(|error| error.to_string())?;
    Ok(select(source, path, &file, selection, registry)?.map(|(_, call)| call.range))
}

/// The registry supplies signatures and names; this never compiles other scripts.
pub fn inspect_call(
    source: &str,
    path: &Path,
    selection: Range<usize>,
    registry: &SymbolRegistry,
    client: bool,
) -> Result<Option<CallInspection>, String> {
    let file = crate::clientscript::parse(source, path).map_err(|error| error.to_string())?;
    let Some((script, call)) = select(source, path, &file, selection, registry)? else {
        return Ok(None);
    };
    inspect_script(script, path, registry, client, InspectionTarget::Call(call)).map(Some)
}

/// Inspect one call when possible, otherwise the complete containing script.
/// Selections crossing declarations are rejected rather than inspecting an arbitrary script.
pub fn inspect_code(
    source: &str,
    path: &Path,
    selection: Range<usize>,
    registry: &SymbolRegistry,
    client: bool,
) -> Result<Option<CallInspection>, String> {
    validate_selection(source, &selection)?;
    let file = crate::clientscript::parse(source, path).map_err(|error| error.to_string())?;
    let Some((script, range)) = containing_script(source, &file, selection.clone())? else {
        return Ok(None);
    };
    let target = match select(source, path, &file, selection, registry)? {
        Some((_, call)) => InspectionTarget::Call(call),
        None => InspectionTarget::Script(range),
    };
    inspect_script(script, path, registry, client, target).map(Some)
}

enum InspectionTarget {
    Call(CallSite),
    Script(Range<usize>),
}

fn inspect_script(
    script: &ScriptDeclaration,
    path: &Path,
    registry: &SymbolRegistry,
    client: bool,
    target: InspectionTarget,
) -> Result<CallInspection, String> {
    let mut checker = TypeChecker::new(registry);
    checker.check_file(
        &ScriptFile {
            scripts: vec![script.clone()],
        },
        path,
    );
    if checker.diagnostics.has_errors() {
        return Err(checker
            .diagnostics
            .errors()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n"));
    }
    let mut compiler = Compiler::new(registry);
    let (compiled, traces) = compiler.compile_traced(script, client);
    if !compiler.errors.is_empty() {
        return Err(compiler.errors.join("\n"));
    }
    let (range, call, selected_instructions, whole_script) = match target {
        InspectionTarget::Call(call) => {
            let trace = traces
                .iter()
                .find(|trace| trace.expression == call.expression)
                .ok_or("The selected expression has no direct call lowering to inspect.")?;
            (call.range, call.name, trace.instructions.clone(), false)
        }
        InspectionTarget::Script(range) => (
            range,
            "entire script".to_owned(),
            0..compiled.instructions.len(),
            true,
        ),
    };
    let effects: HashMap<_, _> = traces
        .iter()
        .filter_map(|trace| {
            Some((
                trace.operation?,
                trace
                    .signature
                    .as_ref()
                    .map(|(params, returns)| StackEffect::signature(params, returns)),
            ))
        })
        .collect();
    let mut positions = Vec::with_capacity(compiled.instructions.len() + 1);
    let mut pc = 0;
    for instruction in &compiled.instructions {
        positions.push(pc);
        if instruction.opcode != Opcode::LineNumber {
            pc += 1;
        }
    }
    positions.push(pc);
    let mut instructions = Vec::new();
    let mut net = Some([0i64; 3]);
    for index in selected_instructions {
        let instruction = &compiled.instructions[index];
        if instruction.opcode == Opcode::LineNumber {
            continue;
        }
        let effect = effects
            .get(&index)
            .copied()
            .unwrap_or_else(|| instruction_effect(instruction, registry));
        if let (Some(total), Some(effect)) = (&mut net, effect) {
            for i in 0..3 {
                total[i] += effect.pushes[i] as i64 - effect.pops[i] as i64;
            }
        } else {
            net = None;
        }
        if matches!(
            instruction.operand,
            Operand::JumpTarget(_) | Operand::SwitchTable(_)
        ) || matches!(instruction.opcode, Opcode::Raw | Opcode::Return)
        {
            net = None;
        }
        let (mnemonic, operand) = display_instruction(&compiled, instruction, &positions, registry);
        instructions.push(InspectedInstruction {
            pc: positions[index],
            mnemonic,
            operand,
            effect,
        });
    }
    Ok(CallInspection {
        range,
        call,
        script: compiled.name,
        instructions,
        net,
        whole_script,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CallKind {
    Command,
    Proc,
    Jump,
}

struct CallSite {
    expression: usize,
    name: String,
    range: Range<usize>,
}

struct Candidate {
    kind: CallKind,
    name: String,
    range: Range<usize>,
}

fn validate_selection(source: &str, selection: &Range<usize>) -> Result<(), String> {
    if selection.start > selection.end
        || selection.end > source.len()
        || !source.is_char_boundary(selection.start)
        || !source.is_char_boundary(selection.end)
    {
        return Err("Invalid source selection.".into());
    }
    Ok(())
}

fn containing_script<'a>(
    source: &str,
    file: &'a ScriptFile,
    mut selection: Range<usize>,
) -> Result<Option<(&'a ScriptDeclaration, Range<usize>)>, String> {
    if !selection.is_empty() {
        let selected = &source[selection.clone()];
        if !selected.trim().is_empty() {
            let leading = selected.len() - selected.trim_start().len();
            selection.end = selection.start + selected.trim_end().len();
            selection.start += leading;
        }
    }
    let lines = LineIndex::new(source);
    let selected_line = lines.line_at(selection.start) + 1;
    let Some((index, script)) = file
        .scripts
        .iter()
        .enumerate()
        .rev()
        .find(|(_, script)| script.line <= selected_line)
    else {
        return Ok(None);
    };
    if file
        .scripts
        .iter()
        .filter(|other| other.line == script.line)
        .count()
        > 1
    {
        return Err(
            "Place script declarations on separate lines before inspecting their bytecode.".into(),
        );
    }
    let start = lines.line(script.line - 1).map_or(0, |line| line.start);
    let end = file
        .scripts
        .get(index + 1)
        .and_then(|next| lines.line(next.line - 1))
        .map_or(source.len(), |line| line.start);
    if selection.end > end {
        return Err("The selection crosses script declarations. Select code within one script to inspect its bytecode.".into());
    }
    if script.trigger == "command" {
        return Err(
            "Command declarations have no script bytecode. Select a command call or a script body."
                .into(),
        );
    }
    Ok(Some((script, start..end)))
}

fn select<'a>(
    source: &str,
    path: &Path,
    file: &'a ScriptFile,
    mut selection: Range<usize>,
    registry: &SymbolRegistry,
) -> Result<Option<(&'a ScriptDeclaration, CallSite)>, String> {
    validate_selection(source, &selection)?;
    if !selection.is_empty() {
        let selected = &source[selection.clone()];
        let leading = selected.len() - selected.trim_start().len();
        let selected = selected.trim_end();
        let selected = selected.strip_suffix(';').unwrap_or(selected).trim_end();
        if selected.len() <= leading {
            return Ok(None);
        }
        selection.end = selection.start + selected.len();
        selection.start += leading;
    }
    let lines = LineIndex::new(source);
    let selected_line = lines.line_at(selection.start) + 1;
    let Some(script) = file
        .scripts
        .iter()
        .rev()
        .find(|script| script.line <= selected_line)
    else {
        return Ok(None);
    };
    if script.trigger == "command" {
        return Ok(None);
    }
    let start = lines.line(script.line - 1).map_or(0, |line| line.start);
    let end = file
        .scripts
        .iter()
        .find(|next| next.line > script.line)
        .and_then(|next| lines.line(next.line - 1))
        .map_or(source.len(), |line| line.start);
    let candidates = candidates(source, path, start..end, registry)?;
    let mut expressions = Vec::new();
    visit_statements(&script.body, &mut |expr| match expr {
        Expr::CommandCall { name, .. } => expressions.push((expr, CallKind::Command, name)),
        Expr::ProcCall { name, .. } => expressions.push((expr, CallKind::Proc, name)),
        Expr::JumpCall { name, .. } => expressions.push((expr, CallKind::Jump, name)),
        Expr::Identifier(name)
            if registry
                .lookup_command(name.trim_start_matches('.'))
                .is_some() =>
        {
            expressions.push((expr, CallKind::Command, name))
        }
        _ => {}
    });
    let mut cursor = 0;
    let mut found: Option<CallSite> = None;
    for (expr, kind, name) in expressions {
        let Some(relative) = candidates[cursor..]
            .iter()
            .position(|candidate| candidate.kind == kind && candidate.name == *name)
        else {
            continue;
        };
        let candidate = &candidates[cursor + relative];
        cursor += relative + 1;
        let contains = candidate.range.start <= selection.start
            && candidate.range.end >= selection.end
            && (selection.start < candidate.range.end || selection.start == candidate.range.start);
        if contains
            && found
                .as_ref()
                .is_none_or(|old| old.range.len() > candidate.range.len())
        {
            found = Some(CallSite {
                expression: expr as *const Expr as usize,
                name: match kind {
                    CallKind::Proc => format!("~{name}"),
                    CallKind::Jump => format!("@{name}"),
                    CallKind::Command => name.clone(),
                },
                range: candidate.range.clone(),
            });
        }
    }
    Ok(found.map(|call| (script, call)))
}

fn candidates(
    source: &str,
    path: &Path,
    range: Range<usize>,
    registry: &SymbolRegistry,
) -> Result<Vec<Candidate>, String> {
    fn tokens(
        source: &str,
        path: &Path,
        range: Range<usize>,
        out: &mut Vec<(Kind, String, Range<usize>)>,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 32 {
            return Ok(());
        }
        let label = path.to_path_buf();
        let (lexed, spans, errors) = Lexer::new(&source[range.clone()], &label).tokenize_spanned();
        if !errors.is_empty() {
            return Err(errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n"));
        }
        for (token, span) in lexed.into_iter().zip(spans) {
            let span = span.start + range.start..span.end + range.start;
            if token.kind == Kind::StringLiteral {
                for (part, code) in interpolations(&source[span.clone()]) {
                    if code {
                        tokens(
                            source,
                            path,
                            span.start + part.start + 1..span.start + part.end - 1,
                            out,
                            depth + 1,
                        )?;
                    }
                }
            }
            if !matches!(
                token.kind,
                Kind::SingleLineComment | Kind::MultiLineComment | Kind::EndOfFile
            ) {
                out.push((token.kind, token.value, span));
            }
        }
        Ok(())
    }
    let mut lexed = Vec::new();
    tokens(source, path, range, &mut lexed, 0)?;
    lexed.sort_by_key(|token| token.2.start);
    let mut body = lexed
        .iter()
        .position(|token| token.0 == Kind::RBracket)
        .map_or(0, |index| index + 1);
    while lexed.get(body).is_some_and(|token| token.0 == Kind::LParen) {
        body = close_parens(&lexed, body).map_or(lexed.len(), |end| end + 1);
    }
    let mut result = Vec::new();
    for i in body..lexed.len() {
        if !matches!(lexed[i].0, Kind::Identifier | Kind::Command | Kind::Trigger) {
            continue;
        }
        let mut first = i;
        let mut name = lexed[i].1.clone();
        if first > 0 && lexed[first - 1].0 == Kind::Dot {
            first -= 1;
            name.insert(0, '.');
        }
        let kind = match first.checked_sub(1).map(|previous| lexed[previous].0) {
            Some(Kind::ScriptCall) => {
                first -= 1;
                CallKind::Proc
            }
            Some(Kind::JumpCall) => {
                first -= 1;
                CallKind::Jump
            }
            Some(Kind::LocalVar | Kind::GameVar | Kind::ConstantVar | Kind::Colon) => continue,
            _ => CallKind::Command,
        };
        let mut next = i + 1;
        if lexed
            .get(next)
            .is_some_and(|token| token.0 == Kind::BinaryOperator && token.1 == "*")
        {
            name.push('*');
            next += 1;
        }
        let parenthesized = lexed.get(next).is_some_and(|token| token.0 == Kind::LParen);
        if kind == CallKind::Command
            && !parenthesized
            && registry
                .lookup_command(name.trim_start_matches('.'))
                .is_none()
        {
            continue;
        }
        let mut end = lexed[i].2.end;
        while lexed.get(next).is_some_and(|token| token.0 == Kind::LParen) {
            let Some(close) = close_parens(&lexed, next) else {
                break;
            };
            end = lexed[close].2.end;
            next = close + 1;
        }
        result.push(Candidate {
            kind,
            name,
            range: lexed[first].2.start..end,
        });
    }
    Ok(result)
}

fn close_parens(tokens: &[(Kind, String, Range<usize>)], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, (kind, _, _)) in tokens.iter().enumerate().skip(start) {
        match kind {
            Kind::LParen => depth += 1,
            Kind::RParen => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn visit_statements<'a>(statements: &'a [Statement], visit: &mut impl FnMut(&'a Expr)) {
    for statement in statements {
        match statement {
            Statement::VarDeclaration { value, .. } => {
                if let Some(value) = value {
                    visit_expr(value, visit);
                }
            }
            Statement::ArrayDeclaration { size, .. } => visit_expr(size, visit),
            Statement::Assignment { target, value, .. } => {
                visit_expr(target, visit);
                visit_expr(value, visit);
            }
            Statement::Expression { expr, .. } => visit_expr(expr, visit),
            Statement::Return { values, .. } => {
                for value in values {
                    visit_expr(value, visit);
                }
            }
            Statement::If {
                condition,
                body,
                else_if,
                else_body,
                ..
            } => {
                visit_expr(condition, visit);
                visit_statements(body, visit);
                for (condition, body, _) in else_if {
                    visit_expr(condition, visit);
                    visit_statements(body, visit);
                }
                if let Some(body) = else_body {
                    visit_statements(body, visit);
                }
            }
            Statement::While {
                condition, body, ..
            } => {
                visit_expr(condition, visit);
                visit_statements(body, visit);
            }
            Statement::Switch {
                expr,
                cases,
                default,
                default_index,
                ..
            } => {
                visit_expr(expr, visit);
                for i in 0..=cases.len() {
                    if i == *default_index {
                        if let Some(body) = default {
                            visit_statements(body, visit);
                        }
                    }
                    if let Some(case) = cases.get(i) {
                        for value in &case.values {
                            visit_expr(value, visit);
                        }
                        visit_statements(&case.body, visit);
                    }
                }
            }
            Statement::OrphanCase { case, .. } => {
                for value in &case.values {
                    visit_expr(value, visit);
                }
                visit_statements(&case.body, visit);
            }
            Statement::Empty => {}
        }
    }
}

fn visit_expr<'a>(expr: &'a Expr, visit: &mut impl FnMut(&'a Expr)) {
    visit(expr);
    match expr {
        Expr::CommandCall { args, .. }
        | Expr::ProcCall { args, .. }
        | Expr::JumpCall { args, .. }
        | Expr::Tuple(args)
        | Expr::MultiAssign(args, _) => {
            for arg in args {
                visit_expr(arg, visit);
            }
        }
        Expr::ArrayAccess { index, .. } | Expr::Calc(index) => visit_expr(index, visit),
        Expr::BinaryOp { lhs, rhs, .. } | Expr::LogicalOp { lhs, rhs, .. } => {
            visit_expr(lhs, visit);
            visit_expr(rhs, visit);
        }
        Expr::JoinedString { parts } => {
            for part in parts {
                if let StringPart::Expr(expr) = part {
                    visit_expr(expr, visit);
                }
            }
        }
        _ => {}
    }
}

fn instruction_effect(instruction: &Instruction, registry: &SymbolRegistry) -> Option<StackEffect> {
    use Opcode::*;
    let effect = |pops, pushes| Some(StackEffect { pops, pushes });
    if matches!(
        instruction.opcode,
        PushVarp
            | PushVarn
            | PushVars
            | PushVarbit
            | PushVarcInt
            | PushVarcString
            | PushVarClan
            | PushVarObj
            | PushVarnBit
            | PushVarCon
            | PushVarConBit
            | PushClanSettingInt
            | PushClanSettingVarbit
            | PushClanSettingLong
            | PushClanSettingString
            | PopVarp
            | PopVarn
            | PopVars
            | PopVarbit
            | PopVarcInt
            | PopVarcString
            | PopVarClan
            | PopVarObj
            | PopVarnBit
            | PopVarCon
            | PopVarConBit
            | PopClanSettingInt
            | PopClanSettingVarbit
            | PopClanSettingLong
            | PopClanSettingString
    ) {
        let Operand::Int(encoded) = instruction.operand else {
            return None;
        };
        return registry.game_variables().find_map(|(_, symbol)| {
            let SymbolKind::GameVar {
                var_id,
                var_type,
                category,
            } = &symbol.kind
            else {
                return None;
            };
            if *var_id != (encoded & 0xffff) {
                return None;
            }
            if Opcode::push_var(category) == instruction.opcode {
                Some(StackEffect::signature(&[], &[*var_type]))
            } else if Opcode::pop_var(category) == instruction.opcode {
                Some(StackEffect::signature(&[*var_type], &[]))
            } else {
                None
            }
        });
    }
    match instruction.opcode {
        PushConstantInt | PushIntLocal => effect([0; 3], [1, 0, 0]),
        PushConstantString | PushStringLocal => effect([0; 3], [0, 1, 0]),
        PushConstantLong | PushLongLocal => effect([0; 3], [0, 0, 1]),
        PopIntLocal | PopIntDiscard | DefineArray => effect([1, 0, 0], [0; 3]),
        PopStringLocal | PopStringDiscard => effect([0, 1, 0], [0; 3]),
        PopLongLocal | PopLongDiscard => effect([0, 0, 1], [0; 3]),
        PushArrayInt => effect([1, 0, 0], [1, 0, 0]),
        PopArrayInt => effect([2, 0, 0], [0; 3]),
        JoinString => match instruction.operand {
            Operand::StringCount(count) => effect([0, count as usize, 0], [0, 1, 0]),
            _ => None,
        },
        Branch => effect([0; 3], [0; 3]),
        BranchIfTrue | BranchIfFalse | Switch => effect([1, 0, 0], [0; 3]),
        BranchNot
        | BranchEquals
        | BranchLessThan
        | BranchGreaterThan
        | BranchLessThanOrEquals
        | BranchGreaterThanOrEquals => effect([2, 0, 0], [0; 3]),
        LongBranchNot
        | LongBranchEquals
        | LongBranchLessThan
        | LongBranchGreaterThan
        | LongBranchLessThanOrEquals
        | LongBranchGreaterThanOrEquals => effect([0, 0, 2], [0; 3]),
        Add | Sub | Multiply | Divide | Modulo | And | Or | Min | Max | Pow | InvPow | SetBit
        | ClearBit | TestBit | AddPercent | ToggleBit => effect([2, 0, 0], [1, 0, 0]),
        Random | RandomInc | BitCount | SinDeg | CosDeg | Abs => effect([1, 0, 0], [1, 0, 0]),
        Interpolate => effect([5, 0, 0], [1, 0, 0]),
        Scale | SetBitRange | ClearBitRange | GetBitRange => effect([3, 0, 0], [1, 0, 0]),
        SetBitRangeToVal => effect([4, 0, 0], [1, 0, 0]),
        Atan2Deg => effect([2, 0, 0], [1, 0, 0]),
        Command => {
            command_symbol(instruction, registry).and_then(|(name, symbol)| match &symbol.kind {
                SymbolKind::Command {
                    param_types,
                    return_types,
                    ..
                } if !(param_types.is_empty() && return_types.is_empty())
                    && !param_types
                        .iter()
                        .chain(return_types)
                        .any(|ty| matches!(ty, Type::Any | Type::Error))
                    && !registry.dynamic_returns.contains_key(name)
                    && !registry.hook_commands.contains_key(name)
                    && Compiler::command_variadic_info(name).is_none() =>
                {
                    Some(StackEffect::signature(param_types, return_types))
                }
                _ => None,
            })
        }
        _ => None,
    }
}

fn command_symbol<'a>(
    instruction: &Instruction,
    registry: &'a SymbolRegistry,
) -> Option<(&'a str, &'a crate::symbol::Symbol)> {
    let Operand::Int(encoded) = instruction.operand else {
        return None;
    };
    registry
        .commands
        .iter()
        .find_map(|(name, symbol)| match symbol.kind {
            SymbolKind::Command { opcode, .. } if opcode == (encoded & 0xffff) => {
                Some((name.as_str(), symbol))
            }
            _ => None,
        })
}

fn display_instruction(
    script: &CompiledScript,
    instruction: &Instruction,
    positions: &[usize],
    registry: &SymbolRegistry,
) -> (String, String) {
    if instruction.opcode == Opcode::Raw
        && let Operand::Long(encoded) = instruction.operand
    {
        return (
            format!("Raw opcode {}", (encoded >> 32) as u16),
            (encoded as i32).to_string(),
        );
    }
    if instruction.opcode == Opcode::Command {
        if let Some((name, _)) = command_symbol(instruction, registry) {
            let Operand::Int(encoded) = instruction.operand else {
                unreachable!()
            };
            return (
                name.to_owned(),
                format!(
                    "opcode {}, operand {}",
                    encoded & 0xffff,
                    (encoded >> 16) & 255
                ),
            );
        }
    }
    let operand = match &instruction.operand {
        Operand::None => String::new(),
        Operand::Int(value) => value.to_string(),
        Operand::Long(value) => value.to_string(),
        Operand::Str(index) => format!(
            "{:?}",
            script
                .strings
                .get(*index as usize)
                .map(|s| s.as_ref())
                .unwrap_or("<missing string>")
        ),
        Operand::JumpTarget(target) => positions
            .get(*target)
            .map_or_else(|| "unknown target".into(), |pc| format!("→ {pc}")),
        Operand::SwitchTable(index) => format!("{:?}", script.switch_tables.get(*index as usize)),
        Operand::StringCount(count) => count.to_string(),
        Operand::ArrayDef(slot, kind) => format!("array {slot}, type {}", *kind as char),
    };
    (format!("{:?}", instruction.opcode), operand)
}
