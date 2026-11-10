use super::*;
use std::cell::Cell;
use std::rc::Rc;

fn analysis_lenses(lenses: &Value) -> Value {
    Value::Array(
        lenses
            .as_array()
            .unwrap()
            .iter()
            .filter(|lens| lens["command"]["command"] == "runec.inspectScript")
            .cloned()
            .collect(),
    )
}

fn lens_labels(lenses: &Value) -> Value {
    Value::Array(
        lenses
            .as_array()
            .unwrap()
            .iter()
            .filter(|lens| lens["command"]["command"] == "runec.inspectScript")
            .map(|lens| json!({"range":lens["range"],"title":lens["command"]["title"]}))
            .collect(),
    )
}

#[test]
fn script_header_clicks_show_full_effects_and_requirements_while_paused() {
    let mut variables = (b'a'..=b'f').enumerate().map(|(id, name)| {
        json!({"name":(name as char).to_string(),"type":"int","id":id,"category":"vars","scope":"rs2"})
    }).collect::<Vec<_>>();
    variables.extend([
        json!({"name":"primary","type":"int","id":20,"category":"varp","scope":"rs2"}),
        json!({"name":"secondary","type":"int","id":21,"category":"varp","scope":"rs2"}),
        // The filesystem loader registers secondary aliases separately.
        json!({"name":".secondary","type":"int","id":21,"category":"varp","scope":"rs2"}),
        json!({"name":"beast","type":"int","id":22,"category":"varn","scope":"rs2"}),
    ]);
    let mut server = Server::new(None, None);
    let initialized = server.handle("initialize", &json!({
        "capabilities":{"workspace":{"applyEdit":true},"window":{"showDocument":{"support":true}}},
        "initializationOptions":{
            "commands":[{"name":"wait_once","opcode":9000,"params":[],"returns":[],"suspends":true}],
            "variables":variables,
            "sources":[
                {"uri":"mem:headers","languageId":"rs2","version":1,"text":"[proc,entry]\n~leaf;\n[proc,pure]\nreturn;\n"},
                {"uri":"mem:leaf","languageId":"rs2","version":1,"text":"[proc,leaf]\n%a = 1; %b = 1; %c = 1; %d = 1; %e = 1; %f = 1; %primary = 1; .%secondary = 2; %beast = 3; wait_once;\n"}
            ]
        }
    })).unwrap();
    assert!(
        initialized["capabilities"]["executeCommandProvider"]["commands"]
            .as_array()
            .unwrap()
            .contains(&json!("runec.inspectScript"))
    );
    let params = json!({"textDocument":{"uri":"mem:headers"}});
    assert_eq!(
        analysis_lenses(&server.request("textDocument/codeLens", &params).unwrap()),
        json!([])
    );
    server.idle(&mut Vec::new()).unwrap();
    assert!(server.project.effects_ready());
    assert!(server.project.pointer_requirements_ready());
    let checked = server.project.latest();
    assert!(
        !checked.diagnostics.has_errors(),
        "{:?}",
        checked.diagnostics
    );
    let lenses = analysis_lenses(&server.request("textDocument/codeLens", &params).unwrap());
    let effects = lenses
        .as_array()
        .unwrap()
        .iter()
        .find(|lens| {
            lens["command"]["title"]
                .as_str()
                .unwrap()
                .starts_with("@effects")
        })
        .unwrap();
    assert!(
        effects["command"]["title"]
            .as_str()
            .unwrap()
            .contains("+3 more")
    );
    let old_command = effects["command"].clone();
    server
        .request(
            "workspace/didChangeConfiguration",
            &json!({"settings":{"performance":{"analysisPaused":true}}}),
        )
        .unwrap();
    let generation = server.project.generation;
    for lens in lenses.as_array().unwrap() {
        assert_eq!(lens["command"]["command"], "runec.inspectScript");
        assert_eq!(
            lens["command"]["arguments"][0]["textDocument"],
            params["textDocument"]
        );
        assert_eq!(lens["command"]["arguments"][0]["range"], lens["range"]);
        assert_eq!(lens["command"]["arguments"][0]["version"], 1);
        assert_eq!(lens["command"]["arguments"][0]["generation"], generation);
        let mut output = Vec::new();
        server.process(json!({"jsonrpc":"2.0","id":10,"method":"workspace/executeCommand","params":lens["command"]}), CancellationToken::default(), &mut output).unwrap();
        let sent = messages(&output);
        let response = sent.iter().find(|message| message["id"] == 10).unwrap();
        assert!(response.get("error").is_none(), "{response}");
        let report = &response["result"];
        if report["script"] == "[proc,entry]" {
            assert_eq!(
                report["writes"],
                json!([
                    ".secondary",
                    "a",
                    "b",
                    "beast",
                    "c",
                    "d",
                    "e",
                    "f",
                    "primary"
                ])
            );
            assert_eq!(report["maySuspend"], true);
            assert_eq!(report["incomplete"], false);
            let pointers = report["requiredPointers"]
                .as_array()
                .unwrap_or_else(|| panic!("Missing pointer report: {report}"));
            assert_eq!(pointers.len(), 3);
            for pointer in ["active_player", ".active_player", "active_npc"] {
                assert!(pointers.contains(&json!(pointer)));
                assert!(report["markdown"].as_str().unwrap().contains(pointer));
            }
            for name in [
                ".%secondary",
                "%a",
                "%b",
                "%beast",
                "%c",
                "%d",
                "%e",
                "%f",
                "%primary",
            ] {
                assert!(report["markdown"].as_str().unwrap().contains(name));
            }
        } else {
            assert_eq!(report["script"], "[proc,pure]");
            assert_eq!(lens["command"]["title"], "@requires none");
            assert_eq!(report["writes"], json!([]));
            assert_eq!(report["requiredPointers"], json!([]));
            assert_eq!(report["maySuspend"], false);
        }
        let preview = sent
            .iter()
            .find(|message| message["method"] == "window/showDocument")
            .unwrap();
        assert_eq!(
            preview["params"]["selection"],
            json!({"start":{"line":0,"character":0},"end":{"line":0,"character":0}})
        );
        let path = uri_path(preview["params"]["uri"].as_str().unwrap()).unwrap();
        assert!(path.starts_with(std::env::temp_dir()));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            report["markdown"].as_str().unwrap()
        );
        let mut fallback = Vec::new();
        server
            .process(
                json!({"jsonrpc":"2.0","id":preview["id"],"result":{"success":false}}),
                CancellationToken::default(),
                &mut fallback,
            )
            .unwrap();
        assert!(
            messages(&fallback)
                .iter()
                .any(|message| message["method"] == "window/showMessage"
                    && message["params"]["message"] == report["markdown"])
        );
        fs::remove_file(path).unwrap();
        assert!(
            !sent
                .iter()
                .any(|message| message["method"] == "workspace/applyEdit")
        );
        assert_eq!(server.project.generation, generation);
    }
    // A callee edit changes the summary even though the caller's own version stays 1.
    server.request("textDocument/didChange", &json!({"textDocument":{"uri":"mem:leaf","version":2},"contentChanges":[{"text":"[proc,leaf]\nreturn;\n"}]})).unwrap();
    assert_eq!(
        server
            .project
            .document(server.find("mem:headers").unwrap())
            .unwrap()
            .version,
        1
    );
    let mut bad_version = old_command.clone();
    bad_version["arguments"][0]["generation"] = json!(server.project.generation);
    bad_version["arguments"][0]["version"] = json!(999);
    for command in [old_command, bad_version] {
        let mut output = Vec::new();
        server.process(json!({"jsonrpc":"2.0","id":20,"method":"workspace/executeCommand","params":command}), CancellationToken::default(), &mut output).unwrap();
        let sent = messages(&output);
        let expected = "Script analysis changed; refresh the header hints.";
        assert!(
            sent.iter()
                .any(|message| message["id"] == 20 && message["error"]["message"] == expected)
        );
        assert!(
            sent.iter()
                .any(|message| message["method"] == "window/showMessage"
                    && message["params"]["type"] == 1
                    && message["params"]["message"] == expected)
        );
        assert!(
            !sent
                .iter()
                .any(|message| message["method"] == "window/showDocument"
                    || message["method"] == "workspace/applyEdit")
        );
        assert!(!server.project.effects_ready());
        assert!(!server.project.pointer_requirements_ready());
    }
}

