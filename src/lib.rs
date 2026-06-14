pub mod bytecode;
pub mod compiler;
pub mod diagnostic_messages;
pub mod diagnostics;
pub mod error;
pub mod lexer;
pub mod lints;
#[cfg(feature = "memprof")]
pub mod memprof;
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

/// How the diagnostic layer fetches a file's source text (by its cache-key path)
/// when building rustc-style help/suggestions. The batch path keeps every source
/// in memory; the streaming path keeps only the on-disk paths and re-reads the
/// handful of files that actually produce a warning — so the ~4 MB of source
/// text isn't resident through the compile's peak. Cheap to copy (just borrows).
#[derive(Clone, Copy)]
pub enum SourceProvider<'a> {
    /// Sources already resident, keyed by cache key.
    Eager(&'a HashMap<String, Arc<String>>),
    /// Cache key → on-disk path; re-read on demand.
    Lazy(&'a HashMap<String, PathBuf>),
}

impl SourceProvider<'_> {
    /// Source text for `key`, if available. Returns an owned `Arc` — a cheap
    /// refcount bump for `Eager`, a fresh disk read for `Lazy`. Called only while
    /// emitting diagnostic help, which is rare, so the re-read cost is immaterial.
    pub fn get(&self, key: &str) -> Option<Arc<String>> {
        match self {
            SourceProvider::Eager(m) => m.get(key).cloned(),
            SourceProvider::Lazy(m) => fs::read_to_string(m.get(key)?).ok().map(Arc::new),
        }
    }
}

/// Log live heap + global peak at a pipeline phase boundary. No-op unless built
/// with `--features memprof`. Lets us attribute the footprint to each phase.
#[cfg(feature = "memprof")]
fn mem_mark(label: &str) {
    info!(
        "[mem] {:<22} live {:>7.1} MB | peak {:>7.1} MB | allocs {}",
        label,
        memprof::current_bytes() as f64 / 1_048_576.0,
        memprof::peak_bytes() as f64 / 1_048_576.0,
        memprof::alloc_count(),
    );
}
#[cfg(not(feature = "memprof"))]
#[inline]
fn mem_mark(_label: &str) {}

/// Per-map breakdown of the symbol registry's heap, to see the irreducible
/// floor for a streaming rebuild. Built only under `--features memprof`.
/// Accounts three components per map: the hashbrown table (inline `(K,V)`
/// buckets + 1 control byte each), the key strings, and value-side heap.
#[cfg(feature = "memprof")]
fn mem_stats_registry(reg: &crate::symbol::SymbolRegistry) {
    use crate::symbol::{Symbol, SymbolKind};
    use crate::types::Type;
    use std::collections::{HashMap, HashSet};
    let t = std::mem::size_of::<Type>();
    let mb = |b: usize| b as f64 / 1_048_576.0;

    // Allocated bucket count: hashbrown keeps `capacity()` ≈ 7/8 of a power-of-two
    // bucket array; an empty map allocates nothing.
    fn tbl<K, V>(m: &HashMap<K, V>) -> usize {
        if m.capacity() == 0 {
            return 0;
        }
        let buckets = (m.capacity() * 8 / 7).next_power_of_two();
        buckets * (std::mem::size_of::<(K, V)>() + 1)
    }
    fn keys_cap<V>(m: &HashMap<String, V>) -> usize {
        m.keys().map(|k| k.capacity()).sum()
    }
    let sym_heap = |s: &Symbol| -> usize {
        s.name.capacity()
            + match &s.kind {
                SymbolKind::Script {
                    trigger,
                    param_types,
                    return_types,
                    ..
                } => trigger.capacity() + (param_types.capacity() + return_types.capacity()) * t,
                SymbolKind::Command {
                    param_types,
                    return_types,
                    ..
                } => (param_types.capacity() + return_types.capacity()) * t,
                SymbolKind::GameVar { category, .. } => category.capacity(),
                SymbolKind::Constant { string_value, .. } => {
                    string_value.as_ref().map_or(0, |s| s.capacity())
                }
                _ => 0,
            }
    };
    let sym_map = |m: &HashMap<String, Symbol>| -> usize {
        tbl(m) + keys_cap(m) + m.values().map(&sym_heap).sum::<usize>()
    };
    let i32_map = |m: &HashMap<String, i32>| -> usize { tbl(m) + keys_cap(m) };

    let mut rows: Vec<(&str, usize, usize)> = vec![
        (
            "scripts",
            reg.scripts.len(),
            tbl(&reg.scripts) + keys_cap(&reg.scripts),
        ),
        ("commands", reg.commands.len(), sym_map(&reg.commands)),
        ("game_vars", reg.game_vars.len(), sym_map(&reg.game_vars)),
        ("constants", reg.constants.len(), sym_map(&reg.constants)),
        (
            "entity_ids",
            reg.entity_ids.len(),
            tbl(&reg.entity_ids)
                + keys_cap(&reg.entity_ids)
                + reg
                    .entity_ids
                    .values()
                    .map(|e| e.variants.capacity() * std::mem::size_of::<(Type, i32)>())
                    .sum::<usize>(),
        ),
        (
            "scripts_by_trigger",
            reg.scripts_by_trigger.len(),
            tbl(&reg.scripts_by_trigger)
                + keys_cap(&reg.scripts_by_trigger)
                + reg
                    .scripts_by_trigger
                    .values()
                    .map(|s| 16 + sym_heap(s.as_ref()))
                    .sum::<usize>(),
        ),
        (
            "command_param_types",
            reg.command_param_types.len(),
            tbl(&reg.command_param_types)
                + keys_cap(&reg.command_param_types)
                + reg
                    .command_param_types
                    .values()
                    .map(|v| v.capacity() * t)
                    .sum::<usize>(),
        ),
        ("script_ids", reg.script_ids.len(), i32_map(&reg.script_ids)),
        (
            "proc_script_ids",
            reg.proc_script_ids.len(),
            i32_map(&reg.proc_script_ids),
        ),
        (
            "label_script_ids",
            reg.label_script_ids.len(),
            i32_map(&reg.label_script_ids),
        ),
        (
            "trigger_script_ids",
            reg.trigger_script_ids.len(),
            i32_map(&reg.trigger_script_ids),
        ),
        (
            "preloaded_script_ids",
            reg.preloaded_script_ids.len(),
            i32_map(&reg.preloaded_script_ids),
        ),
        ("components", reg.components.len(), i32_map(&reg.components)),
        (
            "interface_ids",
            reg.interface_ids.len(),
            i32_map(&reg.interface_ids),
        ),
        ("type_chars", reg.type_chars.len(), i32_map(&reg.type_chars)),
        (
            "dbcolumn_types",
            reg.dbcolumn_types.len(),
            tbl(&reg.dbcolumn_types) + keys_cap(&reg.dbcolumn_types),
        ),
    ];
    let ts_tbl = {
        let s: &HashSet<String> = &reg.test_scripts;
        if s.capacity() == 0 {
            0
        } else {
            (s.capacity() * 8 / 7).next_power_of_two() * (std::mem::size_of::<String>() + 1)
                + s.iter().map(|k| k.capacity()).sum::<usize>()
        }
    };
    rows.push(("test_scripts", reg.test_scripts.len(), ts_tbl));

    rows.sort_by(|a, b| b.2.cmp(&a.2));
    let total: usize = rows.iter().map(|r| r.2).sum();
    info!(
        "[mem] registry ~{:.2} MB (sizeof Symbol={}B, SymbolKind={}B, Type={}B):",
        mb(total),
        std::mem::size_of::<Symbol>(),
        std::mem::size_of::<SymbolKind>(),
        t,
    );
    for (name, n, bytes) in &rows {
        info!(
            "[mem]   {:<20} {:>6} entries  {:>7.3} MB",
            name,
            n,
            mb(*bytes)
        );
    }
}

