pub mod bytecode;
pub mod compiler;
pub mod diagnostic_messages;
pub mod diagnostics;
pub mod error;
pub mod lexer;
pub mod lints;
pub mod parser;
pub mod pointer;
pub mod pointer_checker;
pub mod symbol;
pub mod symloader;
pub mod token;
pub mod trigger_table;
pub mod typechecker;
pub mod types;
pub mod writer;

use crate::bytecode::CompiledScript;
use crate::compiler::Compiler;
use crate::diagnostics::DiagnosticsCollector;
use crate::lexer::Lexer;
use crate::parser::{Parser, ScriptFile};
use crate::symbol::SymbolRegistry;
use crate::typechecker::TypeChecker;
use crate::writer::ScriptWriter;
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::info;

struct ParsedFile {
    path: PathBuf,
    file: ScriptFile,
    testscript_line: Option<usize>,
    /// Canonicalized, normalized source path. Computed once during parsing and
    /// reused as each compiled script's `source_path` (it is byte-identical to
    /// the `source_cache` key), avoiding a second `canonicalize` syscall per
    /// file in codegen.
    source_path: String,
}

/// Map `f` over `items` across worker threads, returning results in the SAME
/// order as the input. Dependency-free (uses `std::thread::scope`); each item
/// is processed independently, so callers must keep the work pure (no shared
/// mutable state). Falls back to a serial map for tiny inputs or single-core.
pub(crate) fn parallel_map<T, R, F>(items: &[T], f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    if threads <= 1 || items.len() <= 1 {
        return items.iter().map(&f).collect();
    }
    let threads = threads.min(items.len());
    let chunk_size = items.len().div_ceil(threads);
    let mut out: Vec<R> = Vec::with_capacity(items.len());
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk_size)
            .map(|chunk| {
                let f = &f;
                scope.spawn(move || chunk.iter().map(f).collect::<Vec<R>>())
            })
            .collect();
        // Joining in spawn order concatenates chunks in input order, so the
        // returned Vec matches a serial map element-for-element.
        for h in handles {
            out.extend(h.join().expect("worker thread panicked"));
        }
    });
    out
}

/// Like `parallel_map`, but hands each worker a whole contiguous CHUNK of items
/// so per-chunk setup (e.g. a checker's caches) can be reused across the chunk.
/// Returns one result per chunk, in input order.
fn parallel_chunks<T, R, F>(items: &[T], f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&[T]) -> R + Sync,
{
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    if threads <= 1 || items.len() <= 1 {
        return vec![f(items)];
    }
    let threads = threads.min(items.len());
    let chunk_size = items.len().div_ceil(threads);
    let mut out: Vec<R> = Vec::new();
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk_size)
            .map(|chunk| {
                let f = &f;
                scope.spawn(move || f(chunk))
            })
            .collect();
        for h in handles {
            out.push(h.join().expect("worker thread panicked"));
        }
    });
    out
}

/// Outcome of parsing one file off-thread. The owned source text and cache key
/// travel back so the (single-threaded) merge can build the `Rc`-based caches
/// without sending `Rc` across threads.
struct FileParseResult {
    path: PathBuf,
    read: Result<ParsedSource, std::io::Error>,
}

struct ParsedSource {
    cache_key: String,
    raw_source: String,
    outcome: ParseOutcome,
}

enum ParseOutcome {
    Parsed {
        file: ScriptFile,
        testscript_line: Option<usize>,
    },
    Error {
        line: usize,
        position: usize,
        message: String,
        phase: diagnostics::Phase,
    },
}

