use clap::{Parser as ClapParser, Subcommand};
use runec::clientscript;
use std::error::Error;
use std::fs;
use std::panic::resume_unwind;
use std::path::PathBuf;
use std::thread;

#[cfg(feature = "memprof")]
#[global_allocator]
static GLOBAL: runec::memprof::Counting = runec::memprof::Counting;

#[derive(ClapParser)]
#[command(author, version, about = "RuneScript Compiler")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Serve compiler diagnostics and editor features over stdio LSP
    Lsp {
        #[arg(long)]
        source: Option<PathBuf>,
        #[arg(long)]
        pack: Option<PathBuf>,
        #[arg(long)]
        client: bool,
    },
    /// Compile .cs2 sources to individual cache script files
    CompileClient {
        #[arg(short, long)]
        source: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        pack: PathBuf,
    },
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
        /// Trade compilation time for lower peak memory by recompiling scripts
        /// on demand. Output is byte-identical to normal compilation.
        #[arg(long, visible_alias = "recompile", default_value_t = false)]
        low_mem: bool,
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
}

fn main() -> Result<(), Box<dyn Error>> {
    match thread::Builder::new()
        .name("runec".into())
        .stack_size(8 << 20)
        .spawn(|| run().map_err(|error| error.to_string()))?
        .join()
    {
        Ok(result) => result.map_err(Into::into),
        Err(panic) => resume_unwind(panic),
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Lsp {
            source,
            pack,
            client,
        } => {
            if let Some(source) = source {
                runec::lsp::serve_with_loader(
                    None,
                    Some(Box::new(move || {
                        runec::filesystem::editor_project(&source, pack.as_deref(), client)
                            .map_err(|error| error.to_string())
                    })),
                )?;
            } else {
                runec::lsp::serve(None)?;
            }
        }
        Commands::CompileClient {
            source,
            output,
            pack,
        } => {
            let compiled = clientscript::compile_directory(&source, &pack)?;
            fs::create_dir_all(&output)?;
            for (id, bytes) in compiled {
                fs::write(output.join(format!("{id}.dat")), bytes)?;
            }
        }
        Commands::Compile {
            source,
            output,
            pack,
            lint,
            low_mem,
        } => {
            let scripts_dir = PathBuf::from(source);
            let pack_dir = pack.map(PathBuf::from);
            let output_dir = PathBuf::from(output);
            runec::compile_with_options(
                &scripts_dir,
                pack_dir.as_deref(),
                &output_dir,
                lint,
                low_mem,
            )?;
        }
        Commands::Lint { source, pack } => {
            let scripts_dir = PathBuf::from(source);
            let pack_dir = pack.map(PathBuf::from);
            runec::lint(&scripts_dir, pack_dir.as_deref())?;
        }
    }

    #[cfg(feature = "memprof")]
    tracing::info!(
        "[mem] PEAK HEAP {:.1} MB across {} allocations",
        runec::memprof::peak_bytes() as f64 / 1_048_576.0,
        runec::memprof::alloc_count(),
    );

    Ok(())
}
