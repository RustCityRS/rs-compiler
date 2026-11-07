use runec::clientscript::{self, format::Format};
use runec::diagnostics::{Phase, Severity};
use runec::project::{Options, Project, Target};
use runec::sources::{Scope, Visibility};
use runec::symbol::SymbolRegistry;
use runec::types::Type;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

const OPCODES: &str = "0=PushConstantInt\n1=PushVarp\n2=PopVarp\n3=PushConstantString\n6=Branch\n7=BranchNot\n8=BranchEquals\n21=Return\n33=PushIntLocal\n34=PopIntLocal\n38=PopIntDiscard\n40=GosubWithParams\n86=BranchIfTrue\n87=BranchIfFalse\n150=cc_create\n151=cc_delete\n152=cc_deleteall\n200=cc_find\n201=if_find\n1112=cc_settext\n2112=if_settext\n";

const ENGINE: &str = "[command,cc_create](component $parent, int $type, int $child)\n\
    [command,cc_delete]()\n[command,cc_deleteall](component $parent)\n\
    [command,cc_find](component $parent, int $child)(int)\n\
    [command,if_find](component $component)(int)\n\
    [command,cc_settext](string $text)\n\
    [command,if_settext](string $text, component $component)\n";

fn project() -> Project {
    let format = Format::parse(OPCODES).unwrap();
    let mut registry = SymbolRegistry::new();
    for command in clientscript::commands(&format, ENGINE, Path::new("engine.cs2")).unwrap() {
        registry.register_command(
            command.name.clone(),
            command.opcode as i32,
            command.params,
            command.returns,
        );
        registry.set_command_behavior(&command.name, true, None, None);
    }
    registry.register_game_var("state".into(), Type::Int, 1, "varp".into());
    let mut visibility = Visibility::new();
    visibility.declare("varp", "state", Scope::Global);
    registry.set_visibility(visibility);
    Project::new(Target::Client(format), registry, Options::default())
}

fn pointers(project: &mut Project) -> Vec<String> {
    let compilation = project.compile();
    assert!(
        !compilation.analysis.diagnostics.has_errors(),
        "{:?}",
        compilation.analysis.diagnostics
    );
    assert!(!compilation.artifacts.is_empty());
    compilation
        .analysis
        .diagnostics
        .diagnostics()
        .iter()
        .filter(|diagnostic| {
            diagnostic.phase == Phase::PointerCheck && diagnostic.severity == Severity::Warning
        })
        .map(|diagnostic| diagnostic.message.clone())
        .collect()
}

#[test]
fn component_selection_is_independent_and_requires_successful_control_flow() {
    for (body, pointer) in [
        ("cc_settext(\"x\");", Some("active_component")),
        (".cc_settext(\"x\");", Some(".active_component")),
        ("cc_create(1, 4, 0); cc_settext(\"x\");", None),
        (
            "cc_create(1, 4, 0); .cc_settext(\"x\");",
            Some(".active_component"),
        ),
        (".cc_create(1, 4, 0); .cc_settext(\"x\");", None),
        ("if (cc_find(1, 0) = true) { cc_settext(\"x\"); }", None),
        (
            "if (cc_find(1, 0) = false) { cc_settext(\"x\"); }",
            Some("active_component"),
        ),
        (
            "if (cc_find(1, 0) = false) { return; } cc_settext(\"x\");",
            None,
        ),
        ("if (cc_find(1, 0) ! 0) { cc_settext(\"x\"); }", None),
        (
            "if (cc_find(1, 0) ! 1) { cc_settext(\"x\"); }",
            Some("active_component"),
        ),
        ("if (.if_find(1) = true) { .cc_settext(\"x\"); }", None),
        (
            "cc_find(1, 0); cc_settext(\"x\");",
            Some("active_component"),
        ),
        (
            "cc_find(1, 0); if ($flag = true) { cc_settext(\"x\"); }",
            Some("active_component"),
        ),
        (
            "if (cc_find(1, 0) = true & $flag = 1) { cc_settext(\"x\"); }",
            None,
        ),
        (
            "if (cc_find(1, 0) = true | $flag = 1) { cc_settext(\"x\"); }",
            Some("active_component"),
        ),
        (
            "if ($flag = 1) { cc_create(1, 4, 0); } cc_settext(\"x\");",
            Some("active_component"),
        ),
        (
            "while ($flag = 1) { cc_create(1, 4, 0); $flag = 0; } cc_settext(\"x\");",
            Some("active_component"),
        ),
        (
            "cc_create(1, 4, 0); cc_find(2, 0); cc_settext(\"x\");",
            None,
        ),
        ("cc_delete;", Some("active_component")),
        ("cc_create(1, 4, 0); cc_delete; cc_settext(\"x\");", None),
        ("cc_deleteall(1); if_settext(\"x\", 1);", None),
        (
            "%state = 1; if (%state = 1) { if_settext(\"x\", 1); }",
            None,
        ),
    ] {
        let mut project = project();
        project.open(
            "entry.cs2",
            Scope::Client,
            1,
            format!("[clientscript,entry](int $flag)\n{body}"),
        );
        let warnings = pointers(&mut project);
        if let Some(pointer) = pointer {
            assert_eq!(warnings.len(), 1, "{body}: {warnings:?}");
            assert!(
                warnings[0].contains(&format!("pointer {pointer}.")),
                "{body}: {warnings:?}"
            );
        } else {
            assert!(warnings.is_empty(), "{body}: {warnings:?}");
        }
    }
}