/// One-off breakdown of where the compiled-bytecode heap goes, to target
/// memory work. Built only under `--features memprof`.
#[cfg(feature = "memprof")]
fn mem_stats_compiled(scripts: &[CompiledScript]) {
    use crate::bytecode::{Instruction, Opcode, Operand};
    let (mut n_instr, mut n_str, mut str_bytes) = (0usize, 0usize, 0usize);
    let (mut n_switch, mut switch_entries) = (0usize, 0usize);
    let (mut source_path_bytes, mut name_bytes) = (0usize, 0usize);
    for s in scripts {
        n_instr += s.instructions.len();
        source_path_bytes += s.source_path.capacity();
        name_bytes += s.name.capacity() + s.trigger.capacity();
        for st in &s.strings {
            n_str += 1;
            str_bytes += st.len();
        }
        for sw in &s.switch_tables {
            n_switch += 1;
            switch_entries += sw.len();
        }
    }
    let sz = std::mem::size_of::<Instruction>();
    let mb = |b: usize| b as f64 / 1_048_576.0;
    info!(
        "[mem] sizeof Instruction={}B Operand={}B Opcode={}B CompiledScript={}B",
        sz,
        std::mem::size_of::<Operand>(),
        std::mem::size_of::<Opcode>(),
        std::mem::size_of::<CompiledScript>(),
    );
    info!(
        "[mem] compiled: {} scripts, {} instrs => {:.2} MB instr structs",
        scripts.len(),
        n_instr,
        mb(n_instr * sz)
    );
    info!(
        "[mem]   Str operands: {} ({:.2} MB content + {:.2} MB Box<str> headers)",
        n_str,
        mb(str_bytes),
        mb(n_str * 16)
    );
    info!(
        "[mem]   SwitchTable operands: {} ({} entries => {:.2} MB)",
        n_switch,
        switch_entries,
        mb(switch_entries * 16)
    );
    info!(
        "[mem]   source_path {:.2} MB, names+triggers {:.2} MB",
        mb(source_path_bytes),
        mb(name_bytes)
    );
}

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
pub(crate) fn parallel_chunks<T, R, F>(items: &[T], f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&[T]) -> R + Sync,
{
    parallel_chunks_capped(items, usize::MAX, f)
}

/// `parallel_chunks` with an upper bound on the number of concurrent chunks.
/// Fewer chunks → less peak memory when each chunk holds large per-chunk scratch
/// (e.g. the pointer checker's CFG caches), at the cost of less parallelism.
/// Result is independent of the chunk count, so output stays byte-identical.
fn parallel_chunks_capped<T, R, F>(items: &[T], max_chunks: usize, f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&[T]) -> R + Sync,
{
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(max_chunks.max(1));
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
    compile_memory_with_options(scripts_dir, pack_dir, lint, false, false)
}

/// Like `compile_memory` but includes `#testscript` sections when
/// `include_tests` is true.
pub fn compile_memory_with_tests(
    scripts_dir: &Path,
    pack_dir: Option<&Path>,
    lint: bool,
    include_tests: bool,
) -> Result<(Vec<u8>, Vec<u8>), Box<dyn Error>> {
    compile_memory_with_options(scripts_dir, pack_dir, lint, include_tests, false)
}

