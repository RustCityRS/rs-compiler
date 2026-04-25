use std::fmt;
use std::path::{Path, PathBuf};

/// Single source of truth for how paths render in compiler diagnostics.
///
/// - Strips the current working directory prefix when present, so a script
///   path stored as `C:\…\runescape\content\scripts\foo.rs2` (canonicalized
///   by the pointer checker) and one stored as `content/scripts/foo.rs2`
///   (the raw walker output) both render as `content/scripts/foo.rs2`.
/// - Normalizes separators to `/` so output is identical across platforms
///   and stable in test fixtures and CI logs.
///
/// All phases must format paths through this helper.
pub fn format_path(path: &Path) -> String {
    let mut s = path.to_string_lossy().replace('\\', "/");
    if let Ok(cwd) = std::env::current_dir() {
        let cwd_norm = cwd.to_string_lossy().replace('\\', "/");
        if let Some(rest) = s.strip_prefix(&format!("{}/", cwd_norm)) {
            s = rest.to_string();
        } else if s == cwd_norm {
            s = ".".to_string();
        }
    }
    s
}

/// Severity level for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// How safe it is to auto-apply a suggestion. Modelled on rustc's
/// `Applicability`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applicability {
    /// The suggestion is definitely correct; a `--fix` tool can apply it
    /// without human review.
    MachineApplicable,
    /// The suggestion is likely correct but might need a minor tweak
    /// (e.g. disambiguating an identifier). Should be reviewed before
    /// auto-applying.
    MaybeIncorrect,
    /// The suggestion contains `<placeholder>` tokens that a human must
    /// fill in before it compiles.
    HasPlaceholders,
    /// We don't know how confident to be.
    Unspecified,
}

/// A single text replacement attached to a Help block.
///
/// Spans are inclusive line ranges (`(start_line, end_line)`) in
/// `file`. The replacement is the full new text for that span. A
/// "pure insertion" encodes as `start_line == end_line + 1` (insert
/// before `start_line`) with a replacement containing the new lines.
#[derive(Debug, Clone)]
pub struct Suggestion {
    pub file: PathBuf,
    pub line_range: (usize, usize),
    pub replacement: String,
    /// Optional short label, e.g. "at the call site" or "at the label
    /// header".
    pub label: Option<String>,
}

/// A help message attached to a diagnostic. Mirrors rustc's
/// `help:` line with an optional list of concrete suggestions.
#[derive(Debug, Clone)]
pub struct Help {
    pub message: String,
    pub suggestions: Vec<Suggestion>,
    pub applicability: Applicability,
}

/// A single diagnostic message with source location.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub file: PathBuf,
    pub line: usize,
    pub column: usize,
    pub message: String,
    pub severity: Severity,
    pub phase: Phase,
    /// Optional help blocks with suggested fixes. Empty by default —
    /// existing phases do not need to attach anything.
    pub help: Vec<Help>,
}

/// Which compiler phase produced the diagnostic.
///
/// `SymbolRegistration` covers per-script validation that runs after parse
/// but before type checking — trigger validity, redeclaration, return-type
/// allowance, and trigger-subject resolvability. These used to claim
/// `TypeChecking`, which lied about the source of the diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Lexing,
    Parsing,
    SymbolRegistration,
    TypeChecking,
    CodeGeneration,
    PointerCheck,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let severity = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        // Suppress `:0` columns. Most phases (typechecker, symbol
        // registration, pointer checker) don't carry column info — emitting
        // `:0` falsely implies they do. Lexer/parser have real columns and
        // still print them.
        if self.column > 0 {
            write!(
                f,
                "{}: {}\n  --> {}:{}:{}",
                severity,
                self.message,
                format_path(&self.file),
                self.line,
                self.column,
            )
        } else {
            write!(
                f,
                "{}: {}\n  --> {}:{}",
                severity,
                self.message,
                format_path(&self.file),
                self.line,
            )
        }
    }
}

