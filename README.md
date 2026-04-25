# RuneScript Compiler (RSC)

A compiler for RuneScript

## Installation

### Prerequisites
- [Rust](https://www.rust-lang.org/tools/install)
- Git

### Windows
```powershell
# Clone the repository
git clone https://github.com/yourusername/runescript-compiler.git
cd runescript-compiler

# Run the installation script (requires PowerShell)
powershell -ExecutionPolicy Bypass -File install.ps1
```

### Linux/macOS/Git Bash
```bash
# Clone the repository
git clone https://github.com/yourusername/runescript-compiler.git
cd runescript-compiler

# Run the installation script
chmod +x install.sh
./install.sh
```

The installation script will:
1. Build the compiler in release mode
2. Install it to `~/.rsc/bin` (or `%USERPROFILE%\.rsc\bin` on Windows)
3. Add the installation directory to your PATH
4. Create an `rsc` alias
5. Update your shell configuration:
   - For Bash: Updates `.bash_profile` or `.bashrc`
   - For Zsh: Updates `.zshrc`
   - For Windows PowerShell: Updates PowerShell profile

After installation, restart your terminal or source your configuration file:
```bash
# The script will tell you which file to source, typically one of:
source ~/.bashrc
source ~/.bash_profile
source ~/.zshrc
```

## Usage

The RuneScript Compiler (RSC) provides the following commands:

### Run a Script
```bash
rsc run <script_name> [arguments...]

# Example: Run Fibonacci script with n=10
rsc run fib 10
```

### Analyze 2004Scape Codebase
```bash
rsc 2004
```

### Update RSC
```bash
# Update to the latest version
rsc update

# This will:
# 1. Pull the latest changes from git
# 2. Rebuild the compiler
# 3. Reinstall it to your system
```

### Get Help
```bash
rsc --help
```

## Development

To build from source:
```bash
cargo build
```

To run tests:
```bash
cargo test
```

## License

[MIT License](LICENSE)