#[test]
fn script_header_clicks_keep_unavailable_analysis_unknown_without_running_it() {
    for effects_only in [true, false] {
        let mut server = Server::new(None, None);
        server.handle("initialize", &json!({"initializationOptions":{
            "performance":{"analysisPaused":true},
            "commands":[{"name":"wait_once","opcode":9000,"params":[],"returns":[],"suspends":true}],
            "sources":[{"uri":"mem:partial","languageId":"rs2","text":"[proc,entry]\nwait_once;\n"}]
        }})).unwrap();
        if effects_only {
            server.project.effect_summaries().unwrap();
        } else {
            assert!(!server.project.check().diagnostics.has_errors());
        }
        let lenses = analysis_lenses(
            &server
                .request(
                    "textDocument/codeLens",
                    &json!({"textDocument":{"uri":"mem:partial"}}),
                )
                .unwrap(),
        );
        assert_eq!(lenses.as_array().unwrap().len(), 1);
        let report = server
            .request("workspace/executeCommand", &lenses[0]["command"])
            .unwrap();
        assert_eq!(report["script"], "[proc,entry]");
        if effects_only {
            assert_eq!(report["requiredPointers"], Value::Null);
            assert_eq!(report["writes"], json!([]));
            assert_eq!(report["maySuspend"], true);
            assert!(!server.project.pointer_requirements_ready());
        } else {
            assert_eq!(report["requiredPointers"], json!([]));
            assert_eq!(report["writes"], Value::Null);
            assert_eq!(report["maySuspend"], Value::Null);
            assert_eq!(report["incomplete"], Value::Null);
            assert!(!server.project.effects_ready());
        }
    }
}

#[test]
fn effect_code_lenses_wait_for_idle_and_preserve_only_current_cache_while_paused() {
    let mut server = Server::new(None, None);
    server.handle("initialize", &json!({"initializationOptions":{
        "performance":{"analysisPaused":true},
        "commands":[{"name":"wait_once","opcode":77,"params":[],"returns":[],"suspends":true}],
        "sources":[{"uri":"mem:effects","languageId":"rs2","text":"[proc,entry]\n~leaf;\n[proc,leaf]\nwait_once;\n"}]
    }})).unwrap();
    let params = json!({"textDocument":{"uri":"mem:effects"}});
    assert_eq!(
        analysis_lenses(&server.request("textDocument/codeLens", &params).unwrap()),
        json!([])
    );
    assert!(!server.project.effects_ready());
    let mut output = Vec::new();
    server.idle(&mut output).unwrap();
    assert!(!server.project.effects_ready());
    server
        .request(
            "workspace/didChangeConfiguration",
            &json!({"settings":{"runec":{"performance":{"analysisPaused":false}}}}),
        )
        .unwrap();
    assert_eq!(
        analysis_lenses(&server.request("textDocument/codeLens", &params).unwrap()),
        json!([])
    );
    assert!(!server.project.effects_ready());
    server.idle(&mut output).unwrap();
    let lenses = analysis_lenses(&server.request("textDocument/codeLens", &params).unwrap());
    assert_eq!(
        lenses
            .as_array()
            .unwrap()
            .iter()
            .filter(|lens| lens["command"]["title"] == "@effects may suspend")
            .count(),
        2
    );
    server
        .request(
            "workspace/didChangeConfiguration",
            &json!({"settings":{"performance":{"analysisPaused":true}}}),
        )
        .unwrap();
    assert_eq!(
        analysis_lenses(&server.request("textDocument/codeLens", &params).unwrap()),
        lenses
    );
    server.request("textDocument/didChange", &json!({"textDocument":{"uri":"mem:effects","version":2},"contentChanges":[{"text":"[proc,entry]\nreturn;\n"}]})).unwrap();
    assert_eq!(
        analysis_lenses(&server.request("textDocument/codeLens", &params).unwrap()),
        json!([])
    );
    assert!(!server.project.effects_ready());
}

