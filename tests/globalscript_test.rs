use runec::clientscript;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Project(PathBuf);

impl Project {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("globalscript")
            .join(format!(
                "globalscript-{}-{}",
                std::process::id(),
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
            ("scripts/runescript/items.obj", "[rune_obj]\n"),
            ("scripts/clientscript/items.obj", "[client_obj]\n"),
            ("scripts/globalscript/vars.varp", "[shared_var]\ntype=int\n"),
            ("scripts/runescript/vars.varp", "[rune_var]\ntype=int\n"),
            ("scripts/clientscript/vars.varp", "[client_var]\ntype=int\n"),
            ("scripts/globalscript/values.constant", "^shared = 42\n"),
            ("scripts/runescript/values.constant", "^rune = 21\n"),
            ("scripts/clientscript/values.constant", "^client = 84\n"),
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
fn moving_a_config_to_globalscript_grants_both_targets_access() {
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
    assert!(project.client());
    project.write("scripts/globalscript/wrong.cs2", "[proc,wrong]()\n");
    assert!(!project.rune(false));
    assert!(!project.client());
}

#[test]
fn shared_constants_and_procs_cannot_be_shadowed_by_private_definitions() {
    let project = Project::new();
    project.write("scripts/runescript/values.constant", "^shared = 21\n");
    assert!(!project.rune(false));
    assert!(!project.client());
    project.write("scripts/runescript/values.constant", "^rune = 21\n");
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
