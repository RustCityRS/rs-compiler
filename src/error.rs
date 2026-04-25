use crate::token::Token;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;

/// Top-level error returned from `main()`. User-facing diagnostics flow
/// through `DiagnosticsCollector` and never go through this Display impl —
/// these variants only carry a process-exit cause for `Result<(), Box<dyn Error>>`.
#[derive(Debug)]
pub enum CompilerError {
    FileNotFound(String),
    IO(std::io::Error),
    TypeError(String),
    CodeGenError(String),
}

impl Error for CompilerError {}

impl fmt::Display for CompilerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CompilerError::IO(err) => writeln!(f, "IOError: {}", err),
            CompilerError::FileNotFound(err) => writeln!(f, "FileNotFoundError: {}", err),
            CompilerError::TypeError(err) => writeln!(f, "TypeError: {}", err),
            CompilerError::CodeGenError(err) => writeln!(f, "CodeGenError: {}", err),
        }
    }
}

/// Lexer error payload. Converted into `Diagnostic` at the call site
/// (`main.rs`); never rendered directly. No `Display` impl by design — all
/// user-facing formatting lives in `Diagnostic`.
#[derive(Debug)]
pub struct LexingError {
    pub path: PathBuf,
    pub message: String,
    pub line: usize,
    pub position: usize,
}

impl Error for LexingError {}

impl fmt::Display for LexingError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // Stub — kept only to satisfy `Error`. Formatting goes through
        // Diagnostic; if you see this output, a phase forgot to convert.
        write!(f, "{} ({}:{})", self.message, self.line, self.position)
    }
}

impl LexingError {
    pub fn new(path: PathBuf, message: String, line: usize, position: usize) -> Self {
        Self {
            path,
            message,
            line,
            position,
        }
    }
}

/// Parser error payload. Converted into `Diagnostic` at the call site
/// (`main.rs`); never rendered directly. See `LexingError`.
#[derive(Debug)]
pub struct SyntaxError {
    pub path: PathBuf,
    pub message: String,
    pub line: usize,
    pub position: usize,
}

impl Error for SyntaxError {}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} ({}:{})", self.message, self.line, self.position)
    }
}

impl SyntaxError {
    pub fn from_token(path: PathBuf, token: &Token, message: String) -> Self {
        Self {
            path,
            message,
            line: token.line,
            position: token.column,
        }
    }
}
