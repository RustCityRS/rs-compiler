extern crate core;

use crate::config::Config;
use clap::{Parser as ClapParser, Subcommand};
use std::error::Error;
use std::path::PathBuf;

mod analysis;
mod bytecode;
mod compiler;
mod config;
mod diagnostic_messages;
mod diagnostics;
mod error;
mod lexer;
mod lints;
mod parser;
mod pointer;
mod pointer_checker;
mod symbol;
mod symloader;
mod token;
mod trigger_table;
mod typechecker;
mod types;
mod writer;

#[derive(ClapParser)]
#[command(author, version, about = "RuneScript Compiler")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compile all .rs2 scripts to binary output
    Compile {
        /// Source directory containing .rs2 files
        #[arg(short, long)]
        source: Option<String>,
        /// Output directory for compiled scripts
        #[arg(short, long, default_value = "./data/pack/server")]
        output: String,
        /// Pack directory (overrides auto-detection)
        #[arg(long)]
        pack: Option<String>,
    },
    /// Analyze the 2004Scape codebase
    #[command(name = "2004")]
    Analyze2004,
    /// Update the RuneScript Compiler to the latest version
    Update,
    /// Manage RuneScript configuration
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Edit the RC file for the current environment
    Edit,
    /// Show the current RC file contents
    Show,
    /// Initialize a new RC file with defaults
    Init,
    /// List all environment variables and aliases
    List,
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    let config = Config::load();

    match cli.command {
        Commands::Compile {
            source,
            output,
            pack,
        } => {
            compile_scripts(source, output, pack, &config)?;
        }
        Commands::Analyze2004 => {
            println!("Analyzing 2004Scape codebase...");
            let mut analyzer = analysis::ScriptAnalysis::new();
            match analyzer.analyze_repository() {
                Ok(_) => analyzer.print_analysis(),
                Err(e) => println!("Error analyzing 2004Scape codebase: {}", e),
            }
        }
        Commands::Update => {
            let current_dir = std::env::current_dir()?;
            let install_script = if cfg!(windows) {
                "install.ps1"
            } else {
                "install.sh"
            };

            if !current_dir.join(install_script).exists() {
                println!("Error: Installation script not found.");
                return Ok(());
            }

            println!(
                "Updating RuneScript Compiler ({} environment)...",
                config.env_name
            );

            let has_git = std::process::Command::new("git")
                .args(["rev-parse", "--git-dir"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);

            let has_remote = if has_git {
                std::process::Command::new("git")
                    .args(["remote", "get-url", "origin"])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
            } else {
                false
            };

            if has_git && has_remote {
                println!("Pulling latest changes...");
                let _ = std::process::Command::new("git")
                    .args(["pull", "origin", "main"])
                    .status();
            }

            println!("Rebuilding...");
            if cfg!(windows) {
                std::process::Command::new("powershell")
                    .args(["-ExecutionPolicy", "Bypass", "-File", install_script])
                    .env("RSC_ENV", &config.env_name)
                    .env("RSC_INSTALL_DIR", config.install_dir.to_str().unwrap())
                    .env("RSC_SCRIPTS_DIR", config.scripts_dir.to_str().unwrap())
                    .status()?;
            } else {
                std::process::Command::new("sh")
                    .arg(install_script)
                    .env("RSC_ENV", &config.env_name)
                    .env("RSC_INSTALL_DIR", config.install_dir.to_str().unwrap())
                    .env("RSC_SCRIPTS_DIR", config.scripts_dir.to_str().unwrap())
                    .status()?;
            }
            println!("Update complete!");
        }
        Commands::Config { command } => match command {
            ConfigCommands::Edit => {
                let editor = std::env::var("EDITOR").unwrap_or_else(|_| {
                    if cfg!(windows) {
                        "notepad".into()
                    } else {
                        "nano".into()
                    }
                });
                let rc_path = Config::get_rc_path();
                if !rc_path.exists() {
                    Config::load_rc_file()?;
                }
                std::process::Command::new(editor).arg(rc_path).status()?;
            }
            ConfigCommands::Show => {
                let contents = Config::load_rc_file()?;
                println!("{}", contents);
            }
            ConfigCommands::Init => {
                let rc_path = Config::get_rc_path();
                if rc_path.exists() {
                    println!("RC file already exists at: {}", rc_path.display());
                } else {
                    Config::load_rc_file()?;
                    println!("Initialized new RC file at: {}", rc_path.display());
                }
            }
            ConfigCommands::List => {
                let contents = Config::load_rc_file()?;
                let (aliases, env_vars) = Config::parse_rc_file(&contents);
                println!("Environment: {}", config.env_name);
                println!("\nEnvironment Variables:");
                for (key, value) in env_vars {
                    println!("  {}={}", key, value);
                }
                println!("\nAliases:");
                for alias in aliases {
                    println!("  {}", alias);
                }
            }
        },
    }

    Ok(())
}

