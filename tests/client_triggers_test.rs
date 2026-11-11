use runec::clientscript::{self, decode::Script, format::Format};
use runec::project::{Options, Project, Target};
use runec::sources::{Scope, Visibility};
use runec::symbol::SymbolRegistry;
use runec::types::Type;
use std::collections::HashMap;
use std::path::Path;

const TRIGGERS: &[(&str, Type, &str, u16)] = &[
    ("opworldmapelement1", Type::MapElement, "", 0),
    ("opworldmapelement2", Type::MapElement, "", 0),
    ("opworldmapelement3", Type::MapElement, "", 0),
    ("opworldmapelement4", Type::MapElement, "", 0),
    ("opworldmapelement5", Type::MapElement, "", 0),
    (
        "worldmapelementmouseover",
        Type::MapElement,
        "(int $x, int $y)",
        2,
    ),
    (
        "worldmapelementmouseleave",
        Type::MapElement,
        "(int $x, int $y)",
        2,
    ),
    (
        "worldmapelementmouserepeat",
        Type::MapElement,
        "(int $x, int $y)",
        2,
    ),
    ("loadnpc", Type::Npc, "", 0),
    ("loadloc", Type::Loc, "", 0),
];

fn format() -> Format {
    Format::parse("21=Return\n").unwrap()
}

fn registry(ty: Type) -> SymbolRegistry {
    let mut registry = SymbolRegistry::new();
    registry.register_entity_id("subject".into(), ty, 7);
    registry.register_entity_id("group".into(), Type::Category, 9);
    let mut visibility = Visibility::new();
    visibility.declare(
        if ty == Type::MapElement {
            "mel"
        } else {
            ty.name()
        },
        "subject",
        Scope::Global,
    );
    visibility.declare("category", "group", Scope::Global);
    registry.set_visibility(visibility);
    registry
}

#[test]
fn client_events_compile_through_file_and_editor_paths() {
    for &(trigger, ty, params, count) in TRIGGERS {
        for subject in ["subject", "_group", "_"] {
            let text = format!("[{trigger},{subject}]{params}\nreturn;\n");
            let path = Path::new("event.cs2");
            let file = clientscript::parse(&text, path).unwrap();
            let mut registry = registry(ty);
            clientscript::register(
                &file,
                &HashMap::from([(format!("[{trigger},{subject}]"), 123)]),
                &mut registry,
            )
            .unwrap();
            let bytes = clientscript::compile(&file, path, &registry, &format()).unwrap();
            let decoded = Script::decode(&bytes[0].1, &format()).unwrap();
            assert_eq!(decoded.int_param_count, count);
            let mut project = Project::new(Target::Client(format()), registry, Options::default());
            project.open("event.cs2", Scope::Client, 1, text);
            let result = project.compile();
            assert!(
                !result.analysis.diagnostics.has_errors(),
                "{trigger}: {:?}",
                result.analysis.diagnostics
            );
            assert_eq!(result.artifacts.len(), 1);
            assert_eq!(result.artifacts[0].bytes, bytes[0].1);
        }
    }
}

#[test]
fn client_trigger_completions_include_every_documented_trigger() {
    let mut project = Project::new(
        Target::Client(format()),
        registry(Type::MapElement),
        Options::default(),
    );
    let source = project.open("entry.cs2", Scope::Client, 1, "[");
    let completions = project.completions(source, 1, 1000);
    for name in TRIGGERS
        .iter()
        .map(|entry| entry.0)
        .chain(["proc", "clientscript"])
    {
        assert!(
            completions.iter().any(|item| item.label == name),
            "missing {name}"
        );
    }
    assert!(!completions.iter().any(|item| item.label == "login"));
}

#[test]
fn client_event_signatures_and_subjects_are_checked_in_both_paths() {
    for (text, expected) in [
        (
            "[worldmapelementmouserepeat,subject]\nreturn;",
            "expected to accept (int, int)",
        ),
        (
            "[worldmapelementmouseover,subject](int $x, string $y)\nreturn;",
            "expected to accept (int, int)",
        ),
        (
            "[opworldmapelement1,subject](int $x)\nreturn;",
            "expected to accept ()",
        ),
        ("[loadloc,_](int $x)\nreturn;", "expected to accept ()"),
        (
            "[loadnpc,_]()(int)\nreturn(1);",
            "not allowed to return values",
        ),
        ("[loadnpc,subject]\nreturn;", "Unknown npc trigger subject"),
        (
            "[opworldmapelement1,missing]\nreturn;",
            "Unknown mapelement trigger subject",
        ),
        (
            "[opworldmapelement1,_missing]\nreturn;",
            "Unknown category trigger subject",
        ),
    ] {
        let path = Path::new("invalid.cs2");
        let file = clientscript::parse(text, path).unwrap();
        let script = &file.scripts[0];
        let mut registry = registry(Type::MapElement);
        clientscript::register(
            &file,
            &HashMap::from([(format!("[{},{}]", script.trigger, script.name), 1)]),
            &mut registry,
        )
        .unwrap();
        let error = clientscript::compile(&file, path, &registry, &format()).unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
        let mut project = Project::new(Target::Client(format()), registry, Options::default());
        project.open("invalid.cs2", Scope::Client, 1, text);
        let result = project.compile();
        assert!(result.artifacts.is_empty());
        assert!(
            result
                .analysis
                .diagnostics
                .errors()
                .iter()
                .any(|error| error.message.contains(expected)),
            "{:?}",
            result.analysis.diagnostics
        );
    }
    for trigger in ["login", "worldmapelementmouseinvalid"] {
        let text = format!("[{trigger},_]\nreturn;");
        let file = clientscript::parse(&text, Path::new("invalid.cs2")).unwrap();
        assert!(
            clientscript::register(&file, &HashMap::new(), &mut registry(Type::MapElement))
                .is_err()
        );
        let mut project = Project::new(
            Target::Client(format()),
            registry(Type::MapElement),
            Options::default(),
        );
        project.open("invalid.cs2", Scope::Client, 1, text);
        assert!(project.analyze().diagnostics.has_errors());
    }
}

#[test]
fn client_event_subjects_complete_and_link_to_the_correct_config_type() {
    for (trigger, ty, extension) in [
        ("opworldmapelement1", Type::MapElement, "mel"),
        ("loadnpc", Type::Npc, "npc"),
        ("loadloc", Type::Loc, "loc"),
    ] {
        let mut project = Project::new(Target::Client(format()), registry(ty), Options::default());
        let config = project.open_config(
            format!("subjects.{extension}"),
            Scope::Global,
            1,
            "[subject]\n",
            extension,
        );
        project.open_config("unrelated.obj", Scope::Global, 1, "[unrelated]\n", "obj");
        let text = format!("[{trigger},subject]\nreturn;");
        let source = project.open("entry.cs2", Scope::Client, 1, text.as_str());
        assert!(!project.analyze().diagnostics.has_errors());
        let offset = text.find("subject").unwrap();
        let completions = project.completions(source, offset, 100);
        assert!(completions.iter().any(|item| item.label == "subject"));
        assert!(!completions.iter().any(|item| item.label == "unrelated"));
        let definition = project.definition(source, offset + 1).unwrap();
        assert_eq!(definition.source, config);
    }
}
