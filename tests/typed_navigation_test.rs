use runec::ide::{SymbolKind, TokenKind};
use runec::lsp::project_from_json;
use runec::project::{Project, SourceId};
use runec::sources::Scope;
use serde_json::json;

fn project(scope: Scope) -> (Project, [SourceId; 3]) {
    let names = ["weapon_poison", "other_poison", "final_poison"];
    let entities = ["struct", "obj", "npc"]
        .into_iter()
        .enumerate()
        .flat_map(|(kind, ty)| {
            names.into_iter().enumerate().map(move |(id, name)| {
                json!({"name":name,"type":ty,"id":kind * 10 + id,"scope":"global"})
            })
        })
        .collect::<Vec<_>>();
    let mut project = project_from_json(&json!({
        "target":if scope == Scope::Client { "client" } else { "rune" },
        "opcodes":"0=PushConstantInt\n3=PushConstantString\n21=Return\n33=PushIntLocal\n34=PopIntLocal\n1000=take_obj\n",
        "commands":[{"name":"take_obj","opcode":1000,"params":["obj"]}],
        "entities":entities,
    }))
    .unwrap();
    let configs = ["struct", "obj", "npc"].map(|kind| {
        project.open_config(
            format!("mem:all.{kind}"),
            Scope::Global,
            1,
            "[weapon_poison]\n[other_poison]\n[final_poison]\n",
            kind,
        )
    });
    (project, configs)
}

fn assert_type(
    project: &Project,
    source: SourceId,
    offset: usize,
    config: SourceId,
    kind: &str,
    name: &str,
) {
    let definition = project.definition(source, offset).unwrap();
    assert_eq!(definition.source, config, "{kind} {name} at {offset}");
    assert_eq!(
        &project.document(config).unwrap().text[definition.range],
        name
    );
    assert_eq!(
        project.hover(source, offset).unwrap().detail,
        format!("{kind} {name}")
    );
}

#[test]
fn switch_navigation_uses_each_switch_type_and_keeps_references_separate() {
    let text = "[proc,test]\n\
switch_obj(weapon_poison) {\n\
    case weapon_poison, other_poison :\n\
        switch_struct(weapon_poison) {\n\
            case weapon_poison : take_obj(weapon_poison);\n\
        }\n\
    case final_poison : take_obj(weapon_poison);\n\
    case default : return;\n\
}\n";
    for scope in [Scope::Rune, Scope::Client, Scope::Global] {
        let (mut project, [structure, obj, _]) = project(scope);
        let source = project.open("mem:test", scope, 1, text);
        let occurrences = text
            .match_indices("weapon_poison")
            .map(|(offset, _)| offset)
            .collect::<Vec<_>>();
        let targets = [obj, obj, structure, structure, obj, obj];
        assert_eq!(occurrences.len(), targets.len());
        for (&offset, config) in occurrences.iter().zip(targets) {
            assert_type(
                &project,
                source,
                offset,
                config,
                if config == obj { "obj" } else { "struct" },
                "weapon_poison",
            );
        }
        for name in ["other_poison", "final_poison"] {
            assert_type(&project, source, text.find(name).unwrap(), obj, "obj", name);
        }
        for config in [obj, structure] {
            let expected = occurrences
                .iter()
                .zip(targets)
                .filter_map(|(&offset, target)| (target == config).then_some(offset))
                .collect::<Vec<_>>();
            let selected = expected[0];
            let references = project.references(source, selected, true);
            assert_eq!(references.len(), expected.len() + 1);
            assert!(
                references
                    .iter()
                    .any(|reference| reference.source == config)
            );
            let mut actual = references
                .iter()
                .filter(|reference| reference.source == source)
                .map(|reference| reference.range.start)
                .collect::<Vec<_>>();
            actual.sort_unstable();
            assert_eq!(actual, expected);
            let edits = project.rename(source, selected, "renamed_poison").unwrap();
            assert_eq!(edits.len(), references.len());
            assert!(edits.iter().all(|edit| {
                edit.replacement == "renamed_poison" && references.contains(&edit.location)
            }));
        }
    }
}

#[test]
fn declarations_assignments_returns_and_comparisons_resolve_the_expected_type() {
    let text = "[proc,test](obj $held, npc $creature)(obj, npc)\n\
def_obj $item = weapon_poison;\n\
def_npc $person = weapon_poison;\n\
$held = weapon_poison;\n\
$creature = weapon_poison;\n\
$held, $creature = weapon_poison, weapon_poison;\n\
if ($held = weapon_poison) { }\n\
if (weapon_poison = $held) { }\n\
if ($creature = weapon_poison) { }\n\
if (weapon_poison = $creature) { }\n\
return(weapon_poison, weapon_poison);\n";
    for scope in [Scope::Rune, Scope::Client, Scope::Global] {
        let (mut project, [_, obj, npc]) = project(scope);
        let source = project.open("mem:test", scope, 1, text);
        let occurrences = text
            .match_indices("weapon_poison")
            .map(|(offset, _)| offset)
            .collect::<Vec<_>>();
        let targets = [obj, npc, obj, npc, obj, npc, obj, obj, npc, npc, obj, npc];
        assert_eq!(occurrences.len(), targets.len());
        for (offset, config) in occurrences.into_iter().zip(targets) {
            assert_type(
                &project,
                source,
                offset,
                config,
                if config == obj { "obj" } else { "npc" },
                "weapon_poison",
            );
        }
    }
}

