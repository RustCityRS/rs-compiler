use serde_json::{Value, json};
use std::cell::Cell;
use std::fs;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::process;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn color_previews_and_presentations_follow_utf16_ranges_and_document_changes() {
    let script = "[proc,colors]\nmes(\"😀<col=ff0000>red</col>\");\ndef_int $count = 123;\n";
    let config = "[grass]\ncolour=0xff0000\ntexture=123\n";
    let script_range = json!({"start":{"line":1,"character":12},"end":{"line":1,"character":18}});
    let config_range = json!({"start":{"line":1,"character":7},"end":{"line":1,"character":15}});
    let green = json!({"red":0.0,"green":1.0,"blue":0.0,"alpha":1.0});
    let result = exchange(vec![
        json!({"id":1,"method":"initialize","params":{}}),
        json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":"mem:colors","languageId":"rs2","version":1,"text":script}}}),
        json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///game/colors.flo","languageId":"rs2config","version":1,"text":config}}}),
        json!({"id":2,"method":"textDocument/documentColor","params":{"textDocument":{"uri":"mem:colors"}}}),
        json!({"id":3,"method":"textDocument/documentColor","params":{"textDocument":{"uri":"file:///game/colors.flo"}}}),
        json!({"id":4,"method":"textDocument/colorPresentation","params":{"textDocument":{"uri":"mem:colors"},"range":script_range,"color":green}}),
        json!({"id":5,"method":"textDocument/colorPresentation","params":{"textDocument":{"uri":"file:///game/colors.flo"},"range":config_range,"color":green}}),
        json!({"method":"textDocument/didChange","params":{"textDocument":{"uri":"mem:colors","version":2},"contentChanges":[{"range":script_range,"text":"0000ff"}]}}),
        json!({"id":6,"method":"textDocument/documentColor","params":{"textDocument":{"uri":"mem:colors"}}}),
        json!({"id":7,"method":"textDocument/colorPresentation","params":{"textDocument":{"uri":"mem:colors"},"range":script_range,"color":{"red":1.1,"green":0.0,"blue":0.0,"alpha":1.0}}}),
        json!({"id":8,"method":"textDocument/colorPresentation","params":{"textDocument":{"uri":"mem:colors"},"range":{"start":{"line":1,"character":18},"end":{"line":1,"character":12}},"color":green}}),
        json!({"id":9,"method":"textDocument/colorPresentation","params":{"textDocument":{"uri":"file:///game/colors.flo"},"range":{"start":{"line":2,"character":8},"end":{"line":2,"character":11}},"color":green}}),
        json!({"id":10,"method":"shutdown"}),
        json!({"method":"exit"}),
    ]);
    let response = |id| result.iter().find(|message| message["id"] == id).unwrap();
    assert_eq!(response(1)["result"]["capabilities"]["colorProvider"], true);
    let red = json!({"red":1.0,"green":0.0,"blue":0.0,"alpha":1.0});
    assert_eq!(
        response(2)["result"],
        json!([{"range":script_range,"color":red}])
    );
    assert_eq!(
        response(3)["result"],
        json!([{"range":config_range,"color":red}])
    );
    assert_eq!(
        response(4)["result"],
        json!([{"label":"00ff00","textEdit":{"range":script_range,"newText":"00ff00"}}])
    );
    assert_eq!(
        response(5)["result"],
        json!([{"label":"0x00ff00","textEdit":{"range":config_range,"newText":"0x00ff00"}}])
    );
    assert_eq!(
        response(6)["result"],
        json!([{"range":script_range,"color":{"red":0.0,"green":0.0,"blue":1.0,"alpha":1.0}}])
    );
    assert_eq!(response(7)["error"]["code"], -32602);
    assert_eq!(response(8)["error"]["code"], -32602);
    assert_eq!(response(9)["result"], json!([]));
}

