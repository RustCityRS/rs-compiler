use runec::bytecode::Opcode;
use runec::clientscript::{self, Source, decode::Script, format::Format};
use runec::project::{Options, Project, Target};
use runec::sources::Scope;
use runec::symbol::SymbolRegistry;
use runec::types::Type;
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;

fn registry(component_first: bool) -> SymbolRegistry {
    let mut registry = SymbolRegistry::new();
    let mut params = vec![Type::ClientScript, Type::String, Type::Component];
    if component_first {
        params.rotate_right(1);
    }
    registry.register_command("if_setonvartransmit".into(), 9007, params, vec![]);
    registry.set_command_behavior("if_setonvartransmit", true, Some(true), None);
    registry.set_component_first_command("if_setonvartransmit", component_first);
    registry.register_entity_id("panel:button".into(), Type::Component, (1 << 16) | 2);
    registry.register_game_var("bank_open".into(), Type::Int, 90, "varp".into());
    registry
}

#[test]
fn component_first_hooks_preserve_expanded_quoted_and_clearing_bytes() {
    let format = Format::parse(
        "0=PushConstantInt\n3=PushConstantString\n21=Return\n9007=if_setonvartransmit\n",
    )
    .unwrap();
    for (canonical, legacy) in [
        (
            "if_setonvartransmit(panel:button, \"callback(7, \\\"hello\\\"){bank_open}\");",
            "if_setonvartransmit(\"callback(7, \\\"hello\\\"){bank_open}\", panel:button);",
        ),
        (
            "if_setonvartransmit(panel:button, callback, 7, \"hello\", 90, 1, \"isY\");",
            "if_setonvartransmit(callback, 7, \"hello\", 90, 1, \"isY\", panel:button);",
        ),
        (
            ".if_setonvartransmit(panel:button, null);",
            ".if_setonvartransmit(null, panel:button);",
        ),
    ] {
        let compile = |body, component_first| {
            let text = format!(
                "[proc,entry]\n{body}\n[clientscript,callback](int $value, string $text)\nreturn;\n"
            );
            let source = Source::parse(&text, Path::new("hooks.cs2")).unwrap();
            let mut registry = registry(component_first);
            clientscript::register(
                &source.file,
                &HashMap::from([
                    ("[proc,entry]".into(), 1),
                    ("[clientscript,callback]".into(), 2),
                ]),
                &mut registry,
            )
            .unwrap();
            source.compile(&registry, &format).unwrap()
        };
        assert_eq!(
            compile(canonical, true),
            compile(legacy, false),
            "{canonical}"
        );
    }
}

#[test]
fn component_first_hook_navigation_and_hints_follow_the_quoted_argument() {
    for component_first in [false, true] {
        let mut registry = registry(component_first);
        registry.register_command(
            "if_getlayer".into(),
            2505,
            vec![Type::Component],
            vec![Type::Component],
        );
        registry.set_command_behavior("if_getlayer", true, None, None);
        let mut project = Project::new(Target::Rune, registry, Options::default());
        let args = if component_first {
            "if_getlayer(panel:button), \"callback(7, \\\"hello\\\"){bank_open}\""
        } else {
            "\"callback(7, \\\"hello\\\"){bank_open}\", if_getlayer(panel:button)"
        };
        let text = format!(
            "[clientscript,callback](int $value, string $text)\nreturn;\n[proc,entry]\nif_setonvartransmit({args});\n"
        );
        let source = project.open("hooks.cs2", Scope::Client, 1, text.clone());
        let callback = text.find("callback(").unwrap();
        let definition = project.definition(source, callback).unwrap();
        assert_eq!(definition.source, source);
        assert_eq!(&text[definition.range], "callback");
        assert_eq!(project.references(source, callback, true).len(), 2);
        let signature = project.signature_help(source, callback).unwrap();
        assert_eq!(signature.active_parameter, usize::from(component_first));
        let mut parameters = vec![
            ("hook".into(), Type::ClientScript),
            ("component".into(), Type::Component),
        ];
        if component_first {
            parameters.reverse();
        }
        assert_eq!(signature.parameters, parameters);
    }
}

#[test]
fn component_evaluation_order_requires_valid_metadata_and_ignores_opcode_ids() {
    let format = Format::parse("0=PushConstantInt\n21=Return\n9007=paint\n").unwrap();
    for tags in [
        "@signature(hook_component)\n@stack(component_last)",
        "@stack(component_last)\n@signature(hook_component)",
    ] {
        let text = format!(
            "{tags}\n[command,paint](component $component, clientscript $callback, string $signature)\n"
        );
        let commands = clientscript::commands(&format, &text, Path::new("engine.cs2")).unwrap();
        assert!(commands[0].component_last);
        assert!(matches!(
            commands[0].kind,
            clientscript::CommandKind::HookComponent
        ));
    }
    for declaration in [
        "@stack(unknown)\n[command,paint](component $component, rgb $colour)",
        "@stack(component_last)\n[command,paint](rgb $colour, component $component)",
        "@stack(component_last)\n[command,paint]()",
        "@stack(component_last)\n@stack(component_last)\n[command,paint](component $component)",
    ] {
        assert!(
            clientscript::commands(&format, declaration, Path::new("engine.cs2")).is_err(),
            "{declaration}"
        );
    }
    let mut registry = SymbolRegistry::new();
    registry.register_command("paint".into(), 2101, vec![Type::Component], vec![]);
    registry.set_command_behavior("paint", true, None, None);
    assert!(!registry.component_first_command("paint"));
    registry.set_component_first_command("paint", true);
    assert!(registry.component_first_command(".paint"));
    registry.set_command_behavior("paint", false, None, None);
    registry.set_command_behavior("paint", true, None, None);
    assert!(!registry.component_first_command("paint"));
    registry.set_component_first_command("paint", true);
    registry.register_command("paint".into(), 2101, vec![Type::Component], vec![]);
    assert!(!registry.component_first_command("paint"));

    let environment = json!({
        "target":"client",
        "opcodes":"0=PushConstantInt\n21=Return\n9007=paint\n",
        "commands":[{"name":"paint","opcode":9007,"params":["component","rgb"],"returns":[],"stack":"component_last"}],
        "sources":[{"uri":"paint.cs2","languageId":"cs2","text":"[clientscript,entry]\npaint(65538, 16711680);\n"}]
    });
    let mut project = runec::lsp::project_from_json(&environment).unwrap();
    let compilation = project.compile();
    assert!(
        !compilation.analysis.diagnostics.has_errors(),
        "{:?}",
        compilation.analysis.diagnostics
    );
    let script = Script::decode(&compilation.artifacts[0].bytes, &format).unwrap();
    let values = script
        .operations
        .iter()
        .zip(&script.int_operands)
        .filter_map(|(opcode, value)| (*opcode == Opcode::PushConstantInt).then_some(*value))
        .collect::<Vec<_>>();
    assert_eq!(values, [16711680, 65538]);
    for (field, value) in [
        ("stack", json!("unknown")),
        ("stack", json!(true)),
        ("params", json!(["rgb", "component"])),
    ] {
        let mut invalid = environment.clone();
        invalid["commands"][0][field] = value;
        assert!(runec::lsp::project_from_json(&invalid).is_err());
    }
    let mut invalid = environment;
    invalid["target"] = json!("rune");
    assert!(runec::lsp::project_from_json(&invalid).is_err());
}
