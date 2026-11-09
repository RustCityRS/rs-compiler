//! A stdio Language Server Protocol adapter. The compiler and editor queries
//! remain usable independently of JSON, filesystem layout, or an editor.
use crate::clientscript::format::Format;
use crate::diagnostics::{Diagnostic, Severity};
use crate::filesystem::config_kind;
use crate::ide::{Location, SymbolKind, TextEdit, TokenKind};
use crate::project::{Analysis, CancellationToken, Options, Project, SourceId, Target};
use crate::sources::{Scope, Visibility};
use crate::symbol::{ReturnType, SymbolRegistry};
use crate::text::{LineIndex, Position};
use crate::types::Type;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead, Write};
use std::iter::from_fn;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

type Snapshot = (Scope, Arc<str>, Option<String>);
type ProjectLoader = Box<dyn Fn() -> Result<Project, String>>;

/// How long the editor must be idle before lints and pointer checks run.
const DEEP_CHECK_DELAY: Duration = Duration::from_millis(300);

fn open_source(
    project: &mut Project,
    uri: &str,
    scope: Scope,
    version: i64,
    text: Arc<str>,
    config: Option<String>,
) -> SourceId {
    if let Some(kind) = config {
        project.open_config(uri, scope, version, text, kind)
    } else {
        project.open(uri, scope, version, text)
    }
}

const TOKEN_TYPES: &[&str] = &[
    "keyword",
    "function",
    "parameter",
    "variable",
    "property",
    "type",
    "number",
    "string",
    "comment",
    "operator",
    "enumMember",
    "namespace",
    "decorator",
    "macro",
];

fn same_uri(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    let (Some(left), Some(right)) = (left.strip_prefix("file:"), right.strip_prefix("file:"))
    else {
        return false;
    };
    file_uri_bytes(left).eq(file_uri_bytes(right))
}

fn file_uri_bytes(uri: &str) -> impl Iterator<Item = u8> + '_ {
    let mut bytes = uri.as_bytes();
    from_fn(move || {
        let mut byte = *bytes.first()?;
        let mut length = 1;
        if let [b'%', high, low, ..] = bytes
            && let (Some(high), Some(low)) =
                ((*high as char).to_digit(16), (*low as char).to_digit(16))
        {
            byte = ((high << 4) | low) as u8;
            length = 3;
        }
        bytes = &bytes[length..];
        Some(if cfg!(windows) {
            byte.to_ascii_lowercase()
        } else {
            byte
        })
    })
}

fn uri_key(uri: &str) -> Vec<u8> {
    match uri.strip_prefix("file:") {
        Some(path) => b"file:"
            .iter()
            .copied()
            .chain(file_uri_bytes(path))
            .collect(),
        None => uri.as_bytes().to_vec(),
    }
}

/// The file a `file:` URI names.
fn uri_path(uri: &str) -> Option<PathBuf> {
    let path = uri.strip_prefix("file://")?;
    let path = path.strip_prefix("localhost").unwrap_or(path);
    let mut bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    while let Some(&byte) = bytes.first() {
        if let [b'%', high, low, ..] = bytes
            && let (Some(high), Some(low)) =
                ((*high as char).to_digit(16), (*low as char).to_digit(16))
        {
            decoded.push(((high << 4) | low) as u8);
            bytes = &bytes[3..];
        } else {
            decoded.push(byte);
            bytes = &bytes[1..];
        }
    }
    let path = String::from_utf8(decoded).ok()?;
    let path = if cfg!(windows) {
        path.strip_prefix('/').unwrap_or(&path).to_string()
    } else {
        path
    };
    Some(PathBuf::from(path))
}

/// Scripts other than the engine's declarations change nothing the loader builds.
fn script_only(label: &Path) -> bool {
    label
        .extension()
        .is_some_and(|ext| matches!(ext.to_str(), Some("rs2" | "cs2" | "gs2")))
        && label
            .file_stem()
            .is_none_or(|stem| stem.to_string_lossy() != "engine")
}

fn has_environment(value: &Value) -> bool {
    value.as_object().is_some_and(|settings| {
        settings.keys().any(|key| {
            matches!(
                key.as_str(),
                "target"
                    | "opcodes"
                    | "commands"
                    | "entities"
                    | "variables"
                    | "constants"
                    | "scriptIds"
                    | "params"
                    | "sources"
            )
        })
    })
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
    value[name]
        .as_str()
        .ok_or_else(|| format!("Expected string '{name}'"))
}
fn integer(value: &Value, name: &str) -> Result<i32, String> {
    value[name]
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| format!("Expected 32-bit integer '{name}'"))
}
fn scope(value: &str) -> Result<Scope, String> {
    match value {
        "rune" | "rs2" | "runescript" => Ok(Scope::Rune),
        "client" | "cs2" | "clientscript" => Ok(Scope::Client),
        "global" | "gs2" | "globalscript" => Ok(Scope::Global),
        _ => Err(format!("Unknown script language '{value}'")),
    }
}
fn ty(value: &str) -> Result<Type, String> {
    Type::from_name(value).ok_or_else(|| format!("Unknown type '{value}'"))
}
fn types(value: &Value) -> Result<Vec<Type>, String> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    value
        .as_array()
        .ok_or("Expected an array of type names")?
        .iter()
        .map(|value| ty(value.as_str().ok_or("Expected type name")?))
        .collect()
}
fn rows<'a>(value: &'a Value, name: &str) -> impl Iterator<Item = &'a Value> {
    value[name].as_array().into_iter().flatten()
}

