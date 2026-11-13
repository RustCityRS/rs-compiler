use runec::diagnostics::{Diagnostic, Phase, Severity};
use runec::project::{Compilation, Options, Project, Target};
use runec::sources::{Scope, Visibility};
use runec::symbol::{ReturnType, SymbolRegistry};
use runec::types::Type;

fn project(check_pointers: bool) -> Project {
    use Type::*;
    let mut registry = SymbolRegistry::new();
    let commands = [
        ("last_useslot", vec![], vec![Int]),
        ("last_int", vec![], vec![Int]),
        ("last_com", vec![], vec![Component]),
        ("mes", vec![String], vec![]),
        ("mes_typed", vec![Int, String], vec![]),
        ("inv_getobj", vec![Inv, Int], vec![Obj]),
        ("inv_total", vec![Inv, Obj], vec![Int]),
        ("inv_del", vec![Inv, Obj, Int], vec![]),
        ("inv_add", vec![Inv, Obj, Int], vec![]),
        ("inv_setslot", vec![Inv, Int, Obj, Int], vec![]),
        ("oc_param", vec![Obj, Param], vec![Any]),
        ("oc_stackable", vec![Obj], vec![Boolean]),
        ("p_countdialog", vec![], vec![]),
        ("p_delay", vec![Int], vec![]),
        ("db_listall", vec![DbTable], vec![]),
        ("db_findnext", vec![], vec![DbRow]),
        ("allowdesign", vec![], vec![]),
        ("sound_speech_volume", vec![Int], vec![]),
        ("sound_speech_stop", vec![], vec![]),
        ("p_loaddelay", vec![], vec![]),
        ("p_finduid", vec![Int], vec![Boolean]),
        ("finduid", vec![Int], vec![Boolean]),
        ("p_stopaction", vec![], vec![]),
        ("npc_finduid", vec![Int], vec![Boolean]),
        ("npc_name", vec![], vec![String]),
        ("obj_find", vec![Coord, Obj], vec![Boolean]),
        ("obj_name", vec![], vec![String]),
        ("npc_delay", vec![Int], vec![]),
        ("world_delay", vec![Int], vec![]),
        ("last_string", vec![], vec![String]),
        ("p_stringdialog", vec![], vec![]),
        ("p_setdisplayname", vec![String], vec![]),
        ("p_namedialog", vec![], vec![]),
        ("p_clanforumqfcdialog", vec![], vec![]),
        ("p_objdialog", vec![], vec![]),
        ("p_hsldialog", vec![], vec![]),
        ("last_coord", vec![], vec![Coord]),
    ];
    for (opcode, (name, params, returns)) in commands.into_iter().enumerate() {
        registry.register_command(name.into(), 1000 + opcode as i32, params, returns);
    }
    registry.set_command_behavior("oc_param", false, None, Some(ReturnType::ParamArgument(1)));
    let mut visibility = Visibility::new();
    for (name, ty, id) in [
        ("inv", Inv, 0),
        ("weapon", Obj, 1),
        ("records", DbTable, 2),
        ("poison_severity", Param, 3),
        ("next_obj_stage", Param, 4),
        ("tbwt_weapon_poison_clear", Param, 5),
    ] {
        registry.register_entity_id(name.into(), ty, id);
        visibility.declare(ty.name(), name, Scope::Global);
    }
    registry.register_param_type(3, Int);
    registry.register_param_type(4, Obj);
    registry.register_param_type(5, String);
    registry.register_constant("dm_default".into(), Int, Some(0), None);
    visibility.declare("constant", "dm_default", Scope::Global);
    registry.set_visibility(visibility);
    Project::new(
        Target::Rune,
        registry,
        Options {
            check_pointers,
            ..Options::default()
        },
    )
}

fn compile(text: &str, check_pointers: bool) -> Compilation {
    let mut project = project(check_pointers);
    project.open("pointer.rs2", Scope::Rune, 1, text);
    let compilation = project.compile();
    assert!(
        !compilation.analysis.diagnostics.has_errors(),
        "{:?}",
        compilation.analysis.diagnostics
    );
    assert!(!compilation.artifacts.is_empty());
    compilation
}

fn implicit(compilation: &Compilation) -> Vec<&Diagnostic> {
    compilation
        .analysis
        .diagnostics
        .diagnostics()
        .iter()
        .filter(|diagnostic| {
            diagnostic.phase == Phase::PointerCheck
                && diagnostic.severity == Severity::Warning
                && diagnostic.message.starts_with("Implicit caller context")
        })
        .collect()
}

