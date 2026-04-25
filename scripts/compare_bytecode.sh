#!/bin/bash
# compare_bytecode.sh - Compare Rust compiler output against Java reference compiler
#
# Usage: ./scripts/compare_bytecode.sh [--rebuild] [--verbose] [--diff SCRIPT_ID]
#
# This script:
# 1. Runs the Java reference compiler (RuneScriptCompiler.jar) via npm run build
# 2. Runs the Rust compiler on the same source scripts
# 3. Compares individual script bytecodes from script.dat/script.idx
# 4. Reports parity statistics

set -e

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
JAVA_OUTPUT="$REPO_ROOT/2004scape/data/pack/server"
RUST_OUTPUT="/tmp/rust_compiler_output"
SCRIPTS_DIR="$REPO_ROOT/2004scape/data/src/scripts"

REBUILD=false
VERBOSE=false
DIFF_SCRIPT=""

for arg in "$@"; do
    case "$arg" in
        --rebuild) REBUILD=true ;;
        --verbose) VERBOSE=true ;;
        --diff)    shift; DIFF_SCRIPT="$1" ;;
        --diff=*)  DIFF_SCRIPT="${arg#--diff=}" ;;
    esac
    shift 2>/dev/null || true
done

# Step 1: Generate Java reference output if needed
if [ ! -f "$JAVA_OUTPUT/script.dat" ] || [ "$REBUILD" = true ]; then
    echo "==> Building Java reference output (npm run build)..."
    cd "$REPO_ROOT/2004scape"
    npm run build 2>&1 | tail -5
    cd "$REPO_ROOT"
else
    echo "==> Using existing Java reference output"
fi

# Step 2: Build and run Rust compiler
echo "==> Building Rust compiler..."
cargo build --release 2>&1 | tail -3

echo "==> Running Rust compiler..."
rm -rf "$RUST_OUTPUT"
"$REPO_ROOT/target/release/runescript_compiler" compile \
    -s "$SCRIPTS_DIR" \
    -o "$RUST_OUTPUT" 2>&1 | tail -5

# Step 3: Compare using Python (more portable for binary parsing)
echo "==> Comparing bytecode..."
python3 "$REPO_ROOT/scripts/compare_scripts.py" \
    "$JAVA_OUTPUT/script.dat" "$JAVA_OUTPUT/script.idx" \
    "$RUST_OUTPUT/script.dat" "$RUST_OUTPUT/script.idx" \
    ${VERBOSE:+--verbose} \
    ${DIFF_SCRIPT:+--diff "$DIFF_SCRIPT"}
