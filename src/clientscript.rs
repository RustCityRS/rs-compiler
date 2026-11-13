use crate::bytecode::{CompiledScript, Opcode, Operand};
use crate::lexer::Lexer;
use crate::parser::{Expr, Parser, ScriptFile, Statement, StringPart};
use crate::pointer::{PointerHolder, PointerType};
use crate::pointer_checker::PointerChecker;
use crate::symbol::{ReturnType, SymbolKind, SymbolRegistry, identifier_form};
use crate::symloader::{load_constant_files, load_game_var_types, load_packs};
use crate::typechecker::TypeChecker;
use crate::types::BaseVarType;
use crate::types::Type;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{Error, ErrorKind, Result};
use std::path::{Path, PathBuf};

pub mod decode;
pub mod decompile;
pub mod format;
pub(crate) mod hooks;
pub(crate) mod triggers;
use format::{Format, Width};
pub use hooks::{HookValue, component_hook};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Fixed,
    Hook,
    HookComponent,
    Enum,
    EnumOutput,
    Param,
}

pub struct Command {
    pub opcode: u16,
    pub name: String,
    pub params: Vec<Type>,
    pub returns: Vec<Type>,
    pub kind: CommandKind,
    pub component_last: bool,
}

pub(crate) fn command_pointers(registry: &SymbolRegistry) -> HashMap<String, PointerHolder> {
    let mut pointers = HashMap::new();
    for name in registry.commands.keys() {
        let name = name.to_lowercase();
        if !(name.starts_with("cc_") || name == "if_find") || name == "cc_deleteall" {
            continue;
        }
        for (prefix, pointer) in [
            ("", PointerType::ActiveComponent),
            (".", PointerType::ActiveComponent2),
        ] {
            let mut holder = PointerHolder::default();
            match name.as_str() {
                "cc_create" => holder.set.insert(pointer),
                "cc_find" | "if_find" => {
                    holder.set.insert(pointer);
                    holder.conditional_set = true;
                }
                _ => holder.required.insert(pointer),
            }
            pointers.insert(format!("{prefix}{name}"), holder);
        }
    }
    pointers
}

pub fn hook_types(signature: &str, triggers: usize, explicit: bool) -> Vec<Type> {
    let mut types = vec![Type::ClientScript];
    types.extend(
        signature
            .trim_end_matches('Y')
            .chars()
            .map(|ch| Type::from_char(ch).unwrap_or(Type::Int)),
    );
    if signature.ends_with('Y') {
        types.extend(vec![Type::Int; triggers + 1]);
    }
    types.push(Type::String);
    if explicit {
        types.push(Type::Component);
    }
    types
}

#[derive(Clone, Debug)]
pub struct Source {
    pub path: PathBuf,
    pub file: ScriptFile,
}

impl Source {
    pub fn parse(text: &str, path: &Path) -> Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
            file: parse(text, path)?,
        })
    }

    pub fn compile(
        &self,
        registry: &SymbolRegistry,
        format: &Format,
    ) -> Result<Vec<(i32, Vec<u8>)>> {
        compile(&self.file, &self.path, registry, format)
    }
}

pub fn script_key(name: &str) -> String {
    let (trigger, name) = name
        .strip_prefix('[')
        .and_then(|name| name.strip_suffix(']'))
        .and_then(|name| name.split_once(','))
        .unwrap_or(("proc", name));
    format!("[{trigger},{}]", identifier_form(name))
}

pub fn script_ids(pack: &Path) -> Result<HashMap<String, i32>> {
    let mut ids = HashMap::new();
    for line in fs::read_to_string(pack.join("clientscript.pack"))?.lines() {
        if let Some((id, name)) = line.split_once('=') {
            let key = script_key(name);
            if ids
                .insert(key.clone(), id.parse::<i32>().map_err(invalid)?)
                .is_some()
            {
                return Err(invalid(format!("Duplicate clientscript name {key}")));
            }
        }
    }
    Ok(ids)
}

pub fn compile_sources(
    sources: &[Source],
    source_dir: &Path,
    pack: &Path,
) -> Result<BTreeMap<u32, Vec<u8>>> {
    compile_registered_sources(sources, pack, registry(source_dir, pack)?)
}