fn pointer_warnings(text: &str) -> Vec<String> {
    compile(text, true)
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
fn entity_commands_require_the_selected_player() {
    for command in [
        "allowdesign",
        "sound_speech_volume(1)",
        "sound_speech_stop",
        "last_com",
        "mes_typed(0, \"text\")",
    ] {
        for dot in ["", "."] {
            let text = format!("[ai_timer,_]\n{dot}{command};\n");
            let warnings = pointer_warnings(&text);
            assert!(
                warnings
                    .iter()
                    .any(|warning| warning.contains(&format!("{dot}active_player"))),
                "{text}: {warnings:?}"
            );
        }
    }
}

#[test]
fn load_delay_requires_protected_player_access() {
    let warnings = pointer_warnings("[ai_timer,_]\np_loaddelay;\n");
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("p_active_player")),
        "{warnings:?}"
    );
}

#[test]
fn only_the_find_result_can_establish_a_pointer() {
    for body in [
        "def_boolean $found = p_finduid(1); if ($flag = true) { p_stopaction; }",
        "def_boolean $found = p_finduid(1); if ($flag ! false) { p_stopaction; }",
        "def_boolean $found = p_finduid(1); if (~unrelated = true) { p_stopaction; }",
    ] {
        let text = format!(
            "[ai_timer,_](boolean $flag)\n{body}\n[proc,unrelated]()(boolean)\nreturn(true);\n"
        );
        let warnings = pointer_warnings(&text);
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("p_active_player")),
            "{text}: {warnings:?}"
        );
    }
    for body in [
        "if (p_finduid(1) = true) { p_stopaction; }",
        "if (p_finduid(1) = false) { return; } p_stopaction;",
        "if (p_finduid(1) ! false) { p_stopaction; }",
    ] {
        let text = format!("[ai_timer,_]\n{body}\n");
        assert!(pointer_warnings(&text).is_empty(), "{text}");
    }
}

#[test]
fn object_find_requires_a_successful_branch() {
    for dot in ["", "."] {
        for body in [
            format!("{dot}obj_find(0_0_0_0_0, weapon); mes({dot}obj_name);"),
            format!("if ({dot}obj_find(0_0_0_0_0, weapon) = false) {{ mes({dot}obj_name); }}"),
        ] {
            let text = format!("[debugproc,find]\n{body}\n");
            let warnings = pointer_warnings(&text);
            assert!(
                warnings
                    .iter()
                    .any(|warning| warning.contains(&format!("{dot}active_obj"))),
                "{text}: {warnings:?}"
            );
        }
        let text = format!(
            "[debugproc,find]\nif ({dot}obj_find(0_0_0_0_0, weapon) = true) {{ mes({dot}obj_name); }}\n"
        );
        assert!(pointer_warnings(&text).is_empty(), "{text}");
    }
}

#[test]
fn calls_propagate_entity_requirements_and_rebinding_corrupts_protection() {
    let text =
        "[debugproc,caller]\n~wrapper;\n[proc,wrapper]\n~read;\n[proc,read]\nmes(npc_name);\n";
    assert!(
        pointer_warnings(text)
            .iter()
            .any(|warning| warning.contains("active_npc"))
    );
    let text = "[debugproc,caller]\nif (npc_finduid(1) = true) { ~wrapper; }\n[proc,wrapper]\n~read;\n[proc,read]\nmes(npc_name);\n";
    assert!(pointer_warnings(text).is_empty());
    for dot in ["", "."] {
        let text = format!(
            "[debugproc,caller]\nif ({dot}p_finduid(1) = true) {{ ~rebind; {dot}p_stopaction; }}\n[proc,rebind]\n{dot}finduid(2);\n"
        );
        let warnings = pointer_warnings(&text);
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains(&format!("{dot}p_active_player"))),
            "{text}: {warnings:?}"
        );
    }
}

#[test]
fn suspension_releases_protected_player_access() {
    for delay in ["world_delay(1)", ".npc_delay(1)"] {
        let text = format!(
            "[debugproc,delay]\nif (p_finduid(1) = true & .npc_finduid(2) = true) {{ {delay}; p_stopaction; }}\n"
        );
        let warnings = pointer_warnings(&text);
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("p_active_player")),
            "{text}: {warnings:?}"
        );
    }
}

#[test]
fn label_jumps_propagate_entity_requirements() {
    let text =
        "[debugproc,caller]\n@wrapper;\n[label,wrapper]\n@read;\n[label,read]\nmes(npc_name);\n";
    let warnings = pointer_warnings(text);
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("active_npc")),
        "{warnings:?}"
    );
}