#[test]
fn bytecode_header_lenses_work_before_analysis_and_reject_stale_versions() {
    for (target, language, text, headers, script) in [
        (
            "rune",
            "rs2",
            "[proc,entry]\nmes(\"first\");\nmes(\"second\");\n[label,resume]\nreturn;\n[debugproc,inspect]\nreturn;\n[login,_]\nreturn;\n[command,declared]()\n",
            vec![0, 3, 5, 7],
            "[proc,entry]",
        ),
        (
            "client",
            "cs2",
            "[clientscript,entry]\nmes(\"first\");\nmes(\"second\");\n[proc,helper]\nreturn;\n[command,declared]()\n",
            vec![0, 3],
            "[clientscript,entry]",
        ),
    ] {
        let mut server = Server::new(None, None);
        let mut output = Vec::new();
        server.process(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "capabilities":{"workspace":{"applyEdit":true},"window":{"showDocument":{"support":true}}},
            "initializationOptions":{
                "target":target,"performance":{"analysisPaused":true},
                "opcodes":"0=PushConstantInt\n3=PushConstantString\n21=Return\n9000=mes\n",
                "commands":[{"name":"mes","opcode":9000,"params":["string"],"returns":[]}],
                "sources":[
                    {"uri":"mem:headers","languageId":language,"version":3,"text":text},
                    {"uri":"mem:shared","languageId":"gs2","version":3,"text":"[proc,shared]\nreturn;\n"},
                    {"uri":"mem:commands","languageId":language,"text":"[command,other]()\n"},
                    {"uri":"file:///game/test.flo","config":"flo","text":"[grass]\ncolour=0xff0000\n"}
                ]
            }
        }}), CancellationToken::default(), &mut output).unwrap();
        assert!(
            messages(&output)
                .iter()
                .all(|message| message.get("error").is_none())
        );
        let mut header_command = Value::Null;
        for (uri, expected_lines) in [
            ("mem:headers", headers),
            ("mem:shared", vec![0]),
            ("mem:commands", vec![]),
            ("file:///game/test.flo", vec![]),
        ] {
            output.clear();
            server.process(json!({"jsonrpc":"2.0","id":2,"method":"textDocument/codeLens","params":{"textDocument":{"uri":uri}}}), CancellationToken::default(), &mut output).unwrap();
            let sent = messages(&output);
            let response = sent.iter().find(|message| message["id"] == 2).unwrap();
            assert!(response.get("error").is_none(), "{response}");
            let lenses = response["result"].as_array().unwrap();
            assert_eq!(
                lenses.len(),
                expected_lines.len(),
                "{target} {uri}: {lenses:?}"
            );
            for (lens, line) in lenses.iter().zip(expected_lines) {
                let range =
                    json!({"start":{"line":line,"character":0},"end":{"line":line,"character":0}});
                assert_eq!(lens["range"], range);
                assert_eq!(lens["command"]["title"], "Inspect bytecode");
                assert_eq!(lens["command"]["command"], "runec.inspectBytecode");
                assert_eq!(
                    lens["command"]["arguments"],
                    json!([{"textDocument":{"uri":uri},"range":range,"version":3}])
                );
            }
            if uri == "mem:headers" {
                header_command = lenses[0]["command"].clone();
            }
            assert!(!server.project.pointer_requirements_ready());
            assert!(!server.project.effects_ready());
        }
        // Execute the command exactly as the client receives it, without an idle pass.
        output.clear();
        server.process(json!({"jsonrpc":"2.0","id":3,"method":"workspace/executeCommand","params":header_command}), CancellationToken::default(), &mut output).unwrap();
        let sent = messages(&output);
        let response = sent.iter().find(|message| message["id"] == 3).unwrap();
        assert!(response.get("error").is_none(), "{target}: {response}");
        let report = &response["result"];
        assert_eq!(report["script"], script);
        assert_eq!(report["wholeScript"], true);
        let instructions = report["instructions"].as_array().unwrap();
        let calls = instructions
            .iter()
            .filter(|row| row["instruction"] == "mes")
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 2);
        for call in calls {
            assert_eq!(
                call["stackEffect"],
                json!({"pops":[0,1,0],"pushes":[0,0,0]})
            );
        }
        assert!(
            instructions
                .iter()
                .any(|row| row["instruction"] == "Return")
        );
        let markdown = report["markdown"].as_str().unwrap();
        assert!(markdown.contains("entire containing script"));
        assert!(markdown.contains("first") && markdown.contains("second"));
        let preview = sent
            .iter()
            .find(|message| message["method"] == "window/showDocument")
            .unwrap();
        assert_eq!(
            preview["params"]["selection"],
            json!({"start":{"line":0,"character":0},"end":{"line":0,"character":0}})
        );
        let path = uri_path(preview["params"]["uri"].as_str().unwrap()).unwrap();
        assert!(path.starts_with(std::env::temp_dir()));
        assert_eq!(fs::read_to_string(&path).unwrap(), markdown);
        fs::remove_file(path).unwrap();
        assert!(
            !sent
                .iter()
                .any(|message| message["method"] == "workspace/applyEdit")
        );
        assert!(!server.project.pointer_requirements_ready());
        assert!(!server.project.effects_ready());
        output.clear();
        server
            .process(
                json!({"jsonrpc":"2.0","id":preview["id"],"result":{"success":true}}),
                CancellationToken::default(),
                &mut output,
            )
            .unwrap();
        server.process(json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":"mem:headers","version":4},"contentChanges":[{"text":text}]}}), CancellationToken::default(), &mut output).unwrap();
        output.clear();
        server.process(json!({"jsonrpc":"2.0","id":4,"method":"workspace/executeCommand","params":header_command}), CancellationToken::default(), &mut output).unwrap();
        let sent = messages(&output);
        let response = sent.iter().find(|message| message["id"] == 4).unwrap();
        assert!(
            response["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("script changed"))
        );
        assert!(
            sent.iter()
                .any(|message| message["method"] == "window/showMessage"
                    && message["params"]["type"] == 1
                    && message["params"]["message"] == response["error"]["message"])
        );
        assert!(
            !sent
                .iter()
                .any(|message| message["method"] == "window/showDocument"
                    || message["method"] == "workspace/applyEdit")
        );
        assert!(!server.project.pointer_requirements_ready());
        assert!(!server.project.effects_ready());
    }
}

