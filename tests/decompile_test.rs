use runec::bytecode::{Opcode, Operand};
use runec::clientscript::{
    self, HookValue, Source,
    decode::Script,
    decompile::{self, Names},
    format::Format,
};
use runec::compiler::Compiler;
use runec::symbol::SymbolRegistry;
use runec::typechecker::TypeChecker;
use runec::types::Type;
use std::collections::HashMap;
use std::path::Path;

struct TestNames;
impl Names for TestNames {
    fn name(&self, kind: &str, id: u32) -> String {
        if kind == "clientscript" && id == 1 {
            "[proc,value]".into()
        } else if kind == "clientscript" && id == 2 {
            "[clientscript,callback]".into()
        } else {
            format!("{kind}_{id}")
        }
    }
}

#[test]
fn quoted_hooks_preserve_arguments_transmits_and_clearing() {
    for explicit in [false, true] {
        for transmits in [false, true] {
            let format = Format::parse(
                "0=PushConstantInt\n3=PushConstantString\n21=Return\n1407=set_hook\n",
            )
            .unwrap();
            let mut registry = SymbolRegistry::new();
            let mut params = vec![Type::ClientScript, Type::String];
            if explicit {
                params.push(Type::Component);
            }
            registry.register_command("set_hook".into(), 1407, params.clone(), vec![]);
            registry.set_command_behavior("set_hook", true, Some(explicit), None);
            registry.set_script_id("clientscript", "callback", 2);
            registry.register_script(
                "callback".into(),
                "clientscript".into(),
                vec![Type::Int, Type::String],
                vec![],
            );
            registry.set_stack_script("clientscript", "callback");
            let component = if explicit { ", 762:95" } else { "" };
            let triggers = if transmits { ", 5, 9, 2" } else { "" };
            let suffix = if transmits { "Y" } else { "" };
            let text = format!(
                "[proc,value]\nset_hook(callback, 7, \"hello\"{triggers}, \"is{suffix}\"{component});\nset_hook(null, \"\"{component});"
            );
            let source = Source::parse(&text, Path::new("hook.cs2")).unwrap();
            clientscript::register(
                &source.file,
                &HashMap::from([("[proc,value]".into(), 1)]),
                &mut registry,
            )
            .unwrap();
            let bytes = source.compile(&registry, &format).unwrap().remove(0).1;
            let script = Script::decode(&bytes, &format).unwrap();
            let scripts = HashMap::from([(1, script)]);
            let commands = vec![clientscript::Command {
                opcode: 1407,
                name: "set_hook".into(),
                params,
                returns: vec![],
                kind: if explicit {
                    clientscript::CommandKind::HookComponent
                } else {
                    clientscript::CommandKind::Hook
                },
            }];
            let text = decompile::decompile(
                &scripts[&1],
                1,
                &TestNames,
                &decompile::signatures(&scripts),
                &HashMap::new(),
                &commands,
                &registry,
            )
            .unwrap();
            assert!(text.starts_with("[proc,value]\n"), "{text}");
            assert!(
                text.contains("set_hook(\"callback(7, \\\"hello\\\")"),
                "{text}"
            );
            assert!(text.contains("set_hook(null"), "{text}");
            if transmits {
                assert!(text.contains("{5, 9}"), "{text}");
            }
            let rebuilt = Source::parse(&text, Path::new("rebuilt.cs2"))
                .unwrap()
                .compile(&registry, &format)
                .unwrap()
                .remove(0)
                .1;
            assert_eq!(bytes, rebuilt);
        }
    }
}

