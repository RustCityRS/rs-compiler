use super::*;

fn workspace() -> Workspace {
    Workspace::new(
        [(Scope::Rune, "rune", 9000), (Scope::Client, "client", 77)]
            .into_iter()
            .map(|(scope, target, opcode)| {
                let exclusive = if scope == Scope::Rune { "server_only" } else { "client_only" };
                let environment = json!({
                    "target":target,
                    "opcodes":format!("0=PushConstantInt\n21=Return\n40=GosubWithParams\n{opcode}=pulse\n78={exclusive}\n"),
                    "commands":[
                        {"name":"pulse","opcode":opcode,"params":["int"],"returns":[]},
                        {"name":exclusive,"opcode":78,"params":[],"returns":[]}
                    ],
                    "scriptIds":[{"trigger":"clientscript","name":"refresh","id":10}]
                });
                let project = project_from_json(&environment).unwrap();
                let loader: ProjectLoader = Box::new(move || project_from_json(&environment));
                (scope, Server::new(Some(project), Some(loader)))
            })
            .collect(),
    )
}

fn send(workspace: &mut Workspace, message: Value) -> Vec<Value> {
    let mut output = Vec::new();
    assert!(
        workspace
            .process(message, CancellationToken::default(), &mut output)
            .unwrap()
    );
    messages(output).unwrap()
}

fn response(sent: &[Value], id: i32) -> &Value {
    let found: Vec<_> = sent.iter().filter(|message| message["id"] == id).collect();
    assert_eq!(found.len(), 1, "{sent:?}");
    assert!(found[0].get("error").is_none(), "{}", found[0]);
    &found[0]["result"]
}

fn initialize(workspace: &mut Workspace) -> Vec<Value> {
    let sent = send(
        workspace,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "capabilities":{"workspace":{"didChangeWatchedFiles":{"dynamicRegistration":true}}},
            "initializationOptions":{"performance":{"maxWorkers":1}}
        }}),
    );
    assert!(response(&sent, 1)["capabilities"]["codeLensProvider"].is_object());
    send(
        workspace,
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
    )
}

fn open(workspace: &mut Workspace, uri: &str, language: &str, text: &str) {
    send(
        workspace,
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri":uri,"languageId":language,"version":1,"text":text}
        }}),
    );
}

fn settle(workspace: &mut Workspace) -> Vec<Value> {
    let mut output = Vec::new();
    workspace.settle(&mut output).unwrap();
    messages(output).unwrap()
}

#[test]
fn one_connection_initializes_both_targets_and_registers_one_watcher() {
    let mut workspace = workspace();
    let sent = initialize(&mut workspace);
    assert!(
        workspace
            .contexts
            .iter()
            .all(|context| context.server.initialized)
    );
    let watchers: Vec<_> = sent
        .iter()
        .filter(|message| message["method"] == "client/registerCapability")
        .collect();
    assert_eq!(watchers.len(), 1, "{sent:?}");
    assert_eq!(
        watchers[0]["params"]["registrations"][0]["id"],
        "runec/workspace/files"
    );
}

#[test]
fn extensions_route_requests_even_in_misleading_folders_and_language_ids() {
    let mut workspace = workspace();
    initialize(&mut workspace);
    for (uri, language, command, expected) in [
        (
            "file:///game/scripts/bank/clientscript/server.rs2",
            "cs2",
            "server_only",
            Scope::Rune,
        ),
        (
            "file:///game/scripts/bank/runescript/client.cs2",
            "rs2",
            "client_only",
            Scope::Client,
        ),
    ] {
        open(
            &mut workspace,
            uri,
            language,
            &format!("[proc,entry]\n{command};\n"),
        );
        let sent = send(
            &mut workspace,
            json!({"jsonrpc":"2.0","id":2,"method":"textDocument/diagnostic","params":{"textDocument":{"uri":uri}}}),
        );
        assert_eq!(response(&sent, 2)["items"], json!([]));
        assert_eq!(workspace.document_scope(uri), expected);
        for context in &workspace.contexts {
            if expected == Scope::Rune && context.scope == Scope::Client {
                assert!(context.server.find(uri).is_none());
                continue;
            }
            let id = context.server.find(uri).unwrap();
            assert_eq!(context.server.project.document(id).unwrap().scope, expected);
        }
    }
}