#[test]
fn bytecode_inspector_is_available_while_paused_and_never_applies_an_edit() {
    let mut server = Server::new(None, None);
    let initialized = server.handle("initialize", &json!({
        "capabilities":{"workspace":{"applyEdit":true}},
        "initializationOptions":{
            "performance":{"analysisPaused":true},
            "target":"client",
            "opcodes":"0=PushConstantInt\n21=Return\n33=PushIntLocal\n34=PopIntLocal\n77=paint\n",
            "commands":[{"name":"paint","opcode":77,"params":["component","colour"],"returns":[],"stack":"component_last"}],
            "sources":[{"uri":"mem:inspect","languageId":"cs2","text":"[proc,main](component $component)\ndef_int $colour = 27;\npaint($component, $colour);\n"}]
        }
    })).unwrap();
    assert!(
        initialized["capabilities"]["executeCommandProvider"]["commands"]
            .as_array()
            .unwrap()
            .contains(&json!("runec.inspectBytecode"))
    );
    let actions = server
        .request(
            "textDocument/codeAction",
            &json!({
                "textDocument":{"uri":"mem:inspect"},
                "range":{"start":{"line":2,"character":1},"end":{"line":2,"character":1}},
                "context":{"triggerKind":2,"diagnostics":[]}
            }),
        )
        .unwrap();
    assert_eq!(actions.as_array().unwrap().len(), 1);
    assert_eq!(actions[0]["kind"], "runec.inspect");
    assert_eq!(actions[0]["command"]["command"], "runec.inspectBytecode");
    assert!(actions[0].get("edit").is_none());
    let mut output = Vec::new();
    server.process(json!({"jsonrpc":"2.0","id":2,"method":"workspace/executeCommand","params":actions[0]["command"]}), CancellationToken::default(), &mut output).unwrap();
    let messages = messages(&output);
    let response = messages.iter().find(|message| message["id"] == 2).unwrap();
    assert!(response.get("error").is_none(), "{response}");
    let instructions = response["result"]["instructions"].as_array().unwrap();
    assert_eq!(
        instructions
            .iter()
            .map(|row| row["instruction"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["PushIntLocal", "PushIntLocal", "paint"]
    );
    assert_eq!(instructions[0]["operand"], "1");
    assert_eq!(instructions[1]["operand"], "0");
    assert_eq!(response["result"]["netStackChange"], json!([0, 0, 0]));
    assert!(
        messages
            .iter()
            .any(|message| message["method"] == "window/showMessage")
    );
    assert!(
        !messages
            .iter()
            .any(|message| message["method"] == "workspace/applyEdit")
    );
    assert!(!server.project.pointer_requirements_ready());
}

#[test]
fn bytecode_action_accepts_highlighted_blocks_and_reports_cross_script_selections() {
    let mut server = Server::new(None, None);
    server.handle("initialize", &json!({
        "capabilities":{"workspace":{"applyEdit":true}},
        "initializationOptions":{
            "performance":{"analysisPaused":true},
            "commands":[{"name":"mes","opcode":9000,"params":["string"],"returns":[]}],
            "sources":[{"uri":"mem:blocks","languageId":"rs2","text":"[proc,main]\nmes(\"first\");\nmes(\"second\");\n[proc,other]\nmes(\"third\");\n"}]
        }
    })).unwrap();
    for (end_line, end_character, crosses_scripts) in [(2, 14, false), (4, 13, true)] {
        let range = json!({"start":{"line":1,"character":0},"end":{"line":end_line,"character":end_character}});
        let actions = server
            .request(
                "textDocument/codeAction",
                &json!({
                    "textDocument":{"uri":"mem:blocks"},"range":range,
                    "context":{"triggerKind":2,"diagnostics":[]}
                }),
            )
            .unwrap();
        let action = actions
            .as_array()
            .unwrap()
            .iter()
            .find(|action| action["command"]["command"] == "runec.inspectBytecode")
            .unwrap_or_else(|| {
                panic!("Inspector missing for highlighted range {range}: {actions}")
            });
        assert!(action.get("edit").is_none());
        assert_eq!(action["command"]["arguments"][0]["range"], range);
        // Merely asking for actions must not run deep analysis while paused.
        assert!(!server.project.pointer_requirements_ready());
        assert!(!server.project.effects_ready());
        assert!(server.pending_bytecode_previews.is_empty());
        let mut output = Vec::new();
        server.process(json!({"jsonrpc":"2.0","id":30,"method":"workspace/executeCommand","params":action["command"]}), CancellationToken::default(), &mut output).unwrap();
        let sent = messages(&output);
        let response = sent.iter().find(|message| message["id"] == 30).unwrap();
        if crosses_scripts {
            let error = response["error"]["message"].as_str().unwrap();
            assert_eq!(
                error,
                "The selection crosses script declarations. Select code within one script to inspect its bytecode."
            );
            assert!(
                sent.iter()
                    .any(|message| message["method"] == "window/showMessage"
                        && message["params"]["type"] == 1
                        && message["params"]["message"] == error)
            );
            assert!(
                !sent
                    .iter()
                    .any(|message| message["method"] == "window/showDocument"
                        || (message["method"] == "window/showMessage"
                            && message["params"]["type"] == 3))
            );
        } else {
            assert!(response.get("error").is_none(), "{response}");
            let report = &response["result"];
            assert_eq!(report["script"], "[proc,main]");
            assert_eq!(
                report["instructions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|instruction| instruction["instruction"] == "mes")
                    .count(),
                2
            );
            let markdown = report["markdown"].as_str().unwrap();
            assert!(markdown.contains("first") && markdown.contains("second"));
            assert!(!markdown.contains("third"));
            assert!(
                sent.iter()
                    .any(|message| message["method"] == "window/showMessage"
                        && message["params"]["type"] == 3
                        && message["params"]["message"] == markdown)
            );
        }
        assert!(
            !sent
                .iter()
                .any(|message| message["method"] == "workspace/applyEdit")
        );
        assert!(!server.project.pointer_requirements_ready());
        assert!(!server.project.effects_ready());
    }
}

#[test]
fn bytecode_action_stays_visible_for_script_errors_and_keeps_configs_excluded() {
    let mut server = Server::new(None, None);
    server.handle("initialize", &json!({"initializationOptions":{
        "performance":{"analysisPaused":true},
        "sources":[
            {"uri":"mem:broken","languageId":"rs2","text":"[proc,broken]\nmes(\n"},
            {"uri":"file:///game/test.flo","config":"flo","text":"[grass]\ncolour=0xff0000\n"}
        ]
    }})).unwrap();
    let range = json!({"start":{"line":1,"character":0},"end":{"line":1,"character":4}});
    let actions = server
        .request(
            "textDocument/codeAction",
            &json!({
                "textDocument":{"uri":"mem:broken"},"range":range,
                "context":{"triggerKind":2,"diagnostics":[]}
            }),
        )
        .unwrap();
    let action = actions
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["command"]["command"] == "runec.inspectBytecode")
        .unwrap();
    assert!(!server.project.pointer_requirements_ready());
    assert!(!server.project.effects_ready());
    let mut output = Vec::new();
    server.process(json!({"jsonrpc":"2.0","id":40,"method":"workspace/executeCommand","params":action["command"]}), CancellationToken::default(), &mut output).unwrap();
    let sent = messages(&output);
    let response = sent.iter().find(|message| message["id"] == 40).unwrap();
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|message| !message.is_empty())
    );
    assert!(
        sent.iter()
            .any(|message| message["method"] == "window/showMessage"
                && message["params"]["type"] == 1
                && message["params"]["message"] == response["error"]["message"])
    );
    let configs = server
        .request(
            "textDocument/codeAction",
            &json!({
                "textDocument":{"uri":"file:///game/test.flo"},"range":range,
                "context":{"triggerKind":2,"diagnostics":[]}
            }),
        )
        .unwrap();
    assert!(
        !configs
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["command"]["command"] == "runec.inspectBytecode")
    );
}

