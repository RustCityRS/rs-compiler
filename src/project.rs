//! Compiler service for caller-owned sources. This module performs no I/O.
use crate::bytecode::{CompiledScript, Opcode, Operand};
use crate::clientscript::{self, Source, format::Format};
use crate::compiler::{Compiler, compile_file};
use crate::diagnostics::{Diagnostic, DiagnosticsCollector, Phase, Severity};
use crate::ide::{Configs, Index};
use crate::lexer::Lexer;
use crate::parser::{Parser, ScriptFile};
use crate::pointer::PointerSet;
use crate::pointer_checker::{PointerChecker, ScriptSource};
use crate::sources::Scope;
use crate::symbol::SymbolRegistry;
use crate::text::LineIndex;
use crate::token::Kind;
use crate::typechecker::TypeChecker;
use crate::writer::encode_script;
use crate::{SourceProvider, lints, parallel_chunks, parallel_map_balanced};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(pub u64);

/// A source for `open_all`: label, scope, version and text.
pub type SourceText = (String, Scope, i64, String);

pub enum Target {
    Rune,
    Client(Format),
}

impl Target {
    fn includes(&self, scope: Scope) -> bool {
        scope == Scope::Global
            || matches!(
                (self, scope),
                (Self::Rune, Scope::Rune) | (Self::Client(_), Scope::Client)
            )
    }
}

#[derive(Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Default)]
pub struct Options {
    pub include_tests: bool,
    pub lint: bool,
    pub skip_shared_lints: bool,
    pub check_pointers: bool,
    pub cancellation: CancellationToken,
}

pub struct Document {
    pub id: SourceId,
    pub label: PathBuf,
    pub scope: Scope,
    pub version: i64,
    pub text: Arc<str>,
    pub lines: LineIndex,
    pub config: Option<String>,
    pub(crate) parsed: Option<Source>,
    pub(crate) tokens: Vec<TokenInfo>,
    pub(crate) spans: Vec<Range<usize>>,
    pub(crate) diagnostics: DiagnosticsCollector,
    test_line: Option<usize>,
}

#[derive(Clone, Copy)]
pub(crate) struct TokenInfo {
    pub kind: Kind,
    pub line: usize,
}

impl Document {
    pub fn syntax(&self) -> Option<&ScriptFile> {
        self.parsed.as_ref().map(|source| &source.file)
    }
}

pub struct Artifact {
    pub source: SourceId,
    pub id: i32,
    pub name: String,
    pub bytes: Vec<u8>,
}

/// New allocations are returned to the host; the compiler never edits ID files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdAssignment {
    pub trigger: String,
    pub name: String,
    pub id: i32,
}

pub struct Analysis {
    pub diagnostics: DiagnosticsCollector,
    pub assignments: Vec<IdAssignment>,
    pub cancelled: bool,
    pub checked_documents: usize,
}

pub struct Compilation {
    pub analysis: Analysis,
    pub artifacts: Vec<Artifact>,
}

type PointerRequirements = HashMap<SourceId, HashMap<String, PointerSet>>;

/// Retains parsed documents between edits and reuses the supplied config and
/// command registry. Source names are opaque: URI, database key, or filename.
pub struct Project {
    pub(crate) documents: BTreeMap<SourceId, Document>,
    pub(crate) registry: SymbolRegistry,
    /// Directory URIs owned by a filesystem-backed editor project. Other hosts
    /// accept arbitrary source names and leave this unrestricted.
    pub(crate) editor_roots: Option<Vec<String>>,
    target: Target,
    options: Options,
    next_source: u64,
    declarations: Vec<(Scope, String, String)>,
    allocated: BTreeMap<(String, String), i32>,
    pub(crate) generation: u64,
    analyzed_generation: Option<u64>,
    cached_diagnostics: DiagnosticsCollector,
    checked_full: Option<(u64, DiagnosticsCollector)>,
    pub(crate) pointer_requirements: Option<(u64, PointerRequirements)>,
    pub(crate) effects: Mutex<Option<Arc<crate::effects::Effects>>>,
    deep: Option<(HashMap<PathBuf, i64>, Vec<Diagnostic>)>,
    pub(crate) index: OnceLock<Index>,
    pub(crate) configs: Mutex<Option<Arc<Configs>>>,
    pub(crate) config_generation: u64,
    pub(crate) registry_generation: u64,
    checked: HashMap<SourceId, (i64, u64, DiagnosticsCollector)>,
}

