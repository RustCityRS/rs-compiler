use runec::lsp::project_from_json;
use runec::project::{Project, SourceId};
use runec::sources::Scope;
use runec::symbol::SymbolRegistry;
use runec::symloader::load_packs;
use runec::types::Type;
use serde_json::json;
use std::cmp::Reverse;
use std::fs;
use std::path::Path;

#[test]
fn quoted_callbacks_have_code_navigation_completion_highlighting_and_diagnostics() {
    for client in [false, true] {
        let mut project = project_from_json(&json!({
            "target": if client { "client" } else { "rune" },
            "opcodes":"0=PushConstantInt\n3=PushConstantString\n21=Return\n33=PushIntLocal\n34=PopIntLocal\n1407=if_setontimer\n",
            "commands":[
                {"name":"if_setontimer","opcode":1407,"params":["clientscript","string","component"],"kind":"hook_component"},
                {"name":"runclientscript","opcode":5000,"params":["int"]},
                {"name":"runclientscriptvararg","opcode":5013,"params":["int"]}
            ],
            "constants":[{"name":"limit","type":"int","value":99,"scope":"global"}],
            "variables":[{"name":"known","type":"int","id":1,"category":"varp","scope":"global"}],
            "scriptIds":[{"trigger":"clientscript","name":"callback","id":2}],
            "sources":[{"uri":"mem:callback","languageId":"cs2","text":"[clientscript,callback](int $first, int $second, int $third)\n"}]
        })).unwrap();
        let call = if client {
            "if_setontimer(\"callback($value, ^limit, %known)\", 762:95);"
        } else {
            "runclientscript(\"callback($value, ^limit, %known)\");"
        };
        let text = format!("[proc,caller]\ndef_int $value = 1;\n{call}\n");
        let doc = project.open(
            "mem:caller",
            if client { Scope::Client } else { Scope::Rune },
            1,
            text.clone(),
        );
        let analysis = project.analyze();
        assert!(
            !analysis.diagnostics.has_errors(),
            "{:?}",
            analysis.diagnostics
        );
        let callback = text.find("callback(").unwrap();
        let argument = text.rfind("$value").unwrap() + 1;
        let definition = project.definition(doc, callback).unwrap();
        assert_eq!(
            project
                .document(definition.source)
                .unwrap()
                .label
                .to_str()
                .unwrap(),
            "mem:callback"
        );
        assert_eq!(project.definition(doc, argument).unwrap().source, doc);
        assert!(project.hover(doc, argument).unwrap().detail.contains("int"));
        assert_eq!(
            project
                .signature_help(doc, text.find("^limit").unwrap())
                .unwrap()
                .active_parameter,
            1
        );
        assert!(
            project
                .completions(doc, callback + 4, 20)
                .iter()
                .any(|item| item.label == "callback")
        );
        assert!(
            project
                .rename(doc, argument, "renamed")
                .unwrap()
                .iter()
                .any(|edit| edit.location.range.start == argument)
        );
        let tokens = project.semantic_tokens(doc);
        assert!(
            tokens.iter().any(|token| token.range.contains(&callback)
                && token.kind != runec::ide::TokenKind::String)
        );
        assert!(
            tokens.iter().any(|token| token.range.contains(&argument)
                && token.kind != runec::ide::TokenKind::String)
        );
        project
            .update(doc, 2, text.replace("callback($value", "callback($missing"))
            .unwrap();
        let analysis = project.analyze();
        assert!(
            analysis
                .diagnostics
                .errors()
                .iter()
                .any(|diagnostic| diagnostic.line == 3 && diagnostic.message.contains("missing")),
            "{:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn transmit_lists_keep_outer_and_callback_hints_separate() {
    let mut project = project_from_json(&json!({
        "target":"client",
        "opcodes":"0=PushConstantInt\n3=PushConstantString\n21=Return\n2407=if_setonvarctransmit\n",
        "commands":[
            {"name":"if_setonvarctransmit","opcode":2407,"params":["clientscript","string","component"],"kind":"hook_component"}
        ],
        "variables":[
            {"name":"varc_192","id":192,"type":"int","category":"varc","scope":"client"},
            {"name":"varc_1038","id":1038,"type":"int","category":"varc","scope":"client"},
            {"name":"varc_1324","id":1324,"type":"int","category":"varc","scope":"client"},
            {"name":"varc_wrong","id":192,"type":"int","category":"varp","scope":"client"}
        ],
        "scriptIds":[
            {"trigger":"proc","name":"clientscript_1465","id":1465},
            {"trigger":"clientscript","name":"callback","id":1466}
        ],
        "sources":[{"uri":"mem:callbacks","languageId":"cs2","text":
            "[proc,clientscript_1465]\n[clientscript,callback](int $count)\n"}]
    })).unwrap();
    for empty in ["()", ""] {
        let text = format!(
            "[proc,caller]\nif_setonvarctransmit(\"clientscript_1465{empty}{{varc_192, varc_1038, varc_1324}}\", 762:95);\nif_setonvarctransmit(\"callback(7){{varc_192, varc_1038}}\", 762:95);\n"
        );
        let doc = project.open(
            format!("mem:caller:{empty}"),
            Scope::Client,
            1,
            text.clone(),
        );
        let hints = project.inlay_hints(doc, 0..text.len());
        let components = hints
            .iter()
            .filter(|hint| hint.label == "component:")
            .map(|hint| hint.offset)
            .collect::<Vec<_>>();
        assert_eq!(
            components,
            text.match_indices("762:95")
                .map(|(offset, _)| offset)
                .collect::<Vec<_>>()
        );
        assert!(
            hints
                .iter()
                .any(|hint| hint.label == "count:" && hint.offset == text.find("(7)").unwrap() + 1)
        );
        assert_eq!(
            project
                .signature_help(doc, text.find("762:95").unwrap())
                .unwrap()
                .active_parameter,
            1
        );
        let trigger = text.find("varc_192").unwrap();
        assert!(project.hover(doc, trigger).unwrap().detail.contains("192"));
        let completions = project.completions(doc, trigger + "varc_".len(), 20);
        assert!(completions.iter().any(|item| item.label == "varc_192"));
        assert!(!completions.iter().any(|item| item.label == "varc_wrong"));
        project.close(doc);
    }
}

#[test]
fn client_hooks_check_the_callback_declaration_instead_of_its_id_alias() {
    let mut project = project_from_json(&json!({
        "target":"client",
        "opcodes":"0=PushConstantInt\n3=PushConstantString\n21=Return\n1407=cc_setonvarctransmit\n",
        "commands":[
            {"name":"cc_setonvarctransmit","opcode":1407,"params":["clientscript","string"],"kind":"hook"}
        ],
        "scriptIds":[
            {"trigger":"proc","name":"clientscript_1472","id":1472},
            {"trigger":"clientscript","name":"caller","id":1473}
        ]
    })).unwrap();
    let callback = project.open(
        "mem:callback",
        Scope::Client,
        1,
        "[proc,clientscript_1472]\n",
    );
    let text = "[clientscript,caller]\ncc_setonvarctransmit(\"clientscript_1472(){190}\");\n";
    let caller = project.open("mem:caller", Scope::Client, 1, text);
    let analysis = project.analyze();
    assert!(
        !analysis.diagnostics.has_errors(),
        "{:?}",
        analysis.diagnostics
    );
    assert_eq!(
        project
            .definition(caller, text.find("clientscript_1472").unwrap())
            .unwrap()
            .source,
        callback
    );

    project.close(callback);
    project.open("mem:server", Scope::Rune, 1, "[proc,clientscript_1472]\n");
    let analysis = project.analyze();
    assert!(
        analysis
            .diagnostics
            .errors()
            .iter()
            .any(|diagnostic| diagnostic.line == 2 && diagnostic.message.contains("not visible")),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn server_callbacks_import_decompiled_client_procs_and_refresh_their_signatures() {
    let mut project = project_from_json(&json!({
        "target":"rune",
        "commands":[
            {"name":"runclientscript","opcode":5000,"params":["clientscript"]},
            {"name":"runclientscriptvararg","opcode":5013,"params":["int"]}
        ]
    }))
    .unwrap();
    let client = project.open(
        "mem:client",
        Scope::Client,
        1,
        "[proc,clientscript_3218](string $name)\n",
    );
    let text = "[proc,caller]\n.runclientscript(\"clientscript_3218(\\\"name\\\")\");\n";
    let caller = project.open("mem:caller", Scope::Rune, 1, text);
    let analysis = project.analyze();
    assert!(
        !analysis.diagnostics.has_errors(),
        "{:?}",
        analysis.diagnostics
    );
    let callback = text.find("clientscript_3218").unwrap();
    assert_eq!(project.definition(caller, callback).unwrap().source, client);
    assert!(
        project
            .completions(caller, callback + 8, 20)
            .iter()
            .any(|item| item.label == "clientscript_3218")
    );
    project
        .update(
            client,
            2,
            "[proc,clientscript_3218](int $component, string $name)\n",
        )
        .unwrap();
    let analysis = project.analyze();
    assert!(
        analysis
            .diagnostics
            .errors()
            .iter()
            .any(|diagnostic| diagnostic.line == 2 && diagnostic.message.contains("expects")),
        "{:?}",
        analysis.diagnostics
    );
}

fn project() -> Project {
    project_from_json(&json!({
        "target":"rune",
        "commands":[
            {"name":"inv_add","opcode":1,"params":["inv","obj","int"]},
            {"name":"mes","opcode":2,"params":["string"]},
            {"name":"uid","opcode":3,"returns":["player_uid"]}
        ],
        "entities":[
            {"name":"bank","type":"inv","id":1,"scope":"global"},
            {"name":"shared","type":"obj","id":2,"scope":"global"},
            {"name":"shared","type":"npc","id":3,"scope":"global"}
        ],
        "constants":[{"name":"limit","type":"int","value":99,"scope":"global"}],
        "sources":[
            {"uri":"mem:engine","languageId":"gs2","text":"[command,inv_add](inv $inventory, obj $item, int $count)\n[command,mes](string $message)\n[command,uid]()(player_uid)\n"},
            {"uri":"mem:items.obj","config":"obj","text":"[shared]\nname=Shared item\ncost=10\n"},
            {"uri":"mem:npcs.npc","config":"npc","text":"[shared]\nname=Shared NPC\n"},
            {"uri":"mem:obj.pack","config":"pack:obj","text":"2=shared\n"},
            {"uri":"mem:limits.constant","config":"constant","text":"^limit = 99\n"}
        ]
    })).unwrap()
}

#[test]
fn typed_config_navigation_hover_and_rename_include_pack_declarations() {
    let mut project = project();
    let text = "[proc,test]\ninv_add(bank, shared, ^limit);\n// shared\nmes(\"shared\");\n";
    let doc = project.open("mem:test", Scope::Global, 1, text);
    let offset = text.find("shared").unwrap();
    let definition = project.definition(doc, offset).unwrap();
    assert_eq!(
        project
            .document(definition.source)
            .unwrap()
            .label
            .to_str()
            .unwrap(),
        "mem:items.obj"
    );
    assert!(
        project
            .hover(doc, offset)
            .unwrap()
            .documentation
            .contains("cost=10")
    );
    let edits = project.rename(doc, offset, "new_item").unwrap();
    assert_eq!(edits.len(), 3, "{edits:?}");
    assert_eq!(
        edits
            .iter()
            .filter(|edit| edit.location.source == doc)
            .count(),
        1
    );
    let constant = project
        .definition(doc, text.find("limit").unwrap())
        .unwrap();
    assert_eq!(
        project
            .document(constant.source)
            .unwrap()
            .label
            .to_str()
            .unwrap(),
        "mem:limits.constant"
    );
    let config = project.open_config(
        "mem:food.obj",
        Scope::Global,
        1,
        "[cheese+tom_batta]\n",
        "obj",
    );
    let text = "[proc,food]\ninv_add(bank, cheese+tom_batta, 1);\n";
    let food = project.open("mem:food", Scope::Global, 1, text);
    let offset = text.find("cheese+").unwrap();
    assert_eq!(project.definition(food, offset + 8).unwrap().source, config);
    assert_eq!(
        project
            .rename(food, offset, "cheese+onion_batta")
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn qualified_config_references_preserve_column_and_component_declarations_on_rename() {
    let mut project = project_from_json(&json!({
        "commands":[
            {"name":"touch","opcode":1,"params":["component"]},
            {"name":"find","opcode":2,"params":["dbcolumn"]}
        ],
        "sources":[
            {"uri":"mem:interface.pack","config":"pack:interface","text":"1=panel\n1:2=button\n"},
            {"uri":"mem:table.dbtable","config":"dbtable","text":"[table]\ncolumn=field,int,string\n"}
        ]
    })).unwrap();
    let text = "[if_button1,panel:button]\nreturn;\n[proc,test]\ntouch(panel:button);\nfind(table:field:0);\n";
    let doc = project.open("mem:test", Scope::Rune, 1, text);
    let component = text.rfind("panel:button").unwrap();
    let definition = project.definition(doc, component).unwrap();
    assert_eq!(
        &project.document(definition.source).unwrap().text[definition.range],
        "button"
    );
    assert_eq!(project.references(doc, component, false).len(), 2);
    let edits = project.rename(doc, component, "close").unwrap();
    assert_eq!(
        edits
            .iter()
            .filter(|edit| edit.replacement == "panel:close")
            .count(),
        2
    );
    assert!(edits.iter().any(|edit| edit.replacement == "close"));
    let column = text.find("table:field:0").unwrap();
    let definition = project.definition(doc, column).unwrap();
    assert_eq!(
        &project.document(definition.source).unwrap().text[definition.range],
        "field"
    );
    let edits = project.rename(doc, column, "value").unwrap();
    assert!(edits.iter().any(|edit| edit.replacement == "value"));
    assert!(edits.iter().any(|edit| edit.replacement == "table:value:0"));
    let table = project
        .documents()
        .find(|doc| doc.label.to_str() == Some("mem:table.dbtable"))
        .unwrap()
        .id;
    let edits = project.rename(table, 1, "renamed").unwrap();
    assert!(
        edits
            .iter()
            .any(|edit| edit.replacement == "renamed:field:0")
    );
}

const CONFIG_TABLES: &[(&str, Type)] = &[
    ("area", Type::Area),
    ("bas", Type::Bas),
    ("billboard", Type::Billboard),
    ("particleeffector", Type::ParticleEffector),
    ("particleemitter", Type::ParticleEmitter),
    ("controller", Type::Controller),
    ("cursor", Type::Cursor),
    ("flo", Type::Flo),
    ("flu", Type::Flu),
    ("hitmark", Type::Hitmark),
    ("hunt", Type::Hunt),
    ("itemcode", Type::ItemCode),
    ("light", Type::Light),
    ("mel", Type::MapElement),
    ("msi", Type::Msi),
    ("quest", Type::Quest),
    ("varc", Type::Varc),
    ("varclan", Type::VarClan),
    ("varclansetting", Type::VarClanSetting),
    ("varcon", Type::VarCon),
    ("varconbit", Type::VarConBit),
    ("varnbit", Type::VarNBit),
    ("varobj", Type::VarObj),
];

fn config_project() -> Project {
    project_from_json(&json!({})).unwrap()
}

/// The definition of `word` where `after` ends in `text`.
fn definition_after(project: &Project, doc: SourceId, text: &str, after: &str) -> Option<SourceId> {
    let offset = text.find(after).unwrap() + after.len();
    project.definition(doc, offset).map(|found| found.source)
}

#[test]
fn a_config_value_resolves_by_the_type_its_key_holds() {
    let mut project = config_project();
    let hunt = project.open_config("mem:all.hunt", Scope::Global, 1, "[cow]\n", "hunt");
    let seq = project.open_config("mem:all.seq", Scope::Global, 1, "[cow]\n", "seq");
    let text = "[cow]\nhuntmode=cow\nreadyanim=cow\nname=cow\n";
    let npc = project.open_config("mem:all.npc", Scope::Global, 1, text, "npc");
    assert_eq!(
        definition_after(&project, npc, text, "huntmode="),
        Some(hunt)
    );
    assert_eq!(
        definition_after(&project, npc, text, "readyanim="),
        Some(seq)
    );
    assert_eq!(definition_after(&project, npc, text, "name="), None);
    let found = project.references(npc, text.find("huntmode=").unwrap() + 9, true);
    assert_eq!(found.len(), 2);
}

#[test]
fn param_enum_and_dbrow_values_take_their_type_from_what_declares_them() {
    let mut project = config_project();
    let npc = project.open_config("mem:all.npc", Scope::Global, 1, "[cow]\n", "npc");
    let obj = project.open_config("mem:all.obj", Scope::Global, 1, "[cow]\n", "obj");
    project.open_config(
        "mem:all.param",
        Scope::Global,
        1,
        "[p_npc]\ntype=npc\n\n[p_obj]\ntype=obj\n\n[p_text]\ntype=string\n",
        "param",
    );
    let text = "[s]\nparam=p_npc,cow\nparam=p_obj,cow\nparam=p_text,cow\n";
    let doc = project.open_config("mem:all.struct", Scope::Global, 1, text, "struct");
    assert_eq!(definition_after(&project, doc, text, "p_npc,"), Some(npc));
    assert_eq!(definition_after(&project, doc, text, "p_obj,"), Some(obj));
    assert_eq!(definition_after(&project, doc, text, "p_text,"), None);

    let text = "[e]\ninputtype=npc\noutputtype=obj\nval=cow,cow\n";
    let doc = project.open_config("mem:all.enum", Scope::Global, 1, text, "enum");
    assert_eq!(definition_after(&project, doc, text, "val="), Some(npc));
    assert_eq!(definition_after(&project, doc, text, "val=cow,"), Some(obj));

    let table = project.open_config(
        "mem:all.dbtable",
        Scope::Global,
        1,
        "[t]\ncolumn=held,obj,npc,LIST\n",
        "dbtable",
    );
    let text = "[r]\ntable=t\ndata=held,cow,cow\n";
    let doc = project.open_config("mem:all.dbrow", Scope::Global, 1, text, "dbrow");
    assert_eq!(definition_after(&project, doc, text, "table="), Some(table));
    assert_eq!(definition_after(&project, doc, text, "data="), Some(table));
    assert_eq!(definition_after(&project, doc, text, "held,"), Some(obj));
    assert_eq!(
        definition_after(&project, doc, text, "held,cow,"),
        Some(npc)
    );
}

#[test]
fn a_sprite_name_keeps_its_comma() {
    let mut project = config_project();
    let sprites = project.open_config(
        "mem:sprites.pack",
        Scope::Global,
        1,
        "3=icon,2\n",
        "pack:sprites",
    );
    let text = "[msi_0]\nsprite=icon,2\n";
    let msi = project.open_config("mem:all.msi", Scope::Global, 1, text, "msi");
    assert_eq!(
        definition_after(&project, msi, text, "sprite="),
        Some(sprites)
    );
}

#[test]
fn a_name_is_one_value_whatever_it_holds() {
    let mut project = config_project();
    let obj = project.open_config("mem:all.obj", Scope::Global, 1, "[na've_pouch]\n", "obj");
    let text = "[inv_1]\nstock1=na've_pouch,5,100\n";
    let inv = project.open_config("mem:all.inv", Scope::Global, 1, text, "inv");
    assert_eq!(definition_after(&project, inv, text, "stock1="), Some(obj));
    assert!(project.config_diagnostics().diagnostics().is_empty());
}

#[test]
fn a_config_value_can_be_an_id_or_a_table_placeholder() {
    let mut project = config_project();
    project.open_config(
        "mem:sprites.pack",
        Scope::Global,
        1,
        "3=icon,2\n",
        "pack:sprites",
    );
    project.open_config("mem:all.seq", Scope::Global, 1, "[walk]\n", "seq");
    project.open_config(
        "mem:all.msi",
        Scope::Global,
        1,
        "[msi_0]\nsprite=1804\n\n[msi_1]\nsprite=sprites_168\n\n[msi_2]\nsprite=null\n",
        "msi",
    );
    project.open_config(
        "mem:all.bas",
        Scope::Global,
        1,
        "[bas_0]\nwalkanim=seq_12\nreadyanim=seq_12_npc\nrunanim=12\n",
        "bas",
    );
    let found = project
        .config_diagnostics()
        .diagnostics()
        .iter()
        .map(|diagnostic| diagnostic.message.clone())
        .collect::<Vec<_>>();
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_config_value_of_the_wrong_type_is_reported() {
    let mut project = config_project();
    project.open_config("mem:all.hunt", Scope::Global, 1, "[cowardly]\n", "hunt");
    project.open_config(
        "mem:all.npc",
        Scope::Global,
        1,
        "[cow]\nhuntmode=cow\n\n[bull]\nhuntmode=nothing\n\n[calf]\nhuntmode=cowardly\nhuntmode=7\nhuntmode=null\nreadyanim=no_seq_table_loaded\n",
        "npc",
    );
    let mut found = project
        .config_diagnostics()
        .diagnostics()
        .iter()
        .map(|diagnostic| (diagnostic.line, diagnostic.message.clone()))
        .collect::<Vec<_>>();
    found.sort();
    assert_eq!(
        found,
        [
            (2, "'cow' is a npc, expected hunt".to_owned()),
            (5, "Unknown hunt 'nothing'".to_owned()),
        ]
    );
}

#[test]
fn config_completion_offers_what_the_key_holds() {
    let mut project = config_project();
    project.open_config("mem:all.hunt", Scope::Global, 1, "[cowardly]\n", "hunt");
    let text = "[cow]\nhuntmode=co\nmembers=\nhun";
    let npc = project.open_config("mem:all.npc", Scope::Global, 1, text, "npc");
    let labels = |offset: usize| {
        project
            .completions(npc, offset, 20)
            .into_iter()
            .map(|item| item.label)
            .collect::<Vec<_>>()
    };
    assert_eq!(labels(text.find("=co").unwrap() + 3), ["cowardly"]);
    assert_eq!(labels(text.find("members=").unwrap() + 8), ["no", "yes"]);
    assert_eq!(labels(text.len()), ["huntmode", "huntrange"]);
}

#[test]
fn hovering_a_config_key_shows_what_it_holds() {
    let mut project = config_project();
    let text = "[man]\nreadyanim=x\nparam=a,b\nrecol1s=2\nunknownkey=3\n";
    let npc = project.open_config("mem:all.npc", Scope::Global, 1, text, "npc");
    let hover = |key: &str| {
        project
            .hover(npc, text.find(key).unwrap() + 1)
            .map(|hover| hover.detail)
    };
    assert_eq!(hover("readyanim").as_deref(), Some("readyanim: seq"));
    assert_eq!(hover("param").as_deref(), Some("param: param, param value"));
    assert_eq!(hover("recol1s").as_deref(), Some("recol1s: int"));
    assert_eq!(hover("unknownkey"), None);
}

#[test]
fn every_config_table_is_a_type_scripts_can_navigate_and_complete() {
    for &(extension, ty) in CONFIG_TABLES {
        assert_eq!(Type::from_name(extension), Some(ty));
        let name = format!("{extension}_0");
        let mut project = project_from_json(&json!({
            "commands":[{"name":"touch","opcode":1,"params":[extension]}],
            "entities":[{"name":name,"type":extension,"id":0,"scope":"global"}],
        }))
        .unwrap();
        let config = project.open_config(
            format!("mem:all.{extension}"),
            Scope::Global,
            1,
            format!("[{name}]\n"),
            extension,
        );
        let text = format!("[proc,test]\ntouch({name});\n");
        let doc = project.open("mem:test", Scope::Rune, 1, text.as_str());
        let offset = text.find(&name).unwrap();
        assert_eq!(
            project.definition(doc, offset).map(|found| found.source),
            Some(config),
            "{extension}"
        );
        assert!(
            project
                .hover(doc, offset)
                .is_some_and(|hover| hover.detail.contains(&name)),
            "{extension}"
        );
        assert!(
            project
                .completions(doc, offset + extension.len(), 20)
                .iter()
                .any(|item| item.label == name),
            "{extension}"
        );
    }
}

#[test]
fn load_packs_reads_every_config_table() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("config_table_packs");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let tables = || {
        CONFIG_TABLES
            .iter()
            .filter(|(extension, _)| !matches!(*extension, "varc" | "varclansetting"))
    };
    for (extension, _) in tables() {
        fs::write(
            dir.join(format!("{extension}.pack")),
            format!("7={extension}_7\n"),
        )
        .unwrap();
    }
    let mut registry = SymbolRegistry::new();
    load_packs(&mut registry, &dir);
    for &(extension, ty) in tables() {
        assert_eq!(
            registry.lookup_reference(&format!("{extension}_7"), ty),
            Some(7),
            "{extension}"
        );
    }
}

#[test]
fn var_tables_resolve_through_their_game_vars() {
    let mut registry = SymbolRegistry::new();
    registry.register_game_var("varc_1".into(), Type::Int, 1, "varc".into());
    registry.register_game_var("varc_2".into(), Type::String, 2, "varcstr".into());
    registry.register_game_var("clan_3".into(), Type::Int, 3, "clansetting_int".into());
    assert_eq!(registry.lookup_reference("varc_1", Type::Varc), Some(1));
    assert_eq!(registry.lookup_reference("varc_2", Type::Varc), Some(2));
    assert_eq!(
        registry.lookup_reference("clan_3", Type::VarClanSetting),
        Some(3)
    );
    assert_eq!(registry.lookup_reference("clan_3", Type::Varc), None);
}

#[test]
fn scripts_reach_map_element_and_clan_setting_configs() {
    let mut project = project_from_json(&json!({
        "commands":[{"name":"touch","opcode":1,"params":["mapelement"]}],
        "variables":[{"name":"varclansetting_0","type":"int","id":0,"category":"clansetting_int","scope":"global"}],
    }))
    .unwrap();
    let mel = project.open_config("mem:all.mel", Scope::Global, 1, "[mel_5]\n", "mel");
    let clan = project.open_config(
        "mem:all.varclansetting",
        Scope::Global,
        1,
        "[varclansetting_0]\ntype=int\n",
        "varclansetting",
    );
    let text = "[proc,test]\ntouch(mel_5);\ndef_int $value = %varclansetting_0;\n";
    let doc = project.open("mem:test", Scope::Rune, 1, text);
    let definition = |project: &Project, word: &str| {
        project
            .definition(doc, text.find(word).unwrap())
            .map(|location| location.source)
    };
    assert_eq!(definition(&project, "mel_5"), Some(mel));
    assert_eq!(definition(&project, "varclansetting_0"), Some(clan));
}

#[test]
fn string_interpolation_resolves_commands_locals_and_mesanim_configs() {
    let mut project = project();
    let config = project.open_config(
        "mem:chat.mesanim",
        Scope::Global,
        1,
        "[p,neutral]\nlen1=chatneutral\n",
        "mesanim",
    );
    let text = "[proc,test](int $count)\nmes(\"<p,neutral><uid> <$count>\");\n";
    let doc = project.open("mem:chat", Scope::Global, 1, text);
    assert_eq!(
        project
            .definition(doc, text.find("p,neutral").unwrap())
            .unwrap()
            .source,
        config
    );
    assert!(
        project
            .hover(doc, text.find("uid").unwrap())
            .unwrap()
            .detail
            .contains("command,uid")
    );
    assert_eq!(
        project
            .references(doc, text.rfind("count").unwrap(), true)
            .len(),
        2
    );
    let tokens = project.semantic_tokens(doc);
    for pair in tokens.windows(2) {
        assert!(pair[0].range.end <= pair[1].range.start, "{pair:?}");
    }
}

#[test]
fn parameter_return_and_constant_hints_and_typed_completion_match_call_context() {
    let mut project = project();
    let text = "[proc,test]\ninv_add(bank, shared, ^limit);\nuid;\nmes(\"inv_add(bank, shared, 1) ^limit\");\n";
    let doc = project.open("mem:test", Scope::Global, 1, text);
    let hints = project.inlay_hints(doc, 0..text.len());
    assert!(hints.iter().any(|hint| hint.label == "inventory:"));
    assert!(hints.iter().any(|hint| hint.label == "item:"));
    assert!(hints.iter().any(|hint| hint.label == "count:"));
    assert_eq!(hints.iter().filter(|hint| hint.label == ": int").count(), 1);
    assert!(hints.iter().any(|hint| hint.label == "-> playeruid"));
    for (text, expected) in [
        ("[proc,test]\ninv_add(", "bank"),
        ("[proc,test]\ninv_add(bank, ", "shared"),
        ("[opnpc1,sha", "shared"),
    ] {
        let doc = project.open(format!("mem:completion:{expected}"), Scope::Global, 1, text);
        let names = project
            .completions(doc, text.len(), 100)
            .into_iter()
            .map(|item| item.label)
            .collect::<Vec<_>>();
        assert_eq!(names, vec![expected]);
    }
    let doc = project.open("mem:trigger", Scope::Rune, 1, "[opnp");
    assert!(
        project
            .completions(doc, 5, 100)
            .iter()
            .any(|item| item.label == "opnpc1")
    );
    let doc = project.open("mem:keyword", Scope::Rune, 1, "[proc,key]\ndef_i");
    assert!(
        project
            .completions(doc, 16, 100)
            .iter()
            .any(|item| item.label == "def_int")
    );
}

#[test]
fn extract_preserves_input_and_changed_output_values() {
    let mut project = project();
    let text = "[proc,total](int $count)(int)\n$count = calc($count + 1);\nreturn($count);\n";
    let doc = project.open("mem:total", Scope::Global, 1, text);
    let start = text.find("$count =").unwrap();
    let end = text[start..].find(';').unwrap() + start + 1;
    let action = project.extract_proc(doc, start..end, "increment").unwrap();
    let mut changed = text.to_string();
    let mut edits = action.edits;
    edits.sort_by_key(|edit| Reverse(edit.location.range.start));
    for edit in edits {
        changed.replace_range(edit.location.range, &edit.replacement);
    }
    assert!(changed.contains("$count = ~increment($count);"));
    project.update(doc, 2, changed).unwrap();
    let analysis = project.analyze();
    assert!(
        !analysis.diagnostics.has_errors(),
        "{:?}",
        analysis.diagnostics.diagnostics()
    );
}

#[test]
fn quick_fixes_suggest_known_commands_and_lints_use_unsaved_buffers() {
    let mut project = project();
    let doc = project.open(
        "mem:test",
        Scope::Global,
        1,
        "[proc,test]\nmse(\"hello\");\n",
    );
    let analysis = project.analyze();
    let actions = project.quick_fixes(doc, analysis.diagnostics.diagnostics());
    assert!(
        actions
            .iter()
            .any(|action| action.title == "Change to 'mes'"),
        "{actions:?}"
    );
    project
        .update(doc, 2, "[proc,test]\ndef_int $unused = 42;\nreturn;\n")
        .unwrap();
    let analysis = project.check();
    assert!(
        analysis
            .diagnostics
            .diagnostics()
            .iter()
            .any(|d| d.message.to_lowercase().contains("unused")),
        "{:?}",
        analysis.diagnostics.diagnostics()
    );
}

#[test]
fn extraction_preserves_returns_and_stays_before_the_test_boundary() {
    let mut project = project();
    let text = "[proc,total](int $count)(int)\nreturn(calc($count + 1));\n#testscript\n[proc,test_only]\nreturn;\n";
    let doc = project.open("mem:returns", Scope::Global, 1, text);
    let start = text.find("return(").unwrap();
    let end = start + text[start..].find(';').unwrap() + 1;
    let action = project.extract_proc(doc, start..end, "increment").unwrap();
    let mut changed = text.to_string();
    let mut edits = action.edits;
    edits.sort_by_key(|edit| Reverse(edit.location.range.start));
    for edit in edits {
        changed.replace_range(edit.location.range, &edit.replacement);
    }
    assert!(changed.contains("return(~increment($count));"));
    assert!(changed.find("[proc,increment]").unwrap() < changed.find("#testscript").unwrap());
    project.update(doc, 2, changed).unwrap();
    let analysis = project.analyze();
    assert!(
        !analysis.diagnostics.has_errors(),
        "{:?}",
        analysis.diagnostics.diagnostics()
    );
}

#[test]
fn extraction_rejects_partial_expressions_and_conditional_early_returns() {
    let mut project = project();
    let text = "[proc,total](int $count)(int)\nif ($count = 0) { return(0); }\nreturn(calc($count + 1));\n";
    let doc = project.open("mem:selection", Scope::Global, 1, text);
    let start = text.find("if (").unwrap();
    let end = start + text[start..].find('}').unwrap() + 1;
    assert!(
        project
            .extract_proc(doc, start..end, "conditional")
            .is_err()
    );
    let start = text.find("$count +").unwrap();
    assert!(
        project
            .extract_proc(doc, start..start + 10, "partial")
            .is_err()
    );
}