/// Read, lex and parse a single file off-thread. Pure: no shared state, so it
/// is safe to run concurrently. Mirrors the original sequential body exactly —
/// same source truncation, same cache-key normalization, same lex-then-parse
/// error reporting — only restructured to return owned data for the merge.
fn parse_one_file(path: &Path, include_tests: bool) -> FileParseResult {
    let raw_source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            return FileParseResult {
                path: path.to_path_buf(),
                read: Err(e),
            };
        }
    };
    let testscript_line = find_testscript_line(&raw_source);

    let source_code = if !include_tests {
        if let Some(line_no) = testscript_line {
            truncate_at_line(&raw_source, line_no)
        } else {
            raw_source.clone()
        }
    } else {
        strip_testscript_annotation(&raw_source)
    };

    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut cache_key = canonical.to_string_lossy().into_owned();
    if cache_key.starts_with("\\\\?\\") {
        cache_key = cache_key[4..].to_string();
    }
    cache_key = cache_key.replace("\\Content\\", "\\content\\");

    // Lexer/Parser borrow a `&PathBuf` for diagnostics; keep one alive locally.
    let path_buf = path.to_path_buf();
    let outcome = match Lexer::new(&source_code, &path_buf).tokenize() {
        Err(e) => ParseOutcome::Error {
            line: e.line,
            position: e.position,
            message: e.message,
            phase: diagnostics::Phase::Lexing,
        },
        Ok(tokens) => match Parser::new(tokens, &path_buf).parse() {
            Ok(file) => ParseOutcome::Parsed {
                file,
                testscript_line: if include_tests { testscript_line } else { None },
            },
            Err(e) => ParseOutcome::Error {
                line: e.line,
                position: e.position,
                message: e.message,
                phase: diagnostics::Phase::Parsing,
            },
        },
    };

    FileParseResult {
        path: path_buf,
        read: Ok(ParsedSource {
            cache_key,
            raw_source,
            outcome,
        }),
    }
}

/// Compile scripts and write output. When `lint` is true, also run lint passes
/// (unused locals, unreachable code) after code generation.
pub fn compile_memory(
    scripts_dir: &Path,
    pack_dir: Option<&Path>,
    lint: bool,
) -> Result<(Vec<u8>, Vec<u8>), Box<dyn Error>> {
    compile_memory_with_tests(scripts_dir, pack_dir, lint, false)
}

/// Like `compile_memory` but includes `#testscript` sections when
/// `include_tests` is true.
pub fn compile_memory_with_tests(
    scripts_dir: &Path,
    pack_dir: Option<&Path>,
    lint: bool,
    include_tests: bool,
) -> Result<(Vec<u8>, Vec<u8>), Box<dyn Error>> {
    let (compiled_scripts, diagnostics) = run_pipeline(scripts_dir, pack_dir, lint, include_tests)?;

    info!("Writing compiled output to memory...");
    let writer = ScriptWriter::new("".into());

    diagnostics.print_all();
    info!("Compilation complete!");

    Ok(writer.build_all(&compiled_scripts)?)
}

/// Compile scripts and write output. When `lint` is true, also run lint passes
/// (unused locals, unreachable code) after code generation.
pub fn compile(
    scripts_dir: &Path,
    pack_dir: Option<&Path>,
    output_dir: &Path,
    lint: bool,
) -> Result<(), Box<dyn Error>> {
    let (compiled_scripts, diagnostics) = run_pipeline(scripts_dir, pack_dir, lint, false)?;

    let output = output_dir.display().to_string();
    // Normalize separators for a clean, consistent log message (input paths may
    // mix `/` and `\`, e.g. `data/pack\server`); the writer keeps the original.
    info!("Writing output to {}...", output.replace('\\', "/"));
    let writer = ScriptWriter::new(output);
    writer.write_all(&compiled_scripts)?;

    diagnostics.print_all();
    info!("Compilation complete!");

    Ok(())
}

/// Run all analysis passes (parse, type-check, codegen, pointer-check, lints)
/// without writing output. Useful for editor tooling and CI lint gates.
pub fn lint(scripts_dir: &Path, pack_dir: Option<&Path>) -> Result<(), Box<dyn Error>> {
    let (_, diagnostics) = run_pipeline(scripts_dir, pack_dir, true, false)?;

    diagnostics.print_all();
    info!("Compilation complete!");

    Ok(())
}