/// In-memory compile — returns the `(script.dat, script.idx)` bytes instead of
/// writing them — choosing the compilation strategy.
///
/// With `low_mem`, the recompile pipeline is used: scripts are re-parsed and
/// re-compiled on demand so the full compiled set is never resident (~16 MB heap
/// vs ~93), at roughly 6× the time; the returned bytes are byte-identical to the
/// default path. `include_tests` keeps `#testscript` sections. The
/// `RUNEC_RECOMPILE` environment variable forces low-memory mode on as well.
pub fn compile_memory_with_options(
    scripts_dir: &Path,
    pack_dir: Option<&Path>,
    lint: bool,
    include_tests: bool,
    low_mem: bool,
) -> Result<(Vec<u8>, Vec<u8>), Box<dyn Error>> {
    if low_mem || std::env::var("RUNEC_RECOMPILE").is_ok_and(|v| !v.is_empty()) {
        info!("[recompile] low-memory recompile pipeline enabled");
        return compile_recompile_memory(scripts_dir, pack_dir, lint, include_tests);
    }

    let (compiled_scripts, diagnostics) = run_pipeline(scripts_dir, pack_dir, lint, include_tests)?;

    info!("Writing compiled output to memory...");
    let writer = ScriptWriter::new("".into());

    diagnostics.print_all();
    info!("Compilation complete!");

    Ok(writer.build_all(&compiled_scripts)?)
}

/// Compile scripts and write output. When `lint` is true, also run lint passes
/// (unused locals, unreachable code) after code generation.
///
/// Convenience wrapper over [`compile_with_options`] using the default (fast,
/// fully-resident) pipeline. Kept signature-compatible for existing callers.
pub fn compile(
    scripts_dir: &Path,
    pack_dir: Option<&Path>,
    output_dir: &Path,
    lint: bool,
) -> Result<(), Box<dyn Error>> {
    compile_with_options(scripts_dir, pack_dir, output_dir, lint, false)
}

/// Compile scripts and write output, choosing the compilation strategy.
///
/// With `low_mem`, the **recompile** pipeline is used: scripts are re-parsed and
/// re-compiled on demand for the pointer check and written file-by-file, so the
/// full set of compiled bytecode is never resident at once. That cuts peak usage
/// to roughly 16 MB heap / 32 MB working set (from ~93 / ~111 MB) at about 6× the
/// compile time. Output is byte-identical to the default path. The
/// `RUNEC_RECOMPILE` environment variable forces this mode on as well.
pub fn compile_with_options(
    scripts_dir: &Path,
    pack_dir: Option<&Path>,
    output_dir: &Path,
    lint: bool,
    low_mem: bool,
) -> Result<(), Box<dyn Error>> {
    // The recompile pipeline never holds all compiled scripts (it re-compiles on
    // demand for the pointer check + writes file-by-file). It does its own
    // output, so it returns here. The default path below is unchanged.
    if low_mem || std::env::var("RUNEC_RECOMPILE").is_ok_and(|v| !v.is_empty()) {
        info!("[recompile] low-memory recompile pipeline enabled");
        return compile_recompile(scripts_dir, pack_dir, output_dir, lint);
    }

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

    // Prototype: opt into the streaming pipeline (never holds all ASTs at once)
    // with RUNEC_STREAM=1. The default batch path below is unchanged.
    if std::env::var("RUNEC_STREAM").is_ok_and(|v| !v.is_empty()) {
        info!("[stream] streaming pipeline enabled (RUNEC_STREAM)");
        return run_pipeline_streaming(scripts_dir, pack_dir, lint, include_tests);
    }

    let mut rs2_files: Vec<PathBuf> = get_rs2_files(scripts_dir, "rs2")?;

    rs2_files.sort();
    info!("Found {} script files", rs2_files.len());
    mem_mark("startup");

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

    mem_mark("after parse");

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
    // `raw_by_path` was only needed to (re)generate script.pack inside
    // `register_scripts`; the source text itself stays reachable via
    // `source_cache` (shared `Arc`s). Drop the redundant index now.
    drop(raw_by_path);
    // Pre-assigned script ids were consumed during registration; free them
    // before codegen (the compile's peak-memory phase) rather than at scope end.
    registry.drop_registration_scratch();

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

    mem_mark("after register");
    #[cfg(feature = "memprof")]
    mem_stats_registry(&registry);

    // Phase 3: Type checking
    info!("Type checking {} files...", all_files.len());
    run_type_checker(&mut diagnostics, &all_files, &registry);

    if diagnostics.has_errors() {
        diagnostics.print_all();
        return Err(Box::new(error::CompilerError::TypeError(
            "Type checking failed".to_string(),
        )));
    }

    mem_mark("after typecheck");

    // Phase 4: Code generation
    info!("Generating bytecode from type-checked scripts...");
    let mut compiled_scripts = codegen(&all_files, &registry);
    info!("  Generated {} compiled scripts", compiled_scripts.len());
    compiled_scripts.sort_by_key(|s| s.id);
    compiled_scripts.shrink_to_fit();
    mem_mark("after codegen");
    #[cfg(feature = "memprof")]
    mem_stats_compiled(&compiled_scripts);

    // The ASTs are consumed by codegen and never read again — pointer checking
    // and lints work purely off `compiled_scripts` + the registry + the source
    // cache. Free the syntax trees now so they don't coexist with the pointer
    // checker's (per-thread) scratch, which is the compile's peak-memory moment.
    drop(all_files);
    mem_mark("after drop asts");

    // Phase 4.5: Pointer checking & lints
    info!("Checking pointers in generated code...");
    check_pointers(
        lint,
        &mut diagnostics,
        SourceProvider::Eager(&source_cache),
        &registry,
        &compiled_scripts,
    );
    mem_mark("after pointercheck");

    Ok((compiled_scripts, diagnostics))
}

