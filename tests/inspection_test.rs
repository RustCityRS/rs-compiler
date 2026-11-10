use runec::inspection::{call_range, inspect_call, inspect_code};
use runec::symbol::SymbolRegistry;
use runec::types::Type;
use std::ops::Range;
use std::path::Path;

fn registry(params: Vec<Type>) -> SymbolRegistry {
    let mut registry = SymbolRegistry::new();
    registry.register_script("main".into(), "proc".into(), params, vec![]);
    registry.set_stack_script("proc", "main");
    registry
}

#[test]
fn broad_code_selections_inspect_the_whole_containing_script() {
    let mut registry = registry(vec![]);
    registry.register_command("mes".into(), 71, vec![Type::String], vec![]);
    let source = "[proc,main]\ndef_int $value = 1;\nmes(\"first\");\n$value = 2;\nmes(\"second\");\n[proc,other]\nmissing_command();";
    let path = Path::new("selected-code.rs2");
    let end = source.find("[proc,other]").unwrap();
    let start = source.find("mes(").unwrap();
    for selection in [
        start..end,
        0..end,
        2..2,
        source.find("$value = 2").unwrap()..source.find("$value = 2").unwrap() + 10,
    ] {
        let report = inspect_code(source, path, selection.clone(), &registry, false)
            .unwrap()
            .unwrap();
        assert!(report.whole_script, "{selection:?}");
        assert_eq!(report.script, "[proc,main]");
        assert_eq!(report.range, 0..end);
        assert!(
            report
                .markdown()
                .contains("Showing the entire containing script")
        );
        let calls = report
            .instructions
            .iter()
            .filter(|row| row.mnemonic == "mes")
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 2);
        assert!(
            calls
                .iter()
                .all(|row| row.effect.unwrap().pops == [0, 1, 0])
        );
        assert!(
            report
                .instructions
                .iter()
                .any(|row| row.mnemonic == "PopIntLocal")
        );
        assert!(
            report
                .instructions
                .iter()
                .any(|row| row.mnemonic == "Return")
        );
    }
    let call = inspect_code(source, path, start..start, &registry, false)
        .unwrap()
        .unwrap();
    assert!(!call.whole_script);
    assert_eq!(call.call, "mes");
    assert_eq!(call.instructions.len(), 2);
}

#[test]
fn broad_inspection_rejects_cross_script_and_invalid_ranges_clearly() {
    let registry = registry(vec![]);
    let source = "[proc,main]\ndef_int $value = 1;\n[proc,other]\nreturn;\n";
    let path = Path::new("selected-code.rs2");
    assert!(
        inspect_code(source, path, 0..source.len(), &registry, false)
            .unwrap_err()
            .contains("crosses script declarations")
    );
    let reversed = Range { start: 9, end: 4 };
    assert!(
        inspect_code(source, path, reversed, &registry, false)
            .unwrap_err()
            .contains("Invalid source selection")
    );
    assert!(
        inspect_code(source, path, 0..source.len() + 1, &registry, false)
            .unwrap_err()
            .contains("Invalid source selection")
    );
    assert!(
        inspect_code("[command,foo]", path, 0..0, &registry, false)
            .unwrap_err()
            .contains("Command declarations have no script bytecode")
    );
}

#[test]
fn inspection_accepts_selected_calls_with_indentation_semicolons_and_crlf() {
    let mut registry = registry(vec![Type::Component]);
    registry.register_command(
        "if_settext".into(),
        77,
        vec![Type::Component, Type::String],
        vec![],
    );
    registry.set_command_behavior("if_settext", true, None, None);
    registry.set_component_first_command("if_settext", true);
    let source = "[proc,main](component $component)\r\n    if_settext($component, \"hello\");  \r\n    if_settext($component, \"again\");\r\n";
    let path = Path::new("selection.cs2");
    let start = source.find("if_settext").unwrap();
    let end = source.find(';').unwrap();
    for selection in [start..start, start..end, start..end + 1, start - 4..end + 5] {
        assert_eq!(
            call_range(source, path, selection.clone(), &registry).unwrap(),
            Some(start..end),
            "{selection:?}"
        );
        let inspected = inspect_call(source, path, selection, &registry, true)
            .unwrap()
            .unwrap();
        assert_eq!(inspected.range, start..end);
        assert_eq!(
            inspected.instructions.last().unwrap().mnemonic,
            "if_settext"
        );
        assert_eq!(inspected.net, Some([0, 0, 0]));
    }
    assert!(
        call_range(source, path, start..source.len(), &registry)
            .unwrap()
            .is_none()
    );
    assert!(
        call_range(source, path, start - 4..start, &registry)
            .unwrap()
            .is_none()
    );
}

