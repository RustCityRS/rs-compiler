extern crate core;

use crate::config::Config;
use clap::{Parser as ClapParser, Subcommand};
use std::error::Error;
use std::path::PathBuf;

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
        /// Run lint passes (unused locals, unreachable code) after compilation
        #[arg(long, default_value_t = false)]
        lint: bool,
    },
    /// Run all analysis passes without writing output
    Lint {
        /// Source directory containing .rs2 files
        #[arg(short, long)]
        source: Option<String>,
        /// Pack directory (overrides auto-detection)
        #[arg(long)]
        pack: Option<String>,
    },
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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    
    let cli = Cli::parse();
    let config = Config::load();

    match cli.command {
        Commands::Compile {
            source,
            output,
            pack,
            lint,
        } => {
            compile_scripts(source, output, pack, lint, &config)?;
        }
        Commands::Lint { source, pack } => {
            lint_scripts(source, pack, &config)?;
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
    lint: bool,
    config: &Config,
) -> Result<(), Box<dyn Error>> {
    let scripts_dir = source
        .map(PathBuf::from)
        .unwrap_or_else(|| config.scripts_dir.clone());
    let pack_dir = pack_override.map(PathBuf::from);
    let output_dir = PathBuf::from(output);
    rs_compiler::compile(&scripts_dir, pack_dir.as_deref(), &output_dir, lint)
}

fn lint_scripts(
    source: Option<String>,
    pack_override: Option<String>,
    config: &Config,
) -> Result<(), Box<dyn Error>> {
    let scripts_dir = source
        .map(PathBuf::from)
        .unwrap_or_else(|| config.scripts_dir.clone());
    let pack_dir = pack_override.map(PathBuf::from);
    rs_compiler::lint(&scripts_dir, pack_dir.as_deref())
}