#[test]
fn semantic_token_legend_modifiers_ranges_and_utf16_match_the_reference_palette() {
    let text = "[proc,test]\nmes(\"😀 <^limit>\");\n%bank = 1;\n";
    let result = exchange(vec![
        json!({"id":1,"method":"initialize","params":{}}),
        json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":"mem:highlight","languageId":"rs2","version":1,"text":text}}}),
        json!({"id":2,"method":"textDocument/semanticTokens/full","params":{"textDocument":{"uri":"mem:highlight"}}}),
        json!({"id":3,"method":"textDocument/semanticTokens/range","params":{"textDocument":{"uri":"mem:highlight"},"range":{"start":{"line":1,"character":0},"end":{"line":2,"character":0}}}}),
        json!({"id":4,"method":"shutdown"}),
        json!({"method":"exit"}),
    ]);
    let response = |id| &result.iter().find(|message| message["id"] == id).unwrap()["result"];
    let provider = &response(1)["capabilities"]["semanticTokensProvider"];
    assert_eq!(provider["range"], true);
    let types = provider["legend"]["tokenTypes"].as_array().unwrap();
    let modifiers = provider["legend"]["tokenModifiers"].as_array().unwrap();
    let kind = |name: &str| types.iter().position(|value| value == name).unwrap() as u64;
    let flag = |name: &str| 1 << modifiers.iter().position(|value| value == name).unwrap();
    let decode = |id| {
        let mut line = 0;
        let mut character = 0;
        response(id)["data"]
            .as_array()
            .unwrap()
            .as_chunks::<5>()
            .0
            .iter()
            .map(|row| {
                let row = row.iter().map(|v| v.as_u64().unwrap()).collect::<Vec<_>>();
                if row[0] != 0 {
                    character = 0;
                }
                line += row[0];
                character += row[1];
                assert!(row[2] > 0);
                assert!((row[3] as usize) < types.len());
                [line, character, row[2], row[3], row[4]]
            })
            .collect::<Vec<_>>()
    };
    let full = decode(2);
    assert!(full.contains(&[1, 0, 3, kind("function"), flag("defaultLibrary")]));
    assert!(full.contains(&[1, 9, 6, kind("variable"), flag("static") | flag("readonly")]));
    assert!(full.contains(&[2, 0, 5, kind("variable"), flag("static")]));
    assert_eq!(
        decode(3),
        full.into_iter()
            .filter(|row| row[0] == 1)
            .collect::<Vec<_>>()
    );
}

#[test]
fn reference_config_extensions_open_with_config_highlighting() {
    let extensions = "pack order hashes constant anim bas cursor dbrow dbtable enum flo flu hitmark hunt idk inv light loc mel mesanim msi npc obj param quest seq spotanim struct varpbit varclansetting varp varn vars if synth midi jingle model stat category fontmetrics overlayinterface locshape config7 config48 varc area billboard controller effector emitter varclan varcon varconbit varnbit varobj itemcode mapped_by_the_editor";
    let mut messages = vec![json!({"id":1,"method":"initialize","params":{}})];
    for (i, extension) in extensions.split_whitespace().enumerate() {
        let uri = format!("file:///game/test.{extension}");
        let text = if matches!(extension, "pack" | "order" | "hashes") {
            "1=entry\n"
        } else {
            "[entry]\nname=value\n"
        };
        messages.push(json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"rs2config","version":1,"text":text}}}));
        messages.push(json!({"id":i + 2,"method":"textDocument/semanticTokens/full","params":{"textDocument":{"uri":uri}}}));
    }
    messages.extend([
        json!({"id":100,"method":"shutdown"}),
        json!({"method":"exit"}),
    ]);
    for response in exchange(messages) {
        if let Some(id) = response["id"].as_u64()
            && id > 1
            && id < 100
        {
            assert!(response.get("error").is_none(), "{response}");
            assert!(!response["result"]["data"].as_array().unwrap().is_empty());
        }
        if response["method"] == "textDocument/publishDiagnostics" {
            assert_eq!(response["params"]["diagnostics"], json!([]));
        }
    }
}

#[test]
fn directory_loading_waits_until_after_the_initialize_handshake() {
    for initialized in [false, true] {
        let count = Rc::new(Cell::new(0));
        let loads = count.clone();
        let mut messages = vec![json!({"id":1,"method":"initialize","params":{}})];
        if initialized {
            messages.push(json!({"method":"initialized","params":{}}));
        }
        messages.extend([
            json!({"id":2,"method":"workspace/symbol","params":{"query":"loaded"}}),
            json!({"id":3,"method":"shutdown"}),
            json!({"method":"exit"}),
        ]);
        let result = exchange_with_loader(
            messages,
            None,
            Some(Box::new(move || {
                loads.set(loads.get() + 1);
                runec::lsp::project_from_json(
                    &json!({"sources":[{"uri":"mem:loaded","languageId":"gs2","text":"[proc,loaded]\nreturn;"}]}),
                )
            })),
        );
        assert_eq!(count.get(), usize::from(initialized));
        let response = result.iter().find(|message| message["id"] == 2).unwrap();
        assert_eq!(
            response["result"].as_array().unwrap().len(),
            usize::from(initialized)
        );
    }
}