// ── Streaming pipeline (prototype, gated by RUNEC_STREAM) ────────────────────
// Goal: never materialize all ASTs (~59 MB) at once. Pass 1 parses in parallel
// but keeps only per-script signatures, dropping each AST in-worker; the registry
// is built from those. Pass 2 re-parses each file in parallel, type-checks and
// compiles it, then drops the AST immediately — so syntax trees never accumulate.
// Output is byte-identical to the batch path (same registration, same per-file
// codegen, same stable id-sort). It still holds all compiled scripts (for the
// cross-script pointer check + writer); shrinking that is the next increment.

/// One script's registration-relevant signature — everything the registry and
/// trigger diagnostics need — so the full AST can be dropped right after parsing.
struct ScriptSig {
    name: String,
    trigger: String,
    param_types: Vec<types::Type>,
    return_types: Vec<types::Type>,
    line: usize,
}

struct FileSigs {
    path: PathBuf,
    testscript_line: Option<usize>,
    sigs: Vec<ScriptSig>,
}

enum SigOutcome {
    Parsed {
        sigs: Vec<ScriptSig>,
        testscript_line: Option<usize>,
    },
    Error {
        line: usize,
        position: usize,
        message: String,
        phase: diagnostics::Phase,
    },
}

struct FileSigsResult {
    path: PathBuf,
    /// `(cache_key, raw_source, outcome)` on success.
    read: Result<(String, String, SigOutcome), std::io::Error>,
}

/// Parse one file, extract signatures, and DROP the AST in-worker, so a parallel
/// sweep never holds every syntax tree at once.
fn parse_one_file_sigs(path: &Path, include_tests: bool) -> FileSigsResult {
    let r = parse_one_file(path, include_tests);
    let read = r.read.map(|ps| {
        let outcome = match ps.outcome {
            ParseOutcome::Parsed {
                file,
                testscript_line,
            } => {
                let sigs = file
                    .scripts
                    .iter()
                    .map(|d| ScriptSig {
                        name: d.name.clone(),
                        trigger: d.trigger.clone(),
                        param_types: d.params.iter().map(|p| p.param_type).collect(),
                        return_types: d.return_types.clone(),
                        line: d.line,
                    })
                    .collect();
                SigOutcome::Parsed {
                    sigs,
                    testscript_line,
                }
                // `file` (the AST) is dropped here — only the sigs travel back.
            }
            ParseOutcome::Error {
                line,
                position,
                message,
                phase,
            } => SigOutcome::Error {
                line,
                position,
                message,
                phase,
            },
        };
        (ps.cache_key, ps.raw_source, outcome)
    });
    FileSigsResult { path: r.path, read }
}

