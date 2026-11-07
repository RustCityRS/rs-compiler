//! Run against a real content corpus; optionally compare every client payload.
use std::path::Path;
use std::time::Instant;

#[cfg(feature = "memprof")]
#[global_allocator]
static ALLOCATOR: runec::memprof::Counting = runec::memprof::Counting;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() < 3 {
        return Err("Usage: benchmark SOURCE PACK rune|client [EXPECTED_CLIENT_OUTPUT]".into());
    }
    let start = Instant::now();
    let mut project = runec::filesystem::editor_project(
        Path::new(&args[0]),
        Some(Path::new(&args[1])),
        args[2] == "client",
    )?;
    let loaded = start.elapsed();
    let start = Instant::now();
    let result = project.compile();
    let compiled = start.elapsed();
    if result.analysis.diagnostics.has_errors() {
        for diagnostic in result.analysis.diagnostics.errors() {
            eprintln!("{}", project.render_diagnostic(diagnostic));
        }
        return Err("Compilation failed".into());
    }
    if let Some(expected) = args.get(3) {
        for artifact in &result.artifacts {
            let bytes = std::fs::read(Path::new(expected).join(format!("{}.dat", artifact.id)))?;
            if bytes != artifact.bytes {
                return Err(format!("Output mismatch: {}", artifact.name).into());
            }
        }
    }
    let start = Instant::now();
    project.analyze();
    let cached = start.elapsed();
    let start = Instant::now();
    let symbols = project.workspace_symbols("", usize::MAX).len();
    let indexed = start.elapsed();
    println!(
        "{} scripts, {symbols} symbols; load {loaded:?}, compile {compiled:?}, cached analysis {cached:?}, index {indexed:?}",
        result.artifacts.len()
    );
    #[cfg(feature = "memprof")]
    println!(
        "Peak {:.1} MiB; {} allocations",
        runec::memprof::peak_bytes() as f64 / 1_048_576.0,
        runec::memprof::alloc_count()
    );
    Ok(())
}