pub fn compile_directory(source_dir: &Path, pack: &Path) -> Result<BTreeMap<u32, Vec<u8>>> {
    let registry = registry(source_dir, pack)?;
    let sources = load_sources(source_dir)?;
    compile_registered_sources(&sources, pack, registry)
}

/// Registers every script, giving the ones the pack doesn't name the next free ids.
fn register_sources(
    sources: &[Source],
    pack: &Path,
    registry: &mut SymbolRegistry,
) -> Result<Vec<(i32, String)>> {
    let mut ids = script_ids(pack)?;
    let mut next = ids.values().copied().max().unwrap_or(-1) + 1;
    for source in sources {
        for script in &source.file.scripts {
            if let Some(id) = script
                .name
                .strip_prefix("clientscript_")
                .and_then(|id| id.parse::<i32>().ok())
            {
                next = next.max(
                    id.checked_add(1)
                        .ok_or_else(|| invalid("Clientscript ID overflow"))?,
                );
            }
        }
    }
    let mut new_ids = Vec::new();
    let mut declared = HashSet::new();
    for source in sources {
        std::sync::Arc::make_mut(&mut registry.visibility)
            .declare_source(&source.path, &source.file);
        for script in &source.file.scripts {
            let key = format!("[{},{}]", script.trigger, script.name);
            if !declared.insert(key.clone()) {
                return Err(invalid(format!("Duplicate script {key}")));
            }
            if !ids.contains_key(&key)
                && !script
                    .name
                    .strip_prefix("clientscript_")
                    .is_some_and(|id| id.parse::<u32>().is_ok())
            {
                ids.insert(key.clone(), next);
                new_ids.push((next, key));
                next = next
                    .checked_add(1)
                    .ok_or_else(|| invalid("Clientscript ID overflow"))?;
            }
        }
        register(&source.file, &ids, registry)?;
    }
    Ok(new_ids)
}

pub fn compile_registered_sources(
    sources: &[Source],
    pack: &Path,
    mut registry: SymbolRegistry,
) -> Result<BTreeMap<u32, Vec<u8>>> {
    let format = Format::load(&pack.join("cs2_command.pack"))?;
    let new_ids = register_sources(sources, pack, &mut registry)?;
    let mut scripts = Vec::new();
    for source in sources {
        scripts.extend(checked_codegen(&source.file, &source.path, &registry)?);
    }
    PointerChecker::new_client(&scripts, &registry)
        .run()
        .print_all();
    let mut compiled = BTreeMap::new();
    for script in scripts {
        if compiled
            .insert(
                u32::try_from(script.id).map_err(invalid)?,
                encode(&script, &format)?,
            )
            .is_some()
        {
            return Err(invalid(format!("Duplicate clientscript ID {}", script.id)));
        }
    }
    if !new_ids.is_empty() {
        let path = pack.join("clientscript.pack");
        let mut text = fs::read_to_string(&path)?;
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        for (id, key) in new_ids {
            text.push_str(&format!("{id}={key}\n"));
        }
        fs::write(path, text)?;
    }
    Ok(compiled)
}

pub fn load_sources(dir: &Path) -> Result<Vec<Source>> {
    let mut sources = Vec::new();
    for path in crate::sources::script_files(dir, "cs2")? {
        if path
            .file_name()
            .is_some_and(|name| name != "engine.cs2" && name != "engine.gs2")
        {
            sources.push(Source::parse(&fs::read_to_string(&path)?, &path)?);
        }
    }
    sources.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(sources)
}

pub fn load_commands(path: &Path, engine_path: &Path) -> Result<Vec<Command>> {
    let format = Format::load(path)?;
    let engine_text = fs::read_to_string(engine_path)?;
    commands(&format, &engine_text, engine_path)
}

