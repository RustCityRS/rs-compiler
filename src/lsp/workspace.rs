//! One protocol connection with independent server/client compiler environments.
//! The host serializes their work so the worker limit remains a process limit.
use super::*;
use std::io::Cursor;

/// Start one language server for the targets available in a revision. Loaders
/// retain separate command tables, opcode formats, symbols, and compiler caches.
/// Source suffixes and explicit config scope select a target; folders never do.
pub fn serve_workspace_with_loaders(loaders: Vec<(Scope, ProjectLoader)>) -> io::Result<()> {
    if loaders.is_empty()
        || loaders.iter().any(|(scope, _)| *scope == Scope::Global)
        || loaders.iter().enumerate().any(|(index, (scope, _))| {
            loaders[..index]
                .iter()
                .any(|(previous, _)| previous == scope)
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "A workspace requires one loader per available server/client target",
        ));
    }
    serve_host(Workspace::new(
        loaders
            .into_iter()
            .map(|(scope, loader)| (scope, Server::new(None, Some(loader))))
            .collect(),
    ))
}

struct Context {
    scope: Scope,
    server: Server,
    diagnostics: HashMap<Vec<u8>, Value>,
}

struct Workspace {
    contexts: Vec<Context>,
    /// Remember the language of untitled/virtual documents as well as file URIs.
    languages: HashMap<Vec<u8>, Scope>,
    published: HashMap<Vec<u8>, Value>,
    watcher_registered: bool,
}

fn target_name(scope: Scope) -> &'static str {
    match scope {
        Scope::Rune => "rune",
        Scope::Client => "client",
        Scope::Global => "global",
    }
}

fn target_label(scope: Scope) -> &'static str {
    match scope {
        Scope::Rune => "server",
        Scope::Client => "client",
        Scope::Global => "shared",
    }
}

fn extension_scope(uri: &str) -> Option<Scope> {
    Scope::from_path(&uri_path(uri).unwrap_or_else(|| PathBuf::from(uri)))
}

fn messages(bytes: Vec<u8>) -> io::Result<Vec<Value>> {
    let mut input = Cursor::new(bytes);
    let mut result = Vec::new();
    while let Some(message) = read_message(&mut input)? {
        result.push(message);
    }
    Ok(result)
}

fn push_unique(items: &mut Vec<Value>, item: Value) {
    if !items.contains(&item) {
        items.push(item);
    }
}

impl Workspace {
    fn new(servers: Vec<(Scope, Server)>) -> Self {
        Self {
            contexts: servers
                .into_iter()
                .map(|(scope, server)| Context {
                    scope,
                    server,
                    diagnostics: HashMap::new(),
                })
                .collect(),
            languages: HashMap::new(),
            published: HashMap::new(),
            watcher_registered: false,
        }
    }

    fn document_scope(&self, uri: &str) -> Scope {
        extension_scope(uri)
            .or_else(|| self.languages.get(&uri_key(uri)).copied())
            .or_else(|| {
                self.contexts.iter().find_map(|context| {
                    let source = context.server.find(uri)?;
                    context.server.project.document(source).map(|doc| doc.scope)
                })
            })
            .unwrap_or(Scope::Global)
    }

    fn targets(&self, uri: Option<&str>) -> Vec<usize> {
        let scope = uri.map_or(Scope::Global, |uri| self.document_scope(uri));
        self.contexts
            .iter()
            .enumerate()
            .filter(|(_, context)| scope == Scope::Global || context.scope == scope)
            .map(|(index, _)| index)
            .collect()
    }

    fn primary(&self, targets: &[usize]) -> Option<usize> {
        targets
            .iter()
            .copied()
            .find(|&index| self.contexts[index].scope == Scope::Rune)
            .or_else(|| targets.first().copied())
    }

    fn call(
        &mut self,
        index: usize,
        message: Value,
        token: CancellationToken,
    ) -> io::Result<(bool, Vec<(usize, Value)>)> {
        let mut output = Vec::new();
        let running = self.contexts[index]
            .server
            .process(message, token, &mut output)?;
        Ok((
            running,
            messages(output)?
                .into_iter()
                .map(|message| (index, message))
                .collect(),
        ))
    }