/// Collects diagnostics across all compiler phases.
#[derive(Debug, Clone)]
pub struct DiagnosticsCollector {
    diagnostics: Vec<Diagnostic>,
}

impl DiagnosticsCollector {
    pub fn new() -> Self {
        DiagnosticsCollector {
            diagnostics: Vec::new(),
        }
    }

    pub fn add(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    pub fn error(
        &mut self,
        file: PathBuf,
        line: usize,
        column: usize,
        message: String,
        phase: Phase,
    ) {
        self.add(Diagnostic {
            file,
            line,
            column,
            message,
            severity: Severity::Error,
            phase,
            help: Vec::new(),
        });
    }

    pub fn warning(
        &mut self,
        file: PathBuf,
        line: usize,
        column: usize,
        message: String,
        phase: Phase,
    ) {
        self.add(Diagnostic {
            file,
            line,
            column,
            message,
            severity: Severity::Warning,
            phase,
            help: Vec::new(),
        });
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }

    pub fn has_errors_in_phase(&self, phase: Phase) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error && d.phase == phase)
    }

    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count()
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn errors(&self) -> Vec<&Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect()
    }

    pub fn print_all(&self) {
        for diag in &self.diagnostics {
            eprintln!("{}", diag);
            render_help(diag);
        }
        let errors = self.error_count();
        let warnings = self.warning_count();
        if errors > 0 || warnings > 0 {
            eprintln!("\n{} error(s), {} warning(s) generated.", errors, warnings);
        }
    }

    pub fn clear(&mut self) {
        self.diagnostics.clear();
    }

    pub fn merge(&mut self, other: DiagnosticsCollector) {
        self.diagnostics.extend(other.diagnostics);
    }
}

impl Default for DiagnosticsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Render a diagnostic's attached help blocks beneath its main message,
/// rustc-style. For each Help:
///   - print a `help: <message>` line
///   - for each Suggestion, print a unified-diff-ish window:
///       -  <original line>                  (red in terminals)
///       +  <replacement line>               (green in terminals)
///     prefixed with the file path + line range and the suggestion label.
///   - close with `= note: Applicability: <level>`
///
/// Source text is read lazily from disk; if the read fails we fall back
/// to printing the replacement alone.
fn render_help(diag: &Diagnostic) {
    if diag.help.is_empty() {
        return;
    }
    for help in &diag.help {
        eprintln!("help: {}", help.message);
        for sug in &help.suggestions {
            let path = format_path(&sug.file);
            if let Some(label) = &sug.label {
                eprintln!(
                    "  ┌─ {} {}:{}-{}",
                    label, path, sug.line_range.0, sug.line_range.1
                );
            } else {
                eprintln!("  ┌─ {}:{}-{}", path, sug.line_range.0, sug.line_range.1);
            }

            // Try to read the original lines for a proper before/after.
            let original = std::fs::read_to_string(&sug.file).ok().and_then(|src| {
                let start = sug.line_range.0.saturating_sub(1);
                let end = sug.line_range.1;
                let lines: Vec<&str> = src.lines().collect();
                if start < lines.len() && end <= lines.len() && start < end {
                    Some(lines[start..end].join("\n"))
                } else if start < lines.len() {
                    Some(lines[start].to_string())
                } else {
                    None
                }
            });

            match original {
                Some(orig) => {
                    for l in orig.lines() {
                        eprintln!("  - {}", l);
                    }
                    for l in sug.replacement.lines() {
                        eprintln!("  + {}", l);
                    }
                }
                None => {
                    for l in sug.replacement.lines() {
                        eprintln!("  + {}", l);
                    }
                }
            }
        }
        let appl = match help.applicability {
            Applicability::MachineApplicable => "MachineApplicable (safe to auto-apply)",
            Applicability::MaybeIncorrect => "MaybeIncorrect (review before applying)",
            Applicability::HasPlaceholders => "HasPlaceholders (fill in the <…> tokens)",
            Applicability::Unspecified => "Unspecified",
        };
        eprintln!("  = note: Applicability: {}", appl);
    }
}