fn compile_scripts(
    source: Option<String>,
    output: String,
    pack_override: Option<String>,
    config: &Config,
) -> Result<(), Box<dyn Error>> {
    use crate::compiler::Compiler;
    use crate::diagnostics::DiagnosticsCollector;
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::symbol::SymbolRegistry;
    use crate::typechecker::TypeChecker;
    use crate::writer::ScriptWriter;
    use std::fs;

    let scripts_dir = source
        .map(PathBuf::from)
        .unwrap_or_else(|| config.scripts_dir.clone());

    if !scripts_dir.exists() || !scripts_dir.is_dir() {
        eprintln!("Scripts directory not found: {}", scripts_dir.display());
        return Err(Box::new(error::CompilerError::FileNotFound(
            scripts_dir.display().to_string(),
        )));
    }

    // Collect all .rs2 files and sort alphabetically (must match Java compiler ordering)
    let mut rs2_files: Vec<PathBuf> = Vec::new();
    collect_files(&scripts_dir, "rs2", &mut rs2_files)?;
    rs2_files.sort();
    println!("Found {} script files", rs2_files.len());

    let mut diagnostics = DiagnosticsCollector::new();

    // Phase 1: Parse all files
    println!("Phase 1: Parsing...");
    let mut all_files = Vec::new();
    // Source text cache, keyed by the canonicalized path string that ends
    // up as `CompiledScript::source_path`. Retained through Phase 4.5 so
    // the pointer checker can render rustc-style help/suggestion blocks
    // against the original source. Read-only; never feeds codegen.
    let mut source_cache: std::collections::HashMap<String, std::rc::Rc<String>> =
        std::collections::HashMap::new();
    for path in &rs2_files {
        let source_code = fs::read_to_string(path)?;
        // Compute the same canonical key that codegen will stamp onto
        // CompiledScript::source_path (see Phase 4 below).
        let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let mut key = canonical.to_string_lossy().into_owned();
        if key.starts_with("\\\\?\\") {
            key = key[4..].to_string();
        }
        key = key.replace("\\Content\\", "\\content\\");
        source_cache.insert(key, std::rc::Rc::new(source_code.clone()));

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

    // Phase 2: Register all scripts (first pass)
    println!("Phase 2: Registering scripts...");
    let mut registry = SymbolRegistry::new();

    // Game commands are loaded from command.pack (in load_packs below),
    // matching RuneScriptTS where the engine passes its ScriptOpcodeMap
    // as symbols['command']. engine.rs2 enriches these with param/return
    // types later.

    // Load pack files (commands, vars, entity IDs).
    // We look for a 'pack' directory next to the scripts directory.
    let pack_dir = pack_override
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .or_else(|| {
            scripts_dir
                .parent()
                .map(|p| p.join("pack"))
                .filter(|p| p.exists())
        })
        .or_else(|| {
            // Also try looking for content/pack from project root
            scripts_dir
                .ancestors()
                .nth(3)
                .map(|p| p.join("content/pack"))
                .filter(|p| p.exists())
        });

    if let Some(ref pd) = pack_dir {
        // Generate/update script.pack before loading (matches 2004scape regenScriptPack)
        symloader::generate_script_pack(&scripts_dir, pd);

        println!("  Loading packs from {}", pd.display());
        symloader::load_packs(&mut registry, pd);
        println!(
            "  Loaded {} commands, {} game vars",
            registry.commands.len(),
            registry.game_vars.len(),
        );
    } else {
        println!("  No pack directory found - symbol resolution will be limited");
    }

    // Load constants from .constant files in the scripts directory
    // (matches 2004scape: loadDirExtFull(scripts_dir, '.constant', ...))
    symloader::load_constant_files(&mut registry, &scripts_dir);
    println!(
        "  Loaded {} constants from .constant files",
        registry.constants.len()
    );

    // Refine game-var types from .varp/.varn/.vars/.varbit config files.
    // Pack files only give `name→id`; the real `type=...` lives in the config.
    symloader::load_game_var_types(&mut registry, &scripts_dir);

    // Scan .dbtable files and register `table:column` compound IDs →
    // packed int `(tableId << 12) | (colIdx << 4) | tupleNibble`.
    if let Some(ref pd) = pack_dir {
        let dbtable_ids = symloader::load_dbtable_pack(&pd.join("dbtable.pack"));
        if !dbtable_ids.is_empty() {
            symloader::load_dbtable_configs(&mut registry, &scripts_dir, &dbtable_ids);
            println!(
                "  Registered {} dbcolumn compound IDs",
                registry.dbcolumn_types.len()
            );
        }
    }

    // Load engine command parameter types from engine.rs2 (for type-aware identifier resolution).
    // This allows `smokepuff` to resolve to synth=164 in sound_synth() but spotanim=86 in spotanim_map().
    let engine_rs2 = scripts_dir.join("engine.rs2");
    if engine_rs2.exists() {
        crate::symloader::load_engine_command_params(&mut registry, &engine_rs2);
    } else {
        // Also search subdirectories one level deep
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
    // Commands whose return types are not declared in engine.rs2 but are known
    // from the Neptune compiler's internal implementation. These commands return
    // values that must be discarded with POP_INT_DISCARD when used as statements.
    crate::symloader::patch_command_return_types(&mut registry);

    {
        use crate::diagnostic_messages as msg;

        // Known valid trigger types (matching 2004scape/Neptune compiler)
        // Complete trigger set from RuneScriptTS ServerTriggerType.ts
        let valid_triggers: std::collections::HashSet<&str> = [
            "proc",
            "label",
            "debugproc",
            "command",
            "opnpc1",
            "opnpc2",
            "opnpc3",
            "opnpc4",
            "opnpc5",
            "opnpct",
            "opnpcu",
            "apnpc1",
            "apnpc2",
            "apnpc3",
            "apnpc4",
            "apnpc5",
            "apnpct",
            "apnpcu",
            "oploc1",
            "oploc2",
            "oploc3",
            "oploc4",
            "oploc5",
            "oploct",
            "oplocu",
            "aploc1",
            "aploc2",
            "aploc3",
            "aploc4",
            "aploc5",
            "aploct",
            "aplocu",
            "opobj1",
            "opobj2",
            "opobj3",
            "opobj4",
            "opobj5",
            "opobjt",
            "opobju",
            "apobj1",
            "apobj2",
            "apobj3",
            "apobj4",
            "apobj5",
            "apobjt",
            "apobju",
            "opplayer1",
            "opplayer2",
            "opplayer3",
            "opplayer4",
            "opplayer5",
            "opplayert",
            "opplayeru",
            "applayer1",
            "applayer2",
            "applayer3",
            "applayer4",
            "applayer5",
            "applayert",
            "applayeru",
            "opheld1",
            "opheld2",
            "opheld3",
            "opheld4",
            "opheld5",
            "opheld6",
            "opheld7",
            "opheld8",
            "opheldt",
            "opheldu",
            "ai_opnpc1",
            "ai_opnpc2",
            "ai_opnpc3",
            "ai_opnpc4",
            "ai_opnpc5",
            "ai_apnpc1",
            "ai_apnpc2",
            "ai_apnpc3",
            "ai_apnpc4",
            "ai_apnpc5",
            "ai_oploc1",
            "ai_oploc2",
            "ai_oploc3",
            "ai_oploc4",
            "ai_oploc5",
            "ai_aploc1",
            "ai_aploc2",
            "ai_aploc3",
            "ai_aploc4",
            "ai_aploc5",
            "ai_opobj1",
            "ai_opobj2",
            "ai_opobj3",
            "ai_opobj4",
            "ai_opobj5",
            "ai_apobj1",
            "ai_apobj2",
            "ai_apobj3",
            "ai_apobj4",
            "ai_apobj5",
            "ai_opplayer1",
            "ai_opplayer2",
            "ai_opplayer3",
            "ai_opplayer4",
            "ai_opplayer5",
            "ai_applayer1",
            "ai_applayer2",
            "ai_applayer3",
            "ai_applayer4",
            "ai_applayer5",
            "ai_queue1",
            "ai_queue2",
            "ai_queue3",
            "ai_queue4",
            "ai_queue5",
            "ai_queue6",
            "ai_queue7",
            "ai_queue8",
            "ai_queue9",
            "ai_queue10",
            "ai_queue11",
            "ai_queue12",
            "ai_queue13",
            "ai_queue14",
            "ai_queue15",
            "ai_queue16",
            "ai_queue17",
            "ai_queue18",
            "ai_queue19",
            "ai_queue20",
            "ai_timer",
            "ai_spawn",
            "ai_despawn",
            "if_button",
            "if_button1",
            "if_button2",
            "if_button3",
            "if_button4",
            "if_button5",
            "if_buttond",
            "if_close",
            "inv_button1",
            "inv_button2",
            "inv_button3",
            "inv_button4",
            "inv_button5",
            "inv_buttond",
            "login",
            "logout",
            "timer",
            "softtimer",
            "queue",
            "walktrigger",
            "mapzone",
            "mapzoneexit",
            "zone",
            "zoneexit",
            "tutorial",
            "advancestat",
            "changestat",
        ]
        .iter()
        .copied()
        .collect();

        // Triggers that allow returns
        let returns_allowed: std::collections::HashSet<&str> =
            ["proc", "clientscript", "command", "logout"]
                .iter()
                .copied()
                .collect();

        // Track registered scripts for redeclaration detection
        let mut registered_scripts: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for (path, file) in &all_files {
            for script in &file.scripts {
                if script.trigger == "command" {
                    continue;
                }

                // Validate trigger type
                if !valid_triggers.contains(script.trigger.as_str()) {
                    diagnostics.warning(
                        path.clone(),
                        script.line,
                        0,
                        msg::fmt(msg::SCRIPT_TRIGGER_INVALID, &[&script.trigger]),
                        crate::diagnostics::Phase::SymbolRegistration,
                    );
                }

                // Check for script redeclaration (same trigger+name)
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

                // Validate return types against trigger
                if !script.return_types.is_empty()
                    && !returns_allowed.contains(script.trigger.as_str())
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
    println!("  Registered {} scripts", registry.scripts.len());

    // Validate that byte-keyed trigger subjects resolve. A `[trigger,name]`
    // header whose trigger has a byte but whose entity name doesn't resolve
    // would silently produce `lookup_key = -1` — the script compiles, loads,
    // and never dispatches. Catch that here.
    for (path, file) in &all_files {
        for script in &file.scripts {
            if let Some(msg) = Compiler::validate_trigger_subject(
                &script.trigger,
                &script.name,
                &registry,
            ) {
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
    println!("Phase 3: Type checking...");
    {
        let mut type_checker = TypeChecker::new(&registry);
        for (path, file) in &all_files {
            type_checker.check_file(file, path);
        }
        diagnostics.merge(type_checker.diagnostics);
    }

    if diagnostics.has_errors() {
        diagnostics.print_all();
        return Err(Box::new(error::CompilerError::TypeError(
            "Type checking failed".to_string(),
        )));
    }

    // Phase 4: Code generation
    println!("Phase 4: Code generation...");
    let mut codegen = Compiler::new(registry);
    let mut compiled_scripts = Vec::new();
    for (path, file) in &all_files {
        for script in &file.scripts {
            // Skip command declarations (engine command type signatures)
            if script.trigger == "command" {
                continue;
            }
            let mut compiled = codegen.compile_script(script);
            let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            let mut source_path = canonical.to_string_lossy().into_owned();
            // Strip Windows extended-length path prefix (\\?\)
            if source_path.starts_with("\\\\?\\") {
                source_path = source_path[4..].to_string();
            }
            // Normalize 'Content' to 'content' to match reference compiler path resolution
            source_path = source_path.replace("\\Content\\", "\\content\\");
            compiled.source_path = source_path;
            compiled_scripts.push(compiled);
        }
    }
    println!("  Generated {} compiled scripts", compiled_scripts.len());

    // Sort by script ID so output order matches the Java compiler's script.pack ordering.
    compiled_scripts.sort_by_key(|s| s.id);

    // Phase 4.5: Pointer checking.
    //
    // Pointer-check findings are emitted at `warn` level: they surface real
    // static-analysis hazards (missing pointers, corruption before use,
    // require-AND-corrupt patterns) but must NOT fail the build. The
    // 2004scape engine enforces active_* and p_active_player at runtime via
    // `ScriptState.pointerCheck`, and the `last_*` / `find_*` family is
    // a compile-time discipline only — none of it warrants an error-severity
    // signal that blocks compilation.
    println!("Phase 4.5: Pointer checking...");
    {
        use crate::pointer_checker::PointerChecker;
        let mut checker = PointerChecker::new(&compiled_scripts, &codegen.registry);
        checker.set_source_cache(&source_cache);
        let pointer_diags = checker.run();
        let ptr_warnings = pointer_diags.warning_count();
        if ptr_warnings > 0 {
            println!("  Pointer check: {} warning(s)", ptr_warnings);
        }
        diagnostics.merge(pointer_diags);
    }

    // Phase 4.6: Lint passes (unused locals, unreachable code).
    //
    // Independent of pointer checking — uses only the compiled
    // instruction stream plus the source cache. Diagnostics emitted at
    // `Severity::Warning`; never influences bytecode or the writer.
    println!("Phase 4.6: Lint checks...");
    {
        let lint_diags = lints::run_lints(&compiled_scripts, Some(&source_cache));
        let lint_warnings = lint_diags.warning_count();
        if lint_warnings > 0 {
            println!("  Lints: {} warning(s)", lint_warnings);
        }
        diagnostics.merge(lint_diags);
    }

    // Phase 5: Write output
    println!("Phase 5: Writing output to {}...", output);
    let writer = ScriptWriter::new(output);
    writer.write_all(&compiled_scripts)?;

    diagnostics.print_all();
    println!("Compilation complete!");

    Ok(())
}

fn collect_files(
    dir: &std::path::Path,
    ext: &str,
    out: &mut Vec<PathBuf>,
) -> Result<(), Box<dyn Error>> {
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