fn exchange(messages: Vec<Value>) -> Vec<Value> {
    exchange_with_project(messages, None)
}

#[test]
fn pull_diagnostics_refresh_edited_and_dependent_files_without_a_push_cache() {
    let caller = "untitled:caller";
    let value = "untitled:value";
    let broken = "[proc,value]()(string)\nreturn(1);\n";
    let fixed = "[proc,value]()(int)\nreturn(1);\n";
    let result = exchange(vec![
        json!({"id":1,"method":"initialize","params":{"capabilities":{
            "textDocument":{"diagnostic":{}},
            "workspace":{"diagnostics":{"refreshSupport":true}}
        }}}),
        json!({"method":"initialized"}),
        json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":value,"languageId":"gs2","version":1,"text":broken}}}),
        json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":caller,"languageId":"rs2","version":1,"text":"[proc,caller]()(int)\nreturn(~value);\n"}}}),
        json!({"id":2,"method":"textDocument/diagnostic","params":{"textDocument":{"uri":value}}}),
        json!({"id":3,"method":"textDocument/diagnostic","params":{"textDocument":{"uri":caller}}}),
        json!({"id":4,"method":"textDocument/documentSymbol","params":{"textDocument":{"uri":value}}}),
        json!({"method":"textDocument/didChange","params":{"textDocument":{"uri":value,"version":2},"contentChanges":[{"text":fixed}]}}),
        json!({"id":5,"method":"textDocument/diagnostic","params":{"textDocument":{"uri":value}}}),
        json!({"id":6,"method":"textDocument/diagnostic","params":{"textDocument":{"uri":caller}}}),
        json!({"id":7,"method":"shutdown"}),
        json!({"method":"exit"}),
    ]);
    let response = |id| &result.iter().find(|message| message["id"] == id).unwrap()["result"];
    assert!(response(1)["capabilities"]["diagnosticProvider"].is_object());
    for id in [2, 3] {
        assert!(!response(id)["items"].as_array().unwrap().is_empty());
    }
    for id in [5, 6] {
        assert_eq!(response(id)["items"], json!([]));
    }
    let before_edit = result
        .iter()
        .position(|message| message["id"] == 4)
        .unwrap();
    let after_edit = result
        .iter()
        .position(|message| message["id"] == 5)
        .unwrap();
    assert!(
        result[before_edit + 1..after_edit].iter().any(|message| {
            message["method"] == "workspace/diagnostic/refresh" && message["id"].is_string()
        }),
        "the client must be told to refresh diagnostics in dependent files"
    );
    assert!(
        !result
            .iter()
            .any(|message| message["method"] == "textDocument/publishDiagnostics"),
        "push and pull diagnostics must not leave independent caches in the editor"
    );
}

#[test]
fn clients_without_diagnostic_refresh_use_push_updates() {
    for capabilities in [json!({}), json!({"textDocument":{"diagnostic":{}}})] {
        let result = exchange(vec![
            json!({"id":1,"method":"initialize","params":{"capabilities":capabilities}}),
            json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":"untitled:value","languageId":"rs2","version":1,"text":"[proc,value]()(int)\nreturn(\"wrong\");\n"}}}),
            json!({"method":"textDocument/didChange","params":{"textDocument":{"uri":"untitled:value","version":2},"contentChanges":[{"text":"[proc,value]()(int)\nreturn(1);\n"}]}}),
            json!({"id":2,"method":"shutdown"}),
            json!({"method":"exit"}),
        ]);
        let capabilities =
            &result.iter().find(|message| message["id"] == 1).unwrap()["result"]["capabilities"];
        assert!(capabilities.get("diagnosticProvider").is_none());
        let diagnostics = result
            .iter()
            .filter(|message| message["method"] == "textDocument/publishDiagnostics")
            .collect::<Vec<_>>();
        assert!(
            !diagnostics[0]["params"]["diagnostics"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(diagnostics.last().unwrap()["params"]["version"], 2);
        assert_eq!(
            diagnostics.last().unwrap()["params"]["diagnostics"],
            json!([])
        );
        assert!(
            !result
                .iter()
                .any(|message| message["method"] == "workspace/diagnostic/refresh")
        );
    }
}

