<div align="center">
<pre>
██████╗ ██╗   ██╗███████╗████████╗ ██████╗██╗████████╗██╗   ██╗
██╔══██╗██║   ██║██╔════╝╚══██╔══╝██╔════╝██║╚══██╔══╝╚██╗ ██╔╝
██████╔╝██║   ██║███████╗   ██║   ██║     ██║   ██║    ╚████╔╝ 
██╔══██╗██║   ██║╚════██║   ██║   ██║     ██║   ██║     ╚██╔╝  
██║  ██║╚██████╔╝███████║   ██║   ╚██████╗██║   ██║      ██║   
╚═╝  ╚═╝ ╚═════╝ ╚══════╝   ╚═╝    ╚═════╝╚═╝   ╚═╝      ╚═╝   
</pre>
</div>

----

# RuneScript Compiler (runec)

A compiler for RuneScript (`.rs2`), the scripting language used by RuneScape's game engine. Compiles source scripts into binary `script.dat`/`script.idx` output consumed by the game server.

## Prerequisites

- [Rust 1.95+](https://www.rust-lang.org/tools/install)

## Installation

### Windows (PowerShell)

```powershell
powershell -ExecutionPolicy Bypass -File install.ps1
```

### Linux / macOS / Git Bash

```bash
chmod +x install.sh
./install.sh
```

This builds the compiler in release mode and installs it to `~/.runec/bin` as `runec`. The install script adds the directory to your `PATH` and creates a shell alias. Restart your terminal after installation.

## Usage

### Compiling Scripts

```bash
runec compile -s <source-dir> -o <output-dir>
```

Compiles all `.rs2` files in the source directory and writes `script.dat` and `script.idx` to the output directory.

```bash
# Compile with default output (./data/pack/server)
runec compile -s data/src/scripts

# Specify both source and output
runec compile -s data/src/scripts -o data/pack/server

# Provide an explicit pack directory for symbol resolution
runec compile -s data/src/scripts -o data/pack/server --pack data/pack

# Compile and run lint passes
runec compile -s data/src/scripts -o data/pack/server --lint

# Low-memory mode: recompiles scripts on demand so all bytecode is never
# resident at once (~16 MB heap vs ~93), at roughly 6x the compile time.
# Output is byte-identical. (`--recompile` is an accepted alias.)
runec compile -s data/src/scripts -o data/pack/server --low-mem
```

The compiler runs the following phases in order:

1. **Parse** -- Lexes and parses all `.rs2` files into an AST
2. **Symbol registration** -- Registers script names, loads entity pack files (items, NPCs, interfaces, etc.), engine commands, and constants
3. **Type check** -- Validates types, resolves references, and checks trigger signatures across all scripts
4. **Code generation** -- Emits bytecode from the AST
5. **Pointer check** -- Static analysis for active-entity pointer hazards (warnings)
6. **Lint** *(optional)* -- Detects unused locals and unreachable code (warnings)
7. **Write output** -- Writes compiled `script.dat` and `script.idx`

### Linting Scripts

Run all analysis passes without writing any output:

```bash
runec lint -s <source-dir>

# With explicit pack directory
runec lint -s data/src/scripts --pack data/pack
```

This runs the full pipeline (parse, type-check, codegen, pointer-check, lints) and reports diagnostics. Useful for editor tooling and CI lint gates.

## Performance

`runec` runs its heavy phases — parsing, type-checking, code generation,
pointer analysis, lint passes, and output encoding — in parallel across all
available cores, while producing **byte-for-byte identical** output to a
single-threaded run (verified against the reference compiler on full 225 and
647 script sets). On a 1,100+ file script set it compiles in roughly half a
second — about **6× faster** than 0.2.x on a multi-core machine. Release builds
use fat LTO for additional throughput.

The unused-locals and unreachable-code lint passes are likewise parallel and
allocation-light, running in a few milliseconds even on the full set, with
diagnostics identical to the serial implementation.

Memory footprint is roughly **half** that of earlier releases — a full compile
of the 1,100+ file set peaks near 110 MB of working set — through tighter
bytecode and registry layout and by releasing each phase's scratch as soon as
it is no longer needed, again with byte-identical output.

For memory-constrained environments, `runec compile --low-mem` lowers the peak
much further — to about **16 MB heap / 32 MB working set** — by re-compiling
scripts on demand for the pointer check and writing output file-by-file, so the
full set of compiled bytecode is never resident at once. The tradeoff is roughly
**6× the compile time**; the output is identical.

## Development

### Building from Source

```bash
cargo build
```

### Running Tests

```bash
cargo test
```

### Running without Installing

```bash
cargo run -- compile -s data/src/scripts -o data/pack/server
cargo run -- lint -s data/src/scripts
```

## Library Usage

The compiler is also available as a library crate on [crates.io](https://crates.io/crates/rs-runec).

### Installation

```bash
cargo add rs-runec
```

Or add it to your `Cargo.toml` manually:

```toml
[dependencies]
rs-runec = "0.3.1"
```

### Example

```rust
use std::path::Path;

// Compile scripts and write output
runec::compile(
    Path::new("data/src/scripts"),
    Some(Path::new("data/pack")),
    Path::new("data/pack/server"),
    true, // run lint passes
).unwrap();

// Same, but in low-memory mode (recompiles on demand; ~16 MB heap, ~6x slower)
runec::compile_with_options(
    Path::new("data/src/scripts"),
    Some(Path::new("data/pack")),
    Path::new("data/pack/server"),
    true, // run lint passes
    true, // low_mem
).unwrap();

// Lint only (no output written)
runec::lint(
    Path::new("data/src/scripts"),
    Some(Path::new("data/pack")),
).unwrap();
```

## License

This project is licensed under the [MIT License](LICENSE).