#[test]
fn named_transmit_triggers_compile_as_ids_and_decompile_as_names() {
    for (command, kind, name, id) in [
        ("cc_setonvartransmit", "varp", "bank_open", 90),
        ("cc_setonvarctransmit", "varc", "varclient_192", 192),
        ("cc_setonvarcstrtransmit", "varcs", "varclient_352", 352),
        ("cc_setoninvtransmit", "inv", "tradeoffer", 94),
        ("cc_setonstattransmit", "stat", "magic", 6),
    ] {
        let format = Format::parse(&format!(
            "0=PushConstantInt\n3=PushConstantString\n21=Return\n1407={command}\n"
        ))
        .unwrap();
        let mut registry = SymbolRegistry::new();
        let params = vec![Type::ClientScript, Type::String];
        registry.register_command(command.into(), 1407, params.clone(), vec![]);
        registry.set_command_behavior(command, true, Some(false), None);
        registry.set_script_id("clientscript", "callback", 2);
        registry.register_script("callback".into(), "clientscript".into(), vec![], vec![]);
        if kind.starts_with("var") {
            // String varcs also work without a pack entry, as with %varcs_N.
            if kind != "varcs" {
                registry.register_game_var(name.into(), Type::Int, id, kind.into());
            }
        } else {
            registry.register_entity_id(name.into(), Type::from_name(kind).unwrap(), id);
        }
        let source = Source::parse(
            &format!("[proc,value]\n{command}(\"callback(){{{name}}}\");\n"),
            Path::new("hook.cs2"),
        )
        .unwrap();
        clientscript::register(
            &source.file,
            &HashMap::from([("[proc,value]".into(), 1)]),
            &mut registry,
        )
        .unwrap();
        let script = Compiler::new(&registry).compile_script(&source.file.scripts[0]);
        let ints: Vec<_> = script
            .instructions
            .iter()
            .filter_map(
                |instruction| match (&instruction.opcode, &instruction.operand) {
                    (Opcode::PushConstantInt, Operand::Int(value)) => Some(*value),
                    _ => None,
                },
            )
            .collect();
        assert_eq!(ints, vec![2, id, 1], "{command}");
        let bytes = source.compile(&registry, &format).unwrap().remove(0).1;
        let script = Script::decode(&bytes, &format).unwrap();
        let scripts = HashMap::from([(1, script)]);
        let commands = vec![clientscript::Command {
            opcode: 1407,
            name: command.into(),
            params,
            returns: vec![],
            kind: clientscript::CommandKind::Hook,
        }];
        let text = decompile::decompile(
            &scripts[&1],
            1,
            &TestNames,
            &decompile::signatures(&scripts),
            &HashMap::new(),
            &commands,
            &registry,
        )
        .unwrap();
        assert!(text.contains(&format!("{{{name}}}")), "{text}");
        let rebuilt = Source::parse(&text, Path::new("rebuilt.cs2"))
            .unwrap()
            .compile(&registry, &format)
            .unwrap()
            .remove(0)
            .1;
        assert_eq!(bytes, rebuilt);
    }
}

#[test]
fn a_component_hook_reads_back_from_how_it_is_written() {
    let mut registry = SymbolRegistry::new();
    registry.set_script_id("clientscript", "callback", 2);
    registry.register_script(
        "callback".into(),
        "clientscript".into(),
        vec![Type::Component, Type::Boolean, Type::String, Type::Int],
        vec![],
    );
    registry.register_constant("event_com".into(), Type::Int, Some(i32::MIN + 3), None);
    registry.register_game_var("bank_open".into(), Type::Int, 90, "varp".into());
    let values = vec![
        HookValue::Int(2),
        HookValue::Int(i32::MIN + 3),
        HookValue::Int(1),
        HookValue::String("a<br>b".into()),
        HookValue::Int(-1),
    ];
    let signatures = HashMap::from([(
        2,
        decompile::Signature {
            param_types: vec![Type::Component, Type::Boolean, Type::String, Type::Int],
            ..decompile::Signature::default()
        },
    )]);
    let text = decompile::component_hook_text(
        &values,
        Some(("varp", &[90])),
        &signatures,
        &TestNames,
        &registry,
    )
    .unwrap();
    assert_eq!(text, "callback(event_com, true, \"a<br>b\", -1){bank_open}");
    let (read, transmits) = clientscript::component_hook(&text, Some("varp"), &registry).unwrap();
    assert_eq!(read, values);
    assert_eq!(transmits, [90]);
}