fn exchange_with_project(
    messages: Vec<Value>,
    project: Option<runec::project::Project>,
) -> Vec<Value> {
    exchange_with_loader(messages, project, None)
}

fn exchange_with_loader(
    messages: Vec<Value>,
    project: Option<runec::project::Project>,
    loader: Option<Box<dyn Fn() -> Result<runec::project::Project, String>>>,
) -> Vec<Value> {
    let mut input = Vec::new();
    for mut message in messages {
        message["jsonrpc"] = json!("2.0");
        let bytes = serde_json::to_vec(&message).unwrap();
        input.extend(format!("Content-Length: {}\r\n\r\n", bytes.len()).as_bytes());
        input.extend(bytes);
    }
    let mut output = Vec::new();
    runec::lsp::serve_io_with_loader(input.as_slice(), &mut output, project, loader).unwrap();
    let mut output = output.as_slice();
    let mut messages = Vec::new();
    while !output.is_empty() {
        let mut header = String::new();
        output.read_line(&mut header).unwrap();
        let count = header
            .trim()
            .split_once(':')
            .unwrap()
            .1
            .trim()
            .parse()
            .unwrap();
        header.clear();
        output.read_line(&mut header).unwrap();
        let mut bytes = vec![0; count];
        output.read_exact(&mut bytes).unwrap();
        messages.push(serde_json::from_slice(&bytes).unwrap());
    }
    messages
}

#[test]
fn standard_lsp_hints_config_navigation_rename_and_extract_apply_edits() {
    let text = "[proc,calculate](int $value, int $step)(int)\n$value = calc($value + 1);\nreturn($value);\n[proc,use]\n~calculate(^limit, 1);\n";
    let result = exchange(vec![
        json!({"id":1,"method":"initialize","params":{"capabilities":{"workspace":{"applyEdit":true}},"initializationOptions":{
            "constants":[{"name":"limit","type":"int","value":99,"scope":"global"}],
            "sources":[{"uri":"mem:script","languageId":"gs2","text":text},{"uri":"file:///game/limits.constant","config":"constant","text":"^limit=99\n"}]
        }}}),
        json!({"id":2,"method":"textDocument/inlayHint","params":{"textDocument":{"uri":"mem:script"},"range":{"start":{"line":0,"character":0},"end":{"line":5,"character":0}}}}),
        json!({"id":3,"method":"textDocument/definition","params":{"textDocument":{"uri":"mem:script"},"position":{"line":4,"character":13}}}),
        json!({"id":4,"method":"textDocument/rename","params":{"textDocument":{"uri":"mem:script"},"position":{"line":4,"character":13},"newName":"maximum"}}),
        json!({"id":5,"method":"workspace/executeCommand","params":{"command":"runec.extractProc","arguments":[{"textDocument":{"uri":"mem:script"},"range":{"start":{"line":1,"character":0},"end":{"line":1,"character":text.lines().nth(1).unwrap().encode_utf16().count()}},"name":"increment"}]}}),
        json!({"id":"runec/edit/1","result":{"applied":true}}),
        json!({"id":6,"method":"shutdown"}),
        json!({"method":"exit"}),
    ]);
    let response = |id| result.iter().find(|message| message["id"] == id).unwrap();
    assert!(response(1)["result"]["capabilities"]["inlayHintProvider"] == true);
    let hints = response(2)["result"].as_array().unwrap();
    assert!(hints.iter().any(|hint| hint["label"] == "step:"
        && hint["kind"] == 2
        && hint["position"] == json!({"line":4,"character":19})));
    assert!(hints.iter().any(|hint| hint["label"] == ": int"));
    assert!(!hints.iter().any(|hint| hint["label"] == "value:"));
    assert_eq!(response(3)["result"]["uri"], "file:///game/limits.constant");
    assert_eq!(
        response(4)["result"]["changes"].as_object().unwrap().len(),
        2
    );
    assert_eq!(response(5)["result"], Value::Null);
    assert!(
        result
            .iter()
            .any(|message| message["method"] == "workspace/applyEdit")
    );
    assert!(
        !result.iter().any(|message| message.get("error").is_some()),
        "{result:?}"
    );
}

