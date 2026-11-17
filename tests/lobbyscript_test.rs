use runec::ScriptIds;
use runec::clientscript;
use runec::filesystem::{editor_project, editor_project_for_scope};
use runec::project::{Options, Project, Target};
use runec::sources::{self, Scope};
use runec::symbol::SymbolRegistry;
use runec::types::Type;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};

const LOBBY: &str = "[proc,lobby_entry]()(int)
mes(\"welcome\");
return(~lobby_helper);
[proc,lobby_helper]()(int)
return(^shared);
";

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("lobbyscript")
            .join(format!(
                "lobbyscript-{}-{}",
                process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
        let fixture = Self(root);
        for (path, text) in [
            ("pack/command.pack", "2062=mes\n2071=p_delay\n"),
            ("pack/ls2_command.pack", "2062=mes\n"),
            ("pack/obj.pack", "1=shared_obj\n"),
            ("pack/varp.pack", "1=shared_var\n"),
            ("pack/varc.pack", "1=shared_varc\n"),
            ("pack/interface.pack", "1=lobby_panel\n1:2=button\n"),
            (
                "pack/clientscript.pack",
                "1=[proc,shared]\n2=[proc,client_entry]\n",
            ),
            (
                "pack/cs2_command.pack",
                "0=PushConstantInt\n1=PushVarp\n21=Return\n40=GosubWithParams\n",
            ),
            ("scripts/values.constant", "^shared = 42\n"),
            ("scripts/items.obj", "[shared_obj]\n"),
            ("scripts/vars.varp", "[shared_var]\ntype=int\n"),
            ("scripts/vars.varc", "[shared_varc]\ntype=int\n"),
            (
                "scripts/engine.rs2",
                "[command,mes](string $text)\n[command,p_delay](int $delay)\n",
            ),
            ("scripts/lobby/engine.ls2", "[command,mes](string $text)\n"),
            ("scripts/client/engine.cs2", ""),
            (
                "scripts/global/shared.gs2",
                "[proc,shared]()(int) return(^shared);\n",
            ),
            (
                "scripts/server/main.rs2",
                "[proc,rune_entry]()(int) return(~shared);\n",
            ),
            (
                "scripts/client/main.cs2",
                "[proc,client_entry]()(int) return(~shared);\n",
            ),
            ("scripts/lobby/main.ls2", LOBBY),
        ] {
            fixture.write(path, text);
        }
        fixture
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn scripts(&self) -> PathBuf {
        self.0.join("scripts")
    }

    fn pack(&self) -> PathBuf {
        self.0.join("pack")
    }

    fn rune(&self) -> Result<(Vec<u8>, Vec<u8>), String> {
        runec::compile_memory(&self.scripts(), Some(&self.pack()), false)
            .map_err(|error| error.to_string())
    }

    fn client(&self) -> Result<Vec<Vec<u8>>, String> {
        clientscript::compile_directory(&self.scripts(), &self.pack())
            .map(|compiled| compiled.into_values().collect())
            .map_err(|error| error.to_string())
    }

    /// Both lobby pipelines: batch, and the low-memory one that registers from signatures.
    fn lobby(&self) -> bool {
        let normal = runec::compile_lobby_memory(&self.scripts(), Some(&self.pack()), false);
        let low_memory = runec::compile_lobby_memory_with_options(
            &self.scripts(),
            Some(&self.pack()),
            false,
            false,
            true,
        );
        assert_eq!(normal.is_ok(), low_memory.is_ok());
        if let (Ok(normal), Ok(low_memory)) = (&normal, &low_memory) {
            assert_eq!(normal, low_memory);
        }
        normal.is_ok()
    }

    fn errors(&self, scope: Scope) -> Vec<String> {
        let mut project = editor_project_for_scope(&self.scripts(), Some(&self.pack()), scope)
            .unwrap_or_else(|error| panic!("{scope:?} project: {error}"));
        project
            .check()
            .diagnostics
            .errors()
            .into_iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect()
    }

    fn assert_lobby_error(&self, expected: &str) {
        assert!(!self.lobby(), "lobby accepted: {expected}");
        let errors = self.errors(Scope::Lobby);
        assert!(
            errors.iter().any(|error| error.contains(expected)),
            "{expected}: {errors:?}"
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let root = self.0.canonicalize().unwrap();
        let workspace = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .canonicalize()
            .unwrap();
        assert!(root.starts_with(&workspace) && root != workspace);
        fs::remove_dir_all(root).unwrap();
    }
}

fn idx_slots(idx: &[u8]) -> u32 {
    u32::from_be_bytes(idx[..4].try_into().unwrap())
}

#[test]
fn lobby_scope_follows_the_ls2_extension() {
    let fixture = Fixture::new();
    assert_eq!(Scope::from_path(Path::new("main.ls2")), Some(Scope::Lobby));
    assert_eq!(Scope::Lobby.extension(), "ls2");
    assert_eq!(Scope::Lobby.folder(), "lobbyscript");
    assert_eq!(
        Scope::for_file(Path::new("runescript/main.ls2"), "// @scope(server)\n").unwrap(),
        Scope::Lobby
    );
    assert_eq!(
        Scope::for_file(Path::new("lobby.constant"), "// @scope(lobby)\n").unwrap(),
        Scope::Lobby
    );
    let error = Scope::for_file(Path::new("bad.constant"), "// @scope(lobyb)\n")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("@scope(server|client|lobby|shared)"),
        "{error}"
    );
    let scripts = fixture.scripts();
    assert_eq!(
        sources::script_files(&scripts, "ls2").unwrap(),
        vec![
            scripts.join("lobby/engine.ls2"),
            scripts.join("lobby/main.ls2")
        ]
    );
    for extension in ["rs2", "cs2"] {
        let files = sources::script_files(&scripts, extension).unwrap();
        assert!(files.iter().any(|path| path.ends_with("global/shared.gs2")));
        assert!(
            !files
                .iter()
                .any(|path| path.extension().is_some_and(|ext| ext == "ls2"))
        );
    }
    assert_eq!(
        sources::engine_file(&scripts, Scope::Lobby).unwrap(),
        Some(scripts.join("lobby/engine.ls2"))
    );
}

#[test]
fn lobby_procs_compile_to_server_bytecode() {
    let fixture = Fixture::new();
    assert!(fixture.lobby());
    let (dat, idx) =
        runec::compile_lobby_memory(&fixture.scripts(), Some(&fixture.pack()), false).unwrap();
    assert_eq!(idx_slots(&idx), 2);
    assert_eq!(dat[..4], idx[..4]);
    assert!(fixture.errors(Scope::Lobby).is_empty());

    let output = fixture.0.join("out");
    runec::compile_lobby(&fixture.scripts(), Some(&fixture.pack()), &output, false).unwrap();
    assert_eq!(fs::read(output.join("script.dat")).unwrap(), dat);
    assert_eq!(fs::read(output.join("script.idx")).unwrap(), idx);
    runec::lint_lobby(&fixture.scripts(), Some(&fixture.pack())).unwrap();

    let compile = |target: Target, scope: Scope| {
        let mut registry = SymbolRegistry::new();
        registry.register_command("mes".into(), 2062, vec![Type::String], vec![]);
        let mut project = Project::new(target, registry, Options::default());
        project.open("memory:main", scope, 1, LOBBY.replace("^shared", "42"));
        let compilation = project.compile();
        assert!(
            !compilation.analysis.diagnostics.has_errors(),
            "{:?}",
            compilation.analysis.diagnostics
        );
        compilation
            .artifacts
            .into_iter()
            .map(|artifact| (artifact.name, artifact.bytes))
            .collect::<Vec<_>>()
    };
    let lobby = compile(Target::Lobby, Scope::Lobby);
    assert_eq!(lobby.len(), 2);
    assert_eq!(lobby, compile(Target::Rune, Scope::Rune));
}

#[test]
fn server_and_client_builds_ignore_lobby_sources() {
    let fixture = Fixture::new();
    let rune = fixture.rune().unwrap();
    let client = fixture.client().unwrap();
    fixture.write(
        "scripts/lobby/main.ls2",
        "[proc,lobby_entry]\nnot_a_command(;\n[opnpc1,nothing]\n",
    );
    assert_eq!(fixture.rune().unwrap(), rune);
    assert_eq!(fixture.client().unwrap(), client);
    for scope in [Scope::Rune, Scope::Client] {
        assert!(fixture.errors(scope).is_empty(), "{scope:?}");
    }
    fixture.write("scripts/lobby/main.ls2", LOBBY);

    fixture.write(
        "scripts/server/main.rs2",
        "[proc,rune_entry]()(int) return(~lobby_entry);\n",
    );
    assert!(fixture.rune().is_err());
    let errors = fixture.errors(Scope::Rune);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("proc 'lobby_entry' is not visible to runescript")),
        "{errors:?}"
    );
    fixture.write(
        "scripts/server/main.rs2",
        "[proc,rune_entry]()(int) return(~shared);\n",
    );

    fixture.write(
        "scripts/client/main.cs2",
        "[proc,client_entry]()(int) return(~lobby_entry);\n",
    );
    let error = fixture.client().unwrap_err();
    assert!(
        error.contains("proc 'lobby_entry' is not visible to clientscript"),
        "{error}"
    );
    fixture.write(
        "scripts/global/shared.gs2",
        "[proc,shared]()(int) return(~lobby_entry);\n",
    );
    assert!(fixture.rune().is_err());
    assert!(fixture.lobby());
}