fn run_pipeline_streaming(
    scripts_dir: &Path,
    pack_dir: Option<&Path>,
    lint: bool,
    include_tests: bool,
) -> Result<(Vec<CompiledScript>, DiagnosticsCollector), Box<dyn Error>> {
    let mut rs2_files: Vec<PathBuf> = get_rs2_files(scripts_dir, "rs2")?;
    rs2_files.sort();
    info!("Found {} script files", rs2_files.len());
    mem_mark("startup");

    let mut diagnostics = DiagnosticsCollector::new();

    // Pass 1: parse for signatures only (each AST dropped in its worker).
    info!("[stream] Pass 1: parsing for signatures...");
    let sig_results = parallel_map(&rs2_files, |path| parse_one_file_sigs(path, include_tests));

    // Streaming keeps only `cache_key -> on-disk path`; source text is re-read
    // lazily for the rare diagnostic that needs it, so the ~4 MB of sources isn't
    // resident through the peak. `raw_by_path` still holds the text transiently
    // for `script.pack` regeneration, then is dropped right after registration.
    let mut source_paths: HashMap<String, PathBuf> = HashMap::new();
    let mut raw_by_path: HashMap<PathBuf, Arc<String>> = HashMap::new();
    let mut file_sigs: Vec<FileSigs> = Vec::new();
    for r in sig_results {
        let (cache_key, raw_source, outcome) = match r.read {
            Ok(t) => t,
            Err(e) => return Err(Box::new(e)),
        };
        source_paths.insert(cache_key, r.path.clone());
        raw_by_path.insert(r.path.clone(), Arc::new(raw_source));
        match outcome {
            SigOutcome::Parsed {
                sigs,
                testscript_line,
            } => file_sigs.push(FileSigs {
                path: r.path,
                testscript_line,
                sigs,
            }),
            SigOutcome::Error {
                line,
                position,
                message,
                phase,
            } => diagnostics.error(r.path, line, position, message, phase),
        }
    }

    if diagnostics.has_errors() {
        diagnostics.print_all();
        return Err(Box::new(error::CompilerError::TypeError(
            "Parsing failed".to_string(),
        )));
    }
    mem_mark("after parse");

    // Registration (from signatures, not ASTs).
    info!("[stream] Registering scripts and building the symbol registry...");
    let resolved_pack_dir = resolve_pack_dir(scripts_dir, pack_dir);
    let mut registry = build_symbol_registry(scripts_dir, resolved_pack_dir.as_deref());
    register_scripts_from_sigs(
        &mut registry,
        scripts_dir,
        resolved_pack_dir.as_deref(),
        &mut diagnostics,
        &file_sigs,
        &raw_by_path,
    );
    info!("  Registered {} scripts", registry.scripts.len());
    drop(raw_by_path);
    registry.drop_registration_scratch();

    for fs in &file_sigs {
        for sig in &fs.sigs {
            if let Some(msg) =
                Compiler::validate_trigger_subject(&sig.trigger, &sig.name, &registry)
            {
                diagnostics.warning(
                    fs.path.clone(),
                    sig.line,
                    0,
                    msg,
                    diagnostics::Phase::SymbolRegistration,
                );
            }
        }
    }
    // Signatures are done; only the registry + source cache carry forward.
    drop(file_sigs);
    mem_mark("after register");
    #[cfg(feature = "memprof")]
    mem_stats_registry(&registry);

    // Pass 2: re-parse + type-check + compile per file, streaming. Parse can't
    // fail here (pass 1 validated), so a failed re-parse just yields nothing.
    info!("[stream] Pass 2: type-check + codegen (streaming)...");
    let per_file = parallel_map(&rs2_files, |path| {
        let parsed = match parse_one_file(path, include_tests).read {
            Ok(p) => p,
            Err(_) => return (Vec::new(), DiagnosticsCollector::new()),
        };
        let (file, testscript_line) = match parsed.outcome {
            ParseOutcome::Parsed {
                file,
                testscript_line,
            } => (file, testscript_line),
            ParseOutcome::Error { .. } => return (Vec::new(), DiagnosticsCollector::new()),
        };
        let mut type_checker = TypeChecker::new(&registry);
        type_checker.check_file_with_test_boundary(&file, path, testscript_line);
        let mut compiled = Vec::new();
        // Compile only when this file type-checks clean (codegen assumes
        // well-typed input). On any global error the whole compile aborts below,
        // so output produced here would be discarded anyway.
        if !type_checker.diagnostics.has_errors() {
            let mut compiler = Compiler::new(&registry);
            for script in &file.scripts {
                if script.trigger == "command" {
                    continue;
                }
                let mut c = compiler.compile_script(script);
                c.source_path = parsed.cache_key.clone();
                c.shrink_to_fit();
                compiled.push(c);
            }
        }
        (compiled, type_checker.diagnostics)
        // `file` (the AST) is dropped here.
    });

    let mut compiled_scripts: Vec<CompiledScript> = Vec::new();
    for (compiled, diags) in per_file {
        compiled_scripts.extend(compiled);
        diagnostics.merge(diags);
    }
    if diagnostics.has_errors() {
        diagnostics.print_all();
        return Err(Box::new(error::CompilerError::TypeError(
            "Type checking failed".to_string(),
        )));
    }
    // Stable sort by id matches the batch path (file order preserved among
    // equal ids → identical writer last-write-wins behaviour).
    compiled_scripts.sort_by_key(|s| s.id);
    compiled_scripts.shrink_to_fit();
    info!("  Generated {} compiled scripts", compiled_scripts.len());
    mem_mark("after codegen");
    #[cfg(feature = "memprof")]
    mem_stats_compiled(&compiled_scripts);

    check_pointers(
        lint,
        &mut diagnostics,
        SourceProvider::Lazy(&source_paths),
        &registry,
        &compiled_scripts,
    );
    mem_mark("after pointercheck");

    Ok((compiled_scripts, diagnostics))
}

/// `register_scripts` for the streaming path: identical logic, driven by the
/// extracted signatures instead of the (already-dropped) ASTs.
fn register_scripts_from_sigs(
    registry: &mut SymbolRegistry,
    scripts_dir: &Path,
    resolved_pack_dir: Option<&Path>,
    diagnostics: &mut DiagnosticsCollector,
    file_sigs: &[FileSigs],
    raw_by_path: &HashMap<PathBuf, Arc<String>>,
) {
    if let Some(pd) = resolved_pack_dir {
        symloader::generate_script_pack(scripts_dir, pd, raw_by_path);
        symloader::load_script_ids(registry, pd);
    }

    use crate::diagnostic_messages as msg;
    let mut registered_scripts: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    for fs in file_sigs.iter() {
        for sig in &fs.sigs {
            if sig.trigger == "command" {
                continue;
            }

            if !trigger_table::is_valid_trigger(&sig.trigger) {
                diagnostics.warning(
                    fs.path.clone(),
                    sig.line,
                    0,
                    msg::fmt(msg::SCRIPT_TRIGGER_INVALID, &[&sig.trigger]),
                    crate::diagnostics::Phase::SymbolRegistration,
                );
            }

            let key = format!("{}:{}", sig.trigger, sig.name);
            if !registered_scripts.insert(key) {
                diagnostics.warning(
                    fs.path.clone(),
                    sig.line,
                    0,
                    msg::fmt(msg::SCRIPT_REDECLARATION, &[&sig.trigger, &sig.name]),
                    crate::diagnostics::Phase::SymbolRegistration,
                );
            }

            if !sig.return_types.is_empty() && !trigger_table::allows_returns(&sig.trigger) {
                diagnostics.warning(
                    fs.path.clone(),
                    sig.line,
                    0,
                    msg::fmt(msg::SCRIPT_TRIGGER_NO_RETURNS, &[&sig.trigger]),
                    diagnostics::Phase::SymbolRegistration,
                );
            }

            registry.register_script(
                sig.name.clone(),
                sig.trigger.clone(),
                sig.param_types.clone(),
                sig.return_types.clone(),
            );

            if let Some(ts_line) = fs.testscript_line
                && sig.line >= ts_line
            {
                registry.mark_test_script(&sig.trigger, &sig.name);
            }
        }
    }
}