#[test]
fn inspection_uses_real_local_slots_and_declared_component_lowering() {
    let mut registry = registry(vec![Type::Component]);
    registry.register_command(
        "paint".into(),
        77,
        vec![Type::Component, Type::Colour],
        vec![],
    );
    registry.set_command_behavior("paint", true, None, None);
    registry.set_component_first_command("paint", true);
    let source = "[proc,main](component $component)\ndef_int $colour = 27;\npaint($component, $colour);\n[proc,unrelated]\nmissing_command();";
    let offset = source.find("paint(").unwrap();
    let path = Path::new("inspection.cs2");
    assert_eq!(
        call_range(source, path, offset..offset, &registry).unwrap(),
        Some(offset..source.find(");").unwrap() + 1)
    );
    let result = inspect_call(source, path, offset..offset, &registry, true)
        .unwrap()
        .unwrap();
    assert_eq!(
        result
            .instructions
            .iter()
            .map(|row| row.mnemonic.as_str())
            .collect::<Vec<_>>(),
        ["PushIntLocal", "PushIntLocal", "paint"]
    );
    assert_eq!(result.instructions[0].operand, "1");
    assert_eq!(result.instructions[1].operand, "0");
    assert_eq!(result.instructions[2].effect.unwrap().pops, [2, 0, 0]);
    assert_eq!(result.net, Some([0, 0, 0]));
}

#[test]
fn inspection_selects_nested_same_line_calls_and_tracks_all_return_stacks() {
    let mut registry = registry(vec![]);
    registry.register_script(
        "triple".into(),
        "proc".into(),
        vec![Type::Int],
        vec![Type::Int, Type::String, Type::Long],
    );
    registry.set_stack_script("proc", "triple");
    registry.register_command(
        "consume".into(),
        42,
        vec![
            Type::Int,
            Type::String,
            Type::Long,
            Type::Int,
            Type::String,
            Type::Long,
        ],
        vec![],
    );
    registry.set_command_behavior("consume", true, None, None);
    let source = "[proc,main]\nconsume(~triple(1), ~triple(2));";
    let offset = source.rfind("~triple").unwrap();
    let result = inspect_call(
        source,
        Path::new("inspection.cs2"),
        offset..offset,
        &registry,
        true,
    )
    .unwrap()
    .unwrap();
    assert_eq!(&source[result.range.clone()], "~triple(2)");
    assert_eq!(result.instructions.len(), 2);
    assert_eq!(result.instructions[0].operand, "2");
    let effect = result.instructions[1].effect.unwrap();
    assert_eq!(effect.pops, [1, 0, 0]);
    assert_eq!(effect.pushes, [1, 1, 1]);
    // The selected call includes its argument push, then leaves all three results.
    assert_eq!(result.net, Some([1, 1, 1]));
}

#[test]
fn inspection_shows_expanded_hook_stack_and_bare_calls() {
    let mut registry = registry(vec![]);
    registry.register_script(
        "callback".into(),
        "clientscript".into(),
        vec![Type::Int, Type::String],
        vec![],
    );
    registry.set_stack_script("clientscript", "callback");
    registry.register_command(
        "set_hook".into(),
        123,
        vec![Type::Component, Type::ClientScript, Type::String],
        vec![],
    );
    registry.set_command_behavior("set_hook", true, Some(true), None);
    registry.set_component_first_command("set_hook", true);
    registry.register_command("current".into(), 456, vec![], vec![Type::Int]);
    let source = r#"[proc,main]
set_hook(42, "callback(7, \"hello\")");
def_int $value = current;
"#;
    let offset = source.find("set_hook").unwrap();
    let result = inspect_call(
        source,
        Path::new("inspection.cs2"),
        offset..offset,
        &registry,
        true,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        result.instructions.last().unwrap().effect.unwrap().pops,
        [3, 2, 0]
    );
    assert_eq!(result.net, Some([0, 0, 0]));
    assert!(result.markdown().contains("set_hook"));
    let offset = source.find("current").unwrap();
    let result = inspect_call(
        source,
        Path::new("inspection.cs2"),
        offset..offset,
        &registry,
        true,
    )
    .unwrap()
    .unwrap();
    assert_eq!(result.instructions.len(), 1);
    assert_eq!(result.instructions[0].mnemonic, "current");
    assert_eq!(result.net, Some([1, 0, 0]));
}

#[test]
fn underspecified_commands_keep_trace_and_fallback_stack_effects_unknown() {
    let mut registry = registry(vec![]);
    registry.register_command("db_getfield".into(), 7, vec![], vec![]);
    registry.register_command("add".into(), 8, vec![], vec![]);
    registry.register_command("consume".into(), 9, vec![Type::Int], vec![]);
    for (call, unknown) in [
        ("db_getfield(1, 2, 3)", "db_getfield"),
        ("consume(calc(1 + 2))", "add"),
    ] {
        let source = format!("[proc,main]\n{call};");
        let start = source.find(call).unwrap();
        let result = inspect_call(
            &source,
            Path::new("inspection.rs2"),
            start..start,
            &registry,
            false,
        )
        .unwrap()
        .unwrap();
        assert!(
            result
                .instructions
                .iter()
                .find(|row| row.mnemonic == unknown)
                .unwrap()
                .effect
                .is_none()
        );
        assert!(result.net.is_none());
    }
}
