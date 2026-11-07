use runec::bytecode::{Opcode, Operand};
use runec::clientscript::{self, format::Format};
use runec::compiler::Compiler;
use runec::symbol::SymbolRegistry;
use runec::types::Type;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process;

fn format(push: u16, ret: u16) -> Format {
    Format::parse(&format!("{push}=PushConstantInt\n{ret}=Return\n")).unwrap()
}

#[test]
fn client_frame_defaults_keep_explicit_initializers_and_loop_resets() {
    let format = Format::parse("0=PushConstantInt\n6=Branch\n10=BranchGreaterThan\n21=Return\n33=PushIntLocal\n34=PopIntLocal\n").unwrap();
    let path = Path::new("defaults.gs2");
    let file = clientscript::parse(
        "[proc,defaults](int $input)(int)\n\
        def_int $zero; def_int $explicit = 0; def_obj $none;\n\
        while ($input > 0) { def_int $reset; $input = 0; }\n\
        return($zero);",
        path,
    )
    .unwrap();
    let mut registry = SymbolRegistry::new();
    clientscript::register(
        &file,
        &HashMap::from([("[proc,defaults]".into(), 1)]),
        &mut registry,
    )
    .unwrap();
    let bytes = clientscript::compile(&file, path, &registry, &format)
        .unwrap()
        .remove(0)
        .1;
    let client = clientscript::decode::Script::decode(&bytes, &format).unwrap();
    let stores: Vec<_> = client
        .operations
        .iter()
        .zip(&client.int_operands)
        .filter_map(|(op, slot)| (*op == Opcode::PopIntLocal).then_some(*slot))
        .collect();
    assert_eq!(stores, [2, 3, 4, 0]);
    assert_eq!(client.local_int_count, 5);
    assert_eq!(
        client
            .operations
            .iter()
            .filter(|&&op| op == Opcode::Return)
            .count(),
        2
    );
    let server = Compiler::new(&registry).compile_script(&file.scripts[0]);
    let stores: Vec<_> = server
        .instructions
        .iter()
        .filter(|instruction| instruction.opcode == Opcode::PopIntLocal)
        .map(|instruction| match instruction.operand {
            Operand::Int(slot) => slot,
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(stores, [1, 2, 3, 4, 0]);
}

#[test]
fn nested_sources_load_engine_and_constants_from_the_source_directory() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("clientscript-layout-{}", process::id()));
    let source = root.join("sources");
    let pack = root.join("registries");
    let files = [
        ("sources/engine.cs2", "[command,print](int $value)\n"),
        ("sources/cs2.constant", "^answer = 42\n"),
        (
            "sources/math/proc/[proc,math_answer].cs2",
            "[proc,math_answer]()(int) return(^answer);",
        ),
        (
            "sources/demo/script/[clientscript,demo_open].cs2",
            "[clientscript,demo_open]() print(~math_answer);",
        ),
        (
            "sources/unpack/[proc,clientscript_12].cs2",
            "[proc,clientscript_12]() print(^answer);",
        ),
        (
            "registries/clientscript.pack",
            "7=[proc,math_answer]\n8=[clientscript,demo_open]\n",
        ),
        (
            "registries/cs2_command.pack",
            "0=PushConstantInt\n21=Return\n40=GosubWithParams\n900=print\n",
        ),
    ];
    for (path, text) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let sources = clientscript::load_sources(&source).unwrap();
    assert_eq!(sources.len(), 3);
    let original = clientscript::compile_directory(&source, &pack).unwrap();
    assert_eq!(original.keys().copied().collect::<Vec<_>>(), [7, 8, 12]);
    fs::write(source.join("cs2.constant"), "^answer = 84\n").unwrap();
    let edited = clientscript::compile_directory(&source, &pack).unwrap();
    assert_ne!(original[&7], edited[&7]);
    assert_eq!(original[&8], edited[&8]);
    assert_ne!(original[&12], edited[&12]);
    for (path, _) in files {
        fs::remove_file(root.join(path)).unwrap();
    }
    for path in [
        "sources/math/proc",
        "sources/math",
        "sources/demo/script",
        "sources/demo",
        "sources/unpack",
        "sources",
        "registries",
        "",
    ] {
        fs::remove_dir(root.join(path)).unwrap();
    }
}

#[test]
fn supplied_definitions_control_core_and_arithmetic_opcodes() {
    let path = Path::new("arithmetic.cs2");
    let file = clientscript::parse("[proc,sum]()(int) return(calc(20 + 22));", path).unwrap();
    let mut registry = SymbolRegistry::new();
    clientscript::register(
        &file,
        &HashMap::from([("[proc,sum]".into(), 4)]),
        &mut registry,
    )
    .unwrap();
    registry.register_command(
        "add".into(),
        900,
        vec![Type::Int, Type::Int],
        vec![Type::Int],
    );
    let first = clientscript::compile(&file, path, &registry, &format(100, 101))
        .unwrap()
        .remove(0)
        .1;
    assert_eq!(
        &first[1..19],
        &[
            0, 100, 0, 0, 0, 20, 0, 100, 0, 0, 0, 22, 3, 132, 0, 0, 101, 0
        ]
    );
    registry.register_command(
        "add".into(),
        901,
        vec![Type::Int, Type::Int],
        vec![Type::Int],
    );
    let second = clientscript::compile(&file, path, &registry, &format(200, 201))
        .unwrap()
        .remove(0)
        .1;
    assert_eq!(
        &second[1..19],
        &[
            0, 200, 0, 0, 0, 20, 0, 200, 0, 0, 0, 22, 3, 133, 0, 0, 201, 0
        ]
    );
}