/// Portable environment exchange for hosts that cannot call the Rust API.
/// All source text, commands, config IDs and ownership arrive in this object.
pub fn project_from_json(value: &Value) -> Result<Project, String> {
    if !value.is_object() {
        return Err("Expected a compiler environment object".into());
    }
    for name in [
        "commands",
        "entities",
        "variables",
        "constants",
        "scriptIds",
        "params",
        "sources",
    ] {
        if let Some(rows) = value.get(name)
            && !rows.is_array()
        {
            return Err(format!("Expected array '{name}'"));
        }
    }
    let client = match value.get("target") {
        None => false,
        Some(Value::String(target)) if matches!(target.as_str(), "rune" | "rs2") => false,
        Some(Value::String(target)) if matches!(target.as_str(), "client" | "cs2") => true,
        _ => return Err("Expected target 'rune' or 'client'".into()),
    };
    let target = if client {
        Target::Client(Format::parse(field(value, "opcodes")?).map_err(|error| error.to_string())?)
    } else {
        Target::Rune
    };
    let mut registry = SymbolRegistry::new();
    let mut visibility = Visibility::new();
    for command in rows(value, "commands") {
        let name = field(command, "name")?;
        let params = types(&command["params"])?;
        let returns = types(&command["returns"])?;
        let kind = command["kind"].as_str().unwrap_or("fixed");
        if !matches!(kind, "fixed" | "enum" | "param" | "hook" | "hook_component") {
            return Err(format!(
                "Unknown command signature kind '{kind}' for '{name}'"
            ));
        }
        if registry.lookup_command(name).is_some() {
            return Err(format!("Duplicate command '{name}'"));
        }
        let dynamic = match kind {
            "enum" => Some(ReturnType::TypeArgument(1)),
            "param" => Some(ReturnType::ParamArgument(params.len().saturating_sub(1))),
            _ => None,
        };
        registry.register_command(name.into(), integer(command, "opcode")?, params, returns);
        registry.set_command_behavior(
            name,
            client,
            match kind {
                "hook" => Some(false),
                "hook_component" => Some(true),
                _ => None,
            },
            dynamic,
        );
    }
    for entity in rows(value, "entities") {
        let name = field(entity, "name")?;
        let kind = field(entity, "type")?;
        registry.register_entity_id(name.into(), ty(kind)?, integer(entity, "id")?);
        visibility.declare(kind, name, scope(field(entity, "scope")?)?);
    }
    for variable in rows(value, "variables") {
        let name = field(variable, "name")?;
        let category = field(variable, "category")?;
        registry.register_game_var(
            name.into(),
            ty(field(variable, "type")?)?,
            integer(variable, "id")?,
            category.into(),
        );
        visibility.declare(category, name, scope(field(variable, "scope")?)?);
    }
    for constant in rows(value, "constants") {
        let name = field(constant, "name")?;
        let kind = ty(field(constant, "type")?)?;
        let (integer, string) = if kind == Type::String {
            (None, Some(field(constant, "value")?.into()))
        } else {
            (Some(integer(constant, "value")?), None)
        };
        registry.register_constant(name.into(), kind, integer, string);
        visibility.declare("constant", name, scope(field(constant, "scope")?)?);
    }
    for script in rows(value, "scriptIds") {
        registry.set_script_id(
            field(script, "trigger")?,
            field(script, "name")?,
            integer(script, "id")?,
        );
    }
    for parameter in rows(value, "params") {
        registry.register_param_type(integer(parameter, "id")?, ty(field(parameter, "type")?)?);
    }
    registry.set_visibility(visibility);
    let mut project = Project::new(
        target,
        registry,
        Options {
            include_tests: true,
            ..Options::default()
        },
    );
    let mut labels = HashSet::new();
    for source in rows(value, "sources") {
        let uri = field(source, "uri")?;
        if !labels.insert(uri) {
            return Err(format!("Duplicate source '{uri}'"));
        }
        open_source(
            &mut project,
            uri,
            if source["config"].is_string() {
                scope(source["scope"].as_str().unwrap_or("global"))?
            } else {
                scope(field(source, "languageId")?)?
            },
            source["version"].as_i64().unwrap_or(0),
            Arc::from(field(source, "text")?),
            source["config"].as_str().map(str::to_string),
        );
    }
    Ok(project)
}

fn position(value: &Value) -> Result<Position, String> {
    Ok(Position {
        line: value["line"].as_u64().ok_or("Expected position line")? as usize,
        character: value["character"]
            .as_u64()
            .ok_or("Expected position character")? as usize,
    })
}

fn read_message(input: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut length = None;
    let mut line = String::new();
    let mut header_bytes = 0;
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        header_bytes += line.len();
        if header_bytes > 8192 {
            return Err(io::Error::other("LSP header exceeds 8 KiB"));
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((key, value)) = line.split_once(':')
            && key.eq_ignore_ascii_case("Content-Length")
        {
            length = Some(value.trim().parse::<usize>().map_err(io::Error::other)?);
        }
    }
    let length = length
        .filter(|&length| length <= 64 * 1024 * 1024)
        .ok_or_else(|| io::Error::other("Missing or excessive LSP Content-Length"))?;
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(io::Error::other)
}

fn write_message(output: &mut impl Write, message: Value) -> io::Result<()> {
    let body = serde_json::to_vec(&message)?;
    let mut bytes = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    bytes.extend_from_slice(&body);
    output.write_all(&bytes)?;
    output.flush()
}

pub fn serve(project: Option<Project>) -> io::Result<()> {
    serve_with_loader(project, None)
}

