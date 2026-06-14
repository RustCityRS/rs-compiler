use clap::{Parser as ClapParser, Subcommand};
use std::error::Error;
use std::path::PathBuf;

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
        /// Low-memory mode: re-compile scripts on demand instead of holding all
        /// compiled bytecode resident (~16 MB heap / ~32 MB working set vs
        /// ~93 / ~111), at roughly 6x the compile time. Output is byte-identical.
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
