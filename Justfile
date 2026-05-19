# Justfile — shared entry point for local + CI workflows.
# Run `just` with no args to list all recipes.

export CARGO_TERM_COLOR := "always"

# Default: show available recipes.
default:
    @just --list

# ── Build ─────────────────────────────────────────────────────────────
# Debug build (all targets: bin, lib, tests).
build:
    cargo build --all-targets

# Release build.
build-release:
    cargo build --all-targets --release

# Fast type-check without producing binaries.
check:
    cargo check --all-targets

# Build and run the compiler binary.
run *ARGS:
    cargo run -- {{ ARGS }}

# ── Format ────────────────────────────────────────────────────────────
# Apply rustfmt in place.
fmt:
    cargo fmt

# Fail if any files are mis-formatted (CI gate).
fmt-check:
    cargo fmt -- --check

# ── Lint ──────────────────────────────────────────────────────────────
# Clippy.
lint:
    cargo clippy --all-targets

# Strict clippy: treat any warning as an error.
lint-strict:
    cargo clippy --all-targets -- -D warnings

# Auto-apply clippy suggestions (works on a dirty tree).
lint-fix:
    cargo clippy --all-targets --fix --allow-dirty --allow-staged

# Apply every available auto-fix: rustc → clippy → fmt.
fix:
    cargo fix --all-targets --allow-dirty --allow-staged
    cargo clippy --all-targets --fix --allow-dirty --allow-staged
    cargo fmt

# ── Test ──────────────────────────────────────────────────────────────
# Run the test suite.
test:
    cargo test

# ── Aggregate ─────────────────────────────────────────────────────────
# Full CI pipeline, mirroring .github/workflows/ci.yml.
ci: fmt-check check lint-strict test

# Everything worth running before you push.
pre-commit: fmt check lint-strict test

# Wipe build artifacts.
clean:
    cargo clean
