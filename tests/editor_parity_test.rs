use runec::lsp::project_from_json;
use runec::project::Project;
use runec::sources::Scope;
use serde_json::json;
use std::cmp::Reverse;

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
            {"name":"varc_int_192","id":192,"type":"int","category":"varc_int","scope":"client"},
            {"name":"varc_int_1038","id":1038,"type":"int","category":"varc_int","scope":"client"},
            {"name":"varc_int_1324","id":1324,"type":"int","category":"varc_int","scope":"client"},
            {"name":"varc_int_wrong","id":192,"type":"int","category":"varp","scope":"client"}
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
            "[proc,caller]\nif_setonvarctransmit(\"clientscript_1465{empty}{{varc_int_192, varc_int_1038, varc_int_1324}}\", 762:95);\nif_setonvarctransmit(\"callback(7){{varc_int_192, varc_int_1038}}\", 762:95);\n"
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
        let trigger = text.find("varc_int_192").unwrap();
        assert!(project.hover(doc, trigger).unwrap().detail.contains("192"));
        let completions = project.completions(doc, trigger + "varc_int_".len(), 20);
        assert!(completions.iter().any(|item| item.label == "varc_int_192"));
        assert!(
            !completions
                .iter()
                .any(|item| item.label == "varc_int_wrong")
        );
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