#[test]
fn shared_diagnostics_merge_both_targets_and_clear_after_one_edit() {
    let mut workspace = workspace();
    initialize(&mut workspace);
    let uri = "file:///game/scripts/bank/shared.gs2";
    open(
        &mut workspace,
        uri,
        "rs2",
        "[proc,shared]\nserver_only;\nclient_only;\n",
    );
    let sent = settle(&mut workspace);
    let published: Vec<_> = sent
        .iter()
        .filter(|message| {
            message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["uri"] == uri
        })
        .collect();
    assert_eq!(published.len(), 1, "{sent:?}");
    let diagnostics = published[0]["params"]["diagnostics"].as_array().unwrap();
    for target in ["runec/server", "runec/client"] {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic["source"] == target),
            "{diagnostics:?}"
        );
    }
    let sent = send(
        &mut workspace,
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/diagnostic","params":{"textDocument":{"uri":uri}}}),
    );
    assert_eq!(
        response(&sent, 2)["items"].as_array().unwrap().len(),
        diagnostics.len()
    );
    send(
        &mut workspace,
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":"[proc,shared]\nreturn;\n"}]
        }}),
    );
    let sent = settle(&mut workspace);
    let published: Vec<_> = sent
        .iter()
        .filter(|message| {
            message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["uri"] == uri
        })
        .collect();
    assert_eq!(published.len(), 1, "{sent:?}");
    assert_eq!(published[0]["params"]["diagnostics"], json!([]));
    for context in &workspace.contexts {
        assert_eq!(context.diagnostics[&uri_key(uri)]["diagnostics"], json!([]));
    }
}

#[test]
fn shared_bytecode_lenses_execute_with_their_own_command_tables() {
    let mut workspace = workspace();
    initialize(&mut workspace);
    let uri = "file:///game/scripts/bank/inspect.gs2";
    open(&mut workspace, uri, "gs2", "[proc,shared]\npulse(7);\n");
    let sent = send(
        &mut workspace,
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/codeLens","params":{"textDocument":{"uri":uri}}}),
    );
    let lenses: Vec<_> = response(&sent, 2)
        .as_array()
        .unwrap()
        .iter()
        .filter(|lens| lens["command"]["command"] == "runec.inspectBytecode")
        .collect();
    assert_eq!(lenses.len(), 2);
    for (target, label, opcode) in [("rune", "server", 9000), ("client", "client", 77)] {
        let lens = lenses
            .iter()
            .find(|lens| lens["command"]["arguments"][0]["target"] == target)
            .unwrap();
        assert_eq!(
            lens["command"]["title"],
            format!("Inspect bytecode ({label})")
        );
        let sent = send(
            &mut workspace,
            json!({"jsonrpc":"2.0","id":3,"method":"workspace/executeCommand","params":lens["command"]}),
        );
        let report = response(&sent, 3);
        let pulse = report["instructions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|instruction| instruction["instruction"] == "pulse")
            .unwrap();
        assert_eq!(pulse["operand"], format!("opcode {opcode}, operand 0"));
        assert_eq!(
            pulse["stackEffect"],
            json!({"pops":[1,0,0],"pushes":[0,0,0]})
        );
    }
}

#[test]
fn unsaved_client_signature_edits_update_the_server_callback_registry() {
    let mut workspace = workspace();
    initialize(&mut workspace);
    let uri = "file:///game/scripts/bank/client/refresh.cs2";
    open(
        &mut workspace,
        uri,
        "cs2",
        "[clientscript,refresh](int $value)\nreturn;\n",
    );
    settle(&mut workspace);
    let server = &workspace
        .contexts
        .iter()
        .find(|context| context.scope == Scope::Rune)
        .unwrap()
        .server;
    assert_eq!(
        server.project.registry().client_script_params["refresh"],
        vec![Type::Int]
    );
    send(
        &mut workspace,
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":"[clientscript,refresh](string $value)\nreturn;\n"}]
        }}),
    );
    settle(&mut workspace);
    let server = &workspace
        .contexts
        .iter()
        .find(|context| context.scope == Scope::Rune)
        .unwrap()
        .server;
    assert_eq!(
        server.project.registry().client_script_params["refresh"],
        vec![Type::String]
    );
}

#[test]
fn shared_references_and_rename_include_both_targets_from_a_server_call() {
    let mut workspace = workspace();
    initialize(&mut workspace);
    let shared = "file:///game/scripts/bank/common.gs2";
    let server = "file:///game/scripts/bank/server.rs2";
    let client = "file:///game/scripts/bank/client.cs2";
    for (uri, language, text) in [
        (shared, "gs2", "[proc,shared]\nreturn;\n"),
        (server, "rs2", "[proc,server_entry]\n~shared;\n"),
        (client, "cs2", "[proc,client_entry]\n~shared;\n"),
    ] {
        open(&mut workspace, uri, language, text);
    }
    settle(&mut workspace);
    let sent = send(
        &mut workspace,
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/references","params":{
            "textDocument":{"uri":server},"position":{"line":1,"character":2},"context":{"includeDeclaration":true}
        }}),
    );
    let references = response(&sent, 2).as_array().unwrap();
    for uri in [shared, server, client] {
        assert!(
            references.iter().any(|location| location["uri"] == uri),
            "{references:?}"
        );
    }
    let sent = send(
        &mut workspace,
        json!({"jsonrpc":"2.0","id":3,"method":"textDocument/rename","params":{
            "textDocument":{"uri":server},"position":{"line":1,"character":2},"newName":"common"
        }}),
    );
    let changes = response(&sent, 3)["changes"].as_object().unwrap();
    for uri in [shared, server, client] {
        let edits = changes
            .get(uri)
            .unwrap_or_else(|| panic!("Missing {uri}: {changes:?}"));
        assert_eq!(edits.as_array().unwrap().len(), 1);
        assert_eq!(edits[0]["newText"], "common");
    }
}