/// Runs the full compilation pipeline (parse, register, type-check, codegen,
/// pointer-check, and optional lint passes), returning the compiled scripts
/// and accumulated diagnostics for the caller to handle output.
fn run_pipeline(
    scripts_dir: &Path,
    pack_dir: Option<&Path>,
    lint: bool,
    include_tests: bool,
) -> Result<(Vec<CompiledScript>, DiagnosticsCollector), Box<dyn Error>> {
    if !scripts_dir.exists() || !scripts_dir.is_dir() {
        return Err(Box::new(error::CompilerError::FileNotFound(
            scripts_dir.display().to_string(),
        )));
    }

    let mut rs2_files: Vec<PathBuf> = get_rs2_files(scripts_dir, "rs2")?;

    rs2_files.sort();
    info!("Found {} script files", rs2_files.len());

    let mut diagnostics = DiagnosticsCollector::new();

    // Phase 1: Parse all files. Reading, lexing and parsing are independent
    // per file, so fan them out across threads; the results are merged back in
    // input order so registration/ID assignment stays deterministic.
    info!("Parsing script files into syntax trees...");
    let parse_results = parallel_map(&rs2_files, |path| parse_one_file(path, include_tests));

    let mut all_files: Vec<ParsedFile> = Vec::new();
    let mut source_cache: HashMap<String, Arc<String>> = HashMap::new();
    // Raw source text keyed by on-disk path, reused by `generate_script_pack`
    // so it doesn't re-read every file from disk a second time.
    let mut raw_by_path: HashMap<PathBuf, Arc<String>> = HashMap::new();
    for result in parse_results {
        let parsed = match result.read {
            Ok(p) => p,
            // A read failure aborts the whole compile, exactly as the previous
            // `fs::read_to_string(path)?` did.
            Err(e) => return Err(Box::new(e)),
        };
        let rc = Arc::new(parsed.raw_source);
        // The cache key is the canonicalized source path; reuse it as each
        // compiled script's `source_path` so codegen needn't canonicalize again.
        let source_path = parsed.cache_key.clone();
        source_cache.insert(parsed.cache_key, rc.clone());
        raw_by_path.insert(result.path.clone(), rc);
        match parsed.outcome {
            ParseOutcome::Parsed {
                file,
                testscript_line,
            } => all_files.push(ParsedFile {
                path: result.path,
                file,
                testscript_line,
                source_path,
            }),
            ParseOutcome::Error {
                line,
                position,
                message,
                phase,
            } => diagnostics.error(result.path, line, position, message, phase),
        }
    }

    if diagnostics.has_errors() {
        diagnostics.print_all();
        return Err(Box::new(error::CompilerError::TypeError(
            "Parsing failed".to_string(),
        )));
    }

    // Phase 2: Register all scripts. Build the pack/config registry, then
    // regenerate script.pack, load pre-assigned IDs, and register every script.
    info!("Registering scripts and building the symbol registry...");
    let resolved_pack_dir = resolve_pack_dir(scripts_dir, pack_dir);
    let mut registry = build_symbol_registry(scripts_dir, resolved_pack_dir.as_deref());
    register_scripts(
        &mut registry,
        scripts_dir,
        resolved_pack_dir.as_deref(),
        &mut diagnostics,
        &all_files,
        &raw_by_path,
    );
    info!("  Registered {} scripts", registry.scripts.len());

    for pf in &all_files {
        for script in &pf.file.scripts {
            if let Some(msg) =
                Compiler::validate_trigger_subject(&script.trigger, &script.name, &registry)
            {
                diagnostics.warning(
                    pf.path.clone(),
                    script.line,
                    0,
                    msg,
                    diagnostics::Phase::SymbolRegistration,
                );
            }
        }
    }

    // Phase 3: Type checking
    info!("Type checking {} files...", all_files.len());
    run_type_checker(&mut diagnostics, &all_files, &registry);

    if diagnostics.has_errors() {
        diagnostics.print_all();
        return Err(Box::new(error::CompilerError::TypeError(
            "Type checking failed".to_string(),
        )));
    }

    // Phase 4: Code generation
    info!("Generating bytecode from type-checked scripts...");
    let mut compiled_scripts = codegen(&all_files, &registry);
    info!("  Generated {} compiled scripts", compiled_scripts.len());
    compiled_scripts.sort_by_key(|s| s.id);

    // Phase 4.5: Pointer checking & lints
    info!("Checking pointers in generated code...");
    check_pointers(
        lint,
        &mut diagnostics,
        &mut source_cache,
        &registry,
        &compiled_scripts,
    );

    Ok((compiled_scripts, diagnostics))
}

fn check_pointers(
    lint: bool,
    diagnostics: &mut DiagnosticsCollector,
    source_cache: &mut HashMap<String, Arc<String>>,
    registry: &SymbolRegistry,
    compiled_scripts: &[CompiledScript],
) {
    {
        use crate::pointer_checker::PointerChecker;
        // Validate scripts across threads: each chunk gets its own checker that
        // shares the read-only scripts/registry/source cache but keeps its own
        // (purely memoizing) caches. Diagnostics are merged in script order, so
        // the result is identical to a single-threaded run.
        let names = PointerChecker::new(compiled_scripts, registry).script_names();
        let cache: &HashMap<String, Arc<String>> = source_cache;
        let chunk_diags = parallel_chunks(&names, |chunk| {
            let mut checker = PointerChecker::new(compiled_scripts, registry);
            checker.set_source_cache(cache);
            checker.validate_names(chunk)
        });
        let mut pointer_diags = DiagnosticsCollector::new();
        for d in chunk_diags {
            pointer_diags.merge(d);
        }
        let ptr_warnings = pointer_diags.warning_count();
        if ptr_warnings > 0 {
            info!("  Pointer check: {} warning(s)", ptr_warnings);
        }
        diagnostics.merge(pointer_diags);
    }

    // Phase 4.6: Lint passes (optional)
    if lint {
        info!("Running lint passes (unused locals, unreachable code)...");
        let lint_diags = lints::run_lints(compiled_scripts, Some(source_cache));
        let lint_warnings = lint_diags.warning_count();
        if lint_warnings > 0 {
            info!("  Lints: {} warning(s)", lint_warnings);
        }
        diagnostics.merge(lint_diags);
    }
}

