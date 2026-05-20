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

use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use tracing::info;
use crate::bytecode::CompiledScript;
use crate::compiler::Compiler;
use crate::diagnostics::DiagnosticsCollector;
use crate::lexer::Lexer;
use crate::parser::{Parser, ScriptFile};
use crate::symbol::SymbolRegistry;
use crate::typechecker::TypeChecker;
use crate::writer::ScriptWriter;

/// Compile scripts and write output. When `lint` is true, also run lint passes
/// (unused locals, unreachable code) after code generation.
pub fn compile_memory(
    scripts_dir: &Path,
    pack_dir: Option<&Path>,
    lint: bool,
) -> Result<(Vec<u8>, Vec<u8>), Box<dyn Error>> {
    let (compiled_scripts, diagnostics) = run_pipeline(scripts_dir, pack_dir, lint)?;

    info!("Phase 5: Writing output to memory...");
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
    let (compiled_scripts, diagnostics) = run_pipeline(scripts_dir, pack_dir, lint)?;

    let output = output_dir.display().to_string();
    info!("Phase 5: Writing output to {}...", output);
    let writer = ScriptWriter::new(output);
    writer.write_all(&compiled_scripts)?;

    diagnostics.print_all();
    info!("Compilation complete!");

    Ok(())
}

/// Run all analysis passes (parse, type-check, codegen, pointer-check, lints)
/// without writing output. Useful for editor tooling and CI lint gates.
pub fn lint(scripts_dir: &Path, pack_dir: Option<&Path>) -> Result<(), Box<dyn Error>> {
    let (_, diagnostics) = run_pipeline(scripts_dir, pack_dir, true)?;

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
) -> Result<(Vec<CompiledScript>, DiagnosticsCollector), Box<dyn Error>> {
    if !scripts_dir.exists() || !scripts_dir.is_dir() {
        return Err(Box::new(error::CompilerError::FileNotFound(
            scripts_dir.display().to_string(),
        )));
    }

    let mut rs2_files: Vec<PathBuf> = Vec::new();
    collect_files(scripts_dir, "rs2", &mut rs2_files)?;
    rs2_files.sort();
    info!("Found {} script files", rs2_files.len());

    let mut diagnostics = DiagnosticsCollector::new();

    // Phase 1: Parse all files
    info!("Phase 1: Parsing...");
    let mut all_files = Vec::new();
    let mut source_cache: HashMap<String, Rc<String>> = HashMap::new();
    for path in &rs2_files {
        let source_code = fs::read_to_string(path)?;
        let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let mut key = canonical.to_string_lossy().into_owned();
        if key.starts_with("\\\\?\\") {
            key = key[4..].to_string();
        }
        key = key.replace("\\Content\\", "\\content\\");
        source_cache.insert(key, Rc::new(source_code.clone()));

        let tokens = match Lexer::new(&source_code, path).tokenize() {
            Ok(t) => t,
            Err(e) => {
                diagnostics.error(
                    path.clone(),
                    e.line,
                    e.position,
                    e.message.clone(),
                    crate::diagnostics::Phase::Lexing,
                );
                continue;
            }
        };
        let mut parser = Parser::new(tokens, path);
        match parser.parse() {
            Ok(file) => all_files.push((path.clone(), file)),
            Err(e) => {
                diagnostics.error(
                    path.clone(),
                    e.line,
                    e.position,
                    e.message.clone(),
                    crate::diagnostics::Phase::Parsing,
                );
            }
        }
    }

    if diagnostics.has_errors() {
        diagnostics.print_all();
        return Err(Box::new(error::CompilerError::TypeError(
            "Parsing failed".to_string(),
        )));
    }

    // Phase 2: Register all scripts
    info!("Phase 2: Registering scripts...");
    let registry = register_scripts(scripts_dir, pack_dir, &mut diagnostics, &mut all_files);
    info!("  Registered {} scripts", registry.scripts.len());

    for (path, file) in &all_files {
        for script in &file.scripts {
            if let Some(msg) =
                Compiler::validate_trigger_subject(&script.trigger, &script.name, &registry)
            {
                diagnostics.warning(
                    path.clone(),
                    script.line,
                    0,
                    msg,
                    crate::diagnostics::Phase::SymbolRegistration,
                );
            }
        }
    }

    // Phase 3: Type checking
    info!("Phase 3: Type checking {} files...", all_files.len());
    run_type_checker(&mut diagnostics, &mut all_files, &registry);

    if diagnostics.has_errors() {
        diagnostics.print_all();
        return Err(Box::new(error::CompilerError::TypeError(
            "Type checking failed".to_string(),
        )));
    }

    // Phase 4: Code generation
    info!("Phase 4: Code generation...");
    let (codegen, mut compiled_scripts) = codegen(&mut all_files, registry);
    info!("  Generated {} compiled scripts", compiled_scripts.len());
    compiled_scripts.sort_by_key(|s| s.id);

    // Phase 4.5: Pointer checking & lints
    info!("Phase 4.5: Pointer checking...");
    check_pointers(lint, &mut diagnostics, &mut source_cache, &codegen, &compiled_scripts);

    Ok((compiled_scripts, diagnostics))
}