/// Parse host-supplied command signatures without reading files.
pub fn commands(format: &Format, engine_text: &str, label: &Path) -> Result<Vec<Command>> {
    let engine = parse(engine_text, label)?;
    let mut signatures = HashMap::new();
    for script in engine.scripts {
        if script.trigger != "command" || !script.body.is_empty() {
            return Err(invalid("engine.cs2 must contain command signatures"));
        }
        if signatures.insert(script.name.clone(), script).is_some() {
            return Err(invalid("Duplicate engine.cs2 command"));
        }
    }
    let mut commands = Vec::new();
    for (&opcode, name) in &format.commands {
        let signature = signatures
            .remove(name)
            .ok_or_else(|| invalid(format!("Missing engine.cs2 signature for {name}")))?;
        let annotation = |name| {
            signature
                .annotations
                .iter()
                .find(|annotation| annotation.name == name)
                .map(|annotation| annotation.value.as_str())
        };
        let kind = match annotation("signature").unwrap_or("fixed") {
            "fixed" => CommandKind::Fixed,
            "hook" => CommandKind::Hook,
            "hook_component" => CommandKind::HookComponent,
            "enum" => CommandKind::Enum,
            "enum_output" => CommandKind::EnumOutput,
            "param" => CommandKind::Param,
            kind => return Err(invalid(format!("Unknown command kind {kind}"))),
        };
        let component_last = annotation("stack") == Some("component_last");
        commands.push(Command {
            opcode,
            name: name.clone(),
            params: signature
                .params
                .iter()
                .map(|param| param.param_type)
                .collect(),
            returns: signature.return_types,
            kind,
            component_last,
        });
    }
    if !signatures.is_empty() {
        return Err(invalid(
            "engine.cs2 contains commands without opcode definitions",
        ));
    }
    Ok(commands)
}

/// The registry an interface's hooks read against: every script and what it takes.
pub fn registry_with_scripts(source_dir: &Path, pack: &Path) -> Result<SymbolRegistry> {
    let mut registry = registry(source_dir, pack)?;
    register_sources(&load_sources(source_dir)?, pack, &mut registry)?;
    Ok(registry)
}

pub fn registry(source_dir: &Path, pack: &Path) -> Result<SymbolRegistry> {
    let root = crate::sources::target_dir(source_dir, crate::sources::Scope::Client);
    let source_dir = root.as_path();
    let mut registry = SymbolRegistry::new();
    load_packs(&mut registry, pack).map_err(invalid)?;
    for (key, id) in script_ids(pack)? {
        let (trigger, name) = key[1..key.len() - 1].split_once(',').unwrap();
        registry
            .trigger_script_ids
            .insert(format!("{trigger}:{name}"), id);
        registry.script_ids.insert(name.to_owned(), id);
        if trigger == "proc" {
            registry.proc_script_ids.insert(name.to_owned(), id);
        }
    }
    registry.clear_commands();
    registry.game_vars.retain(|_, symbol| {
        matches!(
            &symbol.kind, SymbolKind::GameVar { category, .. }
            if matches!(category.as_str(), "varp" | "varpbit")
        )
    });
    let engine_path = crate::sources::engine_file(source_dir, crate::sources::Scope::Client)?
        .ok_or_else(|| {
            invalid(format!(
                "No engine.cs2 found under {}",
                source_dir.display()
            ))
        })?;
    for command in load_commands(&pack.join("cs2_command.pack"), &engine_path)? {
        registry.register_command(
            command.name.to_owned(),
            command.opcode as i32,
            command.params.clone(),
            command.returns.clone(),
        );
        registry.stack_commands.insert(command.name.to_owned());
        registry.set_component_first_command(&command.name, command.component_last);
        if matches!(command.kind, CommandKind::Hook | CommandKind::HookComponent) {
            registry.hook_commands.insert(
                command.name.to_owned(),
                command.kind == CommandKind::HookComponent,
            );
        }
        let dynamic = match command.kind {
            CommandKind::Enum => Some(ReturnType::TypeArgument(1)),
            CommandKind::Param => Some(ReturnType::ParamArgument(
                command.params.len().saturating_sub(1),
            )),
            _ => None,
        };
        if let Some(dynamic) = dynamic {
            registry
                .dynamic_returns
                .insert(command.name.clone(), dynamic);
        }
    }
    for (name, ty) in [
        ("__pop_int", Type::Int),
        ("__pop_string", Type::String),
        ("__pop_long", Type::Long),
        ("__join", Type::String),
        ("__get_array", Type::Int),
    ] {
        registry.register_command(name.to_owned(), 0, Vec::new(), vec![ty]);
    }
    for (kind, ty) in [("varc", Type::Int), ("varcstr", Type::String)] {
        let path = pack.join(format!("{kind}.pack"));
        if !path.exists() {
            continue;
        }
        for line in fs::read_to_string(path)?.lines() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if let Some((id, name)) = line.split_once('=') {
                registry.register_game_var(
                    name.to_owned(),
                    ty,
                    id.parse::<i32>().map_err(invalid)?,
                    kind.to_owned(),
                );
            }
        }
    }
    let configs = crate::sources::target_config_files(source_dir, crate::sources::Scope::Client)?;
    {
        for file in configs
            .iter()
            .filter(|path| path.extension().is_some_and(|ext| ext == "param"))
        {
            let text = fs::read_to_string(file)?;
            let mut id = None;
            for line in text.lines().map(str::trim) {
                if let Some(name) = line
                    .strip_prefix('[')
                    .and_then(|line| line.strip_suffix(']'))
                {
                    id = registry.lookup_reference(name, Type::Param);
                    if let Some(id) = id {
                        registry.register_param_type(id, Type::Int);
                    }
                } else if let Some(ty) = line.strip_prefix("type=")
                    && let Some(id) = id
                {
                    let ty = ty.trim();
                    let ty = if let Some(byte) = ty.strip_prefix("byte:0x") {
                        let byte = u8::from_str_radix(byte, 16).map_err(invalid)?;
                        Type::from_char(byte as char).unwrap_or(Type::Int)
                    } else {
                        Type::from_name(ty)
                            .ok_or_else(|| invalid(format!("Unknown parameter type {ty}")))?
                    };
                    registry.register_param_type(id, ty);
                }
            }
        }
    }
    load_constant_files(&mut registry, &configs);
    load_game_var_types(&mut registry, &configs);
    load_varc_types(&mut registry, &configs)?;
    load_clan_settings(&mut registry, &configs, pack)?;
    registry.visibility =
        std::sync::Arc::new(crate::sources::Visibility::load(source_dir, &registry));
    Ok(registry)
}