fn codegen(all_files: &[ParsedFile], registry: &SymbolRegistry) -> Vec<CompiledScript> {
    // Each file compiles independently against the read-only registry, so fan
    // the files out across threads. Output order doesn't matter — the caller
    // sorts compiled scripts by id — so a simple flatten of the per-file results
    // is enough.
    parallel_map(all_files, |pf| compile_file(pf, registry))
        .into_iter()
        .flatten()
        .collect()
}

/// Compile every (non-`command`) script in one file. Creating a `Compiler` per
/// file is cheap now that it only borrows the registry, and keeps the codegen
/// scratch state thread-local.
fn compile_file(pf: &ParsedFile, registry: &SymbolRegistry) -> Vec<CompiledScript> {
    let mut compiler = Compiler::new(registry);

    let mut compiled_scripts = Vec::new();
    for script in &pf.file.scripts {
        if script.trigger == "command" {
            continue;
        }
        let mut compiled = compiler.compile_script(script);
        // `source_path` was canonicalized once during parsing; reuse it.
        compiled.source_path = pf.source_path.clone();
        compiled_scripts.push(compiled);
    }
    compiled_scripts
}

fn run_type_checker(
    diagnostics: &mut DiagnosticsCollector,
    all_files: &[ParsedFile],
    registry: &SymbolRegistry,
) {
    // Each file type-checks independently against the read-only registry, so
    // check chunks of files in parallel; per-chunk diagnostics are merged in
    // file order, matching a single-threaded run.
    let chunk_diags = parallel_chunks(all_files, |chunk| {
        let mut type_checker = TypeChecker::new(registry);
        for pf in chunk {
            type_checker.check_file_with_test_boundary(&pf.file, &pf.path, pf.testscript_line);
        }
        type_checker.diagnostics
    });
    for d in chunk_diags {
        diagnostics.merge(d);
    }
}

/// Resolve the pack directory: explicit `--pack`, else a sibling `pack/`, else
/// `content/pack` up the tree.
fn resolve_pack_dir(scripts_dir: &Path, pack_dir: Option<&Path>) -> Option<PathBuf> {
    pack_dir
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .or_else(|| {
            scripts_dir
                .parent()
                .map(|p| p.join("pack"))
                .filter(|p| p.exists())
        })
        .or_else(|| {
            scripts_dir
                .ancestors()
                .nth(3)
                .map(|p| p.join("content/pack"))
                .filter(|p| p.exists())
        })
}

/// Build the part of the registry that does NOT depend on the parsed scripts:
/// pack files, `.constant`/`.var`/`.dbtable` configs, and engine command params.
/// This runs concurrently with parsing; `script.pack` (re)generation and script
/// registration — which need the parsed sources/ASTs — happen afterwards in
/// `register_scripts`.
fn build_symbol_registry(scripts_dir: &Path, resolved_pack_dir: Option<&Path>) -> SymbolRegistry {
    let mut registry = SymbolRegistry::new();

    if let Some(pd) = resolved_pack_dir {
        info!("  Loading packs from {}", pd.display());
        symloader::load_packs(&mut registry, pd);
        info!(
            "  Loaded {} commands, {} game vars",
            registry.commands.len(),
            registry.game_vars.len(),
        );
    } else {
        info!("  No pack directory found - symbol resolution will be limited");
    }

    // Walk the scripts tree once and let the constant/game-var/dbtable loaders
    // filter this shared list by extension, instead of each re-walking the tree
    // (previously 6 separate recursive traversals over the same directories).
    let mut config_files: Vec<PathBuf> = Vec::new();
    symloader::collect_all_files(scripts_dir, &mut config_files);

    symloader::load_constant_files(&mut registry, &config_files);

    info!(
        "  Loaded {} constants from .constant files",
        registry.constants.len()
    );

    symloader::load_game_var_types(&mut registry, &config_files);

    if let Some(pd) = resolved_pack_dir {
        let dbtable_ids = symloader::load_dbtable_pack(&pd.join("dbtable.pack"));
        if !dbtable_ids.is_empty() {
            symloader::load_dbtable_configs(&mut registry, &config_files, &dbtable_ids);
            info!(
                "  Registered {} dbcolumn compound IDs",
                registry.dbcolumn_types.len()
            );
        }
    }

    let engine_rs2 = scripts_dir.join("engine.rs2");

    if engine_rs2.exists() {
        symloader::load_engine_command_params(&mut registry, &engine_rs2);
    } else {
        if let Ok(entries) = std::fs::read_dir(scripts_dir) {
            for entry in entries.flatten() {
                let candidate = entry.path().join("engine.rs2");
                if candidate.exists() {
                    crate::symloader::load_engine_command_params(&mut registry, &candidate);
                    break;
                }
            }
        }
    }

    symloader::patch_command_return_types(&mut registry);

    registry
}