pub fn serve_with_loader(
    project: Option<Project>,
    loader: Option<ProjectLoader>,
) -> io::Result<()> {
    let (send, receive) = mpsc::sync_channel(8);
    let pending: Arc<Mutex<HashMap<String, CancellationToken>>> = Arc::default();
    let requests = pending.clone();
    let background = Arc::new(Mutex::new(CancellationToken::default()));
    let changed = background.clone();
    thread::spawn(move || {
        let mut input = io::stdin().lock();
        loop {
            let message = match read_message(&mut input) {
                Ok(Some(message)) => message,
                Ok(None) => break,
                Err(error) => {
                    let _ = send.send(Err(error));
                    break;
                }
            };
            if message["method"] == "$/cancelRequest" {
                if let Some(token) = requests
                    .lock()
                    .unwrap()
                    .get(&message["params"]["id"].to_string())
                {
                    token.cancel();
                }
                continue;
            }
            if message.get("method").is_some() {
                changed.lock().unwrap().cancel();
            }
            let token = CancellationToken::default();
            let key = message.get("id").map(Value::to_string);
            if let Some(key) = &key {
                requests.lock().unwrap().insert(key.clone(), token.clone());
            }
            if send.send(Ok((message, token, key))).is_err() {
                break;
            }
        }
    });
    let mut server = Server::new(project, loader);
    let mut output = io::stdout().lock();
    let mut next = None;
    loop {
        let message = match next.take() {
            Some(message) => message,
            None => match receive.recv_timeout(DEEP_CHECK_DELAY) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => {
                    if server.dirty && server.initialized && !server.shutdown {
                        let token = CancellationToken::default();
                        *background.lock().unwrap() = token.clone();
                        server.project.set_cancellation(token);
                        server.publish_deep(&mut output)?;
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            },
        };
        let (message, token, key) = message?;
        let keep_running = server.process(message, token, &mut output)?;
        if let Some(key) = key {
            pending.lock().unwrap().remove(&key);
        }
        if !keep_running {
            break;
        }
        // Edits arrive in bursts: diagnostics wait until the last is applied.
        match receive.try_recv() {
            Ok(message) => next = Some(message),
            Err(_) => server.settle(&mut output)?,
        }
    }
    Ok(())
}

/// Protocol harness useful to embedded servers and editor integration tests.
pub fn serve_io(
    input: impl BufRead,
    output: impl Write,
    project: Option<Project>,
) -> io::Result<()> {
    serve_io_with_loader(input, output, project, None)
}

pub fn serve_io_with_loader(
    mut input: impl BufRead,
    mut output: impl Write,
    project: Option<Project>,
    loader: Option<ProjectLoader>,
) -> io::Result<()> {
    let mut server = Server::new(project, loader);
    while let Some(message) = read_message(&mut input)? {
        if !server.process(message, CancellationToken::default(), &mut output)? {
            break;
        }
        server.settle(&mut output)?;
    }
    Ok(())
}

struct Server {
    project: Project,
    initialized: bool,
    shutdown: bool,
    baseline: HashMap<String, Snapshot>,
    loader: Option<ProjectLoader>,
    pending_load: bool,
    changed: bool,
    dirty: bool,
    watch_supported: bool,
    apply_supported: bool,
    pull_diagnostics: bool,
    request_id: u64,
    open_documents: HashSet<SourceId>,
    published: HashMap<String, (i64, Vec<Value>)>,
    uris: HashMap<Vec<u8>, SourceId>,
}

impl Server {
    fn new(project: Option<Project>, loader: Option<ProjectLoader>) -> Self {
        let pending_load = project.is_none() && loader.is_some();
        let project = project.unwrap_or_else(|| {
            Project::new(Target::Rune, SymbolRegistry::new(), Options::default())
        });
        let baseline = project
            .documents()
            .map(|doc| {
                (
                    doc.label.to_string_lossy().into_owned(),
                    (doc.scope, doc.text.clone(), doc.config.clone()),
                )
            })
            .collect();
        let mut server = Self {
            project,
            baseline,
            loader,
            pending_load,
            changed: false,
            dirty: false,
            watch_supported: false,
            apply_supported: false,
            pull_diagnostics: false,
            request_id: 0,
            open_documents: HashSet::new(),
            initialized: false,
            shutdown: false,
            published: HashMap::new(),
            uris: HashMap::new(),
        };
        server.index_uris();
        server
    }

    fn index_uris(&mut self) {
        self.uris.clear();
        for document in self.project.documents() {
            self.uris
                .entry(uri_key(&document.label.to_string_lossy()))
                .or_insert(document.id);
        }
    }

    fn find(&self, uri: &str) -> Option<SourceId> {
        self.uris.get(&uri_key(uri)).copied()
    }

    /// Publish once a burst of changes has been applied.
    fn settle(&mut self, output: &mut impl Write) -> io::Result<()> {
        if !self.changed {
            return Ok(());
        }
        self.changed = false;
        self.publish(output)?;
        self.dirty = !self.pull_diagnostics;
        Ok(())
    }

    fn process(
        &mut self,
        message: Value,
        token: CancellationToken,
        output: &mut impl Write,
    ) -> io::Result<bool> {
        if message.get("method").is_none()
            && message["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("runec/"))
        {
            return Ok(true);
        }
        if message["jsonrpc"] != "2.0" || !message["method"].is_string() {
            write_message(
                output,
                json!({"jsonrpc":"2.0", "id":message.get("id").unwrap_or(&Value::Null), "error":{"code":-32600,"message":"Invalid JSON-RPC request"}}),
            )?;
            return Ok(true);
        }
        self.project.set_cancellation(token.clone());
        let method = message["method"].as_str().unwrap_or("");
        if method == "exit" {
            return if self.shutdown {
                Ok(false)
            } else {
                Err(io::Error::other("LSP exit without shutdown"))
            };
        }
        let params = &message["params"];
        let result = if token.is_cancelled() {
            Err((-32800, "Request cancelled".into()))
        } else {
            self.handle(method, params)
        };
        let result = if token.is_cancelled() {
            Err((-32800, "Request cancelled".into()))
        } else {
            result
        };
        if method == "initialized" && self.loader.is_some() && self.watch_supported {
            self.request_id += 1;
            write_message(
                output,
                json!({"jsonrpc":"2.0","id":format!("runec/watch/{}",self.request_id),"method":"client/registerCapability","params":{"registrations":[{"id":"runec/files","method":"workspace/didChangeWatchedFiles","registerOptions":{"watchers":[{"globPattern":"**/*.{rs2,cs2,gs2,pack,constant,obj,npc,loc,inv,enum,struct,param,dbtable,dbrow,varp,varn,vars,varpbit,varc,seq,spotanim,mesanim,hunt,if,interface,idk,flo,flu,anim,mel,order,hashes,config*}","kind":7}]}}]}}),
            )?;
        }
        let result = if method == "workspace/executeCommand" && self.apply_supported {
            if let Ok(edit) = &result {
                self.request_id += 1;
                write_message(
                    output,
                    json!({"jsonrpc":"2.0","id":format!("runec/edit/{}",self.request_id),"method":"workspace/applyEdit","params":{"label":"Extract procedure","edit":edit}}),
                )?;
                Ok(Value::Null)
            } else {
                result
            }
        } else {
            result
        };
        if method == "initialized"
            && let Err((_, message)) = &result
        {
            write_message(
                output,
                json!({"jsonrpc":"2.0","method":"window/logMessage","params":{"type":1,"message":message}}),
            )?;
        }
        if let Some(id) = message.get("id") {
            let response = match result {
                Ok(result) => json!({"jsonrpc":"2.0", "id":id, "result":result}),
                Err((code, message)) => {
                    json!({"jsonrpc":"2.0", "id":id, "error":{"code":code, "message":message}})
                }
            };
            write_message(output, response)?;
        }
        if self.initialized
            && !self.shutdown
            && matches!(
                method,
                "initialized"
                    | "textDocument/didOpen"
                    | "textDocument/didChange"
                    | "textDocument/didSave"
                    | "textDocument/didClose"
                    | "workspace/didChangeConfiguration"
                    | "workspace/didChangeWatchedFiles"
                    | "workspace/didCreateFiles"
                    | "workspace/didRenameFiles"
                    | "workspace/didDeleteFiles"
            )
        {
            self.changed = true;
            if !self.pull_diagnostics
                && method == "textDocument/didClose"
                && self.source(params).is_err()
            {
                write_message(
                    output,
                    json!({"jsonrpc":"2.0", "method":"textDocument/publishDiagnostics", "params":{"uri":params["textDocument"]["uri"],"diagnostics":[]}}),
                )?;
            }
        }
        Ok(true)
    }
    fn source(&self, params: &Value) -> Result<SourceId, String> {
        let uri = field(&params["textDocument"], "uri")?;
        self.find(uri)
            .ok_or_else(|| format!("Document '{uri}' is not open"))
    }

    fn range(&self, source: SourceId, range: Range<usize>) -> Value {
        let document = self.project.document(source).unwrap();
        let start = document
            .lines
            .position(&document.text, range.start)
            .unwrap_or_default();
        let end = document
            .lines
            .position(&document.text, range.end)
            .unwrap_or(start);
        json!({"start":{"line":start.line,"character":start.character},"end":{"line":end.line,"character":end.character}})
    }

    fn location(&self, location: &Location) -> Value {
        json!({"uri":self.project.document(location.source).unwrap().label.to_string_lossy(), "range":self.range(location.source, location.range.clone())})
    }

    fn edits(&self, edits: Vec<TextEdit>) -> Value {
        let mut changes: HashMap<String, Vec<Value>> = HashMap::new();
        for edit in edits {
            changes.entry(self.project.document(edit.location.source).unwrap().label.to_string_lossy().into_owned()).or_default().push(json!({"range":self.range(edit.location.source, edit.location.range), "newText":edit.replacement}));
        }
        json!({"changes":changes})
    }

    fn handle(&mut self, method: &str, params: &Value) -> Result<Value, (i32, String)> {
        if method == "initialize" {
            if self.initialized {
                return Err((-32600, "Server is already initialized".into()));
            }
            if has_environment(&params["initializationOptions"]) {
                self.project = project_from_json(&params["initializationOptions"])
                    .map_err(|error| (-32602, error))?;
                self.baseline = self
                    .project
                    .documents()
                    .map(|doc| {
                        (
                            doc.label.to_string_lossy().into_owned(),
                            (doc.scope, doc.text.clone(), doc.config.clone()),
                        )
                    })
                    .collect();
                self.index_uris();
            }
            self.initialized = true;
            self.watch_supported =
                params["capabilities"]["workspace"]["didChangeWatchedFiles"]["dynamicRegistration"]
                    .as_bool()
                    .unwrap_or(false);
            self.apply_supported = params["capabilities"]["workspace"]["applyEdit"]
                .as_bool()
                .unwrap_or(false);
            self.pull_diagnostics = params["capabilities"]["textDocument"]["diagnostic"]
                .is_object()
                && params["capabilities"]["workspace"]["diagnostics"]["refreshSupport"]
                    .as_bool()
                    .unwrap_or(false);
            let mut result = json!({"capabilities":{
                "positionEncoding":"utf-16", "textDocumentSync":{"openClose":true,"change":2,"save":true},
                "hoverProvider":true,"definitionProvider":true,"referencesProvider":true,
                "renameProvider":{"prepareProvider":true},"documentSymbolProvider":true,"workspaceSymbolProvider":true,
                "completionProvider":{"triggerCharacters":["$","%","^","~","@","[",",",":","(","."]},
                "signatureHelpProvider":{"triggerCharacters":["(",","]},
                "semanticTokensProvider":{"legend":{"tokenTypes":TOKEN_TYPES,"tokenModifiers":["declaration","readonly","static","defaultLibrary"]},"full":true,"range":true},
                "foldingRangeProvider":true,"selectionRangeProvider":true,"documentFormattingProvider":true,
                "inlayHintProvider":true,"documentHighlightProvider":true,
                "codeActionProvider":{"codeActionKinds":["quickfix","refactor.extract"]},
                "executeCommandProvider":{"commands":["runec.extractProc"]}
            }, "serverInfo":{"name":"runec","version":env!("CARGO_PKG_VERSION")}});
            if self.pull_diagnostics {
                result["capabilities"]["diagnosticProvider"] =
                    json!({"interFileDependencies":true,"workspaceDiagnostics":false});
            }
            return Ok(result);
        }
        if !self.initialized {
            return Err((-32002, "Server is not initialized".into()));
        }
        if self.shutdown {
            return Err((-32600, "Server is shut down".into()));
        }
        if method == "shutdown" {
            self.shutdown = true;
            return Ok(Value::Null);
        }
        self.request(method, params).map_err(|error| {
            (
                if error.starts_with("Unsupported method") {
                    -32601
                } else {
                    -32602
                },
                error,
            )
        })
    }

    fn request(&mut self, method: &str, params: &Value) -> Result<Value, String> {
        match method {
            "initialized" => {
                if self.pending_load {
                    self.reload()?;
                    self.pending_load = false;
                }
                return Ok(Value::Null);
            }
            "$/cancelRequest" | "$/setTrace" => return Ok(Value::Null),
            "textDocument/didOpen" => {
                let doc = &params["textDocument"];
                let uri = field(doc, "uri")?;
                let config = config_kind(Path::new(uri)).or_else(|| {
                    (doc["languageId"] == "rs2config").then(|| {
                        Path::new(uri)
                            .extension()
                            .and_then(|extension| extension.to_str())
                            .unwrap_or("config")
                            .to_string()
                    })
                });
                let scope = if config.is_some() {
                    self.source(params)
                        .ok()
                        .and_then(|id| self.project.document(id))
                        .map_or(Scope::Global, |doc| doc.scope)
                } else {
                    scope(field(doc, "languageId")?)?
                };
                let version = doc["version"].as_i64().ok_or("Expected document version")?;
                let text = field(doc, "text")?;
                if let Ok(source) = self.source(params) {
                    self.project.close(source);
                    self.open_documents.remove(&source);
                }
                let source = open_source(
                    &mut self.project,
                    uri,
                    scope,
                    version,
                    Arc::from(text),
                    config,
                );
                self.open_documents.insert(source);
                self.index_uris();
                return Ok(Value::Null);
            }
            "textDocument/didClose" => {
                let uri = field(&params["textDocument"], "uri")?.to_owned();
                let source = self.source(params)?;
                self.project.close(source);
                self.open_documents.remove(&source);
                self.published.remove(&uri);
                if let Some((_, (scope, text, config))) = self
                    .baseline
                    .iter()
                    .find(|(label, _)| same_uri(label, &uri))
                {
                    open_source(
                        &mut self.project,
                        &uri,
                        *scope,
                        0,
                        text.clone(),
                        config.clone(),
                    );
                }
                self.index_uris();
                return Ok(Value::Null);
            }
            "textDocument/didSave" => {
                let doc = self.project.document(self.source(params)?).unwrap();
                self.baseline
                    .retain(|uri, _| !same_uri(uri, &doc.label.to_string_lossy()));
                self.baseline.insert(
                    doc.label.to_string_lossy().into_owned(),
                    (doc.scope, doc.text.clone(), doc.config.clone()),
                );
                // The open buffer already holds what a script's save wrote.
                if self.loader.is_some() && (doc.config.is_some() || !script_only(&doc.label)) {
                    self.reload()?;
                }
                return Ok(Value::Null);
            }
            "textDocument/didChange" => {
                let source = self.source(params)?;
                let mut text = self.project.document(source).unwrap().text.to_string();
                for change in params["contentChanges"]
                    .as_array()
                    .ok_or("Expected contentChanges")?
                {
                    let replacement = field(change, "text")?;
                    if change["range"].is_object() {
                        let lines = LineIndex::new(&text);
                        let start = lines
                            .offset(&text, position(&change["range"]["start"])?)
                            .ok_or("Invalid edit start")?;
                        let end = lines
                            .offset(&text, position(&change["range"]["end"])?)
                            .ok_or("Invalid edit end")?;
                        if start > end {
                            return Err("Edit start exceeds end".into());
                        }
                        text.replace_range(start..end, replacement);
                    } else {
                        text = replacement.into();
                    }
                }
                self.project.update(
                    source,
                    params["textDocument"]["version"]
                        .as_i64()
                        .ok_or("Expected document version")?,
                    text,
                )?;
                return Ok(Value::Null);
            }
            "workspace/symbol" => {
                let symbols = self
                    .project
                    .workspace_symbols(params["query"].as_str().unwrap_or(""), 200);
                let items = symbols.into_iter().filter_map(|symbol| {
                    let location = self.location(symbol.definition.as_ref()?);
                    Some(json!({"name":symbol.name,"kind":symbol_kind(symbol.kind),"location":location}))
                }).collect();
                return Ok(Value::Array(items));
            }
            "workspace/didChangeConfiguration" => {
                if !has_environment(&params["settings"]) {
                    return Ok(Value::Null);
                }
                let mut updated = project_from_json(&params["settings"])?;
                if params["settings"].get("sources").is_some() {
                    self.baseline = updated
                        .documents()
                        .map(|doc| {
                            (
                                doc.label.to_string_lossy().into_owned(),
                                (doc.scope, doc.text.clone(), doc.config.clone()),
                            )
                        })
                        .collect();
                } else {
                    for (uri, (scope, text, config)) in &self.baseline {
                        open_source(&mut updated, uri, *scope, 0, text.clone(), config.clone());
                    }
                }
                let mut open_documents = HashSet::new();
                for doc in self
                    .project
                    .documents()
                    .filter(|doc| self.open_documents.contains(&doc.id))
                {
                    let previous = updated
                        .documents()
                        .find(|old| {
                            same_uri(&old.label.to_string_lossy(), &doc.label.to_string_lossy())
                        })
                        .map(|old| old.id);
                    if let Some(previous) = previous {
                        updated.close(previous);
                    }
                    open_documents.insert(open_source(
                        &mut updated,
                        &doc.label.to_string_lossy(),
                        doc.scope,
                        doc.version,
                        doc.text.clone(),
                        doc.config.clone(),
                    ));
                }
                self.open_documents = open_documents;
                self.project = updated;
                self.index_uris();
                return Ok(Value::Null);
            }
            "workspace/didChangeWatchedFiles" => {
                if self.watched(params) {
                    self.reload()?;
                }
                return Ok(Value::Null);
            }
            "workspace/didCreateFiles"
            | "workspace/didRenameFiles"
            | "workspace/didDeleteFiles" => {
                self.reload()?;
                return Ok(Value::Null);
            }
            "workspace/executeCommand" => {
                if params["command"] != "runec.extractProc" {
                    return Err("Unsupported command".into());
                }
                let args = &params["arguments"][0];
                let source = self.source(args)?;
                let document = self.project.document(source).unwrap();
                let start = document
                    .lines
                    .offset(&document.text, position(&args["range"]["start"])?)
                    .ok_or("Invalid selection")?;
                let end = document
                    .lines
                    .offset(&document.text, position(&args["range"]["end"])?)
                    .ok_or("Invalid selection")?;
                let action = self
                    .project
                    .extract_proc(source, start..end, field(args, "name")?)?;
                return Ok(self.edits(action.edits));
            }
            _ => {}
        }
        let supported = [
            "textDocument/hover",
            "textDocument/definition",
            "textDocument/references",
            "textDocument/rename",
            "textDocument/completion",
            "textDocument/signatureHelp",
            "textDocument/documentSymbol",
            "textDocument/semanticTokens/full",
            "textDocument/semanticTokens/range",
            "textDocument/foldingRange",
            "textDocument/selectionRange",
            "textDocument/formatting",
            "textDocument/codeAction",
            "textDocument/inlayHint",
            "textDocument/prepareRename",
            "textDocument/documentHighlight",
            "textDocument/diagnostic",
        ];
        if !supported.contains(&method) {
            return Err(format!("Unsupported method '{method}'"));
        }
        let source = self.source(params)?;
        let document = self.project.document(source).unwrap();
        let offset = if params["position"].is_object() {
            document
                .lines
                .offset(&document.text, position(&params["position"])?)
                .ok_or("Invalid UTF-16 position")?
        } else {
            0
        };
        Ok(match method {
            "textDocument/hover" => self.project.hover(source, offset).map_or(Value::Null, |hover| {
                let contents = if matches!(hover.kind, SymbolKind::Config | SymbolKind::GameVariable | SymbolKind::Constant)
                    && !hover.documentation.is_empty()
                {
                    let value = hover.documentation.lines().map(|line| format!("    {line}")).collect::<Vec<_>>().join("\n");
                    json!({"kind":"markdown","value":value})
                } else {
                    json!({"kind":"plaintext","value":format!("{}\n{}",hover.detail,hover.documentation)})
                };
                json!({"range":self.range(source,hover.range),"contents":contents})
            }),
            "textDocument/definition" => self.project.definition(source, offset).map_or(Value::Null, |location| self.location(&location)),
            "textDocument/references" => Value::Array(self.project.references(source, offset, params["context"]["includeDeclaration"].as_bool().unwrap_or(false)).iter().map(|location| self.location(location)).collect()),
            "textDocument/rename" => self.edits(self.project.rename(source, offset, field(params,"newName")?)?),
            "textDocument/prepareRename" => {
                let hover = self.project.hover(source, offset).ok_or("No symbol at this position")?;
                if self.project.definition(source, offset).is_none() { return Ok(Value::Null); }
                json!({"range":self.range(source,hover.range.clone()),"placeholder":document.text[hover.range].to_string()})
            }
            "textDocument/documentHighlight" => Value::Array(self.project.references(source,offset,true).into_iter().filter(|location| location.source==source).map(|location|json!({"range":self.range(source,location.range),"kind":1})).collect()),
            "textDocument/inlayHint" => {
                let start = document.lines.offset(&document.text,position(&params["range"]["start"])?).ok_or("Invalid range")?;
                let end = document.lines.offset(&document.text,position(&params["range"]["end"])?).ok_or("Invalid range")?;
                Value::Array(self.project.inlay_hints(source,start..end).into_iter().map(|hint|json!({"position":document.lines.position(&document.text,hint.offset).map(|pos|json!({"line":pos.line,"character":pos.character})),"label":hint.label,"kind":if hint.parameter{2}else{1},"paddingLeft":!hint.parameter,"paddingRight":hint.parameter})).collect())
            }
            "textDocument/diagnostic" => {
                let label = document.label.clone();
                let mut analysis = self.project.check();
                analysis.diagnostics.merge(self.project.config_diagnostics());
                let document = self.project.document(source).unwrap();
                json!({"kind":"full","items":analysis.diagnostics.diagnostics().iter().filter(|d|d.file==label).map(|d|json!({"range":self.range(source,document.lines.diagnostic_range(&document.text,d.line,d.column)),"message":d.message,"source":"runec","code":d.code(),"severity":if d.severity==Severity::Error{1}else{2}})).collect::<Vec<_>>()})
            }
            "textDocument/completion" => {
                let items = self.project.completions(source, offset, 200).into_iter().map(|item| {
                    json!({
                        "label": item.label,
                        "detail": item.detail,
                        "kind": completion_kind(item.kind),
                        "textEdit": {"range": self.range(source, item.replacement), "newText": item.label}
                    })
                }).collect::<Vec<_>>();
                json!({"isIncomplete": true, "items": items})
            }
            "textDocument/signatureHelp" => self.project.signature_help(source, offset).map_or(Value::Null, |help| {
                let parameters = help.parameters.iter().map(|(name, ty)| {
                    json!({"label": format!("{} ${name}", ty.name())})
                }).collect::<Vec<_>>();
                json!({
                    "signatures": [{"label": help.label, "documentation": help.documentation, "parameters": parameters}],
                    "activeSignature": 0,
                    "activeParameter": help.active_parameter
                })
            }),
            "textDocument/documentSymbol" => {
                let symbols = self.project.document_symbols(source).into_iter().filter_map(|symbol| {
                    Some(json!({
                        "name": symbol.name,
                        "detail": symbol.detail,
                        "kind": symbol_kind(symbol.kind),
                        "range": self.range(source, symbol.extent.as_ref()?.range.clone()),
                        "selectionRange": self.range(source, symbol.definition.as_ref()?.range.clone())
                    }))
                }).collect();
                Value::Array(symbols)
            }
            "textDocument/foldingRange" => {
                let ranges = self.project.folding_ranges(source).into_iter().map(|range| {
                    let range = self.range(source, range);
                    json!({"startLine": range["start"]["line"], "endLine": range["end"]["line"]})
                }).collect();
                Value::Array(ranges)
            }
            "textDocument/selectionRange" => {
                let positions = params["positions"].as_array().ok_or("Expected positions")?;
                let mut ranges = Vec::with_capacity(positions.len());
                for value in positions {
                    let offset = document.lines.offset(&document.text, position(value)?).ok_or("Invalid position")?;
                    let mut parent = Value::Null;
                    for range in self.project.selection_ranges(source, offset).into_iter().rev() {
                        parent = if parent.is_null() {
                            json!({"range": self.range(source, range)})
                        } else {
                            json!({"range": self.range(source, range), "parent": parent})
                        };
                    }
                    ranges.push(parent);
                }
                Value::Array(ranges)
            }
            "textDocument/formatting" => {
                let tab_size = params["options"]["tabSize"].as_u64().unwrap_or(4) as usize;
                let spaces = params["options"]["insertSpaces"].as_bool().unwrap_or(true);
                let edits = self.project.format(source, tab_size, spaces).into_iter().map(|edit| {
                    json!({"range": self.range(source, edit.location.range), "newText": edit.replacement})
                }).collect();
                Value::Array(edits)
            }
            "textDocument/codeAction" => {
                let label = document.label.clone();
                let start = params["range"]["start"]["line"].as_u64().unwrap_or(0) as usize;
                let end = params["range"]["end"]["line"].as_u64().unwrap_or(usize::MAX as u64) as usize;
                let selection = params.get("range").and_then(|range| {
                    Some(document.lines.offset(&document.text, position(&range["start"]).ok()?)?..document.lines.offset(&document.text, position(&range["end"]).ok()?)?)
                });
                // Lint fixes follow once the idle check has run.
                let analysis = self.project.latest();
                let diagnostics = analysis.diagnostics.diagnostics().iter().filter(|diagnostic| {
                    diagnostic.file == label && (start..=end).contains(&diagnostic.line.saturating_sub(1))
                }).cloned().collect::<Vec<_>>();
                let mut fixes = self.project.quick_fixes(source, &diagnostics);
                if let Some(selection) = selection.filter(|range| !range.is_empty()) {
                    let names = self.project.workspace_symbols("extracted_proc", usize::MAX);
                    let name = (0..=names.len()).map(|i| if i == 0 { "extracted_proc".to_string() } else { format!("extracted_proc_{i}") })
                        .find(|name| names.iter().all(|symbol| symbol.name != *name)).unwrap();
                    if let Ok(action) = self.project.extract_proc(source, selection, &name) { fixes.push(action); }
                }
                let actions = fixes.into_iter().filter(|action| params["context"]["only"].as_array().is_none_or(|kinds| kinds.iter().any(|kind| kind.as_str().is_some_and(|kind| action.kind.starts_with(kind))))).map(|action| {
                    json!({"title": action.title, "kind": action.kind, "edit": self.edits(action.edits)})
                }).collect();
                Value::Array(actions)
            }
            "textDocument/semanticTokens/full" | "textDocument/semanticTokens/range" => {
                let mut data=Vec::new(); let mut previous=(0,0);
                let requested = if method.ends_with("/range") {
                    let start = document.lines.offset(&document.text, position(&params["range"]["start"])?).ok_or("Invalid range")?;
                    let end = document.lines.offset(&document.text, position(&params["range"]["end"])?).ok_or("Invalid range")?;
                    if start > end { return Err("Invalid range".into()); }
                    Some(start..end)
                } else { None };
                for mut token in self.project.semantic_tokens(source) {
                    if let Some(range) = &requested {
                        token.range.start = token.range.start.max(range.start);
                        token.range.end = token.range.end.min(range.end);
                    }
                    if token.range.start >= token.range.end { continue; }
                    let start=document.lines.position(&document.text,token.range.start).unwrap();
                    let end=document.lines.position(&document.text,token.range.end).unwrap();
                    for line in start.line..=end.line {
                        let character=if line==start.line {start.character} else {0};
                        let last=if line==end.line {end.character} else { let range=document.lines.line(line).unwrap(); document.text[range].trim_end_matches(['\r','\n']).encode_utf16().count() };
                        if last<=character {continue;}
                        data.extend([line-previous.0,if line==previous.0 {character-previous.1} else {character},last-character,token_kind(token.kind),usize::from(token.declaration) | (usize::from(token.readonly)<<1) | (usize::from(token.static_)<<2) | (usize::from(token.default_library)<<3)]); previous=(line,character);
                    }
                }
                json!({"data":data})
            }
            _ => unreachable!(),
        })
    }

    fn publish(&mut self, output: &mut impl Write) -> io::Result<()> {
        if self.pull_diagnostics {
            self.request_id += 1;
            return write_message(
                output,
                json!({
                    "jsonrpc":"2.0",
                    "id":format!("runec/diagnostics/{}", self.request_id),
                    "method":"workspace/diagnostic/refresh"
                }),
            );
        }
        let analysis = self.project.latest();
        self.publish_analysis(analysis, output)
    }

    /// Re-read scripts changed on disk, returning whether anything else needs a
    /// full reload. The editor owns open buffers; other projects' files are not ours.
    fn watched(&mut self, params: &Value) -> bool {
        let mut reload = false;
        for change in params["changes"].as_array().into_iter().flatten() {
            let Some(uri) = change["uri"].as_str() else {
                continue;
            };
            let source = self.find(uri);
            match (change["type"].as_u64(), source) {
                (Some(2), Some(source)) if self.open_documents.contains(&source) => {}
                (Some(2), Some(source)) => {
                    let document = self.project.document(source).unwrap();
                    let text = uri_path(uri).and_then(|path| fs::read_to_string(path).ok());
                    match text {
                        Some(text) if document.config.is_none() && script_only(&document.label) => {
                            if *document.text != *text {
                                let version = document.version + 1;
                                let label = document.label.to_string_lossy().into_owned();
                                let snapshot = (document.scope, Arc::from(text.as_str()), None);
                                let _ = self.project.update(source, version, text);
                                self.baseline.retain(|uri, _| !same_uri(uri, &label));
                                self.baseline.insert(label, snapshot);
                            }
                        }
                        _ => reload = true,
                    }
                }
                (Some(2) | Some(3), None) => {}
                _ => reload = true,
            }
        }
        reload
    }

    fn publish_deep(&mut self, output: &mut impl Write) -> io::Result<()> {
        let analysis = self.project.check();
        self.dirty = analysis.cancelled;
        self.publish_analysis(analysis, output)
    }

    fn reload(&mut self) -> Result<(), String> {
        let Some(loader) = &self.loader else {
            return Ok(());
        };
        let mut project = loader()?;
        let baseline = project
            .documents()
            .map(|doc| {
                (
                    doc.label.to_string_lossy().into_owned(),
                    (doc.scope, doc.text.clone(), doc.config.clone()),
                )
            })
            .collect();
        let mut open_documents = HashSet::new();
        for doc in self
            .project
            .documents()
            .filter(|doc| self.open_documents.contains(&doc.id))
        {
            let previous = project
                .documents()
                .find(|old| same_uri(&old.label.to_string_lossy(), &doc.label.to_string_lossy()))
                .map(|old| old.id);
            if let Some(previous) = previous {
                project.close(previous);
            }
            open_documents.insert(open_source(
                &mut project,
                &doc.label.to_string_lossy(),
                doc.scope,
                doc.version,
                doc.text.clone(),
                doc.config.clone(),
            ));
        }
        self.project = project;
        self.baseline = baseline;
        self.open_documents = open_documents;
        self.index_uris();
        Ok(())
    }

    fn publish_analysis(
        &mut self,
        mut analysis: Analysis,
        output: &mut impl Write,
    ) -> io::Result<()> {
        if analysis.cancelled {
            return Ok(());
        }
        analysis
            .diagnostics
            .merge(self.project.config_diagnostics());
        let current = self
            .project
            .documents()
            .map(|document| document.label.to_string_lossy().into_owned())
            .collect::<HashSet<_>>();
        let removed = self
            .published
            .keys()
            .filter(|uri| !current.contains(*uri))
            .cloned()
            .collect::<Vec<_>>();
        for uri in removed {
            self.published.remove(&uri);
            write_message(
                output,
                json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":uri,"diagnostics":[]}}),
            )?;
        }
        let mut by_file = HashMap::<&Path, Vec<&Diagnostic>>::new();
        for diagnostic in analysis.diagnostics.diagnostics() {
            by_file
                .entry(diagnostic.file.as_path())
                .or_default()
                .push(diagnostic);
        }
        for document in self.project.documents() {
            let found = by_file
                .get(document.label.as_path())
                .map_or(&[][..], Vec::as_slice);
            let uri = document.label.to_string_lossy().into_owned();
            if found.is_empty()
                && !self.published.contains_key(&uri)
                && !self.open_documents.contains(&document.id)
            {
                continue;
            }
            let diagnostics = found.iter()
                .map(|diagnostic| {
                    let mut message = diagnostic.message.clone();
                    for help in &diagnostic.help {
                        message.push_str("\nhelp: ");
                        message.push_str(&help.message);
                    }
                    json!({"range":self.range(document.id,document.lines.diagnostic_range(&document.text,diagnostic.line,diagnostic.column)),"severity":match diagnostic.severity {Severity::Error=>1,Severity::Warning=>2,Severity::Info=>3},"code":diagnostic.code(),"source":"runec","message":message})
                }).collect::<Vec<_>>();
            if self.published.get(&uri).is_some_and(|(version, previous)| {
                *version == document.version && *previous == diagnostics
            }) {
                continue;
            }
            write_message(
                output,
                json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":document.label.to_string_lossy(),"version":document.version,"diagnostics":diagnostics}}),
            )?;
            self.published.insert(uri, (document.version, diagnostics));
        }
        Ok(())
    }
}

fn symbol_kind(kind: SymbolKind) -> usize {
    match kind {
        SymbolKind::Procedure
        | SymbolKind::ClientScript
        | SymbolKind::Trigger
        | SymbolKind::Command => 12,
        SymbolKind::Constant => 14,
        _ => 13,
    }
}
fn completion_kind(kind: SymbolKind) -> usize {
    match kind {
        SymbolKind::Procedure
        | SymbolKind::ClientScript
        | SymbolKind::Trigger
        | SymbolKind::Command => 3,
        SymbolKind::Constant => 21,
        _ => 6,
    }
}
fn token_kind(kind: TokenKind) -> usize {
    match kind {
        TokenKind::Keyword => 0,
        TokenKind::Function => 1,
        TokenKind::Parameter => 2,
        TokenKind::Variable => 3,
        TokenKind::Property => 4,
        TokenKind::Type => 5,
        TokenKind::Number => 6,
        TokenKind::String => 7,
        TokenKind::Comment => 8,
        TokenKind::Operator => 9,
        TokenKind::EnumMember => 10,
        TokenKind::Namespace => 11,
        TokenKind::Decorator => 12,
        TokenKind::Macro => 13,
    }
}