/// `varc.pack` only says which stack a varc is on; its file declares its type.
fn load_varc_types(registry: &mut SymbolRegistry, files: &[PathBuf]) -> Result<()> {
    for file in files
        .iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "varc"))
    {
        let mut section = None;
        for line in fs::read_to_string(file)?.lines().map(str::trim) {
            if let Some(name) = line
                .strip_prefix('[')
                .and_then(|line| line.strip_suffix(']'))
            {
                section = Some(name.to_owned());
            } else if let Some(ty) = line.strip_prefix("type=").and_then(Type::from_name)
                && let Some(SymbolKind::GameVar { var_type, .. }) = section
                    .as_ref()
                    .and_then(|name| registry.game_vars.get_mut(name))
                    .map(|symbol| &mut symbol.kind)
            {
                *var_type = ty;
            }
        }
    }
    Ok(())
}

/// A clan setting's opcodes follow its type, which only its config declares:
/// `basevar` for a bit range, otherwise `type`.
pub(crate) fn load_clan_settings(
    registry: &mut SymbolRegistry,
    files: &[PathBuf],
    pack: &Path,
) -> Result<()> {
    let ids: HashMap<String, i32> = fs::read_to_string(pack.join("varclansetting.pack"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_once('='))
        .filter_map(|(id, name)| Some((name.trim().to_owned(), id.parse().ok()?)))
        .collect();
    let mut add = |name: &str, ty: Option<&str>, bits: bool| -> Result<()> {
        let id = ids
            .get(name)
            .copied()
            .or_else(|| name.strip_prefix("varclansetting_")?.parse().ok())
            .ok_or_else(|| invalid(format!("Unknown clan setting {name}")))?;
        let (category, ty) = match (bits, ty) {
            (true, _) => ("clansetting_varbit", Type::Int),
            (_, Some("int" | "bit")) => ("clansetting_int", Type::Int),
            (_, Some("long")) => ("clansetting_long", Type::Long),
            (_, Some("clanforumqfc")) => ("clansetting_long", Type::ClanForumQfc),
            (_, Some("string")) => ("clansetting_string", Type::String),
            _ => {
                return Err(invalid(format!(
                    "Clan setting {name} has no opcodes for type {ty:?}"
                )));
            }
        };
        registry.register_game_var(name.to_owned(), ty, id, category.to_owned());
        Ok(())
    };
    for file in files {
        if file.extension().is_none_or(|ext| ext != "varclansetting") {
            continue;
        }
        let text = fs::read_to_string(file)?;
        let mut section: Option<(&str, Option<&str>, bool)> = None;
        for line in text.lines().map(str::trim) {
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                if let Some((name, ty, bits)) = section {
                    add(name, ty, bits)?;
                }
                section = Some((name, None, false));
            } else if let Some((_, ty, bits)) = &mut section {
                if let Some(value) = line.strip_prefix("type=") {
                    *ty = Some(value.trim());
                } else if line.starts_with("basevar=") {
                    *bits = true;
                }
            }
        }
        if let Some((name, ty, bits)) = section {
            add(name, ty, bits)?;
        }
    }
    Ok(())
}

