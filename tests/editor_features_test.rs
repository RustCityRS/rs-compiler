use runec::clientscript;
use runec::lsp::project_from_json;
use runec::project::{Options, Project, Target};
use runec::sources::Scope;
use runec::symbol::SymbolRegistry;
use runec::typechecker::TypeChecker;
use runec::types::Type;
use serde_json::json;
use std::path::Path;

#[test]
fn annotations_preserve_command_documentation() {
    let mut project = Project::new(Target::Rune, SymbolRegistry::new(), Options::default());
    let text = "// Wait for the next tick.\n@signature(fixed)\n[command,pause](int $ticks)\n";
    let source = project.open("engine.rs2", Scope::Rune, 1, text);
    let hover = project.hover(source, text.find("pause]").unwrap()).unwrap();
    assert!(hover.documentation.contains("Wait for the next tick."));
}

#[test]
fn annotation_completion_handles_names_values_and_preserves_label_completion() {
    let mut project = Project::new(Target::Rune, SymbolRegistry::new(), Options::default());
    for (text, cursor, expected) in [
        ("@", 1, "stack"),
        ("@sta", 4, "stack"),
        ("@stack(component_last)", 11, "component_last"),
        ("@stack()", 7, "component_last"),
        ("@signature(en", 13, "enum"),
        ("@signature(pa", 13, "param"),
    ] {
        let source = project.open("engine.cs2", Scope::Client, 1, text);
        let items = project.completions(source, cursor, 20);
        let item = items
            .iter()
            .find(|item| item.label == expected)
            .unwrap_or_else(|| panic!("{text}: {items:?}"));
        let mut completed = text.to_string();
        completed.replace_range(item.replacement.clone(), &item.label);
        assert!(!completed.contains("component_lastponent"));
        assert!(project.completions(source, cursor, 1).len() <= 1);
        project.close(source);
    }
    let text = "[label,stackable]\nreturn;\n[proc,caller]\n@sta";
    let source = project.open("labels.rs2", Scope::Rune, 1, text);
    let items = project.completions(source, text.len(), 20);
    assert!(items.iter().any(|item| item.label == "stackable"));
    assert!(!items.iter().any(|item| item.label == "stack"));
}

#[test]
fn annotation_completion_accepts_spaced_headers_and_multiline_values() {
    let mut project = Project::new(Target::Rune, SymbolRegistry::new(), Options::default());
    for text in [
        "@signature(ho)\n[ command , callback ](string $hook)",
        "@signature(\n    ho\n)\n[ command , callback ](string $hook)",
    ] {
        let source = project.open("spaced-engine.cs2", Scope::Client, 1, text);
        let cursor = text.find("ho").unwrap() + 2;
        let item = project
            .completions(source, cursor, 20)
            .into_iter()
            .find(|item| item.label == "hook_component")
            .unwrap();
        assert_eq!(&text[item.replacement], "ho");
        project.close(source);
    }
}

#[test]
fn annotation_completion_does_not_capture_label_calls_before_commands() {
    let mut project = Project::new(Target::Rune, SymbolRegistry::new(), Options::default());
    for text in [
        "[label,stackable]\nreturn;\n[proc,caller]\n@sta();\n[command,after]",
        "[label,stackable]\nreturn;\n[proc,caller]\n@sta\nreturn;\n[command,after]",
        "@sta\n[proc,caller]",
    ] {
        let source = project.open("annotation-labels.rs2", Scope::Rune, 1, text);
        let cursor = text.find("@sta").unwrap() + 4;
        assert!(
            !project
                .completions(source, cursor, 20)
                .iter()
                .any(|item| item.label == "stack"),
            "{text}"
        );
        project.close(source);
    }
}

