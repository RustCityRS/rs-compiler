extern crate core;

use clap::{Parser as ClapParser, Subcommand};
use std::error::Error;
use std::path::PathBuf;

mod bytecode;
mod compiler;
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
        source: String,
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
        source: String,
        /// Pack directory (overrides auto-detection)
        #[arg(long)]
        pack: Option<String>,
    },
    /// Update the RuneScript Compiler to the latest version
    Update,
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Compile {
            source,
            output,
            pack,
            lint,
        } => {
            let scripts_dir = PathBuf::from(source);
            let pack_dir = pack.map(PathBuf::from);
            let output_dir = PathBuf::from(output);
            runec::compile(&scripts_dir, pack_dir.as_deref(), &output_dir, lint)?;
        }
        Commands::Lint { source, pack } => {
            let scripts_dir = PathBuf::from(source);
            let pack_dir = pack.map(PathBuf::from);
            runec::lint(&scripts_dir, pack_dir.as_deref())?;
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

            println!("Updating RuneScript Compiler...");

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
                    .status()?;
            } else {
                std::process::Command::new("sh")
                    .arg(install_script)
                    .status()?;
            }
            println!("Update complete!");
        }
    }

    Ok(())
}