pub fn parse(source: &str, path: &Path) -> Result<ScriptFile> {
    let path = path.to_path_buf();
    let tokens = Lexer::new(source, &path).tokenize().map_err(invalid)?;
    Parser::new(tokens, &path).parse().map_err(invalid)
}

pub fn register(
    file: &ScriptFile,
    ids: &HashMap<String, i32>,
    registry: &mut SymbolRegistry,
) -> Result<()> {
    for script in &file.scripts {
        for statement in &script.body {
            register_variables(statement, registry);
        }
        if triggers::lookup(&script.trigger).is_none() {
            return Err(invalid(format!(
                "Invalid client trigger {}",
                script.trigger
            )));
        }
        let id = ids
            .get(&format!("[{},{}]", script.trigger, script.name))
            .or_else(|| ids.get(&script.name))
            .copied()
            .or_else(|| {
                script
                    .name
                    .strip_prefix("clientscript_")
                    .and_then(|id| id.parse::<i32>().ok())
                    .filter(|id| *id >= 0)
            })
            .ok_or_else(|| invalid(format!("Missing clientscript ID for {}", script.name)))?;
        registry
            .preloaded_script_ids
            .insert(format!("{}:{}", script.trigger, script.name), id);
        registry.register_script(
            script.name.clone(),
            script.trigger.clone(),
            script.params.iter().map(|p| p.param_type).collect(),
            script.return_types.clone(),
        );
        registry
            .stack_scripts
            .insert(format!("{}:{}", script.trigger, script.name));
    }
    Ok(())
}

pub(crate) fn register_variables(statement: &Statement, registry: &mut SymbolRegistry) {
    match statement {
        Statement::VarDeclaration {
            value: Some(value), ..
        }
        | Statement::Expression { expr: value, .. }
        | Statement::ArrayDeclaration { size: value, .. } => {
            register_expr_variables(value, registry)
        }
        Statement::Assignment { target, value, .. } => {
            register_expr_variables(target, registry);
            register_expr_variables(value, registry);
        }
        Statement::Return { values, .. } => {
            for value in values {
                register_expr_variables(value, registry);
            }
        }
        Statement::If {
            condition,
            body,
            else_if,
            else_body,
            ..
        } => {
            register_expr_variables(condition, registry);
            for statement in body {
                register_variables(statement, registry);
            }
            for (condition, body, _) in else_if {
                register_expr_variables(condition, registry);
                for statement in body {
                    register_variables(statement, registry);
                }
            }
            if let Some(body) = else_body {
                for statement in body {
                    register_variables(statement, registry);
                }
            }
        }
        Statement::While {
            condition, body, ..
        } => {
            register_expr_variables(condition, registry);
            for statement in body {
                register_variables(statement, registry);
            }
        }
        Statement::Switch {
            expr,
            cases,
            default,
            ..
        } => {
            register_expr_variables(expr, registry);
            for case in cases {
                for statement in &case.body {
                    register_variables(statement, registry);
                }
            }
            if let Some(body) = default {
                for statement in body {
                    register_variables(statement, registry);
                }
            }
        }
        _ => {}
    }
}