    /// References and renames follow the source declaration into both indexes.
    /// This also catches server references to a client callback and callers of a
    /// shared proc when the request started in a target-specific source file.
    fn navigation_requests(&self, primary: usize, message: &Value) -> Vec<(usize, Value)> {
        let mut requests = vec![(primary, message.clone())];
        let server = &self.contexts[primary].server;
        let params = &message["params"];
        let declaration = (|| {
            let source = server.source(params).ok()?;
            let doc = server.project.document(source)?;
            let offset = doc
                .lines
                .offset(&doc.text, position(&params["position"]).ok()?)?;
            let location = server.project.definition(source, offset)?;
            let doc = server.project.document(location.source)?;
            let position = doc.lines.position(&doc.text, location.range.start)?;
            Some((doc.label.to_string_lossy().into_owned(), position))
        })();
        if let Some((uri, position)) = declaration {
            for (index, context) in self.contexts.iter().enumerate() {
                if index == primary || context.server.find(&uri).is_none() {
                    continue;
                }
                let mut other = message.clone();
                other["params"]["textDocument"]["uri"] = json!(uri);
                other["params"]["position"] =
                    json!({"line":position.line,"character":position.character});
                requests.push((index, other));
            }
        }
        requests
    }

    fn error(output: &mut impl Write, message: &Value, code: i32, error: &str) -> io::Result<()> {
        if let Some(id) = message.get("id") {
            write_message(
                output,
                json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":error}}),
            )?;
        } else {
            write_message(
                output,
                json!({"jsonrpc":"2.0","method":"window/logMessage","params":{"type":1,"message":error}}),
            )?;
        }
        Ok(())
    }

    fn tag_commands(value: &mut Value, scope: Scope, shared: bool) {
        if let Some(items) = value.as_array_mut() {
            for item in items {
                let Some(command) = item
                    .get_mut("command")
                    .filter(|command| command.is_object())
                else {
                    continue;
                };
                if let Some(args) = command["arguments"]
                    .as_array_mut()
                    .and_then(|args| args.first_mut())
                {
                    args["target"] = json!(target_name(scope));
                }
                if shared {
                    if let Some(title) = command["title"].as_str() {
                        command["title"] = json!(format!("{title} ({})", target_label(scope)));
                    }
                    if let Some(title) = item["title"].as_str() {
                        item["title"] = json!(format!("{title} ({})", target_label(scope)));
                    }
                }
            }
        }
    }

    fn diagnostics(&self, uri: &str, entries: Vec<(usize, Vec<Value>)>) -> Vec<Value> {
        let shared = self.document_scope(uri) == Scope::Global && self.contexts.len() > 1;
        let mut merged: Vec<(Value, Vec<usize>)> = Vec::new();
        for (index, items) in entries {
            for item in items {
                if let Some((_, targets)) = merged.iter_mut().find(|(other, _)| *other == item) {
                    if !targets.contains(&index) {
                        targets.push(index);
                    }
                } else {
                    merged.push((item, vec![index]));
                }
            }
        }
        merged
            .into_iter()
            .map(|(mut item, targets)| {
                if shared && targets.len() == 1 {
                    item["source"] = json!(format!(
                        "runec/{}",
                        target_label(self.contexts[targets[0]].scope)
                    ));
                }
                item
            })
            .collect()
    }

