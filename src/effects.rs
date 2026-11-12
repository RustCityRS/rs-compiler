//! Conservative editor summaries of script suspension and game-variable writes.
use crate::clientscript::hooks;
use crate::parser::{Expr, Statement, StringPart};
use crate::pointer::PointerSet;
use crate::project::{Project, SourceId};
use crate::sources::Scope;
use crate::symbol::SymbolRegistry;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::sync::Arc;

/// Effects visible above a script header. These describe possible execution;
/// they do not promise that every path suspends or writes every listed variable.
#[derive(Clone, Debug)]
pub struct ScriptEffects {
    pub range: Range<usize>,
    pub may_suspend: bool,
    /// Game-variable names, without `%`; a leading dot selects the secondary entity.
    pub writes: Vec<String>,
    /// Missing or ambiguous calls, or damaged syntax, prevent a complete summary.
    pub incomplete: bool,
    /// Available only after pointer checking completed for this exact generation.
    pub required: Option<PointerSet>,
}

impl ScriptEffects {
    /// A compact editor label; absence of known effects never claims purity.
    pub fn label(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.may_suspend {
            parts.push("may suspend".to_owned());
        }
        if !self.writes.is_empty() {
            let mut writes = format!(
                "writes {}",
                self.writes
                    .iter()
                    .take(6)
                    .map(|name| match name.strip_prefix('.') {
                        Some(name) => format!(".%{name}"),
                        None => format!("%{name}"),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            if self.writes.len() > 6 {
                writes.push_str(&format!(" +{} more", self.writes.len() - 6));
            }
            parts.push(writes);
        }
        if self.incomplete {
            parts.push("incomplete".to_owned());
        }
        (!parts.is_empty()).then(|| format!("@effects {}", parts.join("; ")))
    }
}

#[derive(Clone, Default)]
pub(crate) struct Summary {
    pub may_suspend: bool,
    pub writes: BTreeSet<String>,
    pub incomplete: bool,
}

impl Summary {
    fn include(&mut self, other: &Self) -> bool {
        let before = (self.may_suspend, self.writes.len(), self.incomplete);
        self.may_suspend |= other.may_suspend;
        self.writes.extend(other.writes.iter().cloned());
        self.incomplete |= other.incomplete;
        before != (self.may_suspend, self.writes.len(), self.incomplete)
    }
}

pub(crate) struct Effects {
    pub generation: u64,
    pub scripts: HashMap<(SourceId, usize, String), Summary>,
}

struct Node {
    source: SourceId,
    line: usize,
    scope: Scope,
    trigger: String,
    name: String,
    summary: Summary,
    calls: Vec<(String, String)>,
}

struct Visitor<'a> {
    registry: &'a SymbolRegistry,
    commands: &'a HashMap<String, bool>,
    server: bool,
    script_names: &'a HashSet<String>,
    callback_names: &'a HashSet<String>,
    node: &'a mut Node,
}

impl Visitor<'_> {
    fn callback(&mut self, callback: &Expr, transmits: bool) {
        match callback {
            Expr::StringLiteral(text) => match hooks::parse(text) {
                Ok(values) => {
                    if !transmits && values.len() > 2 {
                        self.node.summary.incomplete = true;
                    }
                    if let Some(Expr::Identifier(target)) = values.first()
                        && !self.callback_names.contains(target)
                        && self
                            .registry
                            .lookup_script_by_trigger("clientscript", target)
                            .is_none()
                        && self
                            .registry
                            .lookup_script_by_trigger("proc", target)
                            .is_none()
                    {
                        self.node.summary.incomplete = true;
                    }
                    // The callback body runs later. Only its captured argument
                    // expressions execute while this script installs or sends it.
                    if let Some(Expr::Tuple(args)) = values.get(1) {
                        for arg in args {
                            self.expr(arg);
                        }
                    } else {
                        self.node.summary.incomplete = true;
                    }
                }
                Err(_) => self.node.summary.incomplete = true,
            },
            Expr::CommandCall { name, args, .. } if name == "__hook" => {
                if let Some(Expr::Tuple(values)) = args.get(1) {
                    for value in values {
                        self.expr(value);
                    }
                } else {
                    self.node.summary.incomplete = true;
                }
            }
            Expr::NullLiteral => {}
            _ => {
                self.node.summary.incomplete = true;
                self.expr(callback);
            }
        }
    }

    fn command(&mut self, name: &str) {
        let primary = name.trim_start_matches('.');
        match self.commands.get(primary) {
            Some(false) => self.node.summary.incomplete = true,
            Some(true) => {
                self.node.summary.may_suspend |=
                    self.registry.command_suspends_for(name, self.server);
            }
            None if self.registry.lookup_command(primary).is_some() => {
                self.node.summary.may_suspend |=
                    self.registry.command_suspends_for(name, self.server);
            }
            None => self.node.summary.incomplete = true,
        }
    }

    fn write(&mut self, target: &Expr) {
        match target {
            Expr::GameVar(name, _) => {
                self.node.summary.writes.insert(name.clone());
            }
            Expr::MultiAssign(targets, _) | Expr::Tuple(targets) => {
                for target in targets {
                    self.write(target);
                }
            }
            _ => {}
        }
    }

    fn expr(&mut self, expr: &Expr) {
        match expr {
            Expr::CommandCall { name, args, .. } => {
                let command = name.trim_start_matches('.');
                // These compiler intrinsics only expose evaluation or local VM stacks.
                // Raw opcodes deliberately remain incomplete: their effects are unknown.
                if !matches!(
                    name.as_str(),
                    "__push"
                        | "__pop_int"
                        | "__pop_string"
                        | "__pop_long"
                        | "__join"
                        | "__get_array"
                        | "__set_array"
                        | "__define_array"
                ) {
                    self.command(name);
                }
                if matches!(
                    command,
                    "runclientscript" | "runclientscript*" | "runclientscriptvararg"
                ) {
                    if command == "runclientscript"
                        && matches!(args.as_slice(), [Expr::StringLiteral(_)])
                    {
                        self.callback(&args[0], false);
                    } else {
                        self.node.summary.incomplete = true;
                        for arg in args {
                            self.expr(arg);
                        }
                    }
                    return;
                }
                if let Some(&explicit) = self.registry.hook_commands.get(command) {
                    let target =
                        usize::from(explicit && self.registry.component_first_command(command));
                    if args.len() == 1 + usize::from(explicit) {
                        self.callback(&args[target], true);
                        if explicit {
                            self.expr(&args[1 - target]);
                        }
                    } else {
                        if args.len() < 1 + usize::from(explicit) {
                            self.node.summary.incomplete = true;
                        }
                        for (index, arg) in args.iter().enumerate() {
                            if index != target
                                || !matches!(arg, Expr::Identifier(_) | Expr::StringLiteral(_))
                            {
                                self.expr(arg);
                            }
                        }
                    }
                    return;
                }
                for arg in args {
                    self.expr(arg);
                }
            }
            Expr::ProcCall { name, args, .. } | Expr::JumpCall { name, args, .. } => {
                let trigger = if matches!(expr, Expr::JumpCall { .. }) {
                    "label"
                } else {
                    "proc"
                };
                self.node.calls.push((trigger.to_owned(), name.clone()));
                for arg in args {
                    self.expr(arg);
                }
            }
            Expr::Identifier(name) => {
                // A bare identifier can be a zero-argument command or a value reference.
                let component = name.split_once(':').is_some_and(|(interface, component)| {
                    self.registry
                        .lookup_component(interface, component)
                        .is_some()
                });
                if component
                    || self.registry.lookup_entity_id(name).is_some()
                    || self.registry.lookup_constant(name).is_some()
                    || self
                        .registry
                        .lookup_game_var(name.trim_start_matches('.'))
                        .is_some()
                    || self.registry.type_chars.contains_key(name)
                {
                    return;
                }
                let command = name.trim_start_matches('.');
                if !self.commands.contains_key(command)
                    && self.registry.lookup_command(command).is_none()
                    && (self.script_names.contains(name)
                        || self.registry.lookup_script(name).is_some())
                {
                    return;
                }
                self.command(name);
            }
            Expr::BinaryOp { lhs, rhs, .. } | Expr::LogicalOp { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            Expr::Calc(value) | Expr::ArrayAccess { index: value, .. } => self.expr(value),
            Expr::JoinedString { parts } => {
                for part in parts {
                    if let StringPart::Expr(value) = part {
                        self.expr(value);
                    }
                }
            }
            Expr::MultiAssign(values, _) | Expr::Tuple(values) => {
                for value in values {
                    self.expr(value);
                }
            }
            _ => {}
        }
    }

    fn body(&mut self, body: &[Statement]) {
        for statement in body {
            match statement {
                Statement::VarDeclaration {
                    value: Some(value), ..
                }
                | Statement::ArrayDeclaration { size: value, .. }
                | Statement::Expression { expr: value, .. } => self.expr(value),
                Statement::Assignment { target, value, .. } => {
                    self.write(target);
                    self.expr(target);
                    self.expr(value);
                }
                Statement::Return { values, .. } => {
                    for value in values {
                        self.expr(value);
                    }
                }
                Statement::If {
                    condition,
                    body,
                    else_if,
                    else_body,
                    ..
                } => {
                    self.expr(condition);
                    self.body(body);
                    for (condition, body, _) in else_if {
                        self.expr(condition);
                        self.body(body);
                    }
                    if let Some(body) = else_body {
                        self.body(body);
                    }
                }
                Statement::While {
                    condition, body, ..
                } => {
                    self.expr(condition);
                    self.body(body);
                }
                Statement::Switch {
                    expr,
                    cases,
                    default,
                    ..
                } => {
                    self.expr(expr);
                    for case in cases {
                        self.body(&case.body);
                    }
                    if let Some(body) = default {
                        self.body(body);
                    }
                }
                Statement::OrphanCase { case, .. } => {
                    self.node.summary.incomplete = true;
                    self.body(&case.body);
                }
                _ => {}
            }
        }
    }
}

impl Effects {
    fn build(project: &Project) -> Option<Self> {
        let scope = project.target_scope();
        let documents = project
            .documents()
            .filter(|document| document.scope == scope || document.scope == Scope::Global)
            .collect::<Vec<_>>();
        let mut commands = HashMap::new();
        let mut script_names = HashSet::new();
        let callback_names = project
            .documents()
            .filter(|document| matches!(document.scope, Scope::Client | Scope::Global))
            .filter_map(|document| document.syntax())
            .flat_map(|file| &file.scripts)
            .filter(|script| matches!(script.trigger.as_str(), "proc" | "clientscript"))
            .map(|script| script.name.clone())
            .collect::<HashSet<_>>();
        let mut nodes = Vec::new();
        let mut definitions: HashMap<(Scope, String, String), Vec<usize>> = HashMap::new();
        for document in &documents {
            if project.analysis_cancelled() {
                return None;
            }
            let Some(file) = document.syntax() else {
                continue;
            };
            for script in &file.scripts {
                if script.trigger == "command" {
                    let name = script.name.trim_start_matches('.').to_owned();
                    let valid = !document.diagnostics.has_errors();
                    commands
                        .entry(name)
                        .and_modify(|previous: &mut bool| *previous &= valid)
                        .or_insert(valid);
                    continue;
                }
                script_names.insert(script.name.clone());
                definitions
                    .entry((document.scope, script.trigger.clone(), script.name.clone()))
                    .or_default()
                    .push(nodes.len());
                nodes.push(Node {
                    source: document.id,
                    line: script.line,
                    scope: document.scope,
                    trigger: script.trigger.clone(),
                    name: script.name.clone(),
                    summary: Summary {
                        incomplete: document.diagnostics.has_errors(),
                        ..Summary::default()
                    },
                    calls: Vec::new(),
                });
            }
        }
        let mut next = 0;
        for document in &documents {
            let Some(file) = document.syntax() else {
                continue;
            };
            for script in &file.scripts {
                if project.analysis_cancelled() {
                    return None;
                }
                if script.trigger == "command" {
                    continue;
                }
                Visitor {
                    registry: project.registry(),
                    commands: &commands,
                    server: scope == Scope::Rune,
                    script_names: &script_names,
                    callback_names: &callback_names,
                    node: &mut nodes[next],
                }
                .body(&script.body);
                next += 1;
            }
        }
        let mut callers = vec![Vec::new(); nodes.len()];
        for (index, node) in nodes.iter_mut().enumerate() {
            if project.analysis_cancelled() {
                return None;
            }
            let mut incomplete =
                definitions[&(node.scope, node.trigger.clone(), node.name.clone())].len() != 1;
            let mut callees = HashSet::new();
            for (trigger, name) in &node.calls {
                let mut found = Vec::new();
                let triggers = if trigger == "proc" && scope == Scope::Client {
                    vec!["proc", "clientscript"]
                } else {
                    vec![trigger.as_str()]
                };
                for trigger in triggers {
                    for owner in [Scope::Global, node.scope]
                        .into_iter()
                        .take(if node.scope == Scope::Global { 1 } else { 2 })
                    {
                        if let Some(ids) =
                            definitions.get(&(owner, trigger.to_owned(), name.clone()))
                        {
                            found.extend(ids.iter().copied());
                        }
                    }
                    if !found.is_empty() {
                        break;
                    }
                }
                found.sort_unstable();
                found.dedup();
                incomplete |= found.len() != 1;
                callees.extend(found);
            }
            node.summary.incomplete |= incomplete;
            for callee in callees {
                callers[callee].push(index);
            }
        }
        let mut pending = (0..nodes.len()).collect::<VecDeque<_>>();
        let mut queued = vec![true; nodes.len()];
        while let Some(callee) = pending.pop_front() {
            if project.analysis_cancelled() {
                return None;
            }
            queued[callee] = false;
            let summary = nodes[callee].summary.clone();
            for &caller in &callers[callee] {
                if nodes[caller].summary.include(&summary) && !queued[caller] {
                    queued[caller] = true;
                    pending.push_back(caller);
                }
            }
        }
        Some(Self {
            generation: project.generation,
            scripts: nodes
                .into_iter()
                .map(|node| {
                    (
                        (
                            node.source,
                            node.line,
                            format!("[{},{}]", node.trigger, node.name),
                        ),
                        node.summary,
                    )
                })
                .collect(),
        })
    }
}

impl Project {
    /// Whether this generation already has an effect summary; performs no analysis.
    pub fn effects_ready(&self) -> bool {
        self.effects
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|effects| effects.generation == self.generation)
    }

    pub(crate) fn effect_summaries(&self) -> Option<Arc<Effects>> {
        if let Some(effects) = self
            .effects
            .lock()
            .unwrap()
            .as_ref()
            .filter(|effects| effects.generation == self.generation)
        {
            return Some(effects.clone());
        }
        let effects = Arc::new(Effects::build(self)?);
        *self.effects.lock().unwrap() = Some(effects.clone());
        Some(effects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clientscript::format::Format;
    use crate::environment::EngineProfile;
    use crate::pointer::{PointerHolder, default_command_pointers};
    use crate::project::{CancellationToken, Options, Target};
    use crate::types::Type;

    fn project() -> Project {
        let mut registry = SymbolRegistry::new();
        registry.register_command("pause".into(), 1, vec![Type::Int], vec![]);
        registry.set_command_suspends("pause", true);
        Project::new(Target::Rune, registry, Options::default())
    }

    #[test]
    fn suspension_uses_pointer_contracts_and_respects_target_and_profile_overrides() {
        let mut registry = SymbolRegistry::new();
        for (index, name) in [
            "world_delay",
            "p_delay",
            "p_stringdialog",
            "controller_del",
            "custom_pause",
        ]
        .into_iter()
        .enumerate()
        {
            registry.register_command(name.into(), index as i32 + 71, vec![Type::Int], vec![]);
        }
        assert!(registry.command_suspends("world_delay"));
        assert!(registry.command_suspends(".p_delay"));
        assert!(registry.command_suspends("p_loaddelay"));
        assert!(registry.command_suspends("p_stringdialog"));
        let deletion = &default_command_pointers()["controller_del"];
        assert!(!deletion.corrupted.is_empty());
        assert!(!deletion.may_suspend);
        assert!(registry.command_suspends("p_namedialog"));
        assert!(!registry.command_suspends("p_get_suggestion"));
        let mut server = Project::new(Target::Rune, registry.clone(), Options::default());
        let source = server.open("delays.rs2", Scope::Rune, 1,
            "[command,p_delay](int $ticks)\n[proc,delayed]\np_delay(1);\n[proc,caller]\n~delayed;\n[proc,world]\nworld_delay(1);");
        assert!(
            server
                .script_effects(source)
                .iter()
                .all(|hint| hint.may_suspend)
        );

        let format = Format::parse("0=PushConstantInt\n21=Return\n").unwrap();
        let mut client = Project::new(Target::Client(format), registry.clone(), Options::default());
        let source = client.open(
            "delays.cs2",
            Scope::Client,
            1,
            "[proc,client]\np_delay(1); world_delay(1);",
        );
        assert!(!client.script_effects(source)[0].may_suspend);

        registry.set_command_suspends("p_delay", false);
        assert!(!registry.command_suspends("p_delay"));
        assert!(!registry.command_suspends(".p_delay"));
        registry.set_command_suspends("custom_pause", true);
        assert!(registry.command_suspends("custom_pause"));
        let mut profile = EngineProfile::default();
        profile.command_pointers.insert(
            "custom_pause".into(),
            PointerHolder {
                may_suspend: true,
                ..PointerHolder::default()
            },
        );
        registry.set_engine_profile(profile);
        assert!(!registry.command_suspends("world_delay"));
        assert!(registry.command_suspends("custom_pause"));
        registry.set_command_suspends("custom_pause", false);
        assert!(!registry.command_suspends("custom_pause"));
    }

    #[test]
    fn effects_follow_calls_cycles_and_edits_without_compiling() {
        let mut project = project();
        let leaf = project.open(
            "leaf.rs2",
            Scope::Rune,
            1,
            "[proc,leaf]\n%flag = 1; pause(1);",
        );
        let callers = project.open(
            "callers.rs2",
            Scope::Rune,
            1,
            "[proc,a]\n~b;\n[proc,b]\n~a; ~leaf; @done;\n[label,done]\n.%other = 2;",
        );
        let first = project.effect_summaries().unwrap();
        assert!(project.effects_ready());
        assert!(Arc::ptr_eq(&first, &project.effect_summaries().unwrap()));
        let hints = project.script_effects(callers);
        assert_eq!(hints.len(), 3);
        for hint in &hints[..2] {
            assert!(hint.may_suspend);
            assert_eq!(hint.writes, [".other", "flag"]);
            assert_eq!(
                hint.label().as_deref(),
                Some("@effects may suspend; writes .%other, %flag")
            );
            assert!(!hint.incomplete);
            assert!(hint.required.is_none());
        }
        assert!(!project.pointer_requirements_ready());
        project.update(leaf, 2, "[proc,leaf]\nreturn;").unwrap();
        assert!(!project.effects_ready());
        let updated = project.script_effects(callers);
        assert!(!updated[0].may_suspend);
        assert_eq!(updated[0].writes, [".other"]);
        assert!(!Arc::ptr_eq(&first, &project.effect_summaries().unwrap()));
    }

    #[test]
    fn unresolved_and_ambiguous_calls_stay_incomplete_and_source_scoped() {
        let mut project = project();
        let caller = project.open(
            "caller.rs2",
            Scope::Rune,
            1,
            "[proc,caller]\n~duplicate; ~missing; mystery();",
        );
        project.open("a.rs2", Scope::Rune, 1, "[proc,duplicate]\n%first = 1;");
        project.open("b.rs2", Scope::Rune, 1, "[proc,duplicate]\n%second = 2;");
        let client = project.open(
            "client.cs2",
            Scope::Client,
            1,
            "[proc,caller]\npause(1); %client = 1;",
        );
        let hint = project.script_effects(caller).remove(0);
        assert!(hint.incomplete);
        assert!(!hint.may_suspend);
        assert_eq!(hint.writes, ["first", "second"]);
        assert!(project.script_effects(client).is_empty());
        assert!(hint.label().unwrap().ends_with("incomplete"));
    }

    #[test]
    fn cancelled_analysis_does_not_cache_partial_effects() {
        let mut project = project();
        let source = project.open("source.rs2", Scope::Rune, 1, "[proc,leaf]\npause(1);");
        let token = CancellationToken::default();
        token.cancel();
        project.set_cancellation(token);
        assert!(project.script_effects(source).is_empty());
        assert!(project.effects.lock().unwrap().is_none());
        project.set_cancellation(CancellationToken::default());
        assert!(project.script_effects(source)[0].may_suspend);
    }

    #[test]
    fn callback_payload_effects_are_evaluated_without_the_deferred_body() {
        let mut registry = SymbolRegistry::new();
        registry.register_command("pause".into(), 1, vec![Type::Int], vec![]);
        registry.set_command_suspends("pause", true);
        registry.register_command(
            "runclientscript".into(),
            2,
            vec![Type::ClientScript],
            vec![],
        );
        registry.register_command(
            "set_hook".into(),
            3,
            vec![Type::ClientScript, Type::String],
            vec![],
        );
        registry.set_command_behavior("set_hook", true, Some(false), None);
        let mut project = Project::new(Target::Rune, registry, Options::default());
        let source = project.open("callbacks.rs2", Scope::Rune, 1,
            "[proc,send]\nrunclientscript(\"callback(~capture())\"); set_hook(\"callback(~capture())\");\n[proc,capture]()(int)\n%captured = 1; pause(1); return(1);\n[proc,install]\nset_hook(\"callback(1)\");\n[proc,broken]\nset_hook(\"callback(\");");
        project.open(
            "callback.cs2",
            Scope::Client,
            1,
            "[clientscript,callback](int $value)\n%deferred = 1;",
        );
        let effects = project.script_effects(source);
        assert!(effects[0].may_suspend);
        assert_eq!(effects[0].writes, ["captured"]);
        assert!(!effects[0].incomplete);
        assert!(effects[2].writes.is_empty());
        assert!(!effects[2].may_suspend);
        assert!(effects[3].incomplete);
        let mut many = effects[0].clone();
        many.writes = (0..8).map(|index| format!("variable{index}")).collect();
        let label = many.label().unwrap();
        assert!(label.contains("+2 more"));
        assert!(!label.contains("%variable6"));
        assert_eq!(many.writes.len(), 8);
    }
}