fn register_expr_variables(expr: &Expr, registry: &mut SymbolRegistry) {
    match expr {
        Expr::GameVar(name, _) if registry.lookup_game_var(name).is_none() => {
            for (kind, ty) in [
                ("varp", Type::Int),
                ("varpbit", Type::Int),
                ("varc", Type::Int),
                ("varcstr", Type::String),
            ] {
                if let Some(id) = name
                    .strip_prefix(&format!("{}_", hooks::var_placeholder(kind)))
                    .and_then(|s| s.parse::<u16>().ok())
                {
                    registry.register_game_var(name.clone(), ty, id as i32, kind.to_owned());
                }
            }
        }
        Expr::ArrayAccess { index, .. } | Expr::Calc(index) => {
            register_expr_variables(index, registry)
        }
        Expr::BinaryOp { lhs, rhs, .. } | Expr::LogicalOp { lhs, rhs, .. } => {
            register_expr_variables(lhs, registry);
            register_expr_variables(rhs, registry);
        }
        Expr::CommandCall { name, args, .. } => {
            if let Some(&explicit) = registry.hook_commands.get(name.trim_start_matches('.'))
                && args.len() == 1 + usize::from(explicit)
                && let Some(Expr::StringLiteral(text)) = args.first()
                && let Ok(values) = hooks::parse(text)
            {
                if hooks::transmit_kind(name).is_some_and(|kind| kind.starts_with("var"))
                    && let Some(Expr::Tuple(transmits)) = values.get(2)
                {
                    for value in transmits {
                        if let Expr::Identifier(name) = value {
                            register_expr_variables(&Expr::GameVar(name.clone(), 0), registry);
                        }
                    }
                }
                for value in &values {
                    register_expr_variables(value, registry);
                }
            }
            for arg in args {
                register_expr_variables(arg, registry);
            }
        }
        Expr::ProcCall { args, .. }
        | Expr::JumpCall { args, .. }
        | Expr::MultiAssign(args, _)
        | Expr::Tuple(args) => {
            for arg in args {
                register_expr_variables(arg, registry);
            }
        }
        Expr::JoinedString { parts } => {
            for part in parts {
                if let StringPart::Expr(expr) = part {
                    register_expr_variables(expr, registry);
                }
            }
        }
        _ => {}
    }
}

pub fn compile(
    file: &ScriptFile,
    path: &Path,
    registry: &SymbolRegistry,
    format: &Format,
) -> Result<Vec<(i32, Vec<u8>)>> {
    let scripts = checked_codegen(file, path, registry)?;
    PointerChecker::new_client(&scripts, registry)
        .run()
        .print_all();
    scripts
        .into_iter()
        .map(|compiled| Ok((compiled.id, encode(&compiled, format)?)))
        .collect()
}

fn checked_codegen(
    file: &ScriptFile,
    path: &Path,
    registry: &SymbolRegistry,
) -> Result<Vec<CompiledScript>> {
    let mut checker = TypeChecker::new(registry);
    checker.check_file(file, path);
    if checker.diagnostics.has_errors() {
        return Err(invalid(
            checker
                .diagnostics
                .errors()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        ));
    }
    codegen(file, path, registry)
}

pub(crate) fn codegen(
    file: &ScriptFile,
    path: &Path,
    registry: &SymbolRegistry,
) -> Result<Vec<CompiledScript>> {
    let mut diagnostics = crate::diagnostics::DiagnosticsCollector::new();
    let compiled = crate::compiler::compile_file(file, path, registry, true, &mut diagnostics);
    if diagnostics.has_errors() {
        return Err(invalid(
            diagnostics
                .errors()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(
                    "
",
                ),
        ));
    }
    Ok(compiled)
}

pub fn encode(script: &CompiledScript, format: &Format) -> Result<Vec<u8>> {
    encode_named(script, "", format)
}

