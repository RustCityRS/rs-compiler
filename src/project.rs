//! Compiler service for caller-owned sources. This module performs no I/O.
use crate::bytecode::CompiledScript;
use crate::clientscript::{self, Source, format::Format};
use crate::compiler::{Compiler, compile_file};
use crate::diagnostics::{DiagnosticsCollector, Phase, Severity};
use crate::ide::Index;
use crate::lexer::Lexer;
use crate::parser::{Parser, ScriptFile};
use crate::pointer_checker::PointerChecker;
use crate::sources::Scope;
use crate::symbol::SymbolRegistry;
use crate::text::LineIndex;
use crate::token::Kind;
use crate::typechecker::TypeChecker;
use crate::writer::encode_script;
use crate::{SourceProvider, lints};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(pub u64);

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

/// Retains parsed documents between edits and reuses the supplied config and
/// command registry. Source names are opaque: URI, database key, or filename.
pub struct Project {
    pub(crate) documents: BTreeMap<SourceId, Document>,
    pub(crate) registry: SymbolRegistry,
    target: Target,
    options: Options,
    next_source: u64,
    declarations: Vec<(Scope, String, String)>,
    allocated: BTreeMap<(String, String), i32>,
    pub(crate) generation: u64,
    analyzed_generation: Option<u64>,
    cached_diagnostics: DiagnosticsCollector,
    checked_full: Option<(u64, DiagnosticsCollector)>,
    pub(crate) index: OnceLock<Index>,
    registry_generation: u64,
    checked: HashMap<SourceId, (i64, u64, DiagnosticsCollector)>,
}

impl Project {
    pub fn new(target: Target, registry: SymbolRegistry, options: Options) -> Self {
        Self {
            documents: BTreeMap::new(),
            registry,
            target,
            options,
            next_source: 0,
            declarations: Vec::new(),
            allocated: BTreeMap::new(),
            generation: 0,
            analyzed_generation: None,
            cached_diagnostics: DiagnosticsCollector::new(),
            checked_full: None,
            index: OnceLock::new(),
            registry_generation: 0,
            checked: HashMap::new(),
        }
    }

    pub fn registry(&self) -> &SymbolRegistry {
        &self.registry
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
        true
    }

    fn set_document(
        &mut self,
        id: SourceId,
        label: PathBuf,
        scope: Scope,
        version: i64,
        text: Arc<str>,
    ) {
        Arc::make_mut(&mut self.registry.visibility).bind(label.clone(), scope);
        let test_line = crate::find_testscript_line(&text);
        let end = if !self.options.include_tests {
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
        self.documents.insert(
            id,
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
                test_line: test_line.filter(|_| self.options.include_tests),
            },
        );
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
                    && !matches!(script.trigger.as_str(), "proc" | "clientscript")
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
        let mut checked_documents = 0;
        for document in self
            .documents
            .values()
            .filter(|doc| self.target.includes(doc.scope))
        {
            if self.options.cancellation.is_cancelled() {
                return self.analysis(diagnostics, true);
            }
            if let Some((version, signature, cached)) = self.checked.get(&document.id)
                && *version == document.version
                && *signature == interface
            {
                diagnostics.merge(cached.clone());
                continue;
            }
            let mut checker = TypeChecker::new(&self.registry);
            if let Some(source) = &document.parsed {
                checker.check_file_with_test_boundary(
                    &source.file,
                    &document.label,
                    document.test_line,
                );
            }
            checked_documents += 1;
            self.checked.insert(
                document.id,
                (document.version, interface, checker.diagnostics.clone()),
            );
            diagnostics.merge(checker.diagnostics);
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
        let mut compiled = Vec::new();
        for document in self
            .documents
            .values()
            .filter(|doc| self.target.includes(doc.scope))
        {
            if self.options.cancellation.is_cancelled() {
                analysis.cancelled = true;
                return analysis;
            }
            if analysis.diagnostics.diagnostics().iter().any(|diagnostic| {
                diagnostic.file == document.label && diagnostic.severity == Severity::Error
            }) {
                continue;
            }
            let Some(source) = &document.parsed else {
                continue;
            };
            compiled.extend(compile_file(
                &source.file,
                &document.label,
                &self.registry,
                matches!(self.target, Target::Client(_)),
                &mut analysis.diagnostics,
            ));
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
        {
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
        analysis.cancelled = self.options.cancellation.is_cancelled();
        if !analysis.cancelled {
            self.checked_full = Some((self.generation, analysis.diagnostics.clone()));
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