#[test]
fn empty_editor_settings_preserve_loaded_game_variables() {
    let mut registry = runec::symbol::SymbolRegistry::new();
    registry.register_game_var(
        "bank_tab_1".into(),
        runec::types::Type::Int,
        4885,
        "varpbit".into(),
    );
    let mut visibility = runec::sources::Visibility::new();
    visibility.declare("varpbit", "bank_tab_1", runec::sources::Scope::Global);
    registry.set_visibility(visibility);
    let project = runec::project::Project::new(
        runec::project::Target::Rune,
        registry,
        runec::project::Options::default(),
    );
    let result = exchange_with_project(
        vec![
            json!({"id":1,"method":"initialize","params":{"initializationOptions":{}}}),
            json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":"untitled:bank","languageId":"gs2","version":1,"text":"[proc,bank_check]()(int) return(%bank_tab_1);"}}}),
            json!({"method":"workspace/didChangeConfiguration","params":{"settings":{}}}),
            json!({"method":"workspace/didChangeConfiguration","params":{"settings":null}}),
            json!({"method":"workspace/didChangeConfiguration","params":{"settings":{"editor":{}}}}),
            json!({"method":"textDocument/didChange","params":{"textDocument":{"uri":"untitled:bank","version":2},"contentChanges":[{"text":"[proc,bank_check]()(int) return(%bank_tab_1);\n"}]}}),
            json!({"id":2,"method":"shutdown"}),
            json!({"method":"exit"}),
        ],
        Some(project),
    );
    let diagnostics = result
        .iter()
        .filter(|message| message["method"] == "textDocument/publishDiagnostics")
        .collect::<Vec<_>>();
    assert!(!diagnostics.is_empty());
    for message in &diagnostics {
        assert_eq!(message["params"]["diagnostics"], json!([]));
    }
    assert_eq!(diagnostics.last().unwrap()["params"]["version"], 2);
}

#[test]
fn stdio_lifecycle_unsaved_diagnostics_navigation_and_utf16_edits() {
    let result = exchange(vec![
        json!({"id":1,"method":"initialize","params":{}}),
        json!({"method":"initialized"}),
        json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":"untitled:one","languageId":"gs2","version":1,"text":"[proc,value]()(int)\nreturn(7);\n"}}}),
        json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":"untitled:two","languageId":"rs2","version":1,"text":"[proc,caller]()(int)\nreturn(~value);\n"}}}),
        json!({"id":2,"method":"textDocument/definition","params":{"textDocument":{"uri":"untitled:two"},"position":{"line":1,"character":9}}}),
        json!({"id":3,"method":"textDocument/rename","params":{"textDocument":{"uri":"untitled:two"},"position":{"line":1,"character":9},"newName":"renamed"}}),
        json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":"untitled:unicode","languageId":"rs2","version":1,"text":"[proc,unicode]()(string)\nreturn(\"\u{1f600}\");\n"}}}),
        json!({"method":"textDocument/didChange","params":{"textDocument":{"uri":"untitled:unicode","version":2},"contentChanges":[{"range":{"start":{"line":1,"character":8},"end":{"line":1,"character":10}},"text":"ok"}]}}),
        json!({"id":4,"method":"textDocument/semanticTokens/full","params":{"textDocument":{"uri":"untitled:unicode"}}}),
        json!({"method":"textDocument/didChange","params":{"textDocument":{"uri":"untitled:two","version":2},"contentChanges":[{"range":{"start":{"line":1,"character":8},"end":{"line":1,"character":13}},"text":"missing"}]}}),
        json!({"id":5,"method":"unknown","params":{}}),
        json!({"id":6,"method":"shutdown"}),
        json!({"method":"exit"}),
    ]);
    let response = |id| result.iter().find(|message| message["id"] == id).unwrap();
    assert_eq!(
        response(1)["result"]["capabilities"]["positionEncoding"],
        "utf-16"
    );
    assert_eq!(response(2)["result"]["uri"], "untitled:one");
    assert_eq!(
        response(3)["result"]["changes"].as_object().unwrap().len(),
        2
    );
    assert!(!response(4)["result"]["data"].as_array().unwrap().is_empty());
    assert_eq!(response(5)["error"]["code"], -32601);
    let diagnostics = result
        .iter()
        .rev()
        .find(|message| message["params"]["uri"] == "untitled:two")
        .unwrap();
    assert_eq!(diagnostics["params"]["version"], 2);
    assert!(
        diagnostics["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["message"].as_str().unwrap().contains("missing"))
    );
    let unicode = result
        .iter()
        .rev()
        .find(|message| message["params"]["uri"] == "untitled:unicode")
        .unwrap();
    assert_eq!(unicode["params"]["diagnostics"], json!([]));
}