pub fn encode_named(script: &CompiledScript, name: &str, format: &Format) -> Result<Vec<u8>> {
    let mut positions = Vec::with_capacity(script.instructions.len() + 1);
    let mut count = 0;
    for instruction in &script.instructions {
        positions.push(count);
        if instruction.opcode != Opcode::LineNumber {
            count += 1;
        }
    }
    positions.push(count);
    let mut out = Vec::new();
    write_string(&mut out, name)?;
    let mut switches = Vec::new();
    for (i, instruction) in script.instructions.iter().enumerate() {
        if instruction.opcode == Opcode::LineNumber {
            continue;
        }
        if instruction.opcode == Opcode::Raw {
            let Operand::Long(value) = instruction.operand else {
                return Err(invalid("Invalid raw client instruction"));
            };
            let opcode = (value >> 32) as u16;
            let operand = value as i32;
            out.extend_from_slice(&opcode.to_be_bytes());
            format.width(opcode).integer(&mut out, operand as i64)?;
            continue;
        }
        let (opcode, width) = match instruction.opcode {
            Opcode::Command => match instruction.operand {
                Operand::Int(value) => (value as u16, format.width(value as u16)),
                _ => return Err(invalid("Unresolved client command")),
            },
            value => format.instruction(value)?,
        };
        let value = match &instruction.operand {
            Operand::None => 0,
            Operand::Int(value) if instruction.opcode == Opcode::Command => (value >> 16) & 255,
            Operand::Int(value) => *value,
            Operand::JumpTarget(target) => {
                *positions
                    .get(*target)
                    .ok_or_else(|| invalid("Invalid branch target"))?
                    - positions[i]
                    - 1
            }
            Operand::StringCount(value) => i32::try_from(*value).map_err(invalid)?,
            Operand::ArrayDef(slot, kind) => (slot << 16) | *kind as i32,
            Operand::SwitchTable(index) => {
                let table = script
                    .switch_tables
                    .get(*index as usize)
                    .ok_or_else(|| invalid("Invalid switch table"))?;
                let mut cases = Vec::with_capacity(table.len());
                for (key, target) in table {
                    let position = positions
                        .get(*target)
                        .ok_or_else(|| invalid("Invalid switch target"))?;
                    cases.push((*key, *position - positions[i] - 1));
                }
                switches.push(cases);
                i32::try_from(switches.len() - 1).map_err(invalid)?
            }
            Operand::Str(index) => {
                if width != Width::String {
                    return Err(invalid(
                        "String instruction requires a string operand definition",
                    ));
                }
                out.extend_from_slice(&opcode.to_be_bytes());
                write_string(&mut out, &script.strings[*index as usize])?;
                continue;
            }
            Operand::Long(value) => {
                out.extend_from_slice(&opcode.to_be_bytes());
                width.integer(&mut out, *value)?;
                continue;
            }
        };
        out.extend_from_slice(&opcode.to_be_bytes());
        width.integer(&mut out, value as i64)?;
    }
    out.extend_from_slice(&count.to_be_bytes());
    for counts in [
        [
            script.int_local_count,
            script.string_local_count,
            script.long_local_count,
        ],
        [
            script.int_arg_count,
            script.string_arg_count,
            script.long_arg_count,
        ],
    ] {
        for (kind, value) in [BaseVarType::Integer, BaseVarType::String, BaseVarType::Long]
            .into_iter()
            .zip(counts)
        {
            if value != 0 && !format.local_types.contains(&kind) {
                return Err(invalid(format!("Unsupported {kind:?} locals")));
            }
        }
        for kind in &format.local_types {
            let index = match kind {
                BaseVarType::Integer => 0,
                BaseVarType::String => 1,
                BaseVarType::Long => 2,
            };
            out.extend_from_slice(&counts[index].to_be_bytes());
        }
    }
    if !format.switches {
        if !switches.is_empty() {
            return Err(invalid(
                "Switch tables are unsupported by input bytecode definitions",
            ));
        }
        return Ok(out);
    }
    let start = out.len();
    out.push(u8::try_from(switches.len()).map_err(invalid)?);
    for table in switches {
        out.extend_from_slice(&u16::try_from(table.len()).map_err(invalid)?.to_be_bytes());
        for (key, offset) in table {
            out.extend_from_slice(&key.to_be_bytes());
            out.extend_from_slice(&offset.to_be_bytes());
        }
    }
    let size = u16::try_from(out.len() - start).map_err(invalid)?;
    out.extend_from_slice(&size.to_be_bytes());
    Ok(out)
}