#[test]
fn quoted_runclientscript_resolves_client_ids_and_checks_arguments() {
    let mut registry = SymbolRegistry::new();
    registry.register_command(
        "runclientscript".into(),
        5000,
        vec![Type::ClientScript],
        vec![],
    );
    registry.register_command(
        "runclientscriptvararg".into(),
        5013,
        vec![Type::Int],
        vec![],
    );
    registry.register_clientscript("playerdesign4_submit_name".into(), 3944, vec![Type::Int; 3]);
    let path = Path::new("server.rs2");
    let source = clientscript::parse(
        "[proc,run]\nrunclientscript(\"playerdesign4_submit_name(0, 1, 1)\");",
        path,
    )
    .unwrap();
    let mut checker = TypeChecker::new(&registry);
    checker.check_file(&source, path);
    assert!(
        !checker.diagnostics.has_errors(),
        "{:?}",
        checker.diagnostics
    );
    let mut compiler = Compiler::new(&registry);
    let quoted = compiler.compile_script(&source.scripts[0]);
    assert!(compiler.errors.is_empty(), "{:?}", compiler.errors);
    let ints: Vec<_> = quoted
        .instructions
        .iter()
        .filter_map(
            |instruction| match (&instruction.opcode, &instruction.operand) {
                (Opcode::PushConstantInt, Operand::Int(value)) => Some(*value),
                _ => None,
            },
        )
        .collect();
    assert_eq!(ints, vec![3944, 0, 1, 1]);
    assert_eq!(quoted.strings, vec![Box::<str>::from("iii")]);
    assert!(quoted.instructions.iter().any(|instruction| {
        matches!(
            (&instruction.opcode, &instruction.operand),
            (Opcode::Command, Operand::Int(5013))
        )
    }));
    let bad = clientscript::parse(
        "[proc,run]\nrunclientscript(\"playerdesign4_submit_name(0)\");",
        path,
    )
    .unwrap();
    let mut checker = TypeChecker::new(&registry);
    checker.check_file(&bad, path);
    assert!(checker.diagnostics.has_errors());
    for call in [
        "runclientscript*(3944, 0, 1, 1)",
        ".runclientscript*(playerdesign4_submit_name)(0, 1, 1)",
        "runclientscript(3944)",
        "runclientscript(playerdesign4_submit_name)",
        "runclientscriptvararg(3944, 0, 1, 1)",
        "runclientscript(\"playerdesign4_submit_name(0, 1, 1)\", 0)",
    ] {
        let source = clientscript::parse(&format!("[proc,run]\n{call};"), path).unwrap();
        let mut checker = TypeChecker::new(&registry);
        checker.check_file(&source, path);
        assert!(
            checker
                .diagnostics
                .errors()
                .iter()
                .any(|diagnostic| diagnostic.line == 2
                    && diagnostic.message.contains("one quoted expression")),
            "{call}: {:?}",
            checker.diagnostics
        );
        let mut compiler = Compiler::new(&registry);
        compiler.compile_script(&source.scripts[0]);
        assert!(
            compiler
                .errors
                .iter()
                .any(|error| error.contains("one quoted expression")),
            "{call}: {:?}",
            compiler.errors
        );
    }
    registry.register_clientscript("empty".into(), 10, vec![]);
    registry.register_clientscript("inventory".into(), 11, vec![Type::Int, Type::String]);
    for (call, opcode, strings) in [
        (r#"runclientscript("empty()")"#, 5000, vec![]),
        (r#".runclientscript("empty()")"#, 5000 | (1 << 16), vec![]),
        (
            r#".runclientscript("inventory(1, \"Remove\")")"#,
            5013 | (1 << 16),
            vec!["Remove", "is"],
        ),
    ] {
        let source = clientscript::parse(&format!("[proc,run]\n{call};"), path).unwrap();
        let mut checker = TypeChecker::new(&registry);
        checker.check_file(&source, path);
        assert!(
            !checker.diagnostics.has_errors(),
            "{:?}",
            checker.diagnostics
        );
        let mut compiler = Compiler::new(&registry);
        let script = compiler.compile_script(&source.scripts[0]);
        assert!(compiler.errors.is_empty(), "{:?}", compiler.errors);
        assert_eq!(
            script
                .strings
                .iter()
                .map(|s| s.as_ref())
                .collect::<Vec<_>>(),
            strings
        );
        assert!(script.instructions.iter().any(|instruction| {
            matches!((&instruction.opcode, &instruction.operand),
                (Opcode::Command, Operand::Int(value)) if *value == opcode)
        }));
    }
}

#[test]
fn hook_descriptors_restore_callback_parameter_types() {
    let format =
        Format::parse("0=PushConstantInt\n3=PushConstantString\n21=Return\n1407=set_hook\n")
            .unwrap();
    let mut registry = SymbolRegistry::new();
    let params = vec![Type::ClientScript, Type::String];
    registry.register_command("set_hook".into(), 1407, params.clone(), vec![]);
    registry.set_command_behavior("set_hook", true, Some(false), None);
    let source = Source::parse("[proc,value]\nset_hook(callback, 1, 2, 3, 4, 5, \"zf«ke\");\n[clientscript,callback](int $a, int $b, int $c, int $d, int $e)\n", Path::new("hooks.cs2")).unwrap();
    clientscript::register(
        &source.file,
        &HashMap::from([
            ("[proc,value]".into(), 1),
            ("[clientscript,callback]".into(), 2),
        ]),
        &mut registry,
    )
    .unwrap();
    let original: HashMap<_, _> = source
        .compile(&registry, &format)
        .unwrap()
        .into_iter()
        .map(|(id, bytes)| (id as u32, bytes))
        .collect();
    let scripts = original
        .iter()
        .map(|(&id, bytes)| (id, Script::decode(bytes, &format).unwrap()))
        .collect();
    let commands = vec![clientscript::Command {
        opcode: 1407,
        name: "set_hook".into(),
        params,
        returns: vec![],
        kind: clientscript::CommandKind::Hook,
    }];
    let signatures = decompile::infer_signatures(
        &scripts,
        &[],
        &TestNames,
        &HashMap::new(),
        &commands,
        &mut registry,
    )
    .unwrap();
    assert_eq!(
        signatures[&2].param_types,
        vec![
            Type::Char,
            Type::FontMetrics,
            Type::Vorbis,
            Type::ChatCat,
            Type::ChatPhrase
        ]
    );
    let text = decompile::decompile(
        &scripts[&1],
        1,
        &TestNames,
        &signatures,
        &HashMap::new(),
        &commands,
        &registry,
    )
    .unwrap();
    assert!(
        text.contains("set_hook(\"callback(1, 2, 3, 4, 5)\")"),
        "{text}"
    );
    assert_eq!(
        Source::parse(&text, Path::new("rebuilt.cs2"))
            .unwrap()
            .compile(&registry, &format)
            .unwrap()
            .remove(0)
            .1,
        original[&1]
    );
}

#[test]
fn a_zero_or_one_compare_reads_by_type_and_packs_the_same() {
    let format = Format::parse(
        "0=PushConstantInt\n1=PushVarp\n6=Branch\n7=BranchNot\n21=Return\n86=BranchIfTrue\n87=BranchIfFalse\n",
    )
    .unwrap();
    let source = Source::parse(
        "[proc,value]()\n\
         if (%varp_1 = false) { return; }\n\
         if (%varp_1 = true) { return; }\n\
         if (%varp_2 = 0) { return; }\n\
         if (%varp_2 = 1) { return; }\n\
         if (%varp_2 ! 1) { return; }\n",
        Path::new("virtual:input"),
    )
    .unwrap();
    let ids = HashMap::from([("[proc,value]".into(), 1)]);
    let mut registry = SymbolRegistry::new();
    registry.register_game_var("varp_1".into(), Type::Boolean, 1, "varp".into());
    registry.register_game_var("varp_2".into(), Type::Int, 2, "varp".into());
    clientscript::register(&source.file, &ids, &mut registry).unwrap();
    let original = source.compile(&registry, &format).unwrap().remove(0).1;
    let script = Script::decode(&original, &format).unwrap();
    let count = |opcode: u16| {
        script
            .opcodes
            .iter()
            .filter(|&&other| other == opcode)
            .count()
    };
    assert_eq!((count(87), count(86), count(7)), (2, 2, 1));
    let scripts = HashMap::from([(1, script)]);
    let signatures = decompile::signatures(&scripts);
    let text = decompile::decompile(
        &scripts[&1],
        1,
        &TestNames,
        &signatures,
        &HashMap::new(),
        &[],
        &registry,
    )
    .unwrap();
    for condition in [
        "%varp_1 = false",
        "%varp_1 = true",
        "%varp_2 = 0",
        "%varp_2 = 1",
        "%varp_2 ! 1",
    ] {
        assert!(text.contains(condition), "{condition} in {text}");
    }
    let restored = Source::parse(&text, Path::new("virtual:restored")).unwrap();
    assert_eq!(
        original,
        restored.compile(&registry, &format).unwrap().remove(0).1
    );
}

#[test]
fn decompiler_roundtrips_remapped_opcodes_and_footer_layouts() {
    for definition in [
        "61000=PushConstantInt\n62000=Return\n",
        "61000=PushConstantInt\n62000=Return\n63000=PushConstantLong\n64000=Switch\n",
    ] {
        let format = Format::parse(definition).unwrap();
        let source = Source::parse(
            "[proc,value]()(int)\nreturn(42);",
            Path::new("virtual:input"),
        )
        .unwrap();
        let ids = HashMap::from([("[proc,value]".into(), 1)]);
        let mut registry = SymbolRegistry::new();
        clientscript::register(&source.file, &ids, &mut registry).unwrap();
        let original = source.compile(&registry, &format).unwrap().remove(0).1;
        let script = Script::decode(&original, &format).unwrap();
        let scripts = HashMap::from([(1, script)]);
        let signatures = decompile::signatures(&scripts);
        let text = decompile::decompile(
            &scripts[&1],
            1,
            &TestNames,
            &signatures,
            &HashMap::new(),
            &[],
            &registry,
        )
        .unwrap();
        assert!(!text.contains("@cs2"));
        let restored = Source::parse(&text, Path::new("virtual:restored")).unwrap();
        let bytes = restored.compile(&registry, &format).unwrap().remove(0).1;
        assert_eq!(original, bytes);
        for end in 0..original.len() {
            assert!(
                Script::decode(&original[..end], &format).is_err(),
                "Accepted truncated script at {end}"
            );
        }
    }
}

#[test]
fn malformed_binary_reports_errors_without_panicking_or_unbounded_allocations() {
    let format =
        Format::parse("0=PushConstantInt\n21=Return\n51=Switch\n54=PushConstantLong\n").unwrap();
    let mut state = 1u32;
    for len in 0..512 {
        let bytes = (0..len)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 24) as u8
            })
            .collect::<Vec<_>>();
        let _ = Script::decode(&bytes, &format);
    }
}