impl Project {
    pub fn new(target: Target, registry: SymbolRegistry, options: Options) -> Self {
        Self {
            documents: BTreeMap::new(),
            registry,
            editor_roots: None,
            target,
            options,
            next_source: 0,
            declarations: Vec::new(),
            allocated: BTreeMap::new(),
            generation: 0,
            analyzed_generation: None,
            cached_diagnostics: DiagnosticsCollector::new(),
            checked_full: None,
            pointer_requirements: None,
            effects: Mutex::default(),
            deep: None,
            index: OnceLock::new(),
            configs: Mutex::default(),
            config_generation: 0,
            registry_generation: 0,
            checked: HashMap::new(),
        }
    }

    pub fn registry(&self) -> &SymbolRegistry {
        &self.registry
    }
    pub(crate) fn target_scope(&self) -> Scope {
        match self.target {
            Target::Rune => Scope::Rune,
            Target::Client(_) => Scope::Client,
        }
    }
    pub(crate) fn analysis_cancelled(&self) -> bool {
        self.options.cancellation.is_cancelled()
    }
    pub(crate) fn pointer_requirements_ready(&self) -> bool {
        self.pointer_requirements
            .as_ref()
            .is_some_and(|(generation, _)| *generation == self.generation)
    }
    pub fn set_cancellation(&mut self, token: CancellationToken) {
        self.options.cancellation = token;
    }
    pub fn documents(&self) -> impl Iterator<Item = &Document> {
        self.documents.values()
    }
    pub fn document(&self, id: SourceId) -> Option<&Document> {
        self.documents.get(&id)
    }

    /// Replace the environment after config or command definitions change.
    pub fn set_registry(&mut self, registry: SymbolRegistry) {
        self.allocated.retain(|(trigger, name), _| {
            !registry
                .preloaded_script_ids
                .contains_key(&format!("{trigger}:{name}"))
        });
        self.registry = registry;
        self.registry_generation += 1;
        self.index.take();
        self.generation += 1;
    }

    pub fn open(
        &mut self,
        label: impl Into<PathBuf>,
        scope: Scope,
        version: i64,
        text: impl Into<Arc<str>>,
    ) -> SourceId {
        let id = SourceId(self.next_source);
        self.next_source += 1;
        self.set_document(id, label.into(), scope, version, text.into());
        id
    }

    pub fn open_config(
        &mut self,
        label: impl Into<PathBuf>,
        scope: Scope,
        version: i64,
        text: impl Into<Arc<str>>,
        kind: impl Into<String>,
    ) -> SourceId {
        let id = SourceId(self.next_source);
        self.next_source += 1;
        self.set_config(id, label.into(), scope, version, text.into(), kind.into());
        id
    }

    fn set_config(
        &mut self,
        id: SourceId,
        label: PathBuf,
        scope: Scope,
        version: i64,
        text: Arc<str>,
        kind: String,
    ) {
        Arc::make_mut(&mut self.registry.visibility).bind(label.clone(), scope);
        self.documents.insert(
            id,
            Document {
                id,
                label,
                scope,
                version,
                lines: LineIndex::new(&text),
                text,
                config: Some(kind),
                parsed: None,
                tokens: Vec::new(),
                spans: Vec::new(),
                diagnostics: DiagnosticsCollector::new(),
                test_line: None,
            },
        );
        self.index.take();
        self.generation += 1;
        self.config_generation += 1;
    }