fn check_pointers(lint: bool, diagnostics: &mut DiagnosticsCollector, source_cache: &mut HashMap<String, Rc<String>>, codegen: &Compiler, compiled_scripts: &Vec<CompiledScript>) {
    {
        use crate::pointer_checker::PointerChecker;
        let mut checker = PointerChecker::new(&compiled_scripts, &codegen.registry);
        checker.set_source_cache(&source_cache);
        let pointer_diags = checker.run();
        let ptr_warnings = pointer_diags.warning_count();
        if ptr_warnings > 0 {
            info!("  Pointer check: {} warning(s)", ptr_warnings);
        }
        diagnostics.merge(pointer_diags);
    }

    // Phase 4.6: Lint passes (optional)
    if lint {
        info!("Phase 4.6: Lint checks...");
        let lint_diags = lints::run_lints(&compiled_scripts, Some(&source_cache));
        let lint_warnings = lint_diags.warning_count();
        if lint_warnings > 0 {
            info!("  Lints: {} warning(s)", lint_warnings);
        }
        diagnostics.merge(lint_diags);
    }
}

fn codegen(all_files: &mut Vec<(PathBuf, ScriptFile)>, registry: SymbolRegistry) -> (Compiler, Vec<CompiledScript>) {
    let mut codegen = Compiler::new(registry);
    let mut compiled_scripts = Vec::new();
    for (path, file) in all_files {
        for script in &file.scripts {
            if script.trigger == "command" {
                continue;
            }
            let mut compiled = codegen.compile_script(script);
            let canonical = fs::canonicalize(path.clone()).unwrap_or_else(|_| path.to_path_buf());
            let mut source_path = canonical.to_string_lossy().into_owned();
            if source_path.starts_with("\\\\?\\") {
                source_path = source_path[4..].to_string();
            }
            source_path = source_path.replace("\\Content\\", "\\content\\");
            compiled.source_path = source_path;
            compiled_scripts.push(compiled);
        }
    }
    (codegen, compiled_scripts)
}

fn run_type_checker(diagnostics: &mut DiagnosticsCollector, all_files: &mut Vec<(PathBuf, ScriptFile)>, registry: &SymbolRegistry) {
    let mut type_checker = TypeChecker::new(&registry);
    for (path, file) in all_files.iter() {
        type_checker.check_file(file, path);
    }
    diagnostics.merge(type_checker.diagnostics);
}

fn register_scripts(scripts_dir: &Path, pack_dir: Option<&Path>, diagnostics: &mut DiagnosticsCollector, all_files: &mut Vec<(PathBuf, ScriptFile)>) -> SymbolRegistry {
    let mut registry = SymbolRegistry::new();

    let resolved_pack_dir = pack_dir
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
        });

    if let Some(ref pd) = resolved_pack_dir {
        symloader::generate_script_pack(scripts_dir, pd);
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

    symloader::load_constant_files(&mut registry, scripts_dir);

    info!(
        "  Loaded {} constants from .constant files",
        registry.constants.len()
    );

    symloader::load_game_var_types(&mut registry, scripts_dir);

    if let Some(ref pd) = resolved_pack_dir {
        let dbtable_ids = symloader::load_dbtable_pack(&pd.join("dbtable.pack"));
        if !dbtable_ids.is_empty() {
            symloader::load_dbtable_configs(&mut registry, scripts_dir, &dbtable_ids);
            info!(
                "  Registered {} dbcolumn compound IDs",
                registry.dbcolumn_types.len()
            );
        }
    }

    let engine_rs2 = scripts_dir.join("engine.rs2");

    if engine_rs2.exists() {
        crate::symloader::load_engine_command_params(&mut registry, &engine_rs2);
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

    crate::symloader::patch_command_return_types(&mut registry);

    {
        use crate::diagnostic_messages as msg;

        let mut registered_scripts: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for (path, file) in all_files {
            for script in &file.scripts {
                if script.trigger == "command" {
                    continue;
                }

                if !trigger_table::is_valid_trigger(&script.trigger) {
                    diagnostics.warning(
                        path.clone(),
                        script.line,
                        0,
                        msg::fmt(msg::SCRIPT_TRIGGER_INVALID, &[&script.trigger]),
                        crate::diagnostics::Phase::SymbolRegistration,
                    );
                }

                let key = format!("{}:{}", script.trigger, script.name);
                if !registered_scripts.insert(key.clone()) {
                    diagnostics.warning(
                        path.clone(),
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
                        path.clone(),
                        script.line,
                        0,
                        msg::fmt(msg::SCRIPT_TRIGGER_NO_RETURNS, &[&script.trigger]),
                        crate::diagnostics::Phase::SymbolRegistration,
                    );
                }

                let param_types: Vec<crate::types::Type> =
                    script.params.iter().map(|p| p.param_type).collect();
                registry.register_script(
                    script.name.clone(),
                    script.trigger.clone(),
                    param_types,
                    script.return_types.clone(),
                );
            }
        }
    }
    registry
}

fn collect_files(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, ext, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some(ext) {
            out.push(path);
        }
    }
    Ok(())
}
