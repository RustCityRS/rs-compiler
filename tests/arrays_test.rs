use runec::bytecode::{Opcode, Operand};
use runec::clientscript;
use runec::compiler::Compiler;
use runec::project::{Options, Project, Target};
use runec::sources::Scope;
use runec::symbol::SymbolRegistry;
use runec::writer::encode_script;
use std::path::Path;

#[test]
fn server_arrays_preserve_explicit_slots_and_pack_them_above_the_type() {
    let file = clientscript::parse(
        "[proc,test]\ndef_int $intarray4(3); $intarray4(2) = 7;",
        Path::new("arrays.rs2"),
    )
    .unwrap();
    let registry = SymbolRegistry::new();
    let compiled = Compiler::new(&registry).compile_script(&file.scripts[0]);
    let define = compiled
        .instructions
        .iter()
        .find(|i| i.opcode == Opcode::DefineArray)
        .unwrap();
    assert!(matches!(define.operand, Operand::ArrayDef(4, b'i')));
    let store = compiled
        .instructions
        .iter()
        .find(|i| i.opcode == Opcode::PopArrayInt)
        .unwrap();
    assert!(matches!(store.operand, Operand::Int(4)));
    let bytes = encode_script(&compiled);
    assert!(
        bytes
            .windows(6)
            .any(|bytes| bytes == [0, 44, 0, 4, 0, b'i'])
    );
}

#[test]
fn server_arrays_reject_unsupported_element_types_and_excess_slots() {
    for body in [
        "def_string $items(2);",
        "def_long $items(2);",
        "def_clanforumqfc $items(2);",
        "def_int $intarray5(2);",
        "def_int $a(1); def_int $b(1); def_int $c(1); def_int $d(1); def_int $e(1); def_int $f(1);",
    ] {
        let mut project = Project::new(Target::Rune, SymbolRegistry::new(), Options::default());
        project.open("arrays.rs2", Scope::Rune, 1, format!("[proc,test]\n{body}"));
        let result = project.compile();
        assert!(result.analysis.diagnostics.has_errors(), "{body}");
        assert!(result.artifacts.is_empty(), "{body}");
    }
}