#[test]
fn finite_switch_checks_warn_without_changing_open_integer_domains() {
    let mut registry = SymbolRegistry::new();
    registry.set_switch_values(
        Type::ChatFilter,
        [
            (0, "^on".into()),
            (1, "^friends".into()),
            (2, "^off".into()),
        ],
    );
    registry.register_constant("on".into(), Type::Int, Some(0), None);
    registry.register_constant("friends".into(), Type::Int, Some(1), None);
    registry.register_constant("off".into(), Type::Int, Some(2), None);
    registry.register_entity_id_typed_only("option".into(), Type::Boolean, 1);
    registry.register_entity_id("option".into(), Type::Int, 0);
    for (text, missing) in [
        (
            "[proc,test](boolean $flag) switch_boolean($flag) { case true: return; }",
            Some("false"),
        ),
        (
            "[proc,test](boolean $flag) switch_int($flag) { case 0: return; }",
            Some("true"),
        ),
        (
            "[proc,test](boolean $flag) switch_int($flag) { case option: return; }",
            Some("missing true"),
        ),
        (
            "[proc,test](boolean $flag) switch_boolean($flag) { case friends: return; }",
            Some("missing false"),
        ),
        (
            "[proc,test](boolean $flag) switch_boolean($flag) { case false,true: return; }",
            None,
        ),
        (
            "[proc,test](boolean $flag) switch_boolean($flag) { case true: return; case default: return; }",
            None,
        ),
        (
            "[proc,test](chatfilter $flag) switch_chatfilter($flag) { case ^on,^friends: return; }",
            Some("^off"),
        ),
        (
            "[proc,test](int $flag) switch_int($flag) { case 0: return; }",
            None,
        ),
        (
            "[proc,test](string $value) switch_boolean($value) { case true: return; }",
            None,
        ),
    ] {
        let path = Path::new("switch.rs2");
        let file = clientscript::parse(text, path).unwrap();
        let mut checker = TypeChecker::new(&registry);
        checker.check_file(&file, path);
        let warnings = checker.diagnostics.diagnostics();
        let exhaustive = warnings
            .iter()
            .find(|warning| warning.message.starts_with("Non-exhaustive switch"));
        match missing {
            Some(missing) => assert!(
                exhaustive.is_some_and(|warning| warning.message.contains(missing)),
                "{text}: {:?}",
                checker.diagnostics
            ),
            None => assert!(exhaustive.is_none(), "{text}: {:?}", checker.diagnostics),
        }
    }
}

#[test]
fn constant_name_prefixes_do_not_define_closed_switch_domains() {
    let mut project = project_from_json(&json!({
        "constants":[
            {"name":"iftype_layer","type":"int","value":0,"scope":"global"},
            {"name":"iftype_future","type":"int","value":28,"scope":"global"}
        ],
        "sources":[{"uri":"test.rs2","languageId":"rs2","text":"[proc,test](iftype $type) switch_iftype($type) { case ^iftype_layer: return; }"}]
    })).unwrap();
    assert!(project.registry().switch_values(Type::IfType).is_none());
    assert!(
        !project
            .analyze()
            .diagnostics
            .diagnostics()
            .iter()
            .any(|warning| warning.message.starts_with("Non-exhaustive switch"))
    );
}

#[test]
fn effect_and_closed_domain_metadata_are_host_supplied() {
    assert!(
        clientscript::parse(
            "@effect(suspend)\n[command,pause](int $ticks)",
            Path::new("engine.rs2")
        )
        .is_err()
    );
    let mut project = project_from_json(&json!({
        "commands":[{"name":"pause","opcode":9007,"params":["int"],"suspends":true}],
        "switchValues":[{"type":"chatfilter","values":[{"value":7,"name":"private"},{"value":9,"name":"public"}]}],
        "sources":[{"uri":"test.rs2","languageId":"rs2","text":"[proc,test](chatfilter $filter) pause(1); switch_chatfilter($filter) { case 7: return; }"}]
    })).unwrap();
    let source = project.documents().next().unwrap().id;
    assert!(project.script_effects(source)[0].may_suspend);
    assert!(
        project
            .analyze()
            .diagnostics
            .diagnostics()
            .iter()
            .any(|warning| warning.message.contains("missing public"))
    );
}
