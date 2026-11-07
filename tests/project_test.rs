use runec::clientscript::format::Format;
use runec::diagnostics::{Phase, Severity};
use runec::project::{Options, Project, Target};
use runec::sources::{Scope, Visibility};
use runec::symbol::SymbolRegistry;
use runec::text::{LineIndex, Position};
use runec::types::Type;

fn client() -> Project {
    let format =
        Format::parse("17=PushConstantInt\n27=PushIntLocal\n31=Return\n42=GosubWithParams\n")
            .unwrap();
    Project::new(
        Target::Client(format),
        SymbolRegistry::new(),
        Options::default(),
    )
}

#[test]
fn proc_jumps_block_compilation_without_optional_checks() {
    for (target, scope, path) in [
        (Target::Rune, Scope::Rune, "caller.rs2"),
        (Target::Rune, Scope::Global, "caller.gs2"),
        (
            Target::Client(Format::parse("21=Return\n40=GosubWithParams\n").unwrap()),
            Scope::Client,
            "caller.cs2",
        ),
        (
            Target::Client(Format::parse("21=Return\n40=GosubWithParams\n").unwrap()),
            Scope::Global,
            "caller.gs2",
        ),
    ] {
        let mut registry = SymbolRegistry::new();
        registry.register_script("target".into(), "label".into(), vec![], vec![]);
        let mut visibility = Visibility::new();
        visibility.declare("label", "target", Scope::Global);
        registry.set_visibility(visibility);
        let mut project = Project::new(target, registry, Options::default());
        let source = "[proc,caller]\n@target;\n[proc,helper]\nreturn;\n";
        let id = project.open(path, scope, 1, source);
        let result = project.compile();
        assert!(result.artifacts.is_empty());
        let errors = result.analysis.diagnostics.errors();
        assert_eq!(errors.len(), 1, "{:?}", result.analysis.diagnostics);
        assert_eq!(errors[0].severity, Severity::Error);
        assert_eq!(errors[0].phase, Phase::TypeChecking);
        assert_eq!(errors[0].line, 2);
        assert_eq!(
            errors[0].message,
            "Cannot jump to label '@target' from a proc."
        );
        project
            .update(id, 2, source.replace("@target", "~helper"))
            .unwrap();
        let fixed = project.compile();
        assert!(
            !fixed.analysis.diagnostics.has_errors(),
            "{:?}",
            fixed.analysis.diagnostics
        );
        assert_eq!(fixed.artifacts.len(), 2);
    }
}

#[test]
fn virtual_sources_share_globals_and_keep_ids_across_edits() {
    let mut project = client();
    let shared = project.open(
        "db:shared",
        Scope::Global,
        1,
        "[proc,value]()(int)\nreturn(7);",
    );
    project.open(
        "unsaved:entry",
        Scope::Client,
        1,
        "[proc,entry]()(int)\nreturn(~value);",
    );
    let first = project.compile();
    assert!(
        !first.analysis.diagnostics.has_errors(),
        "{:?}",
        first.analysis.diagnostics
    );
    assert_eq!(first.artifacts.len(), 2);
    assert_eq!(first.analysis.assignments.len(), 2);
    project
        .update(shared, 2, "[proc,value]()(int)\nreturn(8);")
        .unwrap();
    let second = project.compile();
    assert_eq!(second.analysis.checked_documents, 1);
    assert_eq!(first.analysis.assignments, second.analysis.assignments);
    assert_ne!(first.artifacts[0].bytes, second.artifacts[0].bytes);
    assert_eq!(first.artifacts[1].bytes, second.artifacts[1].bytes);
    assert!(project.update(shared, 2, "stale").is_err());
    project
        .update(shared, 3, "[proc,value]()(string)\nreturn(\"changed\");")
        .unwrap();
    let changed = project.analyze();
    assert_eq!(changed.checked_documents, 2);
    assert!(changed.diagnostics.has_errors());
}