#[test]
fn a_previous_shared_target_cannot_hide_diagnostics_after_scope_changes() {
    let mut workspace = workspace();
    initialize(&mut workspace);
    let uri = "file:///game/scripts/bank/settings.constant";
    open(
        &mut workspace,
        uri,
        "rs2config",
        "// @scope(shared)\n^limit=1\n",
    );
    let diagnostic = json!({"range":{"start":{"line":1,"character":0},"end":{"line":1,"character":1}},"message":"Current server warning","source":"runec","severity":2});
    // A cached contribution from the old scope/version must not participate in
    // the newly private document's version selection, or hide its new errors.
    workspace.contexts[1].diagnostics.insert(
        uri_key(uri),
        json!({
            "uri":uri,"version":99,"diagnostics":[{"message":"Previous client warning"}]
        }),
    );
    send(
        &mut workspace,
        json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
            "textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":"// @scope(server)\n^limit=1\n"}]
        }}),
    );
    let mut output = Vec::new();
    workspace
        .emit(
            vec![(
                0,
                json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{
                    "uri":uri,"version":2,"diagnostics":[diagnostic]
                }}),
            )],
            &mut output,
        )
        .unwrap();
    let sent = messages(output).unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["params"]["version"], 2);
    assert_eq!(sent[0]["params"]["diagnostics"], json!([diagnostic]));
}

#[test]
fn interleaved_preview_replies_return_to_their_own_target() {
    let mut workspace = workspace();
    let sent = send(
        &mut workspace,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "capabilities":{"window":{"showDocument":{"support":true}}},
            "initializationOptions":{"performance":{"maxWorkers":1}}
        }}),
    );
    response(&sent, 1);
    send(
        &mut workspace,
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
    );
    let uri = "file:///game/scripts/bank/preview.gs2";
    open(&mut workspace, uri, "gs2", "[proc,shared]\npulse(7);\n");
    let sent = send(
        &mut workspace,
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/codeLens","params":{"textDocument":{"uri":uri}}}),
    );
    let lenses = response(&sent, 2).as_array().unwrap();
    let mut previews = Vec::new();
    for (index, target) in ["rune", "client"].into_iter().enumerate() {
        let lens = lenses
            .iter()
            .find(|lens| {
                lens["command"]["command"] == "runec.inspectBytecode"
                    && lens["command"]["arguments"][0]["target"] == target
            })
            .unwrap();
        let id = 3 + index as i32;
        let sent = send(
            &mut workspace,
            json!({"jsonrpc":"2.0","id":id,"method":"workspace/executeCommand","params":lens["command"]}),
        );
        let report = response(&sent, id);
        let preview = sent
            .iter()
            .find(|message| message["method"] == "window/showDocument")
            .unwrap();
        let prefix = format!("runec/{target}/");
        assert!(preview["id"].as_str().unwrap().starts_with(&prefix));
        let path = uri_path(preview["params"]["uri"].as_str().unwrap()).unwrap();
        assert!(path.starts_with(std::env::temp_dir()));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            report["markdown"].as_str().unwrap()
        );
        previews.push((preview["id"].clone(), report["markdown"].clone(), path));
    }
    assert_ne!(previews[0].0, previews[1].0);
    assert_ne!(previews[0].1, previews[1].1);
    for index in [1, 0] {
        let (id, markdown, _) = &previews[index];
        let sent = send(
            &mut workspace,
            json!({"jsonrpc":"2.0","id":id,"result":{"success":false}}),
        );
        let shown = sent
            .iter()
            .filter(|message| message["method"] == "window/showMessage")
            .collect::<Vec<_>>();
        assert_eq!(shown.len(), 1, "{sent:?}");
        assert_eq!(shown[0]["params"]["message"], *markdown);
        assert!(
            workspace.contexts[index]
                .server
                .pending_bytecode_previews
                .is_empty()
        );
        if index == 1 {
            assert_eq!(
                workspace.contexts[0].server.pending_bytecode_previews.len(),
                1
            );
        }
    }
    for (_, _, path) in previews {
        fs::remove_file(path).unwrap();
    }
}