#[test]
fn shared_compiler_uses_long_comparisons_and_index_before_array_value() {
    let file = clientscript::parse(
        "[proc,test](long $a, long $b)\n\
        def_int $items(5); $items(2) = 7; if ($a < $b) { return; }",
        Path::new("shared.rs2"),
    )
    .unwrap();
    let registry = SymbolRegistry::new();
    let compiled = Compiler::new(&registry).compile_script(&file.scripts[0]);
    let ops: Vec<_> = compiled
        .instructions
        .iter()
        .filter(|instruction| instruction.opcode != Opcode::LineNumber)
        .collect();
    assert!(
        ops.iter()
            .any(|instruction| instruction.opcode == Opcode::LongBranchLessThan)
    );
    let store = ops
        .iter()
        .position(|instruction| instruction.opcode == Opcode::PopArrayInt)
        .unwrap();
    assert!(matches!(ops[store - 2].operand, Operand::Int(2)));
    assert!(matches!(ops[store - 1].operand, Operand::Int(7)));
}

#[test]
fn multiple_return_values_can_be_forwarded_to_a_call_or_return() {
    let path = Path::new("calls.cs2");
    let file = clientscript::parse(
        "[proc,pair]()(int,string,long) return(1,r\"ok\",2L);\n\
        [proc,forward]()(int,string,long) return(~pair());\n\
        [proc,sink](int $i,string $s,long $l) return;\n\
        [proc,caller]() ~sink(~pair());",
        path,
    )
    .unwrap();
    let mut registry = SymbolRegistry::new();
    let ids = ["pair", "forward", "sink", "caller"]
        .into_iter()
        .enumerate()
        .map(|(i, name)| (format!("[proc,{name}]"), i as i32))
        .collect();
    clientscript::register(&file, &ids, &mut registry).unwrap();
    let mut checker = runec::typechecker::TypeChecker::new(&registry);
    checker.check_file(&file, path);
    assert!(
        !checker.diagnostics.has_errors(),
        "{:?}",
        checker.diagnostics.errors()
    );
}

#[test]
fn input_format_rejects_missing_operations_and_bad_definitions() {
    for text in [
        "bogus=Return",
        "21=",
        "21=Return\n21=mes",
        "21=Return\n22=Return",
    ] {
        assert!(Format::parse(text).is_err(), "Accepted {text}");
    }
    let file = clientscript::parse("[proc,test]() return;", Path::new("test.cs2")).unwrap();
    let script = Compiler::new(&SymbolRegistry::new()).compile_script(&file.scripts[0]);
    let format = Format::parse("100=mes\n").unwrap();
    assert!(
        clientscript::encode(&script, &format)
            .unwrap_err()
            .to_string()
            .contains("Return")
    );
}

#[test]
fn instruction_features_determine_client_footer_without_a_revision() {
    let path = Path::new("footer.cs2");
    let file = clientscript::parse("[proc,test](long $unused)", path).unwrap();
    let registry = SymbolRegistry::new();
    let script = Compiler::new(&registry).compile_script(&file.scripts[0]);
    let basic = Format::parse("900=Return\n1000=mes\n").unwrap();
    assert!(
        clientscript::encode(&script, &basic)
            .unwrap_err()
            .to_string()
            .contains("Long locals")
    );
    let extended =
        Format::parse("900=Return\n901=PushConstantLong\n902=Switch\n1000=mes\n").unwrap();
    let encoded = clientscript::encode(&script, &extended).unwrap();
    assert_eq!(&encoded[8..20], &[0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1]);
    assert_eq!(&encoded[20..], &[0, 0, 1]);
    let file = clientscript::parse("[proc,test]()", path).unwrap();
    let script = Compiler::new(&registry).compile_script(&file.scripts[0]);
    let encoded = clientscript::encode(&script, &basic).unwrap();
    assert_eq!(encoded.len(), 16);
    assert_eq!(basic.commands.get(&1000).map(String::as_str), Some("mes"));
}

#[test]
fn tuple_assignments_resolve_typed_references_and_reject_bad_counts() {
    let path = Path::new("assignments.cs2");
    let mut registry = SymbolRegistry::new();
    registry.register_entity_id("shared".into(), Type::Obj, 10);
    registry.register_entity_id("shared".into(), Type::Graphic, 20);
    let source = "[proc,test]() def_obj $item; def_graphic $image; $item, $image = shared, shared;";
    let file = clientscript::parse(source, path).unwrap();
    let compiled = Compiler::new(&registry).compile_script(&file.scripts[0]);
    let pushes: Vec<_> = compiled
        .instructions
        .iter()
        .filter(|op| op.opcode == Opcode::PushConstantInt)
        .map(|op| op.operand.clone())
        .collect();
    assert!(matches!(pushes[pushes.len() - 2], Operand::Int(10)));
    assert!(matches!(pushes[pushes.len() - 1], Operand::Int(20)));
    for values in ["1", "1, 2, 3", "1, \"wrong\""] {
        let file = clientscript::parse(&source.replace("shared, shared", values), path).unwrap();
        let mut checker = runec::typechecker::TypeChecker::new(&registry);
        checker.check_file(&file, path);
        assert!(checker.diagnostics.has_errors(), "Accepted {values}");
    }
    registry.register_command("row".into(), 1, vec![Type::Int], Vec::new());
    let file = clientscript::parse(&source.replace("shared, shared", "row(0)"), path).unwrap();
    let mut checker = runec::typechecker::TypeChecker::new(&registry);
    checker.check_file(&file, path);
    assert!(
        !checker.diagnostics.has_errors(),
        "{:?}",
        checker.diagnostics.errors()
    );
}