fn check_pointers(
    lint: bool,
    diagnostics: &mut DiagnosticsCollector,
    source: SourceProvider,
    registry: &SymbolRegistry,
    compiled_scripts: &[CompiledScript],
) {
    {
        use crate::pointer_checker::{PointerChecker, ScriptSource};
        // Validate scripts across threads: each chunk gets its own checker that
        // shares the read-only scripts/registry/source cache but keeps its own
        // (purely memoizing) caches. Diagnostics are merged in script order, so
        // the result is identical to a single-threaded run.
        //
        // The read-only command/script lookup tables are built ONCE here and
        // shared (`Arc`) across every chunk checker, instead of each thread
        // rebuilding the full ~700-command pointer table + per-script reverse
        // maps. `names` is just the script order, so derive it directly rather
        // than spinning up a throwaway checker for `script_names()`.
        let shared = PointerChecker::build_shared(compiled_scripts, registry);
        let names: Vec<String> = compiled_scripts.iter().map(|s| s.name.clone()).collect();
        // Each chunk's checker retains its CFG caches until the chunk finishes,
        // so concurrency multiplies peak scratch. RUNEC_PTR_CHUNKS caps the
        // concurrent chunk count to trade pointer-check speed for memory; the
        // result is chunk-count-independent, so diagnostics stay byte-identical.
        let max_chunks = std::env::var("RUNEC_PTR_CHUNKS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|&n| n > 0)
            .unwrap_or(usize::MAX);
        let chunk_diags = parallel_chunks_capped(&names, max_chunks, |chunk| {
            let mut checker = PointerChecker::with_shared(
                ScriptSource::Resident(compiled_scripts),
                registry,
                shared.clone(),
            );
            checker.set_source_cache(source);
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
    #[cfg(feature = "memprof")]
    crate::pointer_checker::cfg_trace_report();

    // Phase 4.6: Lint passes (optional)
    if lint {
        info!("Running lint passes (unused locals, unreachable code)...");
        let lint_diags = lints::run_lints(compiled_scripts, Some(source));
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
        compiled.shrink_to_fit();
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

// ── Streaming recompile pipeline (prototype, gated by RUNEC_RECOMPILE) ────────
// Never holds all compiled scripts. Pass 2 only type-checks; the pointer check
// re-compiles each script on demand (bounded cache — measured ~1.3x re-parse on
// rs-majula, the call graph has high file locality); output is produced by
// re-compiling file-by-file into a by-id buffer. Output + diagnostics are
// byte-identical to the batch path (deterministic re-compile, same id order).

/// Re-compiles scripts on demand for the pointer checker, caching `Rc`s with a
/// bounded FIFO so the full compiled set is never resident. A cache miss
/// re-parses the script's file and compiles all its scripts at once (siblings
/// share the parse), keying each by its full `[trigger,name]` to its index.
struct RecompileStore<'a> {
    idx_to_path: Vec<PathBuf>,
    /// file path -> its script idxs in declaration order (command-skipped).
    /// Position-based so duplicate `[trigger,name]` declarations each map to the
    /// correct distinct idx.
    file_to_idxs: HashMap<PathBuf, Vec<usize>>,
    registry: &'a SymbolRegistry,
    include_tests: bool,
    cache: HashMap<usize, std::rc::Rc<CompiledScript>>,
    order: std::collections::VecDeque<usize>,
    cap: usize,
}

impl RecompileStore<'_> {
    fn get(&mut self, idx: usize) -> std::rc::Rc<CompiledScript> {
        if let Some(rc) = self.cache.get(&idx) {
            return std::rc::Rc::clone(rc);
        }
        let path = self.idx_to_path[idx].clone();
        let parsed = parse_one_file(&path, self.include_tests);
        if let Ok(ps) = parsed.read
            && let ParseOutcome::Parsed { file, .. } = ps.outcome
        {
            // The j-th non-command declaration in this file maps to idxs[j],
            // matching how the metas were built (same parse, same skip rule).
            let idxs = self.file_to_idxs.get(&path).cloned().unwrap_or_default();
            let mut compiler = Compiler::new(self.registry);
            let mut j = 0usize;
            for script in &file.scripts {
                if script.trigger == "command" {
                    continue;
                }
                if let Some(&i) = idxs.get(j) {
                    let mut c = compiler.compile_script(script);
                    c.source_path = ps.cache_key.clone();
                    self.insert(i, std::rc::Rc::new(c));
                }
                j += 1;
            }
        }
        std::rc::Rc::clone(
            self.cache
                .get(&idx)
                .expect("recompile store: script not produced by its file"),
        )
    }

    fn insert(&mut self, idx: usize, rc: std::rc::Rc<CompiledScript>) {
        if self.cache.insert(idx, rc).is_none() {
            self.order.push_back(idx);
            while self.order.len() > self.cap {
                if let Some(old) = self.order.pop_front()
                    && old != idx
                {
                    self.cache.remove(&old);
                }
            }
        }
    }
}

/// Per-slot encoded output blobs (indexed by script id) plus accumulated
/// diagnostics — what the recompile core hands back before the bytes are written
/// to disk or assembled in memory.
type RecompiledOutput = (Vec<Vec<u8>>, DiagnosticsCollector);

/// Core of the recompile (low-memory) pipeline: runs every phase re-compiling
/// scripts on demand — never holding all compiled bytecode at once — and returns
/// the per-slot encoded output blobs plus diagnostics, without writing or
/// printing. The disk and in-memory entry points (`compile_recompile`,
/// `compile_recompile_memory`) wrap this. On a phase error it prints diagnostics
/// and returns `Err`, matching the batch path.
fn recompile_to_encoded(
    scripts_dir: &Path,
    pack_dir: Option<&Path>,
    lint: bool,
    include_tests: bool,
) -> Result<RecompiledOutput, Box<dyn Error>> {
    use crate::pointer_checker::{PointerChecker, ScriptSource};

    let mut rs2_files: Vec<PathBuf> = get_rs2_files(scripts_dir, "rs2")?;
    rs2_files.sort();
    info!("Found {} script files", rs2_files.len());
    mem_mark("startup");
    let mut diagnostics = DiagnosticsCollector::new();

    // Pass 1: signatures only (ASTs dropped in-worker); keep file paths for lazy
    // source + raw text for script.pack regeneration.
    info!("[recompile] Pass 1: parsing for signatures...");
    let sig_results = parallel_map(&rs2_files, |path| parse_one_file_sigs(path, include_tests));
    let mut source_paths: HashMap<String, PathBuf> = HashMap::new();
    let mut raw_by_path: HashMap<PathBuf, Arc<String>> = HashMap::new();
    let mut file_sigs: Vec<FileSigs> = Vec::new();
    for r in sig_results {
        let (cache_key, raw_source, outcome) = match r.read {
            Ok(t) => t,
            Err(e) => return Err(Box::new(e)),
        };
        source_paths.insert(cache_key, r.path.clone());
        raw_by_path.insert(r.path.clone(), Arc::new(raw_source));
        match outcome {
            SigOutcome::Parsed {
                sigs,
                testscript_line,
            } => file_sigs.push(FileSigs {
                path: r.path,
                testscript_line,
                sigs,
            }),
            SigOutcome::Error {
                line,
                position,
                message,
                phase,
            } => diagnostics.error(r.path, line, position, message, phase),
        }
    }
    if diagnostics.has_errors() {
        diagnostics.print_all();
        return Err(Box::new(error::CompilerError::TypeError(
            "Parsing failed".to_string(),
        )));
    }
    mem_mark("after parse");

    // Registration.
    info!("[recompile] Registering scripts and building the symbol registry...");
    let resolved_pack_dir = resolve_pack_dir(scripts_dir, pack_dir);
    let mut registry = build_symbol_registry(scripts_dir, resolved_pack_dir.as_deref());
    register_scripts_from_sigs(
        &mut registry,
        scripts_dir,
        resolved_pack_dir.as_deref(),
        &mut diagnostics,
        &file_sigs,
        &raw_by_path,
    );
    info!("  Registered {} scripts", registry.scripts.len());
    drop(raw_by_path);
    registry.drop_registration_scratch();
    for fs in &file_sigs {
        for sig in &fs.sigs {
            if let Some(msg) =
                Compiler::validate_trigger_subject(&sig.trigger, &sig.name, &registry)
            {
                diagnostics.warning(
                    fs.path.clone(),
                    sig.line,
                    0,
                    msg,
                    diagnostics::Phase::SymbolRegistration,
                );
            }
        }
    }
    mem_mark("after register");

    // Pass 2: type-check only (re-parse, no compiled scripts retained).
    info!("[recompile] Pass 2: type-check (streaming, no bytecode held)...");
    let tc_diags = parallel_map(&rs2_files, |path| {
        let parsed = match parse_one_file(path, include_tests).read {
            Ok(p) => p,
            Err(_) => return DiagnosticsCollector::new(),
        };
        let (file, testscript_line) = match parsed.outcome {
            ParseOutcome::Parsed {
                file,
                testscript_line,
            } => (file, testscript_line),
            ParseOutcome::Error { .. } => return DiagnosticsCollector::new(),
        };
        let mut type_checker = TypeChecker::new(&registry);
        type_checker.check_file_with_test_boundary(&file, path, testscript_line);
        type_checker.diagnostics
    });
    for d in tc_diags {
        diagnostics.merge(d);
    }
    if diagnostics.has_errors() {
        diagnostics.print_all();
        return Err(Box::new(error::CompilerError::TypeError(
            "Type checking failed".to_string(),
        )));
    }
    mem_mark("after typecheck");

    // Build script metas in the same idx order as the batch path: file order
    // (file_sigs / declaration order, command-skipped), then a stable sort by id.
    // ids + names come from the registry — no bytecode needed.
    let mut pre: Vec<(i32, String, PathBuf)> = Vec::new();
    for fs in &file_sigs {
        for sig in &fs.sigs {
            if sig.trigger == "command" {
                continue;
            }
            let id = registry
                .script_id_for_trigger(&sig.trigger, &sig.name)
                .or_else(|| registry.script_id(&sig.name))
                .unwrap_or(-1);
            pre.push((
                id,
                format!("[{},{}]", sig.trigger, sig.name),
                fs.path.clone(),
            ));
        }
    }
    let n = pre.len();
    // Stable sort indices by id → the batch's idx order (file/decl pre-order
    // preserved among equal ids). `inv` maps a pre-order position back to its idx.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&p| pre[p].0);
    let idents: Vec<(i32, String)> = order
        .iter()
        .map(|&p| (pre[p].0, pre[p].1.clone()))
        .collect();
    let idx_to_path: Vec<PathBuf> = order.iter().map(|&p| pre[p].2.clone()).collect();
    let mut inv = vec![0usize; n];
    for (idx, &p) in order.iter().enumerate() {
        inv[p] = idx;
    }
    let mut file_to_idxs: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    for (p, m) in pre.iter().enumerate() {
        file_to_idxs.entry(m.2.clone()).or_default().push(inv[p]);
    }
    let max_id = pre.iter().map(|m| m.0).max().unwrap_or(-1);
    let slot_count = if max_id < 0 { 0 } else { (max_id + 1) as usize };
    let len = n;
    drop(pre);

    // Pointer check via on-demand re-compile (single-threaded: the store getter
    // is `FnMut`/`!Sync`). Diagnostics come out in id order, as in the batch path.
    info!("[recompile] Pointer check (re-compile on demand)...");
    let shared = PointerChecker::build_shared_idents(&idents, &registry);
    {
        // Cache cap must exceed the largest file's script count, or loading that
        // file would evict its own just-compiled scripts. A small margin over
        // that keeps cross-file locality high (measured ~1.1-1.3x re-parse).
        let cap = file_to_idxs
            .values()
            .map(|v| v.len())
            .max()
            .unwrap_or(0)
            .max(128);
        let mut store = RecompileStore {
            idx_to_path,
            file_to_idxs,
            registry: &registry,
            include_tests,
            cache: HashMap::new(),
            order: std::collections::VecDeque::new(),
            cap,
        };
        let getter: Box<dyn FnMut(usize) -> std::rc::Rc<CompiledScript>> =
            Box::new(move |idx| store.get(idx));
        let scripts = ScriptSource::Recompile {
            len,
            get: std::cell::RefCell::new(getter),
        };
        let mut checker = PointerChecker::with_shared(scripts, &registry, shared);
        checker.set_source_cache(SourceProvider::Lazy(&source_paths));
        let names: Vec<String> = idents.iter().map(|(_, n)| n.clone()).collect();
        let ptr_diags = checker.validate_names(&names);
        let w = ptr_diags.warning_count();
        if w > 0 {
            info!("  Pointer check: {} warning(s)", w);
        }
        diagnostics.merge(ptr_diags);
    }
    mem_mark("after pointercheck");

    // Output: re-compile each file once, encode each script into its id slot,
    // optionally lint — never holding all compiled scripts at once. The caller
    // writes the blobs to disk or assembles them in memory.
    info!("[recompile] Re-compiling + encoding output...");
    let mut encoded: Vec<Vec<u8>> = vec![Vec::new(); slot_count];
    for path in &rs2_files {
        let parsed = match parse_one_file(path, include_tests).read {
            Ok(p) => p,
            Err(_) => continue,
        };
        let file = match parsed.outcome {
            ParseOutcome::Parsed { file, .. } => file,
            ParseOutcome::Error { .. } => continue,
        };
        let mut compiler = Compiler::new(&registry);
        let mut file_scripts: Vec<CompiledScript> = Vec::new();
        for script in &file.scripts {
            if script.trigger == "command" {
                continue;
            }
            let mut c = compiler.compile_script(script);
            c.source_path = parsed.cache_key.clone();
            file_scripts.push(c);
        }
        for c in &file_scripts {
            if c.id >= 0 && (c.id as usize) < slot_count {
                encoded[c.id as usize] = crate::writer::encode_script(c);
            }
        }
        if lint {
            diagnostics.merge(lints::run_lints(
                &file_scripts,
                Some(SourceProvider::Lazy(&source_paths)),
            ));
        }
    }
    mem_mark("after codegen");
    Ok((encoded, diagnostics))
}

/// Full compile via the recompile (low-memory) pipeline: writes `script.dat` /
/// `script.idx` to `output_dir`, then prints diagnostics.
fn compile_recompile(
    scripts_dir: &Path,
    pack_dir: Option<&Path>,
    output_dir: &Path,
    lint: bool,
) -> Result<(), Box<dyn Error>> {
    let (encoded, diagnostics) = recompile_to_encoded(scripts_dir, pack_dir, lint, false)?;
    info!(
        "Writing output to {}...",
        output_dir.display().to_string().replace('\\', "/")
    );
    ScriptWriter::new(output_dir.display().to_string()).write_encoded(&encoded)?;
    diagnostics.print_all();
    info!("Compilation complete!");
    Ok(())
}

/// In-memory compile via the recompile (low-memory) pipeline: returns the
/// `(script.dat, script.idx)` bytes, then prints diagnostics. Byte-identical to
/// the in-memory batch path (`compile_memory*`).
fn compile_recompile_memory(
    scripts_dir: &Path,
    pack_dir: Option<&Path>,
    lint: bool,
    include_tests: bool,
) -> Result<(Vec<u8>, Vec<u8>), Box<dyn Error>> {
    let (encoded, diagnostics) = recompile_to_encoded(scripts_dir, pack_dir, lint, include_tests)?;
    info!("Writing compiled output to memory...");
    let out = ScriptWriter::new("".into()).build_encoded(&encoded);
    diagnostics.print_all();
    info!("Compilation complete!");
    Ok(out)
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