#[test]
fn recursive_calls_keep_transitive_entity_requirements() {
    let first =
        "[proc,first](boolean $again)\nif ($again = true) { ~second(false); }\nmes(npc_name);\n";
    let second = "[proc,second](boolean $again)\nif ($again = true) { ~first(false); }\n";
    let caller = "[debugproc,caller]\n~second(true);\n";
    for text in [
        format!("{first}{second}{caller}"),
        format!("{caller}{second}{first}"),
    ] {
        let warnings = pointer_warnings(&text);
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("active_npc")),
            "{text}: {warnings:?}"
        );
    }
    let caller = "[debugproc,caller]\nif (npc_finduid(1) = true) { ~second(true); }\n";
    assert!(pointer_warnings(&format!("{first}{second}{caller}")).is_empty());
    let first =
        "[proc,first](boolean $again)\nif ($again = true) { ~second(false); }\nfinduid(2);\n";
    let caller = "[debugproc,caller]\nif (p_finduid(1) = true) { ~second(true); p_stopaction; }\n";
    let warnings = pointer_warnings(&format!("{first}{second}{caller}"));
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("p_active_player")),
        "{warnings:?}"
    );
}

#[test]
fn weapon_cleaning_reports_both_implicit_slot_reads_without_changing_bytecode() {
    let text = r#"[label,clean_weapon_poison](int $weapon_slot)
def_obj $weapon = inv_getobj(inv, $weapon_slot);
def_int $count = inv_total(inv, $weapon);

def_string $clean_str;
if(oc_param($weapon, poison_severity) > 0) {
    if(oc_param($weapon, next_obj_stage) ! null) {
        $clean_str = oc_param($weapon, tbwt_weapon_poison_clear);
        if (oc_stackable($weapon) = true) {
            mes("<$clean_str>.");
            if(inv_total(inv, $weapon) > 0) {
                inv_del(inv, $weapon, $count);
                inv_add(inv, oc_param($weapon, next_obj_stage), $count);
            } else {
                inv_setslot(inv, last_useslot, oc_param($weapon, next_obj_stage), $count);
            }
        } else {
            mes("<$clean_str>.");
            inv_setslot(inv, last_useslot, oc_param($weapon, next_obj_stage), 1);
        }
        return;
    }

}
~displaymessage(^dm_default);

[proc,displaymessage](int $message)
mes("Nothing happens.");
"#;
    let checked = compile(text, true);
    let warnings = implicit(&checked);
    let expected = text
        .lines()
        .enumerate()
        .filter_map(|(line, text)| text.contains("last_useslot").then_some(line + 1))
        .collect::<Vec<_>>();
    assert_eq!(
        warnings
            .iter()
            .map(|warning| warning.line)
            .collect::<Vec<_>>(),
        expected
    );
    assert!(
        warnings
            .iter()
            .all(|warning| warning.message.contains("last_useslot")
                && warning.message.contains("explicit argument"))
    );
    assert!(
        !checked
            .analysis
            .diagnostics
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.phase == Phase::PointerCheck
                && diagnostic.message.contains("stale"))
    );
    let unchecked = compile(text, false);
    assert!(implicit(&unchecked).is_empty());
    let artifacts = |compilation: Compilation| {
        compilation
            .artifacts
            .into_iter()
            .map(|artifact| (artifact.id, artifact.name, artifact.bytes))
            .collect::<Vec<_>>()
    };
    assert_eq!(artifacts(checked), artifacts(unchecked));
}

#[test]
fn reusable_scripts_report_direct_event_and_iterator_reads_only() {
    for trigger in ["proc", "label"] {
        let text = format!(
            "[{trigger},read]\ndef_int $slot = last_useslot;\ndef_dbrow $row = db_findnext;\n"
        );
        let compilation = compile(&text, true);
        let warnings = implicit(&compilation);
        assert_eq!(warnings.len(), 2, "{trigger}: {warnings:?}");
        assert!(
            warnings
                .iter()
                .any(|warning| warning.message.contains("last_useslot"))
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.message.contains("find_db"))
        );
    }
    let entry = compile("[opheldu,weapon]\ndef_int $slot = last_useslot;\n", true);
    assert!(implicit(&entry).is_empty());
    assert!(
        !entry
            .analysis
            .diagnostics
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.phase == Phase::PointerCheck)
    );
    let transitive = compile(
        "[proc,read]\ndef_int $slot = last_useslot;\n[proc,wrapper]\n~read;\n",
        true,
    );
    let warnings = implicit(&transitive);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].line, 2);
    let explicit = compile(
        "[label,read](int $slot)\ninv_setslot(inv, $slot, weapon, 1);\n",
        true,
    );
    assert!(implicit(&explicit).is_empty());
}