    /// Reject stale editor updates. Unchanged text reuses its existing syntax.
    pub fn update(
        &mut self,
        id: SourceId,
        version: i64,
        text: impl Into<Arc<str>>,
    ) -> Result<(), &'static str> {
        let old = self.documents.get_mut(&id).ok_or("Unknown source ID")?;
        if version <= old.version {
            return Err("Document version must increase");
        }
        let text = text.into();
        if old.text == text {
            old.version = version;
            return Ok(());
        }
        let (label, scope, config) = (old.label.clone(), old.scope, old.config.clone());
        if let Some(kind) = config {
            self.set_config(id, label, scope, version, text, kind);
        } else {
            self.set_document(id, label, scope, version, text);
        }
        Ok(())
    }

    pub fn close(&mut self, id: SourceId) -> bool {
        let Some(document) = self.documents.remove(&id) else {
            return false;
        };
        Arc::make_mut(&mut self.registry.visibility).unbind(&document.label);
        self.checked.remove(&id);
        self.index.take();
        self.generation += 1;
        self.config_generation += u64::from(document.config.is_some());
        true
    }

    /// Open many sources at once, lexing and parsing them in parallel. IDs are
    /// assigned in input order, as `open` would.
    pub fn open_all(&mut self, sources: Vec<SourceText>) -> Vec<SourceId> {
        let first = self.next_source;
        self.next_source += sources.len() as u64;
        let include_tests = self.options.include_tests;
        let sources = sources.into_iter().enumerate().collect::<Vec<_>>();
        let documents = parallel_map_balanced(&sources, |(i, (label, scope, version, text))| {
            parse_document(
                SourceId(first + *i as u64),
                label.into(),
                *scope,
                *version,
                text.as_str().into(),
                include_tests,
            )
        });
        documents
            .into_iter()
            .map(|document| {
                let id = document.id;
                self.insert_document(document);
                id
            })
            .collect()
    }

    fn set_document(
        &mut self,
        id: SourceId,
        label: PathBuf,
        scope: Scope,
        version: i64,
        text: Arc<str>,
    ) {
        let include_tests = self.options.include_tests;
        self.insert_document(parse_document(
            id,
            label,
            scope,
            version,
            text,
            include_tests,
        ));
    }

    fn insert_document(&mut self, document: Document) {
        Arc::make_mut(&mut self.registry.visibility).bind(document.label.clone(), document.scope);
        self.documents.insert(document.id, document);
        self.index.take();
        self.generation += 1;
    }

    pub fn analyze(&mut self) -> Analysis {
        let mut diagnostics = DiagnosticsCollector::new();
        if self.options.cancellation.is_cancelled() {
            return self.analysis(diagnostics, true);
        }
        if self.analyzed_generation == Some(self.generation) {
            return self.analysis(self.cached_diagnostics.clone(), false);
        }
        for (scope, trigger, name) in self.declarations.drain(..) {
            if scope == Scope::Client && matches!(trigger.as_str(), "proc" | "clientscript") {
                self.registry.client_script_params.remove(&name);
            }
            self.registry.remove_script(&trigger, &name);
            Arc::make_mut(&mut self.registry.visibility).undeclare(&trigger, &name, scope);
        }
        let mut used: HashMap<i32, String> = self
            .registry
            .preloaded_script_ids
            .iter()
            .map(|(name, &id)| (id, name.clone()))
            .collect();
        for ((trigger, name), &id) in &self.allocated {
            used.insert(id, format!("{trigger}:{name}"));
        }
        let mut next = used.keys().copied().max().unwrap_or(-1).checked_add(1);
        let mut declared = HashSet::new();
        for document in self.documents.values() {
            let visibility = Arc::make_mut(&mut self.registry.visibility);
            visibility.bind(document.label.clone(), document.scope);
            let Some(source) = &document.parsed else {
                continue;
            };
            for script in &source.file.scripts {
                if script.trigger == "command" {
                    continue;
                }
                visibility.declare(&script.trigger, &script.name, document.scope);
                self.declarations.push((
                    document.scope,
                    script.trigger.clone(),
                    script.name.clone(),
                ));
            }
        }
        clientscript::hooks::register_callbacks(
            &mut self.registry,
            self.documents
                .values()
                .filter(|document| document.scope == Scope::Client)
                .filter_map(|document| document.parsed.as_ref())
                .flat_map(|source| &source.file.scripts),
        );
        for document in self
            .documents
            .values()
            .filter(|doc| self.target.includes(doc.scope))
        {
            if self.options.cancellation.is_cancelled() {
                return self.analysis(diagnostics, true);
            }
            diagnostics.merge(document.diagnostics.clone());
            let Some(source) = &document.parsed else {
                continue;
            };
            for script in &source.file.scripts {
                if script.trigger == "command" {
                    continue;
                }
                if matches!(self.target, Target::Client(_)) {
                    for statement in &script.body {
                        clientscript::register_variables(statement, &mut self.registry);
                    }
                }
                let key = format!("{}:{}", script.trigger, script.name);
                if !declared.insert(key.clone()) {
                    diagnostics.error(
                        document.label.clone(),
                        script.line,
                        1,
                        format!("Duplicate script [{},{}]", script.trigger, script.name),
                        Phase::SymbolRegistration,
                    );
                    continue;
                }
                if matches!(self.target, Target::Client(_))
                    && clientscript::triggers::lookup(&script.trigger).is_none()
                {
                    diagnostics.error(
                        document.label.clone(),
                        script.line,
                        1,
                        format!("Invalid client trigger {}", script.trigger),
                        Phase::SymbolRegistration,
                    );
                    continue;
                }
                if let Some(trigger) = self.registry.trigger(&script.trigger) {
                    if !trigger.allows_returns && !script.return_types.is_empty() {
                        diagnostics.warning(
                            document.label.clone(),
                            script.line,
                            1,
                            format!("Trigger '{}' does not allow return values", script.trigger),
                            Phase::SymbolRegistration,
                        );
                    }
                } else if self.registry.profile.is_some() {
                    diagnostics.error(
                        document.label.clone(),
                        script.line,
                        1,
                        format!(
                            "Trigger '{}' is not defined by the supplied engine profile",
                            script.trigger
                        ),
                        Phase::SymbolRegistration,
                    );
                }
                if let Some(message) = Compiler::validate_trigger_subject(
                    &script.trigger,
                    &script.name,
                    &self.registry,
                ) {
                    diagnostics.warning(
                        document.label.clone(),
                        script.line,
                        1,
                        message,
                        Phase::SymbolRegistration,
                    );
                }
                let allocated_key = (script.trigger.clone(), script.name.clone());
                let supplied = self.registry.preloaded_script_ids.get(&key).copied();
                let id = supplied
                    .or_else(|| self.allocated.get(&allocated_key).copied())
                    .or(next);
                let Some(id) = id.filter(|id| *id >= 0) else {
                    diagnostics.error(
                        document.label.clone(),
                        script.line,
                        1,
                        "Script IDs must fit a nonnegative i32".into(),
                        Phase::SymbolRegistration,
                    );
                    continue;
                };
                if let Some(other) = used.get(&id).filter(|other| **other != key) {
                    diagnostics.error(
                        document.label.clone(),
                        script.line,
                        1,
                        format!("Script ID {id} is also assigned to {other}"),
                        Phase::SymbolRegistration,
                    );
                    continue;
                }
                used.insert(id, key);
                next = next.and_then(|next| {
                    if id >= next {
                        id.checked_add(1)
                    } else {
                        Some(next)
                    }
                });
                if supplied.is_none() {
                    self.allocated.insert(allocated_key, id);
                }
                self.registry
                    .set_script_id(&script.trigger, &script.name, id);
                self.registry.register_script(
                    script.name.clone(),
                    script.trigger.clone(),
                    script.params.iter().map(|p| p.param_type).collect(),
                    script.return_types.clone(),
                );
                if matches!(self.target, Target::Client(_)) {
                    self.registry
                        .set_stack_script(&script.trigger, &script.name);
                }
                if document.test_line.is_some_and(|line| script.line >= line) {
                    self.registry
                        .mark_test_script(&script.trigger, &script.name);
                }
            }
        }
        let mut interface = DefaultHasher::new();
        self.registry_generation.hash(&mut interface);
        self.registry.game_vars.len().hash(&mut interface);
        for document in self.documents.values() {
            document.id.hash(&mut interface);
            document.scope.hash(&mut interface);
            if let Some(source) = &document.parsed {
                for script in &source.file.scripts {
                    script.trigger.hash(&mut interface);
                    script.name.hash(&mut interface);
                    script.params.len().hash(&mut interface);
                    document
                        .test_line
                        .is_some_and(|line| script.line >= line)
                        .hash(&mut interface);
                    for parameter in &script.params {
                        parameter.param_type.hash(&mut interface);
                    }
                    script.return_types.hash(&mut interface);
                }
            }
        }
        let interface = interface.finish();
        let stale = self
            .documents
            .values()
            .filter(|doc| {
                self.target.includes(doc.scope)
                    && !self
                        .checked
                        .get(&doc.id)
                        .is_some_and(|(version, signature, _)| {
                            *version == doc.version && *signature == interface
                        })
            })
            .collect::<Vec<_>>();
        let cancellation = &self.options.cancellation;
        let fresh = parallel_map_balanced(&stale, |document| {
            if cancellation.is_cancelled() {
                return None;
            }
            let mut checker = TypeChecker::new(&self.registry);
            if let Some(source) = &document.parsed {
                checker.check_file_with_test_boundary(
                    &source.file,
                    &document.label,
                    document.test_line,
                );
            }
            Some(checker.diagnostics)
        });
        let checked_documents = stale.len();
        let fresh = stale
            .iter()
            .map(|document| (document.id, document.version))
            .zip(fresh)
            .collect::<Vec<_>>();
        if self.options.cancellation.is_cancelled() {
            return self.analysis(diagnostics, true);
        }
        for ((id, version), found) in fresh {
            self.checked
                .insert(id, (version, interface, found.unwrap_or_default()));
        }
        for document in self
            .documents
            .values()
            .filter(|doc| self.target.includes(doc.scope))
        {
            diagnostics.merge(self.checked[&document.id].2.clone());
        }
        self.cached_diagnostics = diagnostics.clone();
        self.analyzed_generation = Some(self.generation);
        let mut analysis = self.analysis(diagnostics, false);
        analysis.checked_documents = checked_documents;
        analysis
    }

    fn analysis(&self, diagnostics: DiagnosticsCollector, cancelled: bool) -> Analysis {
        Analysis {
            diagnostics,
            cancelled,
            checked_documents: 0,
            assignments: self
                .allocated
                .iter()
                .map(|((trigger, name), &id)| IdAssignment {
                    trigger: trigger.clone(),
                    name: name.clone(),
                    id,
                })
                .collect(),
        }
    }

    pub fn compile(&mut self) -> Compilation {
        let mut analysis = self.analyze();
        if analysis.cancelled || analysis.diagnostics.has_errors() {
            return Compilation {
                analysis,
                artifacts: Vec::new(),
            };
        }
        let mut scripts = Vec::<(SourceId, CompiledScript)>::new();
        for document in self
            .documents
            .values()
            .filter(|doc| self.target.includes(doc.scope))
        {
            if self.options.cancellation.is_cancelled() {
                analysis.cancelled = true;
                break;
            }
            let Some(source) = &document.parsed else {
                continue;
            };
            let compiled = compile_file(
                &source.file,
                &document.label,
                &self.registry,
                matches!(self.target, Target::Client(_)),
                &mut analysis.diagnostics,
            );
            scripts.extend(compiled.into_iter().map(|script| (document.id, script)));
        }

        analysis.cancelled |= self.options.cancellation.is_cancelled();
        if analysis.cancelled || analysis.diagnostics.has_errors() {
            return Compilation {
                analysis,
                artifacts: Vec::new(),
            };
        }

        let check_pointers =
            self.options.check_pointers || matches!(self.target, Target::Client(_));
        if check_pointers || self.options.lint {
            let sources: HashMap<_, _> = self
                .documents
                .values()
                .map(|doc| {
                    (
                        doc.label.to_string_lossy().into_owned(),
                        Arc::new(doc.text.to_string()),
                    )
                })
                .collect();
            let source = SourceProvider::Eager(&sources);
            let (ids, compiled): (Vec<_>, Vec<_>) = scripts.into_iter().unzip();
            if check_pointers {
                let mut checker = match self.target {
                    Target::Client(_) => PointerChecker::new_client(&compiled, &self.registry),
                    Target::Rune => PointerChecker::new(&compiled, &self.registry),
                };
                checker.set_source_cache(source);
                let names = compiled
                    .iter()
                    .map(|script| script.name.clone())
                    .collect::<Vec<_>>();
                analysis.diagnostics.merge(checker.validate_names(&names));
            }
            if self.options.lint {
                analysis
                    .diagnostics
                    .merge(lints::run_lints(&compiled, Some(source)));
            }
            scripts = ids.into_iter().zip(compiled).collect();
        }
        let mut artifacts = Vec::with_capacity(scripts.len());
        for (id, script) in scripts {
            if self.options.cancellation.is_cancelled() {
                analysis.cancelled = true;
                break;
            }
            let document = &self.documents[&id];
            let bytes = match &self.target {
                Target::Rune => Ok(encode_script(&script)),
                Target::Client(format) => clientscript::encode(&script, format),
            };
            match bytes {
                Ok(bytes) => artifacts.push(Artifact {
                    source: id,
                    id: script.id,
                    name: script.name,
                    bytes,
                }),
                Err(error) => analysis.diagnostics.error(
                    document.label.clone(),
                    1,
                    1,
                    error.to_string(),
                    Phase::CodeGeneration,
                ),
            }
        }
        analysis.cancelled |= self.options.cancellation.is_cancelled();
        if analysis.cancelled || analysis.diagnostics.has_errors() {
            artifacts.clear();
        }
        artifacts.sort_by_key(|artifact| artifact.id);
        Compilation {
            analysis,
            artifacts,
        }
    }

    /// The fullest diagnostics on hand without more work: `analyze`, with what
    /// the last `check` added for each document that has not changed since.
    pub fn latest(&mut self) -> Analysis {
        let mut analysis = self.analyze();
        if analysis.cancelled {
            return analysis;
        }
        if let Some((generation, diagnostics)) = &self.checked_full
            && *generation == self.generation
        {
            analysis.diagnostics = diagnostics.clone();
        } else if let Some((versions, added)) = &self.deep {
            let current = self
                .documents
                .values()
                .map(|document| (document.label.as_path(), document.version))
                .collect::<HashMap<_, _>>();
            for diagnostic in added {
                if versions.get(&diagnostic.file) == current.get(diagnostic.file.as_path()) {
                    analysis.diagnostics.add(diagnostic.clone());
                }
            }
        }
        analysis
    }

    pub fn check(&mut self) -> Analysis {
        let mut analysis = self.analyze();
        if analysis.cancelled {
            return analysis;
        }
        if let Some((generation, diagnostics)) = &self.checked_full
            && *generation == self.generation
        {
            analysis.diagnostics = diagnostics.clone();
            return analysis;
        }
        let analyzed = analysis.diagnostics.diagnostics().len();
        let failed = analysis
            .diagnostics
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .map(|diagnostic| diagnostic.file.as_path())
            .collect::<HashSet<_>>();
        let documents = self
            .documents
            .values()
            .filter(|doc| {
                self.target.includes(doc.scope)
                    && doc.parsed.is_some()
                    && !failed.contains(doc.label.as_path())
            })
            .collect::<Vec<_>>();
        let client = matches!(self.target, Target::Client(_));
        let cancellation = &self.options.cancellation;
        let files = parallel_map_balanced(&documents, |document| {
            let mut diagnostics = DiagnosticsCollector::new();
            if cancellation.is_cancelled() {
                return (Vec::new(), diagnostics);
            }
            let source = document.parsed.as_ref().unwrap();
            let compiled = compile_file(
                &source.file,
                &document.label,
                &self.registry,
                client,
                &mut diagnostics,
            );
            (compiled, diagnostics)
        });
        if cancellation.is_cancelled() {
            analysis.cancelled = true;
            return analysis;
        }
        let mut compiled = Vec::new();
        for (scripts, diagnostics) in files {
            compiled.extend(scripts);
            analysis.diagnostics.merge(diagnostics);
        }
        let sources = self
            .documents
            .values()
            .filter(|doc| doc.config.is_none())
            .map(|doc| {
                (
                    doc.label.to_string_lossy().into_owned(),
                    Arc::new(doc.text.to_string()),
                )
            })
            .collect::<HashMap<_, _>>();
        let source = SourceProvider::Eager(&sources);
        let lint_diagnostics = lints::run_lints(&compiled, Some(source));
        if self.options.skip_shared_lints {
            let shared: HashSet<_> = self
                .documents
                .values()
                .filter(|document| document.scope == Scope::Global)
                .map(|document| &document.label)
                .collect();
            for diagnostic in lint_diagnostics.diagnostics() {
                if !shared.contains(&diagnostic.file) {
                    analysis.diagnostics.add(diagnostic.clone());
                }
            }
        } else {
            analysis.diagnostics.merge(lint_diagnostics);
        }
        let shared = PointerChecker::build_shared(&compiled, &self.registry, client);
        let names = compiled
            .iter()
            .map(|script| script.name.clone())
            .collect::<Vec<_>>();
        let mut pointer_requirements = HashMap::new();
        for (diagnostics, requirements) in parallel_chunks(&names, |chunk| {
            let mut checker = PointerChecker::with_shared(
                ScriptSource::Resident(&compiled),
                &self.registry,
                shared.clone(),
            );
            checker.set_source_cache(source);
            checker.validate_names_with_requirements_until(chunk, &|| cancellation.is_cancelled())
        }) {
            analysis.diagnostics.merge(diagnostics);
            pointer_requirements.extend(requirements);
        }
        analysis.cancelled = self.options.cancellation.is_cancelled();
        if !analysis.cancelled {
            let Some(incomplete) = incomplete_pointer_scripts(&compiled, cancellation) else {
                analysis.cancelled = true;
                return analysis;
            };
            let source_ids = documents
                .iter()
                .map(|document| (document.label.to_string_lossy(), document.id))
                .collect::<HashMap<_, _>>();
            let mut requirements_by_source: HashMap<_, HashMap<_, _>> = HashMap::new();
            for script in &compiled {
                if !incomplete.contains(&script.id)
                    && let Some(&source) = source_ids.get(script.source_path.as_str())
                    && let Some(&required) = pointer_requirements.get(&script.name)
                {
                    requirements_by_source
                        .entry(source)
                        .or_default()
                        .insert(script.name.clone(), required);
                }
            }
            if cancellation.is_cancelled() {
                analysis.cancelled = true;
                return analysis;
            }
            self.checked_full = Some((self.generation, analysis.diagnostics.clone()));
            self.pointer_requirements = Some((self.generation, requirements_by_source));
            self.deep = Some((
                self.documents
                    .values()
                    .map(|document| (document.label.clone(), document.version))
                    .collect(),
                analysis.diagnostics.diagnostics()[analyzed..].to_vec(),
            ));
        }
        analysis
    }

    pub fn source_text(&self, label: &Path) -> Option<&str> {
        self.documents
            .values()
            .find(|doc| doc.label == label)
            .map(|doc| doc.text.as_ref())
    }
}