#[test]
fn shared_procedure_requirements_and_setters_reach_client_callers() {
    let mut project = project();
    project.open(
        "paint.gs2",
        Scope::Global,
        1,
        "[proc,paint]\ncc_settext(\"x\");",
    );
    project.open(
        "select.gs2",
        Scope::Global,
        1,
        "[proc,select]\ncc_create(1, 4, 0);",
    );
    project.open("nested.gs2", Scope::Global, 1, "[proc,nested]\n~paint;");
    let entry = project.open(
        "entry.cs2",
        Scope::Client,
        1,
        "[clientscript,entry]\n~nested;",
    );
    assert_eq!(pointers(&mut project).len(), 1);
    assert!(
        project
            .check()
            .diagnostics
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.phase == Phase::PointerCheck
                && diagnostic.severity == Severity::Warning
                && diagnostic.file == Path::new("entry.cs2")
                && diagnostic.line == 2)
    );
    project
        .update(entry, 2, "[clientscript,entry]\n~select;\n~nested;")
        .unwrap();
    assert!(pointers(&mut project).is_empty());
    assert!(
        !project
            .check()
            .diagnostics
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.phase == Phase::PointerCheck)
    );
}

#[derive(Clone)]
struct Output(Arc<Mutex<Vec<u8>>>);

impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn directory_compilation_reports_shared_pointer_warnings_and_keeps_output() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("client-pointers-{}", std::process::id()));
    let files = [
        ("pack/cs2_command.pack", OPCODES),
        (
            "pack/clientscript.pack",
            "1=[proc,paint]\n2=[clientscript,entry]\n",
        ),
        ("scripts/clientscript/engine.cs2", ENGINE),
        (
            "scripts/globalscript/paint.gs2",
            "[proc,paint]\ncc_settext(\"x\");",
        ),
        (
            "scripts/clientscript/entry.cs2",
            "[clientscript,entry]\n~paint;",
        ),
    ];
    for (name, text) in files {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let output = Output(Arc::new(Mutex::new(Vec::new())));
    let writer = output.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_writer(move || writer.clone())
        .finish();
    let compiled = tracing::subscriber::with_default(subscriber, || {
        clientscript::compile_directory(&dir.join("scripts/clientscript"), &dir.join("pack"))
            .unwrap()
    });
    assert_eq!(compiled.len(), 2);
    let log = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
    assert!(
        log.contains("active_component") && log.contains("entry.cs2:2"),
        "{log}"
    );
    let format = Format::parse(OPCODES).unwrap();
    let script = clientscript::decode::Script::decode(&compiled[&2], &format).unwrap();
    assert_eq!(script.opcodes, [40, 21]);
    fs::remove_dir_all(dir).unwrap();
}