#[test]
fn bytecode_preview_supplies_lsp4ij_selection_and_falls_back_on_client_failure() {
    let mut server = Server::new(None, None);
    server.handle("initialize", &json!({
        "capabilities":{"window":{"showDocument":{"support":true}}},
        "initializationOptions":{
            "performance":{"analysisPaused":true},
            "commands":[{"name":"mes","opcode":9000,"params":["string"],"returns":[]}],
            "sources":[{"uri":"mem:preview","languageId":"rs2","text":"[proc,main]\nmes(\"hello\");\n"}]
        }
    })).unwrap();
    // This is the request shape used by LSP4IJ 0.21.0's intention actions.
    let actions = server
        .request(
            "textDocument/codeAction",
            &json!({
                "textDocument":{"uri":"mem:preview"},
                "range":{"start":{"line":1,"character":1},"end":{"line":1,"character":1}},
                "context":{"triggerKind":2,"diagnostics":[]}
            }),
        )
        .unwrap();
    let command = actions
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["command"]["command"] == "runec.inspectBytecode")
        .unwrap()["command"]
        .clone();
    for (reply, fallback) in [
        (json!({"result":{"success":false}}), true),
        (
            json!({"error":{"code":-32603,"message":"Cannot open document"}}),
            true,
        ),
        (json!({"result":{"success":true}}), false),
    ] {
        let mut output = Vec::new();
        server.process(json!({"jsonrpc":"2.0","id":2,"method":"workspace/executeCommand","params":command}), CancellationToken::default(), &mut output).unwrap();
        let sent = messages(&output);
        let preview = sent
            .iter()
            .find(|message| message["method"] == "window/showDocument")
            .unwrap();
        assert_eq!(
            preview["params"]["selection"],
            json!({"start":{"line":0,"character":0},"end":{"line":0,"character":0}})
        );
        let path = uri_path(preview["params"]["uri"].as_str().unwrap()).unwrap();
        assert!(path.starts_with(std::env::temp_dir()));
        let markdown = fs::read_to_string(&path).unwrap();
        assert!(markdown.contains("Bytecode: mes"));
        let mut response = reply;
        response["jsonrpc"] = json!("2.0");
        response["id"] = preview["id"].clone();
        output.clear();
        server
            .process(response.clone(), CancellationToken::default(), &mut output)
            .unwrap();
        let reported = messages(&output);
        assert_eq!(
            reported
                .iter()
                .any(|message| message["method"] == "window/showMessage"),
            fallback
        );
        if fallback {
            assert_eq!(reported[0]["params"]["message"], markdown);
        }
        assert!(server.pending_bytecode_previews.is_empty());
        // Repeated responses cannot show the same report twice.
        output.clear();
        server
            .process(response, CancellationToken::default(), &mut output)
            .unwrap();
        assert!(output.is_empty());
        fs::remove_file(path).unwrap();
    }
    // A client that never answers cannot retain an unbounded number of reports.
    for _ in 0..MAX_PENDING_BYTECODE_PREVIEWS + 1 {
        let mut output = Vec::new();
        server.process(json!({"jsonrpc":"2.0","id":3,"method":"workspace/executeCommand","params":command}), CancellationToken::default(), &mut output).unwrap();
        let sent = messages(&output);
        let preview = sent
            .iter()
            .find(|message| message["method"] == "window/showDocument")
            .unwrap();
        let path = uri_path(preview["params"]["uri"].as_str().unwrap()).unwrap();
        assert!(path.starts_with(std::env::temp_dir()));
        fs::remove_file(path).unwrap();
        assert!(server.pending_bytecode_previews.len() <= MAX_PENDING_BYTECODE_PREVIEWS);
    }
    assert_eq!(
        server.pending_bytecode_previews.len(),
        MAX_PENDING_BYTECODE_PREVIEWS
    );
    server.handle("shutdown", &Value::Null).unwrap();
    assert!(server.pending_bytecode_previews.is_empty());
}

#[test]
fn mutation_notifications_cancel_stale_queries_without_dropping_edits() {
    let query = CancellationToken::default();
    let initialize = CancellationToken::default();
    let pending = HashMap::from([
        ("query".to_owned(), (query.clone(), true)),
        ("initialize".to_owned(), (initialize.clone(), false)),
    ]);
    cancel_obsolete_requests("textDocument/hover", &pending);
    assert!(!query.is_cancelled());
    cancel_obsolete_requests("textDocument/didChange", &pending);
    assert!(query.is_cancelled());
    assert!(!initialize.is_cancelled());
    let mut server = Server::new(None, None);
    let mut output = Vec::new();
    server
        .process(
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
            initialize,
            &mut output,
        )
        .unwrap();
    server
        .process(
            json!({"jsonrpc":"2.0","id":2,"method":"workspace/symbol","params":{"query":""}}),
            query,
            &mut output,
        )
        .unwrap();
    server.process(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"mem:new","languageId":"rs2","version":1,"text":"[proc,retained]\nreturn;"}}}), CancellationToken::default(), &mut output).unwrap();
    assert_eq!(
        messages(&output)
            .iter()
            .find(|message| message["id"] == 2)
            .unwrap()["error"]["code"],
        -32800
    );
    assert!(server.find("mem:new").is_some());
}