fn incomplete_pointer_scripts(
    compiled: &[CompiledScript],
    cancellation: &CancellationToken,
) -> Option<HashSet<i32>> {
    let available = compiled
        .iter()
        .map(|script| script.id)
        .collect::<HashSet<_>>();
    let mut callers: HashMap<i32, Vec<i32>> = HashMap::new();
    let mut incomplete = HashSet::new();
    let mut pending = Vec::new();
    for script in compiled {
        if cancellation.is_cancelled() {
            return None;
        }
        for instruction in &script.instructions {
            if matches!(
                instruction.opcode,
                Opcode::Gosub | Opcode::GosubWithParams | Opcode::Jump | Opcode::JumpWithParams
            ) && let Operand::Int(callee) = instruction.operand
            {
                callers.entry(callee).or_default().push(script.id);
                if !available.contains(&callee) && incomplete.insert(callee) {
                    pending.push(callee);
                }
            }
        }
    }
    while let Some(callee) = pending.pop() {
        if cancellation.is_cancelled() {
            return None;
        }
        for &caller in callers.get(&callee).into_iter().flatten() {
            if incomplete.insert(caller) {
                pending.push(caller);
            }
        }
    }
    Some(incomplete)
}

fn parse_document(
    id: SourceId,
    label: PathBuf,
    scope: Scope,
    version: i64,
    text: Arc<str>,
    include_tests: bool,
) -> Document {
    let test_line = crate::find_testscript_line(&text);
    let end = if !include_tests {
        test_line
            .and_then(|line| LineIndex::new(&text).line(line - 1))
            .map_or(text.len(), |range| range.start)
    } else {
        text.len()
    };
    let (tokens, spans, lex_errors) = Lexer::new(&text[..end], &label).tokenize_spanned();
    let mut diagnostics = DiagnosticsCollector::new();
    for error in lex_errors {
        diagnostics.error(
            label.clone(),
            error.line,
            error.position,
            error.message,
            Phase::Lexing,
        );
    }
    let token_info = tokens
        .iter()
        .map(|token| TokenInfo {
            kind: token.kind,
            line: token.line,
        })
        .collect();
    let (file, errors) = Parser::new(tokens, &label).parse_recovering();
    for error in errors {
        diagnostics.error(
            label.clone(),
            error.line,
            error.position,
            error.message,
            Phase::Parsing,
        );
    }
    let parsed = Some(Source {
        path: label.clone(),
        file,
    });
    Document {
        id,
        label,
        scope,
        version,
        lines: LineIndex::new(&text),
        config: None,
        text,
        parsed,
        tokens: token_info,
        spans,
        diagnostics,
        test_line: test_line.filter(|_| include_tests),
    }
}