#[test]
fn lobby_sees_shared_configs_but_not_other_scripts() {
    let fixture = Fixture::new();
    fixture.write(
        "scripts/lobby/configs.ls2",
        "[proc,lobby_configs]()(int, component, obj, int, int)
return(^shared, lobby_panel:button, shared_obj, %shared_var, %shared_varc);
",
    );
    assert!(fixture.lobby());
    assert!(fixture.errors(Scope::Lobby).is_empty());
    for (callee, expected) in [
        (
            "shared",
            "proc 'shared' is not visible to lobbyscript; define it in a .ls2 source",
        ),
        (
            "rune_entry",
            "proc 'rune_entry' is not visible to lobbyscript; define it in a .ls2 source",
        ),
        (
            "client_entry",
            "proc 'client_entry' is not visible to lobbyscript; define it in a .ls2 source",
        ),
    ] {
        fixture.write(
            "scripts/lobby/configs.ls2",
            &format!("[proc,lobby_call]()(int) return(~{callee});\n"),
        );
        fixture.assert_lobby_error(expected);
    }
    fixture.write(
        "scripts/server/private.constant",
        "// @scope(server)\n^server_only = 1\n",
    );
    fixture.write(
        "scripts/lobby/configs.ls2",
        "[proc,lobby_call]()(int) return(^server_only);\n",
    );
    fixture.assert_lobby_error("constant 'server_only' is not visible to lobbyscript");
    fixture.write(
        "scripts/lobby/private.constant",
        "// @scope(lobby)\n^lobby_only = 1\n",
    );
    fixture.write(
        "scripts/lobby/configs.ls2",
        "[proc,lobby_call]()(int) return(^lobby_only);\n",
    );
    assert!(fixture.lobby());
    fixture.write(
        "scripts/server/main.rs2",
        "[proc,rune_entry]()(int) return(^lobby_only);\n",
    );
    assert!(fixture.rune().is_err());
}