pub fn write_string(out: &mut Vec<u8>, text: &str) -> Result<()> {
    const CP1252: &str = "€\u{81}‚ƒ„…†‡ˆ‰Š‹Œ\u{8d}Ž\u{8f}\u{90}‘’“”•–—˜™š›œ\u{9d}žŸ";
    for ch in text.chars() {
        let value = ch as u32;
        if value == 0 {
            return Err(invalid("NUL in clientscript string"));
        }
        if value < 128 || (160..=255).contains(&value) {
            out.push(value as u8);
        } else if let Some(index) = CP1252.chars().position(|c| c == ch) {
            out.push(128 + index as u8);
        } else {
            return Err(invalid(format!(
                "Character {ch:?} cannot be encoded in CP1252"
            )));
        }
    }
    out.push(0);
    Ok(())
}

fn invalid(message: impl ToString) -> Error {
    Error::new(ErrorKind::InvalidData, message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clan_settings_take_their_opcodes_from_their_type() {
        let dir = std::env::temp_dir().join(format!("runec_clan_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("all.varclansetting");
        fs::write(
            &file,
            "// 0\n[varclansetting_0]\ntype=int\n\n// 1\n[named]\ntype=long\n\n\
             // 2\n[varclansetting_2]\ntype=string\n\n\
             // 3\n[varclansetting_3]\ntype=byte:0x01\nbasevar=2\nstartbit=0\nendbit=0\n",
        )
        .unwrap();
        fs::write(dir.join("varclansetting.pack"), "1=named\n").unwrap();
        let mut registry = SymbolRegistry::new();
        load_clan_settings(&mut registry, std::slice::from_ref(&file), &dir).unwrap();
        for (name, id, expected) in [
            ("varclansetting_0", 0, "clansetting_int"),
            ("named", 1, "clansetting_long"),
            ("varclansetting_2", 2, "clansetting_string"),
            ("varclansetting_3", 3, "clansetting_varbit"),
        ] {
            let Some(SymbolKind::GameVar {
                var_id, category, ..
            }) = registry.lookup_game_var(name).map(|symbol| &symbol.kind)
            else {
                panic!("{name} is not registered");
            };
            assert_eq!((*var_id, category.as_str()), (id, expected), "{name}");
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_varc_takes_the_type_its_file_declares() {
        let dir = std::env::temp_dir().join(format!("runec-varc-types-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("all.varc");
        fs::write(
            &file,
            "// 5\n[varclient_5]\ntype=boolean\n\n// 6\n[varclient_6]\ntype=byte:0x65\n\n// 7\n[varclient_7]\n",
        )
        .unwrap();
        let mut registry = SymbolRegistry::new();
        for id in 5..8 {
            registry.register_game_var(format!("varclient_{id}"), Type::Int, id, "varc".into());
        }
        load_varc_types(&mut registry, std::slice::from_ref(&file)).unwrap();
        let declared = |name: &str| match registry.lookup_game_var(name).map(|symbol| &symbol.kind)
        {
            Some(SymbolKind::GameVar { var_type, .. }) => *var_type,
            _ => panic!("{name} is not registered"),
        };
        assert_eq!(declared("varclient_5"), Type::Boolean);
        assert_eq!(declared("varclient_6"), Type::Int);
        assert_eq!(declared("varclient_7"), Type::Int);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_transmit_name_is_never_the_dotted_alias() {
        let mut registry = SymbolRegistry::new();
        registry.register_game_var("bank_open".into(), Type::Int, 90, "varp".into());
        registry.register_game_var(".bank_open".into(), Type::Int, 90, "varp".into());
        assert_eq!(
            hooks::transmit_name(&registry, "varp", 90).as_deref(),
            Some("bank_open")
        );
    }
}