    fn merge_result(
        &self,
        method: &str,
        uri: Option<&str>,
        results: &[(usize, Value)],
    ) -> Result<Value, String> {
        let first = &results[0].1;
        if results.len() == 1 {
            return Ok(first.clone());
        }
        match method {
            "textDocument/diagnostic" => Ok(
                json!({"kind":"full","items":self.diagnostics(uri.unwrap_or(""), results.iter().map(|(index, result)| (*index, result["items"].as_array().cloned().unwrap_or_default())).collect())}),
            ),
            "textDocument/hover" => {
                let mut variants = Vec::new();
                for (index, value) in results {
                    if !value.is_null() && !variants.iter().any(|(_, other)| other == value) {
                        variants.push((*index, value.clone()));
                    }
                }
                if variants.len() <= 1 {
                    return Ok(variants
                        .first()
                        .map_or(Value::Null, |(_, value)| value.clone()));
                }
                let contents = variants
                    .iter()
                    .map(|(index, value)| {
                        let content = &value["contents"];
                        let text = content["value"]
                            .as_str()
                            .or_else(|| content.as_str())
                            .unwrap_or("");
                        let text = if content["kind"] == "plaintext" {
                            format!("```text\n{text}\n```")
                        } else {
                            text.to_string()
                        };
                        format!(
                            "**{}**\n\n{text}",
                            target_label(self.contexts[*index].scope)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n---\n\n");
                Ok(
                    json!({"range":variants[0].1["range"],"contents":{"kind":"markdown","value":contents}}),
                )
            }
            "textDocument/completion" => {
                let mut items = Vec::new();
                for (_, value) in results {
                    for item in value["items"].as_array().into_iter().flatten() {
                        push_unique(&mut items, item.clone());
                    }
                }
                Ok(json!({"isIncomplete":true,"items":items}))
            }
            "textDocument/signatureHelp" => {
                let mut signatures = Vec::new();
                for (_, value) in results {
                    for signature in value["signatures"].as_array().into_iter().flatten() {
                        push_unique(&mut signatures, signature.clone());
                    }
                }
                if signatures.is_empty() {
                    return Ok(Value::Null);
                }
                let active = results.iter().find(|(_, value)| !value.is_null()).unwrap();
                Ok(
                    json!({"signatures":signatures,"activeSignature":0,"activeParameter":active.1["activeParameter"]}),
                )
            }
            "textDocument/rename" => {
                let mut changes = serde_json::Map::new();
                let mut uris = HashMap::new();
                for (_, value) in results {
                    for (uri, edits) in value["changes"].as_object().into_iter().flatten() {
                        let uri = uris.entry(uri_key(uri)).or_insert_with(|| uri.clone());
                        let list = changes
                            .entry(uri.clone())
                            .or_insert_with(|| json!([]))
                            .as_array_mut()
                            .unwrap();
                        for edit in edits.as_array().into_iter().flatten() {
                            if list.iter().any(|other| {
                                other["range"] == edit["range"]
                                    && other["newText"] != edit["newText"]
                            }) {
                                return Err("The server and client targets require conflicting rename edits".into());
                            }
                            push_unique(list, edit.clone());
                        }
                    }
                }
                Ok(json!({"changes":changes}))
            }
            "textDocument/definition" => {
                let mut locations = Vec::new();
                for (_, value) in results {
                    if let Some(items) = value.as_array() {
                        for item in items {
                            push_unique(&mut locations, item.clone());
                        }
                    } else if !value.is_null() {
                        push_unique(&mut locations, value.clone());
                    }
                }
                Ok(match locations.len() {
                    0 => Value::Null,
                    1 => locations.remove(0),
                    _ => json!(locations),
                })
            }
            _ if first.is_array() => {
                let mut items = Vec::new();
                for (_, value) in results {
                    for item in value.as_array().into_iter().flatten() {
                        push_unique(&mut items, item.clone());
                    }
                }
                Ok(json!(items))
            }
            _ => Ok(first.clone()),
        }
    }

    /// Combine all contributions before publishing so a clean target cannot
    /// clear an error emitted by the other target for the same shared file.
    fn emit(&mut self, captured: Vec<(usize, Value)>, output: &mut impl Write) -> io::Result<()> {
        let mut dirty = Vec::new();
        let mut refreshes = HashSet::new();
        for (index, mut message) in captured {
            let method = message["method"].as_str().unwrap_or("");
            if method == "textDocument/publishDiagnostics" {
                let Some(uri) = message["params"]["uri"].as_str() else {
                    continue;
                };
                let key = uri_key(uri);
                let scope = self.document_scope(uri);
                if scope == Scope::Global || scope == self.contexts[index].scope {
                    self.contexts[index]
                        .diagnostics
                        .insert(key.clone(), message["params"].clone());
                    if !dirty.contains(&key) {
                        dirty.push(key);
                    }
                }
                continue;
            }
            if method == "client/registerCapability" {
                if self.watcher_registered {
                    continue;
                }
                self.watcher_registered = true;
                message["params"]["registrations"][0]["id"] = json!("runec/workspace/files");
            } else if matches!(
                method,
                "workspace/diagnostic/refresh" | "workspace/codeLens/refresh"
            ) && !refreshes.insert(method.to_string())
            {
                continue;
            }
            if message.get("method").is_some()
                && let Some(id) = message["id"].as_str()
            {
                message["id"] = json!(format!(
                    "runec/{}/{id}",
                    target_name(self.contexts[index].scope)
                ));
            }
            write_message(output, message)?;
        }
        for key in dirty {
            let entries = self
                .contexts
                .iter()
                .enumerate()
                .filter_map(|(index, context)| {
                    context.diagnostics.get(&key).map(|value| (index, value))
                })
                .collect::<Vec<_>>();
            let Some((_, first)) = entries.first() else {
                continue;
            };
            let uri = first["uri"].as_str().unwrap();
            let scope = self.document_scope(uri);
            let version = entries
                .iter()
                .filter(|(index, _)| scope == Scope::Global || scope == self.contexts[*index].scope)
                .filter_map(|(_, value)| value["version"].as_i64())
                .max();
            let diagnostics = self.diagnostics(
                uri,
                entries
                    .iter()
                    .filter(|(index, value)| {
                        (scope == Scope::Global || scope == self.contexts[*index].scope)
                            && (value["version"].as_i64().is_none()
                                || value["version"].as_i64() == version)
                    })
                    .map(|(index, value)| {
                        (
                            *index,
                            value["diagnostics"].as_array().cloned().unwrap_or_default(),
                        )
                    })
                    .collect(),
            );
            let mut params = json!({"uri":uri,"diagnostics":diagnostics});
            if let Some(version) = version {
                params["version"] = json!(version);
            }
            if self.published.get(&key) == Some(&params) {
                continue;
            }
            write_message(
                output,
                json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":params}),
            )?;
            self.published.insert(key, params);
        }
        Ok(())
    }
}

impl Host for Workspace {
    fn idle_delay(&self) -> Duration {
        self.contexts
            .iter()
            .map(|context| context.server.idle_delay())
            .min()
            .unwrap_or(DEEP_CHECK_DELAY)
    }

    fn set_cancellation(&mut self, token: CancellationToken) {
        for context in &mut self.contexts {
            context.server.project.set_cancellation(token.clone());
        }
    }

    fn idle(&mut self, output: &mut impl Write) -> io::Result<()> {
        let mut captured = Vec::new();
        for (index, context) in self.contexts.iter_mut().enumerate() {
            let mut buffer = Vec::new();
            context.server.idle(&mut buffer)?;
            captured.extend(
                messages(buffer)?
                    .into_iter()
                    .map(|message| (index, message)),
            );
        }
        self.emit(captured, output)
    }

    fn settle(&mut self, output: &mut impl Write) -> io::Result<()> {
        let mut captured = Vec::new();
        for (index, context) in self.contexts.iter_mut().enumerate() {
            let mut buffer = Vec::new();
            context.server.settle(&mut buffer)?;
            captured.extend(
                messages(buffer)?
                    .into_iter()
                    .map(|message| (index, message)),
            );
        }
        self.emit(captured, output)
    }

    fn process(
        &mut self,
        mut message: Value,
        token: CancellationToken,
        output: &mut impl Write,
    ) -> io::Result<bool> {
        if message.get("method").is_none() {
            let Some(id) = message["id"].as_str() else {
                return Ok(true);
            };
            for index in 0..self.contexts.len() {
                let prefix = format!("runec/{}/", target_name(self.contexts[index].scope));
                if let Some(original) = id.strip_prefix(&prefix) {
                    message["id"] = json!(original);
                    let (running, captured) = self.call(index, message, token)?;
                    self.emit(captured, output)?;
                    return Ok(running);
                }
            }
            return Ok(true);
        }
        let method = message["method"].as_str().unwrap_or("").to_string();
        // A portable JSON environment describes one target. Replacing both
        // filesystem projects with it would silently erase the client registry.
        let settings = match method.as_str() {
            "initialize" => runec_settings(&message["params"]["initializationOptions"]),
            "workspace/didChangeConfiguration" => runec_settings(&message["params"]["settings"]),
            _ => &Value::Null,
        };
        if has_environment(settings) {
            Self::error(
                output,
                &message,
                -32602,
                "Portable compiler environments require a single-target LSP; a filesystem workspace loads its environments from the revision",
            )?;
            return Ok(true);
        }
        if method == "textDocument/didOpen"
            && let Some(uri) = message["params"]["textDocument"]["uri"].as_str()
        {
            let language = extension_scope(uri).or_else(|| {
                message["params"]["textDocument"]["languageId"]
                    .as_str()
                    .and_then(|value| scope(value).ok())
            });
            if let Some(scope) = language {
                self.languages.insert(uri_key(uri), scope);
                message["params"]["textDocument"]["languageId"] = json!(target_name(scope));
            }
        }
        let uri = message["params"]["textDocument"]["uri"]
            .as_str()
            .or_else(|| message["params"]["arguments"][0]["textDocument"]["uri"].as_str())
            .map(str::to_string);
        let mut targets = self.targets(uri.as_deref());
        let notification = message.get("id").is_none();
        let broadcast = notification
            || matches!(
                method.as_str(),
                "initialize" | "shutdown" | "workspace/symbol"
            );
        if broadcast {
            targets = (0..self.contexts.len()).collect();
        }
        if matches!(
            method.as_str(),
            "textDocument/didOpen"
                | "textDocument/didChange"
                | "textDocument/didSave"
                | "textDocument/didClose"
        ) && let Some(uri) = uri.as_deref()
            && let Some(scope) =
                extension_scope(uri).or_else(|| self.languages.get(&uri_key(uri)).copied())
        {
            let engine = uri_path(uri)
                .unwrap_or_else(|| PathBuf::from(uri))
                .file_stem()
                .is_some_and(|name| name == "engine");
            targets.retain(|&index| {
                let target = self.contexts[index].scope;
                scope == Scope::Global || scope == target
                    // The server's index needs current callback declarations;
                    // its engine commands remain independent of client commands.
                    || (scope == Scope::Client && target == Scope::Rune && !engine)
            });
        }
        if method == "workspace/executeCommand" {
            if let Some(target) = message["params"]["arguments"][0]["target"].as_str() {
                let selected = self
                    .contexts
                    .iter()
                    .position(|context| target_name(context.scope) == target);
                if let Some(index) = selected.filter(|index| targets.contains(index)) {
                    targets = vec![index];
                } else {
                    Self::error(
                        output,
                        &message,
                        -32602,
                        "The requested compiler target is unavailable for this document",
                    )?;
                    return Ok(true);
                }
            } else if let Some(index) = self.primary(&targets) {
                targets = vec![index];
            }
        }
        let multi = matches!(
            method.as_str(),
            "textDocument/diagnostic"
                | "textDocument/codeLens"
                | "textDocument/codeAction"
                | "textDocument/hover"
                | "textDocument/completion"
                | "textDocument/signatureHelp"
                | "textDocument/inlayHint"
                | "textDocument/definition"
                | "textDocument/references"
                | "textDocument/rename"
        );
        if !broadcast
            && !multi
            && let Some(index) = self.primary(&targets)
        {
            targets = vec![index];
        }
        if targets.is_empty() {
            Self::error(
                output,
                &message,
                -32602,
                "No compiler target is available for this document",
            )?;
            return Ok(true);
        }
        let requests = if matches!(
            method.as_str(),
            "textDocument/references" | "textDocument/rename"
        ) {
            let primary = self.primary(&targets).unwrap();
            with_worker_limit(
                self.contexts[primary].server.performance.max_workers,
                || self.navigation_requests(primary, &message),
            )
        } else {
            targets
                .into_iter()
                .map(|index| (index, message.clone()))
                .collect()
        };
        let shared = requests.len() > 1 && uri.is_some();
        let mut running = true;
        let mut captured = Vec::new();
        let mut responses = Vec::new();
        for (index, request) in requests {
            let (keep_running, replies) = self.call(index, request, token.clone())?;
            running &= keep_running;
            for (index, mut reply) in replies {
                if reply.get("method").is_none() && reply.get("id") == message.get("id") {
                    if matches!(
                        method.as_str(),
                        "textDocument/codeLens" | "textDocument/codeAction"
                    ) {
                        Self::tag_commands(
                            &mut reply["result"],
                            self.contexts[index].scope,
                            shared,
                        );
                    }
                    responses.push((index, reply));
                } else {
                    captured.push((index, reply));
                }
            }
        }
        self.emit(captured, output)?;
        if let Some(id) = message.get("id") {
            if let Some((_, error)) = responses
                .iter()
                .find(|(_, response)| response.get("error").is_some())
            {
                write_message(output, error.clone())?;
            } else if !responses.is_empty() {
                let results = responses
                    .into_iter()
                    .map(|(index, response)| (index, response["result"].clone()))
                    .collect::<Vec<_>>();
                match self.merge_result(&method, uri.as_deref(), &results) {
                    Ok(result) => {
                        write_message(output, json!({"jsonrpc":"2.0","id":id,"result":result}))?
                    }
                    Err(error) => Self::error(output, &message, -32602, &error)?,
                }
            }
        }
        Ok(running)
    }
}

#[cfg(test)]
mod tests;
