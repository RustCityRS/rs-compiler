pub mod arguments;
pub mod bytecode;
pub mod clientscript;
pub mod compiler;
pub mod diagnostic_messages;
pub mod diagnostics;
pub mod environment;
pub mod error;
pub mod ide;
pub mod lexer;
pub mod lints;
pub mod lsp;
#[cfg(feature = "memprof")]
pub mod memprof;
pub mod parser;
pub mod pointer;
pub mod pointer_checker;
pub mod project;
pub mod sources;
pub mod symbol;
pub mod symloader;
pub mod text;
pub mod token;
pub mod trigger_table;
pub mod typechecker;
pub mod types;
pub mod writer;

pub mod filesystem;

pub(crate) use filesystem::find_testscript_line;
pub use filesystem::{
    ScriptIds, SourceProvider, compile, compile_memory, compile_memory_with_ids,
    compile_memory_with_options, compile_memory_with_tests, compile_with_options, lint,
};

mod parallel;
pub(crate) use parallel::{parallel_chunks, parallel_map, parallel_map_balanced};