#[test]
fn lobby_declarations_conflict_with_shared_ones_only_in_the_lobby() {
    let fixture = Fixture::new();
    fixture.write(
        "scripts/lobby/values.constant",
        "// @scope(lobby)\n^shared = 7\n",
    );
    fixture.assert_lobby_error("constant 'shared' is declared in both shared and private scopes");
    assert!(fixture.rune().is_ok());
    assert!(fixture.client().is_ok());
    fixture.write("scripts/lobby/values.constant", "// @scope(lobby)\n");
    fixture.write(
        "scripts/lobby/shadow.ls2",
        "[proc,shared]()(int) return(7);\n[proc,lobby_shadow]()(int) return(~shared);\n",
    );
    assert!(fixture.rune().is_ok());
    assert!(fixture.client().is_ok());
    assert!(fixture.lobby());
    assert!(fixture.errors(Scope::Lobby).is_empty());
    fixture.write(
        "scripts/lobby/shadow.ls2",
        "[proc,rune_entry]()(int) return(7);\n[login,_]\n",
    );
    fixture.write("scripts/server/login.rs2", "[login,_]\nmes(\"server\");\n");
    assert!(fixture.rune().is_ok());
    assert!(fixture.lobby());
}

#[test]
fn only_minimal_triggers_are_accepted() {
    let fixture = Fixture::new();
    let mut accepted = String::from(
        "[login,_]\n[logout,_]\n[if_button,lobby_panel:button]\n[if_buttond,lobby_panel:button]\n[if_close,lobby_panel]\n[label,lobby_label]\n",
    );
    for button in 1..=10 {
        accepted.push_str(&format!("[if_button{button},lobby_panel:button]\n"));
    }
    fixture.write("scripts/lobby/triggers.ls2", &accepted);
    assert!(fixture.lobby());
    assert!(fixture.errors(Scope::Lobby).is_empty());
    for (header, trigger) in [
        ("[opnpc1,nothing]", "opnpc1"),
        ("[ai_queue1,nothing]", "ai_queue1"),
        ("[opheld1,shared_obj]", "opheld1"),
        ("[debugproc,lobby_debug]", "debugproc"),
        ("[queue,lobby_queue]", "queue"),
        ("[clientscript,lobby_client]", "clientscript"),
    ] {
        fixture.write("scripts/lobby/triggers.ls2", &format!("{header}\n"));
        fixture.assert_lobby_error(&format!(
            "The trigger type '{trigger}' is not available in lobbyscript."
        ));
        assert!(fixture.rune().is_ok(), "{header}");
    }

    let mut project = Project::new(Target::Lobby, SymbolRegistry::new(), Options::default());
    let source = project.open("memory:lobby", Scope::Lobby, 1, "[opnpc1,nothing]\n");
    let errors = project.analyze().diagnostics;
    assert!(
        errors
            .errors()
            .iter()
            .any(|error| error.message
                == "The trigger type 'opnpc1' is not available in lobbyscript."),
        "{errors:?}"
    );
    project.update(source, 2, "[login,_]\n").unwrap();
    assert!(!project.analyze().diagnostics.has_errors());
}