#[test]
fn a_missing_expected_config_type_does_not_navigate_to_an_unrelated_type() {
    let mut project = project_from_json(&json!({
        "commands":[{"name":"take_obj","opcode":1000,"params":["obj"]}],
        "entities":[{"name":"weapon_poison","type":"struct","id":1,"scope":"global"}],
    }))
    .unwrap();
    let structure = project.open_config(
        "mem:all.struct",
        Scope::Global,
        1,
        "[weapon_poison]\n",
        "struct",
    );
    let text = "[proc,test]\ntake_obj(weapon_poison);\n";
    let source = project.open("mem:test", Scope::Rune, 1, text);
    let offset = text.find("weapon_poison").unwrap();
    assert_eq!(project.definition(source, offset), None);
    assert!(project.hover(source, offset).is_none());
    assert!(project.references(source, offset, true).is_empty());
    assert!(project.rename(source, offset, "renamed_poison").is_err());
    let refs = project.references(structure, 1, true);
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].source, structure);
}

#[test]
fn typed_switches_keep_bare_command_arguments_highlighted_and_navigable() {
    let mut project = project_from_json(&json!({
        "commands":[
            {"name":"last_useitem","opcode":1,"returns":["obj"]},
            {"name":"last_slot","opcode":2,"returns":["int"]},
            {"name":"last_useslot","opcode":3,"returns":["int"]}
        ],
        "constants":[{"name":"dm_default","type":"int","value":0,"scope":"global"}],
        "entities":[
            {"name":"weapon_poison","type":"struct","id":1,"scope":"global"},
            {"name":"weapon_poison","type":"obj","id":2,"scope":"global"},
            {"name":"tbwt_cleaning_cloth","type":"obj","id":3,"scope":"global"}
        ]
    }))
    .unwrap();
    project.open_config(
        "mem:command.pack",
        Scope::Global,
        1,
        "1=last_useitem\n2=last_slot\n3=last_useslot\n",
        "pack:command",
    );
    let engine = project.open(
        "mem:engine.rs2",
        Scope::Rune,
        1,
        "[command,last_useitem]()(obj)\n[command,last_slot]()(int)\n[command,last_useslot]()(int)\n",
    );
    project.open(
        "mem:helpers.rs2",
        Scope::Rune,
        1,
        "[label,make_weapon_poisoned](int $weapon_slot, int $poison_slot)\n[label,clean_weapon_poison](int $slot)\n[proc,displaymessage](int $message)\n",
    );
    project.open_config(
        "mem:all.struct",
        Scope::Global,
        1,
        "[weapon_poison]\n",
        "struct",
    );
    let obj = project.open_config(
        "mem:all.obj",
        Scope::Global,
        1,
        "[weapon_poison]\n[tbwt_cleaning_cloth]\n",
        "obj",
    );
    let text = "[opheldu,_weapon_stab]\nswitch_obj(last_useitem) {\n    case weapon_poison : @make_weapon_poisoned(last_slot, last_useslot);\n    case tbwt_cleaning_cloth : @clean_weapon_poison(last_slot);\n    case default : ~displaymessage(^dm_default);\n}\n";
    let source = project.open("mem:weapon.rs2", Scope::Rune, 1, text);
    let tokens = project.semantic_tokens(source);
    for name in ["last_useitem", "last_slot", "last_useslot"] {
        for (offset, _) in text.match_indices(name) {
            let token = tokens
                .iter()
                .find(|token| token.range == (offset..offset + name.len()))
                .unwrap_or_else(|| panic!("Missing token for {name} at {offset}"));
            assert_eq!(token.kind, TokenKind::Function, "{name} at {offset}");
            assert!(token.default_library, "{name} at {offset}");
            assert_eq!(
                project.hover(source, offset).unwrap().kind,
                SymbolKind::Command
            );
            let definition = project.definition(source, offset).unwrap();
            assert_eq!(definition.source, engine);
            assert_eq!(
                &project.document(engine).unwrap().text[definition.range],
                name
            );
        }
    }
    for name in ["weapon_poison", "tbwt_cleaning_cloth"] {
        assert_type(&project, source, text.find(name).unwrap(), obj, "obj", name);
    }
}