/// Finalize the registry once parsing is done: regenerate `script.pack` from the
/// parsed sources, load pre-assigned script IDs, then register every parsed
/// script (emitting trigger/redeclaration/return diagnostics). Builds on the
/// registry produced concurrently by `build_symbol_registry`.
fn register_scripts(
    registry: &mut SymbolRegistry,
    scripts_dir: &Path,
    resolved_pack_dir: Option<&Path>,
    diagnostics: &mut DiagnosticsCollector,
    all_files: &[ParsedFile],
    raw_by_path: &HashMap<PathBuf, Arc<String>>,
) {
    if let Some(pd) = resolved_pack_dir {
        symloader::generate_script_pack(scripts_dir, pd, raw_by_path);
        symloader::load_script_ids(registry, pd);
    }

    {
        use crate::diagnostic_messages as msg;

        let mut registered_scripts: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for pf in all_files.iter() {
            for script in &pf.file.scripts {
                if script.trigger == "command" {
                    continue;
                }

                if !trigger_table::is_valid_trigger(&script.trigger) {
                    diagnostics.warning(
                        pf.path.clone(),
                        script.line,
                        0,
                        msg::fmt(msg::SCRIPT_TRIGGER_INVALID, &[&script.trigger]),
                        crate::diagnostics::Phase::SymbolRegistration,
                    );
                }

                let key = format!("{}:{}", script.trigger, script.name);
                if !registered_scripts.insert(key.clone()) {
                    diagnostics.warning(
                        pf.path.clone(),
                        script.line,
                        0,
                        msg::fmt(msg::SCRIPT_REDECLARATION, &[&script.trigger, &script.name]),
                        crate::diagnostics::Phase::SymbolRegistration,
                    );
                }

                if !script.return_types.is_empty()
                    && !trigger_table::allows_returns(&script.trigger)
                {
                    diagnostics.warning(
                        pf.path.clone(),
                        script.line,
                        0,
                        msg::fmt(msg::SCRIPT_TRIGGER_NO_RETURNS, &[&script.trigger]),
                        diagnostics::Phase::SymbolRegistration,
                    );
                }

                let param_types: Vec<types::Type> =
                    script.params.iter().map(|p| p.param_type).collect();
                registry.register_script(
                    script.name.clone(),
                    script.trigger.clone(),
                    param_types,
                    script.return_types.clone(),
                );

                if let Some(ts_line) = pf.testscript_line
                    && script.line >= ts_line
                {
                    registry.mark_test_script(&script.trigger, &script.name);
                }
            }
        }
    }
}

fn find_testscript_line(source: &str) -> Option<usize> {
    for (i, line) in source.lines().enumerate() {
        if line.trim() == "#testscript" {
            return Some(i + 1);
        }
    }
    None
}

fn truncate_at_line(source: &str, line_no: usize) -> String {
    source
        .lines()
        .take(line_no - 1)
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_testscript_annotation(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            if line.trim() == "#testscript" {
                ""
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn get_rs2_files(dir: &Path, ext: &str) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut rs2_files: Vec<PathBuf> = Vec::new();
    search_dir_recursively(dir, ext, &mut rs2_files)?;
    Ok(rs2_files)
}

fn search_dir_recursively(
    dir: &Path,
    ext: &str,
    out: &mut Vec<PathBuf>,
) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if symloader::entry_is_dir(&entry, &path) {
            search_dir_recursively(&path, ext, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some(ext) {
            out.push(path);
        }
    }
    Ok(())
}