#[test]
fn explicit_namespace_ownership_checks_names_and_numeric_ids() {
    let mut registry = SymbolRegistry::new();
    registry.register_entity_id("private_obj".into(), Type::Obj, 123);
    let mut visibility = Visibility::new();
    visibility.declare("obj", "private_obj", Scope::Rune);
    registry.set_visibility(visibility);
    let mut project = Project::new(Target::Rune, registry, Options::default());
    let doc = project.open(
        "database-key",
        Scope::Global,
        1,
        "[proc,value]()(obj)\nreturn(private_obj);",
    );
    assert!(project.analyze().diagnostics.has_errors());
    project
        .update(doc, 2, "[proc,value]()(obj)\nreturn(123);")
        .unwrap();
    assert!(project.analyze().diagnostics.has_errors());
}

#[test]
fn diagnostics_recover_and_use_unsaved_source() {
    let mut project = client();
    project.open("untitled:broken", Scope::Client, 1, "[proc,broken]\nreturn(;\n[proc,valid]()(int)\nreturn($missing);\n[proc,also_broken]\nreturn(;\n");
    let result = project.analyze();
    assert!(
        result.diagnostics.error_count() >= 3,
        "{:?}",
        result.diagnostics
    );
    let rendered = project.render_diagnostic(&result.diagnostics.diagnostics()[0]);
    assert!(rendered.contains("return(;"));
    assert!(rendered.contains("RUNEC1002"));
    assert!(project.compile().artifacts.is_empty());
}

#[test]
fn ide_navigation_rename_signatures_and_interpolated_locals() {
    let mut project = client();
    let text = "// Choose a value.\n[proc,choose](int $value)(int)\nreturn($value);\n";
    let definition = project.open("mem:definition", Scope::Global, 1, text);
    let use_text = "[proc,caller]()(int)\nreturn(~choose(5));\n";
    let use_id = project.open("mem:caller", Scope::Client, 1, use_text);
    let offset = use_text.find("choose").unwrap();
    assert_eq!(
        project.definition(use_id, offset).unwrap().source,
        definition
    );
    assert_eq!(project.references(use_id, offset, true).len(), 2);
    assert_eq!(
        project
            .rename(use_id, offset, "select_value")
            .unwrap()
            .len(),
        2
    );
    assert!(project.rename(use_id, offset, "return").is_err());
    assert!(project.rename(use_id, offset, "caller").is_err());
    assert!(
        project
            .hover(use_id, offset)
            .unwrap()
            .documentation
            .contains("Choose a value.")
    );
    assert_eq!(
        project
            .signature_help(use_id, use_text.find('5').unwrap())
            .unwrap()
            .parameters,
        vec![("value".into(), Type::Int)]
    );
    let string = "[proc,strings](string $value)(string)\nreturn(\"Hello <$value>\");\n";
    let doc = project.open("mem:strings", Scope::Global, 1, string);
    let edits = project
        .rename(doc, string.find("value").unwrap(), "name")
        .unwrap();
    assert_eq!(edits.len(), 2);
    assert!(
        project
            .semantic_tokens(doc)
            .windows(2)
            .all(|pair| pair[0].range.end <= pair[1].range.start)
    );
}

#[test]
fn unicode_positions_roundtrip_and_reject_split_surrogates() {
    let text = "a\u{1f600}b\r\n\u{e9}";
    let index = LineIndex::new(text);
    for (byte, _) in text
        .char_indices()
        .filter(|(_, ch)| !matches!(ch, '\r' | '\n'))
    {
        assert_eq!(
            index.offset(text, index.position(text, byte).unwrap()),
            Some(byte)
        );
    }
    assert_eq!(
        index.offset(
            text,
            Position {
                line: 0,
                character: 2
            }
        ),
        None
    );
    assert_eq!(
        index.offset(
            text,
            Position {
                line: 0,
                character: 8
            }
        ),
        None
    );
}

#[test]
fn cancellation_and_failed_codegen_never_publish_partial_artifacts() {
    let options = Options::default();
    let cancellation = options.cancellation.clone();
    let mut project = Project::new(Target::Rune, SymbolRegistry::new(), options);
    project.open("mem:test", Scope::Rune, 1, "[proc,test] return;");
    cancellation.cancel();
    let result = project.compile();
    assert!(result.analysis.cancelled);
    assert!(result.artifacts.is_empty());
    let mut project = Project::new(
        Target::Client(Format::parse("31=Return").unwrap()),
        SymbolRegistry::new(),
        Options::default(),
    );
    project.open(
        "mem:unsupported",
        Scope::Client,
        1,
        "[proc,test]()(int) return(1);",
    );
    let result = project.compile();
    assert!(result.analysis.diagnostics.has_errors());
    assert!(result.artifacts.is_empty());
}