#[test]
fn portable_environment_checks_private_configs_without_paths() {
    let mut project=runec::lsp::project_from_json(&json!({
        "target":"rune",
        "entities":[{"name":"item","type":"obj","id":14,"scope":"rune"}],
        "sources":[{"uri":"db:shared","languageId":"gs2","text":"[proc,item]()(obj)\nreturn(item);"}]
    })).unwrap();
    assert!(project.analyze().diagnostics.has_errors());
    for bad in [
        json!({"target":"typo"}),
        json!({"entities":{}}),
        json!({"commands":[{"name":"bad","opcode":1,"params":"int"}]}),
    ] {
        assert!(runec::lsp::project_from_json(&bad).is_err());
    }
}

#[test]
fn environment_reload_preserves_open_buffers_and_close_restores_saved_sources() {
    let baseline = json!({"target":"rune","sources":[{"uri":"mem:one","languageId":"rs2","text":"[proc,saved] return;"}]});
    let result = exchange(vec![
        json!({"id":1,"method":"initialize","params":{"initializationOptions":baseline}}),
        json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":"mem:one","languageId":"rs2","version":1,"text":"[proc,unsaved] return;"}}}),
        json!({"method":"workspace/didChangeConfiguration","params":{"settings":baseline}}),
        json!({"id":2,"method":"workspace/symbol","params":{"query":""}}),
        json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":"mem:one","languageId":"unknown","version":2,"text":"invalid"}}}),
        json!({"id":3,"method":"workspace/symbol","params":{"query":""}}),
        json!({"method":"textDocument/didClose","params":{"textDocument":{"uri":"mem:one"}}}),
        json!({"id":4,"method":"workspace/symbol","params":{"query":""}}),
        json!({"id":5,"method":"shutdown"}),
        json!({"method":"exit"}),
    ]);
    for (id, name) in [(2, "unsaved"), (3, "unsaved"), (4, "saved")] {
        let symbols = result.iter().find(|message| message["id"] == id).unwrap()["result"]
            .as_array()
            .unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0]["name"], name);
    }
}

#[test]
fn file_uri_spellings_share_one_document_through_the_editor_lifecycle() {
    let disk = "file:///C:/game/globalscript/bank/proc/%5Bproc%2Cbank_v2_get_tab%5D.gs2";
    for editor in [
        "file:///C:/game/globalscript/bank/proc/%5Bproc,bank_v2_get_tab%5D.gs2",
        "file:///C:/game/globalscript/bank/proc/[proc,bank_v2_get_tab].gs2",
        "file:///C:/game/globalscript/bank/proc/%5bproc%2cbank_v2_get_tab%5d.gs2",
    ] {
        for target in ["rune", "client"] {
            let saved = "[proc,bank_v2_get_tab](int $slot)(int) return($slot);";
            let edited = "[proc,edited](int $slot)(int) return($slot);";
            let baseline = json!({"target":target,"opcodes":"21=return","sources":[{"uri":disk,"languageId":"gs2","text":saved}]});
            let result = exchange(vec![
                json!({"id":1,"method":"initialize","params":{"initializationOptions":baseline}}),
                json!({"method":"initialized"}),
                json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":editor,"languageId":"gs2","version":1,"text":saved}}}),
                json!({"id":2,"method":"workspace/symbol","params":{"query":""}}),
                json!({"method":"textDocument/didChange","params":{"textDocument":{"uri":editor,"version":2},"contentChanges":[{"text":edited}]}}),
                json!({"method":"workspace/didChangeConfiguration","params":{"settings":baseline}}),
                json!({"id":3,"method":"workspace/symbol","params":{"query":""}}),
                json!({"method":"textDocument/didSave","params":{"textDocument":{"uri":editor}}}),
                json!({"method":"textDocument/didClose","params":{"textDocument":{"uri":editor}}}),
                json!({"id":4,"method":"workspace/symbol","params":{"query":""}}),
                json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":disk,"languageId":"gs2","version":1,"text":edited}}}),
                json!({"id":5,"method":"workspace/symbol","params":{"query":""}}),
                json!({"id":6,"method":"shutdown"}),
                json!({"method":"exit"}),
            ]);
            for message in &result {
                assert!(message.get("error").is_none(), "{target}: {message}");
                if message["method"] == "textDocument/publishDiagnostics" {
                    assert_eq!(
                        message["params"]["diagnostics"],
                        json!([]),
                        "{target}: {editor}"
                    );
                }
            }
            for (id, name) in [
                (2, "bank_v2_get_tab"),
                (3, "edited"),
                (4, "edited"),
                (5, "edited"),
            ] {
                let symbols = result.iter().find(|message| message["id"] == id).unwrap()["result"]
                    .as_array()
                    .unwrap();
                assert_eq!(symbols.len(), 1, "{target}: {editor}");
                assert_eq!(symbols[0]["name"], name);
            }
        }
    }
}