#[test]
fn performance_pause_preserves_navigation_diagnostics_and_cached_requirements() {
    let mut server = Server::new(None, None);
    assert_eq!(server.performance.max_workers, 2);
    assert!(
        server
            .handle(
                "initialize",
                &json!({"initializationOptions":{"performance":{"maxWorkers":0}}})
            )
            .is_err()
    );
    server.handle("initialize", &json!({"initializationOptions":{"runec":{
        "performance":{"maxWorkers":1,"analysisPaused":true,"watchDebounceMs":50},
        "sources":[
            {"uri":"mem:script","languageId":"rs2","text":"[proc,good]\nreturn;\n"},
            {"uri":"mem:bad","languageId":"rs2","text":"[proc,bad]()(int)\nreturn(\"wrong\");\n"}
        ]
    }}})).unwrap();
    let params = json!({"textDocument":{"uri":"mem:script"}});
    server.request("textDocument/codeLens", &params).unwrap();
    let diagnostics = server
        .request(
            "textDocument/diagnostic",
            &json!({"textDocument":{"uri":"mem:bad"}}),
        )
        .unwrap();
    assert!(!diagnostics["items"].as_array().unwrap().is_empty());
    assert_eq!(
        server
            .request("textDocument/documentSymbol", &params)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(!server.project.pointer_requirements_ready());
    let mut output = Vec::new();
    server.idle(&mut output).unwrap();
    assert!(!server.project.pointer_requirements_ready());
    let generation = server.project.generation;
    server
        .request(
            "workspace/didChangeConfiguration",
            &json!({"settings":{"runec":{"performance":{"analysisPaused":false,"maxWorkers":2}}}}),
        )
        .unwrap();
    assert_eq!(server.project.generation, generation);
    server.idle(&mut output).unwrap();
    assert!(server.project.pointer_requirements_ready());
    server
        .request(
            "workspace/didChangeConfiguration",
            &json!({"settings":{"runec":{"performance":{"analysisPaused":true}}}}),
        )
        .unwrap();
    assert!(server.project.pointer_requirements_ready());
    let lenses = server.request("textDocument/codeLens", &params).unwrap();
    assert!(
        lenses
            .as_array()
            .unwrap()
            .iter()
            .any(|lens| lens["command"]["title"] == "@requires none")
    );
    let previous = server.performance;
    for invalid in [
        json!({"maxWorkers":33}),
        json!({"maxWorkers":1.5}),
        json!({"analysisPaused":"yes"}),
        json!({"watchDebounceMs":0}),
        json!({"unexpected":true}),
    ] {
        assert!(
            server
                .request(
                    "workspace/didChangeConfiguration",
                    &json!({"settings":{"runec":{"performance":invalid}}})
                )
                .is_err()
        );
        assert_eq!(server.performance, previous);
    }
}

#[test]
fn watched_notifications_debounce_across_navigation_requests_and_keep_open_buffers() {
    let loaded = project_from_json(
        &json!({"sources":[{"uri":"mem:one","languageId":"gs2","text":"[proc,old]\nreturn;"}]}),
    )
    .unwrap();
    let loads = Rc::new(Cell::new(0));
    let count = loads.clone();
    let loader = Box::new(move || {
        count.set(count.get() + 1);
        project_from_json(&json!({"sources":[
            {"uri":"mem:one","languageId":"gs2","text":"[proc,saved]\nreturn;"},
            {"uri":"mem:created","languageId":"gs2","text":"[proc,created]\nreturn;"}
        ]}))
    });
    let mut server = Server::new(Some(loaded), Some(loader));
    server.handle("initialize", &json!({"initializationOptions":{"performance":{"analysisPaused":true,"watchDebounceMs":50}}})).unwrap();
    server.request("textDocument/didOpen", &json!({"textDocument":{"uri":"mem:one","languageId":"gs2","version":1,"text":"[proc,unsaved]\nreturn;"}})).unwrap();
    let mut output = Vec::new();
    for (uri, kind) in [("mem:created", 1), ("mem:created", 2), ("mem:other", 1)] {
        server
            .request(
                "workspace/didChangeWatchedFiles",
                &json!({"changes":[{"uri":uri,"type":kind}]}),
            )
            .unwrap();
        server
            .process(
                json!({"jsonrpc":"2.0","id":2,"method":"workspace/symbol","params":{"query":""}}),
                CancellationToken::default(),
                &mut output,
            )
            .unwrap();
        server.settle(&mut output).unwrap();
        assert_eq!(loads.get(), 0);
    }
    assert_eq!(server.pending_watched.len(), 2);
    assert_eq!(server.pending_watched[&uri_key("mem:created")]["type"], 1);
    server.watched_at = Some(Instant::now());
    server.idle(&mut output).unwrap();
    assert_eq!(loads.get(), 0);
    server.watched_at = Some(Instant::now() - server.performance.watch_debounce);
    server.idle(&mut output).unwrap();
    assert_eq!(loads.get(), 1);
    assert!(server.pending_watched.is_empty());
    assert!(!server.project.pointer_requirements_ready());
    let symbols = server
        .request("workspace/symbol", &json!({"query":""}))
        .unwrap();
    let mut names = symbols
        .as_array()
        .unwrap()
        .iter()
        .map(|symbol| symbol["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, ["created", "unsaved"]);
    server.idle(&mut output).unwrap();
    assert_eq!(loads.get(), 1);
}

fn messages(output: &[u8]) -> Vec<Value> {
    let mut input = output;
    let mut messages = Vec::new();
    while let Some(message) = read_message(&mut input).unwrap() {
        messages.push(message);
    }
    messages
}

fn exchange(input: Vec<Value>) -> Vec<Value> {
    exchange_with_loader(input, None, None)
}

fn exchange_with_loader(
    input: Vec<Value>,
    project: Option<Project>,
    loader: Option<ProjectLoader>,
) -> Vec<Value> {
    let mut server = Server::new(project, loader);
    let mut output = Vec::new();
    for mut message in input {
        if message["method"] == "test/idle" {
            if server.watched_at.is_some() {
                server.watched_at = Some(Instant::now() - server.performance.watch_debounce);
            }
            server.idle(&mut output).unwrap();
            continue;
        }
        message["jsonrpc"] = json!("2.0");
        if !server
            .process(message, CancellationToken::default(), &mut output)
            .unwrap()
        {
            break;
        }
        server.settle(&mut output).unwrap();
    }
    messages(&output)
}

#[test]
fn pointer_code_lenses_refresh_transitive_requirements_for_every_script_kind() {
    let caller = "file:///game/scripts/runescript/callers.rs2";
    let leaf = "file:///game/scripts/globalscript/leaf.gs2";
    let config = "file:///game/colors.flo";
    let text = "[proc,entry]\n~leaf;\n[label,resume]\n~leaf;\n[debugproc,inspect]\n~leaf;\n[login,_]\n~leaf;\n[proc,secondary]\nmes(\"self\");\n.mes(\"other\");\n[command,ignored]()\n";
    let result = exchange(vec![
        json!({"id":1,"method":"initialize","params":{
            "capabilities":{
                "textDocument":{"diagnostic":{}},
                "workspace":{"diagnostics":{"refreshSupport":true},"codeLens":{"refreshSupport":true}}
            },
            "initializationOptions":{
                "commands":[{"name":"mes","opcode":1000,"params":["string"],"returns":[]}],
                "sources":[
                    {"uri":caller,"languageId":"rs2","text":text},
                    {"uri":leaf,"languageId":"gs2","text":"[proc,leaf]\nmes(\"hello\");\n"},
                    {"uri":config,"config":"flo","text":"[grass]\ncolour=0xff0000\n"}
                ]
            }
        }}),
        json!({"id":2,"method":"textDocument/codeLens","params":{"textDocument":{"uri":caller}}}),
        json!({"id":3,"method":"textDocument/diagnostic","params":{"textDocument":{"uri":caller}}}),
        json!({"method":"test/idle"}),
        json!({"id":4,"method":"textDocument/codeLens","params":{"textDocument":{"uri":caller}}}),
        json!({"id":5,"method":"textDocument/codeLens","params":{"textDocument":{"uri":config}}}),
        json!({"id":6,"method":"textDocument/codeLens","params":{"textDocument":{"uri":leaf}}}),
        json!({"id":7,"method":"textDocument/codeLens","params":{"textDocument":{"uri":caller}}}),
        json!({"method":"textDocument/didChange","params":{"textDocument":{"uri":leaf,"version":2},"contentChanges":[{"text":"[proc,leaf]\nreturn;\n"}]}}),
        json!({"id":8,"method":"textDocument/codeLens","params":{"textDocument":{"uri":caller}}}),
        json!({"id":9,"method":"textDocument/diagnostic","params":{"textDocument":{"uri":caller}}}),
        json!({"method":"test/idle"}),
        json!({"id":10,"method":"textDocument/codeLens","params":{"textDocument":{"uri":caller}}}),
        json!({"method":"textDocument/didChange","params":{"textDocument":{"uri":leaf,"version":3},"contentChanges":[{"text":"[proc,leaf]\nmes(123);\n"}]}}),
        json!({"id":11,"method":"textDocument/codeLens","params":{"textDocument":{"uri":caller}}}),
        json!({"id":12,"method":"textDocument/diagnostic","params":{"textDocument":{"uri":leaf}}}),
        json!({"method":"test/idle"}),
        json!({"id":13,"method":"textDocument/codeLens","params":{"textDocument":{"uri":caller}}}),
        json!({"id":14,"method":"textDocument/codeLens","params":{"textDocument":{"uri":leaf}}}),
        json!({"id":15,"method":"shutdown"}),
        json!({"method":"exit"}),
    ]);
    let response = |id| &result.iter().find(|message| message["id"] == id).unwrap()["result"];
    assert_eq!(
        response(1)["capabilities"]["codeLensProvider"],
        json!({"resolveProvider":false})
    );
    assert_eq!(response(5), &json!([]));
    for id in [2, 8, 11, 14] {
        assert_eq!(analysis_lenses(response(id)), json!([]), "response {id}");
    }
    let lens = |line, title| json!({"range":{"start":{"line":line,"character":0},"end":{"line":line,"character":0}},"title":title});
    let secondary = lens(8, "@requires active_player, .active_player");
    assert_eq!(
        &lens_labels(response(4)),
        &json!([
            lens(0, "@requires active_player"),
            lens(2, "@requires active_player"),
            lens(4, "@requires active_player"),
            lens(6, "@requires active_player"),
            secondary
        ])
    );
    assert_eq!(
        &lens_labels(response(6)),
        &json!([lens(0, "@requires active_player")])
    );
    assert_eq!(response(7), response(4));
    assert_eq!(
        &lens_labels(response(10)),
        &json!([
            lens(0, "@requires none"),
            lens(2, "@requires none"),
            lens(4, "@requires none"),
            lens(6, "@requires none"),
            secondary
        ])
    );
    assert!(
        response(12)["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["severity"] == 1)
    );
    assert_eq!(&lens_labels(response(13)), &json!([secondary]));
    let refreshes = result
        .iter()
        .enumerate()
        .filter(|(_, message)| message["method"] == "workspace/codeLens/refresh")
        .map(|(index, message)| {
            assert!(message["id"].is_string());
            index
        })
        .collect::<Vec<_>>();
    assert_eq!(refreshes.len(), 3);
    for (index, (checked, ready)) in refreshes.into_iter().zip([(3, 4), (9, 10), (12, 13)]) {
        let position = |id| {
            result
                .iter()
                .position(|message| message["id"] == id)
                .unwrap()
        };
        assert!(position(checked) < index && index < position(ready));
    }
}

#[test]
fn pointer_code_lenses_without_refresh_wait_for_idle_and_cache_shared_components() {
    let shared = "file:///game/scripts/globalscript/paint.gs2";
    let caller = "file:///game/scripts/clientscript/entry.cs2";
    let other_scope = "file:///game/scripts/runescript/secondary.rs2";
    let result = exchange(vec![
        json!({"id":1,"method":"initialize","params":{"initializationOptions":{
            "target":"client",
            "opcodes":"0=PushConstantInt\n3=PushConstantString\n21=Return\n40=GosubWithParams\n150=cc_create\n1112=cc_settext\n",
            "commands":[
                {"name":"cc_create","opcode":150,"params":["component","int","int"],"returns":[]},
                {"name":"cc_settext","opcode":1112,"params":["string"],"returns":[]}
            ],
            "sources":[
                {"uri":shared,"languageId":"gs2","text":"[proc,paint]\ncc_settext(\"x\");\n"},
                {"uri":caller,"languageId":"cs2","text":"[clientscript,entry]\n~paint;\n[clientscript,local]\ncc_create(1, 4, 0);\n~paint;\n[proc,secondary]\n.cc_settext(\"x\");\n"},
                {"uri":other_scope,"languageId":"rs2","text":"[proc,secondary]\nreturn;\n"}
            ]
        }}}),
        json!({"id":2,"method":"textDocument/codeLens","params":{"textDocument":{"uri":shared}}}),
        json!({"id":8,"method":"textDocument/diagnostic","params":{"textDocument":{"uri":shared}}}),
        json!({"method":"test/idle"}),
        json!({"id":9,"method":"textDocument/codeLens","params":{"textDocument":{"uri":shared}}}),
        json!({"id":3,"method":"textDocument/codeLens","params":{"textDocument":{"uri":caller}}}),
        json!({"id":4,"method":"textDocument/codeLens","params":{"textDocument":{"uri":caller}}}),
        json!({"id":5,"method":"textDocument/codeLens","params":{"textDocument":{"uri":other_scope}}}),
        json!({"method":"textDocument/didChange","params":{"textDocument":{"uri":shared,"version":2},"contentChanges":[{"text":"[proc,paint]\nreturn;\n"}]}}),
        json!({"id":6,"method":"textDocument/codeLens","params":{"textDocument":{"uri":caller}}}),
        json!({"method":"test/idle"}),
        json!({"id":10,"method":"textDocument/codeLens","params":{"textDocument":{"uri":caller}}}),
        json!({"id":7,"method":"shutdown"}),
        json!({"method":"exit"}),
    ]);
    let response = |id| &result.iter().find(|message| message["id"] == id).unwrap()["result"];
    let lens = |line, title| json!({"range":{"start":{"line":line,"character":0},"end":{"line":line,"character":0}},"title":title});
    assert_eq!(
        &lens_labels(response(9)),
        &json!([lens(0, "@requires active_component")]),
        "diagnostics: {}",
        response(8)
    );
    assert_eq!(
        &lens_labels(response(3)),
        &json!([
            lens(0, "@requires active_component"),
            lens(2, "@requires none"),
            lens(5, "@requires .active_component")
        ])
    );
    assert_eq!(response(4), response(3));
    assert_eq!(response(5), &json!([]));
    assert_eq!(analysis_lenses(response(2)), json!([]));
    assert_eq!(analysis_lenses(response(6)), json!([]));
    assert_eq!(
        &lens_labels(response(10)),
        &json!([
            lens(0, "@requires none"),
            lens(2, "@requires none"),
            lens(5, "@requires .active_component")
        ])
    );
    assert!(
        !result
            .iter()
            .any(|message| message["method"] == "workspace/codeLens/refresh")
    );
}

#[test]
fn client_pointer_diagnostics_cross_shared_procedures_and_clear_after_edit() {
    let uri = "file:///game/scripts/clientscript/entry.cs2";
    let result = exchange(vec![
        json!({"id":1,"method":"initialize","params":{"initializationOptions":{
            "target":"client",
            "opcodes":"0=PushConstantInt\n3=PushConstantString\n21=Return\n40=GosubWithParams\n150=cc_create\n1112=cc_settext\n",
            "commands":[
                {"name":"cc_create","opcode":150,"params":["component","int","int"],"returns":[]},
                {"name":"cc_settext","opcode":1112,"params":["string"],"returns":[]}
            ],
            "sources":[
                {"uri":"file:///game/scripts/globalscript/paint.gs2","languageId":"gs2","text":"[proc,paint]\ncc_settext(\"x\");"},
                {"uri":uri,"languageId":"cs2","text":"[clientscript,entry]\n~paint;"}
            ]
        }}}),
        json!({"id":20,"method":"textDocument/diagnostic","params":{"textDocument":{"uri":uri}}}),
        json!({"method":"test/idle"}),
        json!({"id":2,"method":"textDocument/diagnostic","params":{"textDocument":{"uri":uri}}}),
        json!({"method":"textDocument/didChange","params":{"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":"[clientscript,entry]\ncc_create(1, 4, 0);\n~paint;"}]}}),
        json!({"method":"test/idle"}),
        json!({"id":3,"method":"textDocument/diagnostic","params":{"textDocument":{"uri":uri}}}),
        json!({"id":4,"method":"shutdown"}),
        json!({"method":"exit"}),
    ]);
    let response = |id| result.iter().find(|message| message["id"] == id).unwrap();
    assert!(
        response(2)["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["severity"] == 2
                && diagnostic["message"]
                    .as_str()
                    .is_some_and(|text| text.contains("active_component")))
    );
    assert_eq!(response(3)["result"]["items"], json!([]));
}

#[test]
fn watched_file_reload_replaces_disk_sources_and_keeps_unsaved_buffers() {
    let loaded = project_from_json(
        &json!({"sources":[{"uri":"mem:one","languageId":"gs2","text":"[proc,old]\nreturn;"}]}),
    )
    .unwrap();
    let loader = Box::new(|| {
        project_from_json(&json!({"sources":[
            {"uri":"mem:one","languageId":"gs2","text":"[proc,saved]\nreturn;"},
            {"uri":"mem:created","languageId":"gs2","text":"[proc,created]\nreturn;"}
        ]}))
    });
    let result = exchange_with_loader(
        vec![
            json!({"id":1,"method":"initialize","params":{"capabilities":{"workspace":{"didChangeWatchedFiles":{"dynamicRegistration":true}}}}}),
            json!({"method":"initialized"}),
            json!({"id":"runec/watch/1","result":null}),
            json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":"mem:one","languageId":"gs2","version":1,"text":"[proc,unsaved]\nreturn;"}}}),
            json!({"method":"workspace/didChangeWatchedFiles","params":{"changes":[{"uri":"mem:created","type":1}]}}),
            json!({"method":"test/idle"}),
            json!({"id":2,"method":"workspace/symbol","params":{"query":""}}),
            json!({"method":"textDocument/didClose","params":{"textDocument":{"uri":"mem:one"}}}),
            json!({"id":3,"method":"workspace/symbol","params":{"query":""}}),
            json!({"id":4,"method":"shutdown"}),
            json!({"method":"exit"}),
        ],
        Some(loaded),
        Some(loader),
    );
    assert!(
        result
            .iter()
            .any(|message| message["method"] == "client/registerCapability")
    );
    for (id, expected) in [
        (2, vec!["created", "unsaved"]),
        (3, vec!["created", "saved"]),
    ] {
        let response = result.iter().find(|message| message["id"] == id).unwrap();
        let mut names = response["result"]
            .as_array()
            .unwrap()
            .iter()
            .map(|symbol| symbol["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, expected);
    }
    assert!(!result.iter().any(|message| message.get("error").is_some()));
}