#[test]
fn rename_respects_shadowing_and_typed_callback_references() {
    let mut registry = SymbolRegistry::new();
    registry.register_command("hook".into(), 200, vec![Type::ClientScript], vec![]);
    let mut project = Project::new(Target::Rune, registry, Options::default());
    let text = "[proc,outer](int $value)(int)\nif ($value = 1) { def_int $value = 2; return($value); }\nreturn($value);\n[clientscript,callback]\nreturn;\n[proc,wire]\nhook(callback);\n";
    let doc = project.open("opaque", Scope::Rune, 1, text);
    let outer = project.references(doc, text.find("$value").unwrap() + 1, true);
    assert_eq!(outer.len(), 3);
    let inner = project.references(doc, text.find("def_int $value").unwrap() + 9, true);
    assert_eq!(inner.len(), 2);
    let callback = project.references(doc, text.find("hook(callback)").unwrap() + 5, true);
    assert_eq!(callback.len(), 2);
    let text =
        "[proc,first](int $first)(int) return($first); [proc,second]()(int) return(~first(1));";
    let doc = project.open("single-line", Scope::Global, 1, text);
    assert_eq!(
        project
            .references(doc, text.find("$first").unwrap() + 1, true)
            .len(),
        2
    );
    assert_eq!(
        project
            .references(doc, text.find("~first").unwrap() + 1, true)
            .len(),
        2
    );
}

#[test]
fn supplied_engine_profile_controls_triggers_and_dispatch() {
    let mut registry = SymbolRegistry::new();
    let mut profile = runec::environment::EngineProfile::default();
    profile.triggers.insert(
        "custom".into(),
        runec::environment::Trigger {
            byte: Some(42),
            ..Default::default()
        },
    );
    profile.lookup_key = Some(|_, _, _| 9876);
    registry.set_engine_profile(profile);
    let mut project = Project::new(Target::Rune, registry, Options::default());
    project.open("emulator:custom", Scope::Rune, 1, "[custom,_]\nreturn;");
    assert!(!project.compile().analysis.diagnostics.has_errors());
    project.open(
        "emulator:unsupported",
        Scope::Rune,
        1,
        "[proc,unsupported]\nreturn;",
    );
    assert!(project.compile().analysis.diagnostics.has_errors());
}

#[test]
fn incomplete_proc_keeps_parameters_and_previous_locals_available() {
    let mut project = client();
    let text = "[proc,typing](int $argument)(int)\ndef_int $local = 1;\nreturn($\n";
    let id = project.open("unsaved:typing", Scope::Global, 1, text);
    assert!(project.analyze().diagnostics.has_errors());
    let completions = project.completions(id, text.len() - 1, 20);
    assert!(
        completions
            .iter()
            .any(|completion| completion.label == "argument")
    );
    assert!(
        completions
            .iter()
            .any(|completion| completion.label == "local")
    );
    assert!(project.compile().artifacts.is_empty());
}

#[test]
fn formatting_preserves_multiline_literal_bytes_and_nested_blocks() {
    let mut project = Project::new(Target::Rune, SymbolRegistry::new(), Options::default());
    let text = "[proc,format](int $input)(string)\nif ($input = 1) {\ndef_string $prefix = \"before\"; return(r\"literal  \n    contents  \nend\"); }\n      return(\"fallback\");\n";
    let id = project.open("unsaved:format", Scope::Rune, 1, text);
    let before = project.compile();
    assert!(
        !before.analysis.diagnostics.has_errors(),
        "{:?}",
        before.analysis.diagnostics
    );
    let mut formatted = text.to_owned();
    let edits = project.format(id, 4, true);
    assert!(!edits.is_empty());
    for edit in edits.into_iter().rev() {
        formatted.replace_range(edit.location.range, &edit.replacement);
    }
    assert!(formatted.contains("literal  \n    contents  \nend"));
    project.update(id, 2, formatted).unwrap();
    let after = project.compile();
    assert!(
        !after.analysis.diagnostics.has_errors(),
        "{:?}",
        after.analysis.diagnostics
    );
    assert_eq!(before.artifacts[0].bytes, after.artifacts[0].bytes);
}
