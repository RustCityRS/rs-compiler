# RuneScript Compiler (RSC)

A compiler for RuneScript (.rs2), the scripting language used by RuneScape. Part of the rs-server workspace.

## Installation

### Prerequisites
- [Rust](https://www.rust-lang.org/tools/install)

### Windows (PowerShell)
```powershell
cd rs-compiler
powershell -ExecutionPolicy Bypass -File install.ps1
```

### Linux/macOS/Git Bash
```bash
cd rs-compiler
chmod +x install.sh
./install.sh
```

The installation script will:
1. Build the compiler in release mode (`cargo build --release`)
2. Install it to `~/.rsc/bin` (or `%USERPROFILE%\.rsc\bin` on Windows)
3. Add the installation directory to your PATH
4. Create an `rsc` alias

After installation, restart your terminal or source your shell config.

## Usage

### Compile Scripts
```bash
rsc compile -s content/scripts -o data/pack/server

# With explicit pack directory
rsc compile -s content/scripts -o data/pack/server --pack content/pack
```

Compilation phases:
1. **Parsing** - Lexes and parses all .rs2 files
2. **Symbol registration** - Registers scripts, loads pack files, constants, engine commands
3. **Type checking** - Validates types across all scripts
4. **Code generation** - Emits bytecode
5. **Pointer checking** - Static analysis for active-entity pointer hazards (warnings only)
6. **Lint checks** - Unused locals, unreachable code (warnings only)
7. **Write output** - Writes script.dat/script.idx to the output directory

### Analyze 2004Scape Codebase
```bash
rsc 2004
```

### Update RSC
```bash
rsc update
```

### Manage Configuration
```bash
rsc config show    # Show current RC file
rsc config edit    # Open RC file in $EDITOR
rsc config init    # Initialize a new RC file
rsc config list    # List environment variables and aliases
```

### Get Help
```bash
rsc --help
```

## Development

Build from source (from workspace root):
```bash
cargo build -p rs-compiler
```

Run tests:
```bash
cargo test -p rs-compiler
```

Run directly without installing:
```bash
cargo run -p rs-compiler -- compile -s content/scripts -o data/pack/server
```
