use runec::bytecode::Opcode;
use runec::clientscript::{self, decode::Script, format::Format};
use runec::error::CompilerError;
use runec::sources::{self, Scope};
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Project(PathBuf);

impl Project {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("globalscript")
            .join(format!(
                "globalscript-{}-{}",
                process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
        let project = Self(root);
        for (path, text) in [
            (
                "pack/obj.pack",
                "1=shared_obj\n2=rune_obj\n3=client_obj\n4=pack_only\n",
            ),
            ("pack/varp.pack", "1=shared_var\n2=rune_var\n3=client_var\n"),
            (
                "pack/clientscript.pack",
                "1=[proc,shared]\n2=[proc,client_entry]\n",
            ),
            (
                "pack/cs2_command.pack",
                "0=PushConstantInt\n1=PushVarp\n21=Return\n40=GosubWithParams\n",
            ),
            ("scripts/globalscript/items.obj", "[shared_obj]\n"),
            (
                "scripts/runescript/items.obj",
                "// @scope(server)\n[rune_obj]\n",
            ),
            (
                "scripts/clientscript/items.obj",
                "// @scope(client)\n[client_obj]\n",
            ),
            ("scripts/globalscript/vars.varp", "[shared_var]\ntype=int\n"),
            (
                "scripts/runescript/vars.varp",
                "// @scope(server)\n[rune_var]\ntype=int\n",
            ),
            (
                "scripts/clientscript/vars.varp",
                "// @scope(client)\n[client_var]\ntype=int\n",
            ),
            ("scripts/globalscript/values.constant", "^shared = 42\n"),
            (
                "scripts/runescript/values.constant",
                "// @scope(server)\n^rune = 21\n",
            ),
            (
                "scripts/clientscript/values.constant",
                "// @scope(client)\n^client = 84\n",
            ),
            (
                "scripts/globalscript/shared.gs2",
                "[proc,shared]()(int) return(^shared);\n",
            ),
            (
                "scripts/runescript/main.rs2",
                "[proc,rune_entry]()(int) return(~shared);\n",
            ),
            (
                "scripts/clientscript/main.cs2",
                "[proc,client_entry]()(int) return(~shared);\n",
            ),
            ("scripts/clientscript/engine.cs2", ""),
        ] {
            project.write(path, text);
        }
        project
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn rune(&self, low_mem: bool) -> bool {
        runec::compile_memory_with_options(
            &self.0.join("scripts/runescript"),
            Some(&self.0.join("pack")),
            false,
            false,
            low_mem,
        )
        .is_ok()
    }

    fn client(&self) -> bool {
        clientscript::compile_directory(&self.0.join("scripts/clientscript"), &self.0.join("pack"))
            .is_ok()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let root = self.0.canonicalize().unwrap();
        let workspace = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .canonicalize()
            .unwrap();
        assert!(root.starts_with(&workspace) && root != workspace);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn proc_jumps_fail_batch_compilation_in_both_memory_modes() {
    let project = Project::new();
    project.write(
        "scripts/globalscript/shared.gs2",
        "[proc,shared]()(int)\n@done;\nreturn(0);\n[label,done]\nreturn;\n",
    );
    assert!(!project.rune(false));
    assert!(!project.rune(true));
    project.write(
        "scripts/globalscript/shared.gs2",
        "[proc,shared]()(int)\nreturn(0);\n[label,done]\nreturn;\n",
    );
    assert!(project.rune(false));
    assert!(project.rune(true));
}

#[test]
fn shared_sources_compile_in_both_targets_and_low_memory_mode() {
    let project = Project::new();
    let dir = project.0.join("scripts/runescript");
    let pack = project.0.join("pack");
    let normal = runec::compile_memory(&dir, Some(&pack), false).unwrap();
    let low_memory =
        runec::compile_memory_with_options(&dir, Some(&pack), false, false, true).unwrap();
    assert_eq!(normal, low_memory);
    assert!(project.client());
    let original =
        clientscript::compile_directory(&project.0.join("scripts/clientscript"), &pack).unwrap();
    project.write("scripts/globalscript/values.constant", "^shared = 43\n");
    let changed =
        clientscript::compile_directory(&project.0.join("scripts/clientscript"), &pack).unwrap();
    assert_ne!(original[&1], changed[&1]);
    assert_eq!(original[&2], changed[&2]);
    assert_ne!(
        normal,
        runec::compile_memory(&dir, Some(&pack), false).unwrap()
    );
}

#[test]
fn server_callbacks_load_named_clientscript_signatures() {
    let project = Project::new();
    project.write(
        "pack/command.pack",
        "5000=runclientscript\n5013=runclientscriptvararg\n",
    );
    project.write(
        "pack/clientscript.pack",
        "7=[clientscript,refresh]\n8=[clientscript,inventory]\n9=[proc,inventory]\n10=[proc,named_proc]\n",
    );
    project.write(
        "scripts/clientscript/main.cs2",
        "[clientscript,refresh]\n[clientscript,inventory](int $slot, string $op)\n[proc,inventory](int $slot)\n[proc,named_proc](string $name)\n",
    );
    project.write(
        "scripts/runescript/main.rs2",
        r#"[proc,rune_entry]
runclientscript("refresh()");
runclientscript("inventory(1, \"Remove\")");
.runclientscript("named_proc(\"name\")");
"#,
    );
    let dir = project.0.join("scripts/runescript");
    let pack = project.0.join("pack");
    let normal = runec::compile_memory(&dir, Some(&pack), false).unwrap();
    let low_memory =
        runec::compile_memory_with_options(&dir, Some(&pack), false, false, true).unwrap();
    assert_eq!(normal, low_memory);

    project.write(
        "scripts/runescript/main.rs2",
        "[proc,rune_entry]\nrunclientscript(\"inventory(1)\");\n",
    );
    assert!(!project.rune(false));
    assert!(!project.rune(true));
}

#[test]
fn config_constants_vars_and_procs_cannot_cross_private_namespaces() {
    let project = Project::new();
    for (folder, ext, entry, forbidden) in [
        ("runescript", "rs2", "rune_entry", "client"),
        ("clientscript", "cs2", "client_entry", "rune"),
        ("globalscript", "gs2", "shared", "rune"),
        ("globalscript", "gs2", "shared", "client"),
    ] {
        let path = format!(
            "scripts/{folder}/{}.{ext}",
            if folder == "globalscript" {
                "shared"
            } else {
                "main"
            }
        );
        let id = if forbidden == "rune" { 2 } else { 3 };
        for (ty, value) in [
            ("obj", format!("{forbidden}_obj")),
            ("obj", format!("\"{forbidden}_obj\"")),
            ("obj", id.to_string()),
            ("int", format!("^{forbidden}")),
            ("int", format!("%{forbidden}_var")),
            ("int", format!("~{forbidden}_entry")),
        ] {
            project.write(&path, &format!("[proc,{entry}]()({ty}) return({value});\n"));
            if folder != "clientscript" {
                assert!(
                    !project.rune(false),
                    "runescript accepted {folder}: {value}"
                );
                assert!(!project.rune(true), "low-memory accepted {folder}: {value}");
            }
            if folder != "runescript" {
                assert!(!project.client(), "clientscript accepted {folder}: {value}");
            }
        }
        project.write(&path, &format!("[proc,{entry}]()(int) return(^shared);\n"));
    }
    for (folder, ext, entry) in [
        ("runescript", "rs2", "rune_entry"),
        ("clientscript", "cs2", "client_entry"),
    ] {
        project.write(
            &format!("scripts/{folder}/main.{ext}"),
            &format!("[proc,{entry}]()(obj) return(pack_only);\n"),
        );
    }
    assert!(!project.rune(false));
    assert!(!project.client());
}

#[test]
fn config_scope_is_explicit_and_does_not_change_when_moved() {
    let project = Project::new();
    for (folder, ext, entry) in [
        ("runescript", "rs2", "rune_entry"),
        ("clientscript", "cs2", "client_entry"),
    ] {
        project.write(
            &format!("scripts/{folder}/main.{ext}"),
            &format!("[proc,{entry}]()(obj) return(rune_obj);\n"),
        );
    }
    assert!(project.rune(false));
    assert!(!project.client());
    fs::rename(
        project.0.join("scripts/runescript/items.obj"),
        project.0.join("scripts/globalscript/moved.obj"),
    )
    .unwrap();
    assert!(project.rune(false));
    assert!(!project.client());
    project.write("scripts/globalscript/moved.obj", "[rune_obj]\n");
    assert!(project.rune(false));
    assert!(project.client());
}

#[test]
fn shared_constants_and_procs_cannot_be_shadowed_by_private_definitions() {
    let project = Project::new();
    project.write(
        "scripts/runescript/values.constant",
        "// @scope(server)\n^shared = 21\n",
    );
    assert!(!project.rune(false));
    assert!(!project.client());
    project.write(
        "scripts/runescript/values.constant",
        "// @scope(server)\n^rune = 21\n",
    );
    project.write(
        "scripts/runescript/duplicate.rs2",
        "[proc,shared]()(int) return(21);\n",
    );
    assert!(!project.rune(false));
    assert!(!project.client());
}

#[test]
fn trigger_subjects_and_normalized_names_respect_ownership() {
    let project = Project::new();
    project.write("scripts/runescript/main.rs2", "[opheld1,client_obj]\n");
    assert!(!project.rune(false));
    project.write(
        "scripts/runescript/main.rs2",
        "[proc,rune_entry]()(obj) return(Rune_Obj);\n",
    );
    assert!(project.rune(false));
    project.write(
        "scripts/clientscript/main.cs2",
        "[proc,client_entry]()(obj) return(Rune_Obj);\n",
    );
    assert!(!project.client());
}

#[test]
fn compiling_the_scripts_parent_keeps_target_boundaries() {
    let project = Project::new();
    let scripts = project.0.join("scripts");
    let pack = project.0.join("pack");
    assert!(runec::compile_memory(&scripts, Some(&pack), false).is_ok());
    assert!(clientscript::compile_directory(&scripts, &pack).is_ok());
    project.write(
        "scripts/runescript/main.rs2",
        "[proc,rune_entry]()(obj) return(client_obj);\n",
    );
    project.write(
        "scripts/clientscript/main.cs2",
        "[proc,client_entry]()(obj) return(rune_obj);\n",
    );
    assert!(runec::compile_memory(&scripts, Some(&pack), false).is_err());
    assert!(runec::compile_memory_with_options(&scripts, Some(&pack), false, false, true).is_err());
    assert!(clientscript::compile_directory(&scripts, &pack).is_err());
}

#[test]
fn private_configs_fail_as_type_errors_with_lint_disabled() {
    for (folder, ext, entry, forbidden) in [
        ("runescript", "rs2", "rune_entry", "client"),
        ("clientscript", "cs2", "client_entry", "rune"),
        ("globalscript", "gs2", "shared", "rune"),
        ("globalscript", "gs2", "shared", "client"),
    ] {
        let project = Project::new();
        let path = format!(
            "scripts/{folder}/{}.{}",
            if folder == "globalscript" {
                "shared"
            } else {
                "main"
            },
            ext
        );
        project.write(
            &path,
            &format!("[proc,{entry}]()(obj) return({forbidden}_obj);\n"),
        );
        let pack = project.0.join("pack");
        if folder != "clientscript" {
            let error =
                runec::compile_memory(&project.0.join("scripts/runescript"), Some(&pack), false)
                    .unwrap_err();
            assert!(
                matches!(
                    error.downcast_ref::<CompilerError>(),
                    Some(CompilerError::TypeError(_))
                ),
                "{folder}: {error}"
            );
        }
        if folder != "runescript" {
            let error =
                clientscript::compile_directory(&project.0.join("scripts/clientscript"), &pack)
                    .unwrap_err()
                    .to_string();
            assert!(
                error.contains(&format!("obj '{forbidden}_obj' is not visible to {folder}")),
                "{error}"
            );
        }
    }
}

#[test]
fn stat_and_type_names_do_not_resolve_as_private_configs() {
    let project = Project::new();
    project.write(
        "pack/obj.pack",
        "1=shared_obj\n2=rune_obj\n3=client_obj\n4=smithing\n",
    );
    project.write("pack/stat.pack", "13=smithing\n");
    project.write("pack/param.pack", "0=namedobj\n");
    project.write(
        "scripts/runescript/collisions.obj",
        "// @scope(server)\n[smithing]\n",
    );
    project.write(
        "scripts/runescript/collisions.param",
        "// @scope(server)\n[namedobj]\ntype=int\n",
    );
    for (ty, value, expected) in [
        ("stat", "smithing", 13),
        ("char", "namedobj", i32::from(b'O')),
    ] {
        project.write(
            "scripts/globalscript/shared.gs2",
            &format!("[proc,shared]()({ty}) return({value});\n"),
        );
        project.write(
            "scripts/runescript/main.rs2",
            &format!("[proc,rune_entry]()({ty}) return(~shared);\n"),
        );
        project.write(
            "scripts/clientscript/main.cs2",
            &format!("[proc,client_entry]()({ty}) return(~shared);\n"),
        );
        assert!(project.rune(false), "runescript rejected {ty} {value}");
        let client = clientscript::compile_directory(
            &project.0.join("scripts/clientscript"),
            &project.0.join("pack"),
        )
        .unwrap_or_else(|error| panic!("clientscript rejected {ty} {value}: {error}"));
        let format = Format::load(&project.0.join("pack/cs2_command.pack")).unwrap();
        let shared = Script::decode(&client[&1], &format).unwrap();
        assert_eq!(shared.operations[0], Opcode::PushConstantInt);
        assert_eq!(shared.int_operands[0], expected);
    }
}

#[test]
fn feature_folders_and_legacy_folder_names_do_not_select_the_target() {
    let project = Project::new();
    let scripts = project.0.join("scripts");
    let pack = project.0.join("pack");
    let original_client = clientscript::compile_directory(&scripts, &pack).unwrap();
    for (from, to) in [
        ("runescript/main.rs2", "bank/clientscript/main.rs2"),
        ("clientscript/main.cs2", "bank/runescript/main.cs2"),
        ("globalscript/shared.gs2", "bank/_shared/shared.gs2"),
        ("clientscript/engine.cs2", "system/commands/engine.cs2"),
        ("runescript/values.constant", "bank/server.constant"),
        ("clientscript/values.constant", "bank/client.constant"),
        ("globalscript/values.constant", "bank/shared.constant"),
    ] {
        let destination = scripts.join(to);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::rename(scripts.join(from), destination).unwrap();
    }
    project.write("scripts/system/commands/engine.rs2", "");
    let normal = runec::compile_memory(&scripts, Some(&pack), false).unwrap();
    let nested = scripts.join("bank/runescript");
    assert_eq!(
        normal,
        runec::compile_memory(&nested, Some(&pack), false).unwrap()
    );
    assert_eq!(
        normal,
        runec::compile_memory_with_options(&scripts, Some(&pack), false, false, true).unwrap()
    );
    assert_eq!(
        original_client,
        clientscript::compile_directory(&nested, &pack).unwrap()
    );
    assert_eq!(
        sources::script_files(&nested, "gs2").unwrap(),
        vec![scripts.join("bank/_shared/shared.gs2")]
    );
    for client in [false, true] {
        let mut editor = runec::filesystem::editor_project(&nested, Some(&pack), client).unwrap();
        assert!(!editor.check().diagnostics.has_errors());
    }
}

#[test]
fn private_constants_with_the_same_name_keep_their_target_values() {
    let project = Project::new();
    project.write(
        "scripts/runescript/values.constant",
        "// @scope(server)\n^target = 21\n",
    );
    project.write(
        "scripts/clientscript/values.constant",
        "// @scope(client)\n^target = 84\n",
    );
    project.write(
        "scripts/runescript/main.rs2",
        "[proc,rune_entry]()(int) return(^target);\n",
    );
    project.write(
        "scripts/clientscript/main.cs2",
        "[proc,client_entry]()(int) return(^target);\n",
    );
    let pack = project.0.join("pack");
    let source = project.0.join("scripts");
    let server = runec::filesystem::editor_project(&source, Some(&pack), false).unwrap();
    let client = runec::filesystem::editor_project(&source, Some(&pack), true).unwrap();
    for (editor, expected) in [(server, 21), (client, 84)] {
        assert!(matches!(
            editor.registry().lookup_constant("target").map(|symbol| &symbol.kind),
            Some(runec::symbol::SymbolKind::Constant { int_value: Some(value), .. }) if *value == expected
        ));
    }
    assert!(project.rune(false));
    assert!(project.client());
    project.write(
        "scripts/globalscript/shared.gs2",
        "[proc,shared]()(int) return(^target);\n",
    );
    assert!(!project.rune(false));
    assert!(!project.client());
}

#[test]
fn duplicate_engine_files_and_invalid_config_scope_are_errors() {
    let project = Project::new();
    project.write("scripts/commands/engine.cs2", "");
    let source = project.0.join("scripts");
    let error = sources::engine_file(&source, Scope::Client)
        .unwrap_err()
        .to_string();
    assert!(error.contains("Multiple engine.cs2 files"));
    assert!(!project.client());
    project.write("scripts/commands/engine.rs2", "");
    project.write("scripts/bank/engine.rs2", "");
    assert!(!project.rune(false));
    assert!(!project.rune(true));
    project.write(
        "scripts/bank/invalid.constant",
        "// @scope(clinet)\n^bad = 1\n",
    );
    assert!(
        sources::target_config_files(&source, Scope::Rune)
            .unwrap_err()
            .to_string()
            .contains("expected // @scope")
    );
    assert_eq!(
        Scope::for_file(Path::new("bank/runescript/view.cs2"), "// @scope(server)\n").unwrap(),
        Scope::Client
    );
    assert!(
        Scope::for_file(
            Path::new("items.obj"),
            "// @scope(server)\n// @scope(client)\n"
        )
        .is_err()
    );
}