#[test]
fn commands_are_declared_only_in_engine_ls2() {
    let fixture = Fixture::new();
    fixture.write("scripts/lobby/greet.ls2", "[command,greet]\nmes(\"hi\");\n");
    fixture.assert_lobby_error("The trigger type 'command' is not available in lobbyscript.");
    assert!(fixture.rune().is_ok());

    let mut project = Project::new(Target::Lobby, SymbolRegistry::new(), Options::default());
    let engine = project.open(
        "lobby/engine.ls2",
        Scope::Lobby,
        1,
        "[command,mes](string $text)\n",
    );
    let main = project.open("lobby/main.ls2", Scope::Lobby, 1, "[");
    let offers_command = |source| {
        project
            .completions(source, 1, 100)
            .iter()
            .any(|item| item.label == "command")
    };
    assert!(offers_command(engine));
    assert!(!offers_command(main));
    project.update(main, 2, "[command,greet]\n").unwrap();
    let errors = project.analyze().diagnostics;
    assert_eq!(
        errors
            .errors()
            .iter()
            .map(|error| (error.file.clone(), error.message.as_str()))
            .collect::<Vec<_>>(),
        [(
            PathBuf::from("lobby/main.ls2"),
            "The trigger type 'command' is not available in lobbyscript."
        )]
    );
}

#[test]
fn secondary_player_forms_are_errors() {
    let fixture = Fixture::new();
    for (body, form) in [
        ("\n.mes(\"hi\");\n", ".mes"),
        ("\n.%shared_var = 1;\n", ".%shared_var"),
        ("()(int)\nreturn(.%shared_var);\n", ".%shared_var"),
    ] {
        fixture.write(
            "scripts/lobby/secondary.ls2",
            &format!("[proc,lobby_secondary]{body}"),
        );
        fixture.assert_lobby_error(&format!(
            "'{form}' uses the secondary player pointer, which is not available in lobbyscript."
        ));
    }
    fixture.write(
        "scripts/lobby/secondary.ls2",
        "[proc,lobby_secondary]\nmes(\"hi\");\n%shared_var = 1;\n",
    );
    assert!(fixture.lobby());

    let mut registry = SymbolRegistry::new();
    registry.register_command("findhero".into(), 2018, vec![], vec![Type::Boolean]);
    let mut project = Project::new(Target::Lobby, registry, Options::default());
    project.open(
        "memory:hero.ls2",
        Scope::Lobby,
        1,
        "[proc,hero]()(boolean)\nreturn(findhero);\n",
    );
    let errors = project.analyze().diagnostics;
    assert!(
        errors.errors().iter().any(|error| error.message
            == "'findhero' uses the secondary player pointer, which is not available in lobbyscript."),
        "{errors:?}"
    );
}

#[test]
fn server_commands_are_unresolved_in_lobby() {
    let fixture = Fixture::new();
    fixture.write(
        "scripts/server/delay.rs2",
        "[proc,rune_delay]\np_delay(1);\n",
    );
    assert!(fixture.rune().is_ok());
    fixture.write(
        "scripts/lobby/delay.ls2",
        "[proc,lobby_delay]\np_delay(1);\n",
    );
    fixture.assert_lobby_error("'p_delay' cannot be resolved to a command.");
    assert!(fixture.rune().is_ok());
}