#[test]
fn local_setters_suppress_implicit_context_only_on_paths_they_cover() {
    let local = compile(
        "[proc,read](boolean $flag)\np_countdialog;\ndef_int $count = last_int;\nif ($flag = true) {\n    db_listall(records);\n    def_dbrow $row = db_findnext;\n}\n",
        true,
    );
    assert!(implicit(&local).is_empty());
    let partial = compile(
        "[proc,read](boolean $flag)\nif ($flag = true) { db_listall(records); }\ndef_dbrow $row = db_findnext;\n",
        true,
    );
    let warnings = implicit(&partial);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].message.contains("find_db"));
    let inherited = compile(
        "[proc,read]\np_countdialog;\ndef_int $count = last_int;\ndef_int $slot = last_useslot;\n",
        true,
    );
    assert!(
        implicit(&inherited)
            .iter()
            .all(|warning| !warning.message.contains("last_int"))
    );
    let callee_setter = compile(
        "[proc,select]\ndb_listall(records);\n[proc,read]\n~select;\ndef_dbrow $row = db_findnext;\n",
        true,
    );
    assert!(implicit(&callee_setter).is_empty());
}

#[test]
fn string_dialogs_establish_last_string_until_the_next_suspension() {
    let inputs = |text: &str| -> Vec<String> {
        pointer_warnings(text)
            .into_iter()
            .filter(|warning| warning.contains("last_string") || warning.contains("last_int"))
            .collect()
    };
    for command in ["p_stringdialog", "p_namedialog", "p_clanforumqfcdialog"] {
        let dialog = inputs(&format!(
            "[opheld1,weapon]\n{command};\ndef_string $name = last_string;\n"
        ));
        assert!(dialog.is_empty(), "{command}: {dialog:?}");
    }
    for command in ["p_objdialog", "p_hsldialog"] {
        let dialog = inputs(&format!(
            "[opheld1,weapon]\n{command};\ndef_int $value = last_int;\n"
        ));
        assert!(dialog.is_empty(), "{command}: {dialog:?}");
    }
    let named = inputs(
        "[opheld1,weapon]\np_setdisplayname(\"name\");\ndef_int $result = last_int;\ndef_string $name = last_string;\n",
    );
    assert!(named.is_empty(), "{named:?}");
    let missing = inputs("[opheld1,weapon]\ndef_string $name = last_string;\n");
    assert!(
        missing
            .iter()
            .any(|warning| warning.contains("not available") && warning.contains("last_string")),
        "{missing:?}"
    );
    let stale =
        inputs("[opheld1,weapon]\np_stringdialog;\np_delay(1);\ndef_string $name = last_string;\n");
    assert!(
        stale
            .iter()
            .any(|warning| warning.contains("stale") && warning.contains("last_string")),
        "{stale:?}"
    );
}

#[test]
fn a_ground_target_gives_last_coord_until_the_next_suspension() {
    let coord = |text: &str| -> Vec<String> {
        pointer_warnings(text)
            .into_iter()
            .filter(|warning| warning.contains("last_coord"))
            .collect()
    };
    let aimed = coord("[apcoordt,_]\ndef_coord $tile = last_coord;\n");
    assert!(aimed.is_empty(), "{aimed:?}");
    let elsewhere = coord("[opheld1,weapon]\ndef_coord $tile = last_coord;\n");
    assert!(
        elsewhere
            .iter()
            .any(|warning| warning.contains("not available")),
        "{elsewhere:?}"
    );
    let stale = coord("[apcoordt,_]\np_delay(1);\ndef_coord $tile = last_coord;\n");
    assert!(
        stale.iter().any(|warning| warning.contains("stale")),
        "{stale:?}"
    );
}

#[test]
fn missing_caller_context_and_delay_keep_the_existing_pointer_warnings() {
    let missing = compile(
        "[opheld1,weapon]\n@read;\n[label,read]\ndef_int $slot = last_useslot;\n",
        true,
    );
    assert!(
        missing
            .analysis
            .diagnostics
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.phase == Phase::PointerCheck
                && diagnostic.severity == Severity::Warning
                && diagnostic.line == 2
                && diagnostic.message.contains("not available")
                && diagnostic.message.contains("last_useslot"))
    );
    let delayed = compile(
        "[opheldu,weapon]\np_delay(1);\ndef_int $slot = last_useslot;\n",
        true,
    );
    assert!(
        delayed
            .analysis
            .diagnostics
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.phase == Phase::PointerCheck
                && diagnostic.severity == Severity::Warning
                && diagnostic.line == 3
                && diagnostic.message.contains("stale")
                && diagnostic.message.contains("last_useslot"))
    );
}