#[test]
fn distinct_files_still_report_duplicate_scripts() {
    let text = "[proc,bank_v2_get_tab](int $slot)(int) return($slot);";
    let result = exchange(vec![
        json!({"id":1,"method":"initialize","params":{"initializationOptions":{"sources":[{"uri":"file:///game/bank.gs2","languageId":"gs2","text":text}]}}}),
        json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///game/other.gs2","languageId":"gs2","version":1,"text":text}}}),
        json!({"id":2,"method":"shutdown"}),
        json!({"method":"exit"}),
    ]);
    assert!(result.iter().any(|message| {
        message["params"]["diagnostics"]
            .as_array()
            .is_some_and(|diagnostics| {
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic["code"] == "RUNEC2001")
            })
    }));
}

#[test]
fn directory_servers_ignore_other_revisions_but_accept_new_project_files() {
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let root = fs::canonicalize(&self.0).unwrap();
            let base = fs::canonicalize(env!("CARGO_TARGET_TMPDIR")).unwrap();
            assert!(root != base && root.starts_with(&base));
            fs::remove_dir_all(root).unwrap();
        }
    }
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "lsp-roots-{}-{}",
        process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let fixture = Fixture(fs::canonicalize(root).unwrap());
    let script = "[proc,scroll_elena_hangover_cure_recipe]\nreturn;\n";
    let callback = "[proc,callback]\nreturn;\n";
    for (path, text) in [
        ("647/pack/command.pack", ""),
        ("647/scripts/runescript/original.rs2", script),
        (
            "647/scripts/globalscript/base.gs2",
            "[proc,shared]\nreturn;\n",
        ),
        ("647/scripts/clientscript/callback.cs2", callback),
        ("647/interfaces/base.if3", "[main]\ntype=layer\n"),
        ("647/defaults/base.defaults", ""),
        ("225/scripts/runescript/foreign.rs2", script),
        ("647/scripts-other/foreign.rs2", script),
    ] {
        let path = fixture.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fs::create_dir_all(fixture.0.join("647/textures")).unwrap();
    let uri = |relative: &str| {
        let path = fixture
            .0
            .join(relative)
            .to_string_lossy()
            .replace('\\', "/");
        let path = path.strip_prefix("//?/").unwrap_or(&path);
        format!(
            "file://{}{path}",
            if path.starts_with('/') { "" } else { "/" }
        )
    };
    let active = uri("647/scripts/runescript/original.rs2");
    let foreign = uri("225/scripts/runescript/foreign.rs2");
    let boundary = uri("647/scripts-other/foreign.rs2");
    let global = uri("647/scripts/globalscript/new.gs2");
    let texture = uri("647/textures/paint.texture");
    let material = uri("647/materials/new.material");
    let client = uri("647/scripts/clientscript/callback.cs2");
    let duplicate = uri("647/scripts/runescript/duplicate.rs2");
    let watched = uri("647/pack/command.pack");
    let source = fixture.0.join("647/scripts/runescript");
    let pack = fixture.0.join("647/pack");
    let material_path = fixture.0.join("647/materials/new.material");
    let loads = Rc::new(Cell::new(0));
    let count = loads.clone();
    let loader = Box::new(move || {
        count.set(count.get() + 1);
        let project = runec::filesystem::editor_project(&source, Some(&pack), false)
            .map_err(|error| error.to_string())?;
        if count.get() == 1 {
            fs::create_dir_all(material_path.parent().unwrap()).unwrap();
            fs::write(&material_path, "averagecolour=39\n").unwrap();
        }
        Ok(project)
    });
    let result = exchange_with_loader(
        vec![
            json!({"id":1,"method":"initialize","params":{}}),
            json!({"method":"initialized"}),
            json!({"method":"workspace/didChangeWatchedFiles","params":{"changes":[{"uri":material,"type":1}]}}),
            json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":material,"languageId":"rs2config","version":1,"text":"averagecolour=39\n"}}}),
            json!({"id":12,"method":"textDocument/documentColor","params":{"textDocument":{"uri":material}}}),
            json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":active,"languageId":"rs2","version":1,"text":script}}}),
            json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":foreign,"languageId":"rs2","version":1,"text":script}}}),
            json!({"method":"textDocument/didChange","params":{"textDocument":{"uri":foreign,"version":2},"contentChanges":[{"text":format!("{script}[proc,foreign_only]\nreturn;\n")}]}}),
            json!({"method":"textDocument/didSave","params":{"textDocument":{"uri":foreign}}}),
            json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":global,"languageId":"gs2","version":1,"text":"[proc,new_shared]\nreturn;\n"}}}),
            json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":texture,"languageId":"rs2config","version":1,"text":"[op0]\ntype=constant_color\ncolor=0xff0000\n"}}}),
            json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":"untitled:scoped","languageId":"rs2","version":1,"text":"[proc,new_virtual]\nreturn;\n"}}}),
            json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":client,"languageId":"cs2","version":1,"text":callback}}}),
            json!({"method":"workspace/didChangeWatchedFiles","params":{"changes":[{"uri":watched,"type":2}]}}),
            json!({"method":"workspace/didChangeConfiguration","params":{"settings":{"target":"rune"}}}),
            json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":boundary,"languageId":"rs2","version":1,"text":script}}}),
            json!({"id":2,"method":"workspace/symbol","params":{"query":"scroll_elena_hangover_cure_recipe"}}),
            json!({"id":3,"method":"textDocument/diagnostic","params":{"textDocument":{"uri":active}}}),
            json!({"id":4,"method":"textDocument/diagnostic","params":{"textDocument":{"uri":foreign}}}),
            json!({"id":5,"method":"textDocument/semanticTokens/full","params":{"textDocument":{"uri":foreign}}}),
            json!({"id":6,"method":"workspace/symbol","params":{"query":"new_"}}),
            json!({"id":7,"method":"textDocument/documentColor","params":{"textDocument":{"uri":texture}}}),
            json!({"id":8,"method":"workspace/symbol","params":{"query":"callback"}}),
            json!({"method":"textDocument/didClose","params":{"textDocument":{"uri":foreign}}}),
            json!({"method":"textDocument/didClose","params":{"textDocument":{"uri":boundary}}}),
            json!({"id":9,"method":"textDocument/diagnostic","params":{"textDocument":{"uri":active}}}),
            json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":duplicate,"languageId":"rs2","version":1,"text":script}}}),
            json!({"id":10,"method":"textDocument/diagnostic","params":{"textDocument":{"uri":duplicate}}}),
            json!({"id":11,"method":"shutdown"}),
            json!({"method":"exit"}),
        ],
        None,
        Some(loader),
    );
    assert_eq!(loads.get(), 2);
    assert!(
        !result.iter().any(|message| message.get("error").is_some()),
        "{result:?}"
    );
    let response = |id| &result.iter().find(|message| message["id"] == id).unwrap()["result"];
    assert_eq!(response(2).as_array().unwrap().len(), 1);
    for id in [3, 4, 9] {
        assert_eq!(response(id)["items"], json!([]), "{id}: {}", response(id));
    }
    assert_eq!(response(5)["data"], json!([]));
    let mut names = response(6)
        .as_array()
        .unwrap()
        .iter()
        .map(|symbol| symbol["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, ["new_shared", "new_virtual"]);
    assert_eq!(response(7).as_array().unwrap().len(), 1);
    assert_eq!(response(8).as_array().unwrap().len(), 1);
    assert_eq!(response(12).as_array().unwrap().len(), 1);
    assert!(
        response(10)["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "RUNEC2001")
    );
}