#[test]
fn lobby_callbacks_resolve_clientscripts_like_the_server() {
    let fixture = Fixture::new();
    fixture.write(
        "pack/ls2_command.pack",
        "2062=mes\n2186=runclientscript\n2187=runclientscriptvararg\n",
    );
    fixture.write(
        "pack/clientscript.pack",
        "1=[proc,shared]\n2=[proc,client_entry]\n7=[clientscript,refresh]\n",
    );
    fixture.write(
        "scripts/client/refresh.cs2",
        "[clientscript,refresh](int $value)\n",
    );
    fixture.write(
        "scripts/lobby/callback.ls2",
        "[proc,lobby_refresh]\nrunclientscript(\"refresh(1)\");\n",
    );
    assert!(fixture.lobby());
    assert!(fixture.errors(Scope::Lobby).is_empty());
    fixture.write(
        "scripts/lobby/callback.ls2",
        "[proc,lobby_refresh]\nrunclientscript(\"refresh()\");\n",
    );
    assert!(!fixture.lobby());
    assert!(!fixture.errors(Scope::Lobby).is_empty());
}

#[test]
fn lobby_script_ids_are_independent_of_server_ids() {
    let fixture = Fixture::new();
    fixture.write(
        "scripts/server/login.rs2",
        "[login,_]\n[proc,lobby_helper]()(int) return(1);\n",
    );
    fixture.write("scripts/lobby/login.ls2", "[login,_]\nmes(\"lobby\");\n");
    let mut server = ScriptIds::new();
    runec::compile_memory_with_ids(&fixture.scripts(), Some(&fixture.pack()), &mut server).unwrap();
    let mut lobby = ScriptIds::new();
    let (_, idx) =
        runec::compile_lobby_memory_with_ids(&fixture.scripts(), Some(&fixture.pack()), &mut lobby)
            .unwrap();
    assert_eq!(idx_slots(&idx), 3);
    let expected = |names: &[&str]| {
        names
            .iter()
            .enumerate()
            .map(|(id, name)| (name.to_string(), id as i32))
            .collect::<ScriptIds>()
    };
    assert_eq!(
        server,
        expected(&[
            "login:_",
            "proc:lobby_helper",
            "proc:rune_entry",
            "proc:shared"
        ])
    );
    assert_eq!(
        lobby,
        expected(&["login:_", "proc:lobby_entry", "proc:lobby_helper"])
    );
    let mut reused = ScriptIds::from([("proc:lobby_helper".to_string(), 40)]);
    runec::compile_lobby_memory_with_ids(&fixture.scripts(), Some(&fixture.pack()), &mut reused)
        .unwrap();
    assert_eq!(reused["proc:lobby_helper"], 40);
    assert_eq!(reused["login:_"], 41);
}

#[test]
fn editor_projects_keep_lobby_documents_in_the_lobby_target() {
    let fixture = Fixture::new();
    let labels = |scope: Scope| {
        let project =
            editor_project_for_scope(&fixture.scripts(), Some(&fixture.pack()), scope).unwrap();
        project
            .documents()
            .filter(|document| document.config.is_none())
            .map(|document| {
                (
                    document.scope,
                    document.label.to_string_lossy().into_owned(),
                )
            })
            .collect::<Vec<_>>()
    };
    let lobby = labels(Scope::Lobby);
    assert!(
        lobby
            .iter()
            .any(|(scope, label)| *scope == Scope::Lobby && label.ends_with("lobby/main.ls2"))
    );
    assert!(lobby.iter().any(|(scope, _)| *scope == Scope::Client));
    assert!(
        lobby
            .iter()
            .all(|(scope, _)| matches!(scope, Scope::Lobby | Scope::Client))
    );
    for scope in [Scope::Rune, Scope::Client] {
        assert!(
            labels(scope)
                .iter()
                .all(|(scope, _)| *scope != Scope::Lobby),
            "{scope:?}"
        );
    }
    let server = editor_project(&fixture.scripts(), Some(&fixture.pack()), false).unwrap();
    assert!(
        server
            .documents()
            .all(|document| document.scope != Scope::Lobby)
    );
    assert!(
        editor_project_for_scope(&fixture.scripts(), Some(&fixture.pack()), Scope::Global).is_err()
    );

    fixture.write(
        "scripts/lobby/main.ls2",
        "[proc,lobby_entry]\nlobby_missing;\n",
    );
    assert!(!fixture.errors(Scope::Lobby).is_empty());
    assert!(fixture.errors(Scope::Rune).is_empty());
}
