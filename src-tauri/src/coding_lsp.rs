use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use serde::Serialize;
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
};
use url::Url;

use crate::app_error::AppError;

const LSP_TIMEOUT_SECS: u64 = 20;
const MAX_SOURCE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_FALLBACK_FILES: usize = 1_200;
const MAX_SYMBOL_RESULTS: usize = 100;
const MAX_REFERENCE_RESULTS: usize = 200;
const MAX_LSP_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_LSP_HEADER_BYTES: usize = 16 * 1024;
const MAX_PROJECT_ROOTS: usize = 32;
const MAX_PROJECT_SCAN_DIRS: usize = 250;
const MAX_PROJECT_SCAN_DEPTH: usize = 5;
const MAX_WORKSPACE_LSP_SESSIONS: usize = 8;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationResult {
    pub engine: String,
    pub server: Option<String>,
    pub result: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ServerSpec {
    command: &'static str,
    args: &'static [&'static str],
    language_id: &'static str,
}

#[derive(Debug, Clone)]
struct WorkspaceServer {
    root: PathBuf,
    spec: ServerSpec,
}

struct LspSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    server_name: String,
    root_uri: String,
    root_name: String,
}

pub async fn workspace_symbols(
    root: &Path,
    query: &str,
    allow_language_server: bool,
) -> Result<NavigationResult, AppError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(AppError::internal("symbol_search query cannot be empty"));
    }
    let root = canonical_root(root)?;

    if allow_language_server {
        let mut collected = Vec::new();
        let mut seen = HashSet::new();
        let mut servers = Vec::new();
        let mut server_succeeded = false;

        for candidate in discover_workspace_servers(&root)?
            .into_iter()
            .take(MAX_WORKSPACE_LSP_SESSIONS)
        {
            if collected.len() >= MAX_SYMBOL_RESULTS {
                break;
            }
            let Ok(mut session) = LspSession::start(&root, &candidate.root, candidate.spec).await
            else {
                continue;
            };
            let request = session
                .request("workspace/symbol", json!({"query": query}))
                .await;
            match request {
                Ok(result) => {
                    server_succeeded = true;
                    servers.push(format!(
                        "{}@{}",
                        candidate.spec.command,
                        relative_display(&root, &candidate.root)
                    ));
                    let remaining = MAX_SYMBOL_RESULTS.saturating_sub(collected.len());
                    let sanitized = sanitize_lsp_result(&root, result, remaining)?;
                    append_unique_results(&mut collected, &mut seen, sanitized, remaining);
                }
                Err(_) => {}
            }
            session.close().await;
        }

        if !collected.is_empty() {
            return Ok(NavigationResult {
                engine: "lsp".to_string(),
                server: Some(servers.join(",")),
                result: Value::Array(collected),
            });
        }
        if server_succeeded {
            return Ok(NavigationResult {
                engine: "lsp+lexical-fallback".to_string(),
                server: Some(servers.join(",")),
                result: fallback_workspace_symbols(&root, query)?,
            });
        }
    }

    Ok(NavigationResult {
        engine: "lexical-fallback".to_string(),
        server: None,
        result: fallback_workspace_symbols(&root, query)?,
    })
}

pub async fn definition(
    root: &Path,
    relative_path: &str,
    line: u64,
    character: u64,
    allow_language_server: bool,
) -> Result<NavigationResult, AppError> {
    position_navigation(
        root,
        relative_path,
        line,
        character,
        NavigationKind::Definition,
        allow_language_server,
    )
    .await
}

pub async fn references(
    root: &Path,
    relative_path: &str,
    line: u64,
    character: u64,
    allow_language_server: bool,
) -> Result<NavigationResult, AppError> {
    position_navigation(
        root,
        relative_path,
        line,
        character,
        NavigationKind::References,
        allow_language_server,
    )
    .await
}

#[derive(Debug, Clone, Copy)]
enum NavigationKind {
    Definition,
    References,
}

async fn position_navigation(
    root: &Path,
    relative_path: &str,
    line: u64,
    character: u64,
    kind: NavigationKind,
    allow_language_server: bool,
) -> Result<NavigationResult, AppError> {
    if line == 0 {
        return Err(AppError::internal(
            "symbol navigation line is 1-based and must be >= 1",
        ));
    }
    let line_index = usize::try_from(line)
        .map_err(|_| AppError::internal("symbol navigation line is too large"))?;
    let character_index = usize::try_from(character)
        .map_err(|_| AppError::internal("symbol navigation character is too large"))?;

    let root = canonical_root(root)?;
    let file = resolve_source_file(&root, relative_path)?;
    let text = read_source(&file)?;
    let line_text = source_line(&text, line_index)?;
    validate_character_position(line_text, character_index)?;

    if let (true, Some(spec)) = (allow_language_server, select_server_for_file(&file)) {
        let server_root = nearest_project_root(&root, &file, spec);
        if let Ok(mut session) = LspSession::start(&root, &server_root, spec).await {
            if session
                .open_document(&file, &text, spec.language_id)
                .await
                .is_ok()
            {
                let uri = file_uri(&file)?;
                let lsp_character = utf16_character_offset(line_text, character_index)?;
                let params = match kind {
                    NavigationKind::Definition => json!({
                        "textDocument": {"uri": uri},
                        "position": {"line": line - 1, "character": lsp_character}
                    }),
                    NavigationKind::References => json!({
                        "textDocument": {"uri": uri},
                        "position": {"line": line - 1, "character": lsp_character},
                        "context": {"includeDeclaration": true}
                    }),
                };
                let (method, limit) = match kind {
                    NavigationKind::Definition => ("textDocument/definition", MAX_SYMBOL_RESULTS),
                    NavigationKind::References => {
                        ("textDocument/references", MAX_REFERENCE_RESULTS)
                    }
                };
                if let Ok(result) = session.request(method, params).await {
                    let result = sanitize_lsp_result(&root, result, limit)?;
                    if result_has_items(&result) {
                        session.close().await;
                        return Ok(NavigationResult {
                            engine: "lsp".to_string(),
                            server: Some(format!(
                                "{}@{}",
                                spec.command,
                                relative_display(&root, &server_root)
                            )),
                            result,
                        });
                    }
                }
            }
            session.close().await;
        }
    }

    let symbol = symbol_at_position(&text, line_index, character_index)?;
    let result = match kind {
        NavigationKind::Definition => fallback_definition(&root, &symbol)?,
        NavigationKind::References => fallback_references(&root, &symbol)?,
    };
    Ok(NavigationResult {
        engine: "lexical-fallback".to_string(),
        server: None,
        result,
    })
}

fn resolve_server_executable(workspace_root: &Path, name: &str) -> Result<PathBuf, AppError> {
    let path = env::var_os("PATH")
        .ok_or_else(|| AppError::internal("PATH is unavailable for language server discovery"))?;
    for directory in env::split_paths(&path) {
        if !directory.is_absolute() {
            continue;
        }
        let mut candidates = vec![directory.join(name)];
        if cfg!(windows) {
            candidates.push(directory.join(format!("{name}.exe")));
            candidates.push(directory.join(format!("{name}.cmd")));
            candidates.push(directory.join(format!("{name}.bat")));
        }
        for candidate in candidates {
            if !candidate.is_file() {
                continue;
            }
            let Ok(executable) = fs::canonicalize(candidate) else {
                continue;
            };
            if executable.starts_with(workspace_root) {
                continue;
            }
            return Ok(executable);
        }
    }
    Err(AppError::internal(format!(
        "language server `{name}` is not installed in a trusted PATH location"
    )))
}

impl LspSession {
    async fn start(
        workspace_root: &Path,
        server_root: &Path,
        spec: ServerSpec,
    ) -> Result<Self, AppError> {
        if !server_root.starts_with(workspace_root) || !server_root.is_dir() {
            return Err(AppError::internal(
                "language server root is outside the attached workspace",
            ));
        }
        let executable = resolve_server_executable(workspace_root, spec.command)?;
        let mut process = Command::new(executable);
        process
            .args(spec.args)
            .current_dir(server_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = process.spawn().map_err(|error| {
            AppError::internal(format!("failed to start {}: {error}", spec.command))
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppError::internal("language server stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::internal("language server stdout unavailable"))?;
        let root_uri = directory_uri(server_root)?;
        let root_name = server_root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("workspace")
            .to_string();
        let mut session = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            server_name: spec.command.to_string(),
            root_uri: root_uri.clone(),
            root_name: root_name.clone(),
        };
        session
            .request(
                "initialize",
                json!({
                    "processId": std::process::id(),
                    "clientInfo": {"name":"OpenMindAI Coding Workspace","version":"2"},
                    "rootUri": root_uri.clone(),
                    "workspaceFolders": [{"uri": root_uri, "name": root_name}],
                    "capabilities": {
                        "general": {"positionEncodings": ["utf-16"]},
                        "workspace": {
                            "workspaceFolders": true,
                            "configuration": true,
                            "symbol": {"dynamicRegistration": false}
                        },
                        "textDocument": {
                            "definition": {"dynamicRegistration": false, "linkSupport": true},
                            "references": {"dynamicRegistration": false},
                            "synchronization": {"dynamicRegistration": false, "didOpen": true}
                        }
                    }
                }),
            )
            .await?;
        session.notify("initialized", json!({})).await?;
        Ok(session)
    }

    async fn open_document(
        &mut self,
        file: &Path,
        text: &str,
        language_id: &str,
    ) -> Result<(), AppError> {
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": file_uri(file)?,
                    "languageId": language_id,
                    "version": 1,
                    "text": text
                }
            }),
        )
        .await
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, AppError> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.send(json!({
            "jsonrpc":"2.0",
            "id": id,
            "method": method,
            "params": params
        }))
        .await?;
        let response = tokio::time::timeout(
            Duration::from_secs(LSP_TIMEOUT_SECS),
            self.read_response(id),
        )
        .await
        .map_err(|_| {
            AppError::internal(format!("{} timed out handling {method}", self.server_name))
        })??;
        if let Some(error) = response.get("error") {
            return Err(AppError::internal(format!(
                "{} returned an LSP error for {method}: {error}",
                self.server_name
            )));
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), AppError> {
        self.send(json!({"jsonrpc":"2.0","method":method,"params":params}))
            .await
    }

    async fn send(&mut self, value: Value) -> Result<(), AppError> {
        let body = serde_json::to_vec(&value).map_err(|error| {
            AppError::internal(format!("failed to encode LSP request: {error}"))
        })?;
        if body.len() > MAX_LSP_MESSAGE_BYTES {
            return Err(AppError::internal("LSP request exceeds safety limit"));
        }
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.stdin.write_all(header.as_bytes()).await?;
        self.stdin.write_all(&body).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn read_response(&mut self, id: u64) -> Result<Value, AppError> {
        loop {
            let value = read_lsp_message(&mut self.stdout).await?;
            let is_response = value.get("method").is_none()
                && value.get("id").and_then(Value::as_u64) == Some(id);
            if is_response {
                return Ok(value);
            }
            if value.get("method").is_some() && value.get("id").is_some() {
                self.respond_to_server_request(&value).await?;
            }
        }
    }

    async fn respond_to_server_request(&mut self, request: &Value) -> Result<(), AppError> {
        let Some(id) = request.get("id").cloned() else {
            return Ok(());
        };
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = request.get("params").cloned().unwrap_or(Value::Null);

        let result = match method {
            "workspace/configuration" => {
                let count = params
                    .get("items")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or(0);
                Some(Value::Array(vec![Value::Null; count]))
            }
            "workspace/workspaceFolders" => Some(json!([{
                "uri": self.root_uri.clone(),
                "name": self.root_name.clone()
            }])),
            "workspace/applyEdit" => Some(json!({
                "applied": false,
                "failureReason": "OpenMindAI symbol navigation sessions are read-only"
            })),
            "client/registerCapability"
            | "client/unregisterCapability"
            | "window/workDoneProgress/create"
            | "workspace/semanticTokens/refresh"
            | "workspace/inlayHint/refresh"
            | "workspace/codeLens/refresh"
            | "window/showMessageRequest" => Some(Value::Null),
            _ => None,
        };

        if let Some(result) = result {
            self.send(json!({"jsonrpc":"2.0","id":id,"result":result}))
                .await
        } else {
            self.send(json!({
                "jsonrpc":"2.0",
                "id":id,
                "error":{"code":-32601,"message":"Method not supported by read-only OpenMindAI LSP client"}
            }))
            .await
        }
    }

    async fn close(&mut self) {
        let _ = self.request("shutdown", Value::Null).await;
        let _ = self.notify("exit", Value::Null).await;
        let _ = self.child.kill().await;
    }
}

async fn read_lsp_message(reader: &mut BufReader<ChildStdout>) -> Result<Value, AppError> {
    let mut content_length = None;
    let mut header_bytes = 0usize;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).await?;
        if read == 0 {
            return Err(AppError::internal("language server closed its output"));
        }
        header_bytes = header_bytes.saturating_add(read);
        if header_bytes > MAX_LSP_HEADER_BYTES {
            return Err(AppError::internal(
                "LSP response headers exceed safety limit",
            ));
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.trim().eq_ignore_ascii_case("Content-Length") {
                if content_length.is_some() {
                    return Err(AppError::internal("duplicate LSP Content-Length header"));
                }
                content_length = Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .map_err(|_| AppError::internal("invalid LSP Content-Length header"))?,
                );
            }
        }
    }
    let length = content_length.ok_or_else(|| AppError::internal("missing LSP Content-Length"))?;
    if length > MAX_LSP_MESSAGE_BYTES {
        return Err(AppError::internal(
            "language server response exceeds safety limit",
        ));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).await?;
    serde_json::from_slice(&body)
        .map_err(|error| AppError::internal(format!("invalid LSP JSON response: {error}")))
}

fn select_server_for_file(file: &Path) -> Option<ServerSpec> {
    match extension(file).as_str() {
        "rs" => Some(ServerSpec {
            command: "rust-analyzer",
            args: &[],
            language_id: "rust",
        }),
        "ts" => Some(ServerSpec {
            command: "typescript-language-server",
            args: &["--stdio"],
            language_id: "typescript",
        }),
        "tsx" => Some(ServerSpec {
            command: "typescript-language-server",
            args: &["--stdio"],
            language_id: "typescriptreact",
        }),
        "js" => Some(ServerSpec {
            command: "typescript-language-server",
            args: &["--stdio"],
            language_id: "javascript",
        }),
        "jsx" => Some(ServerSpec {
            command: "typescript-language-server",
            args: &["--stdio"],
            language_id: "javascriptreact",
        }),
        "py" => Some(ServerSpec {
            command: "pyright-langserver",
            args: &["--stdio"],
            language_id: "python",
        }),
        "go" => Some(ServerSpec {
            command: "gopls",
            args: &["serve"],
            language_id: "go",
        }),
        "c" | "h" => Some(ServerSpec {
            command: "clangd",
            args: &[],
            language_id: "c",
        }),
        "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => Some(ServerSpec {
            command: "clangd",
            args: &[],
            language_id: "cpp",
        }),
        _ => None,
    }
}

fn server_specs_for_directory(root: &Path) -> Vec<ServerSpec> {
    let mut specs = Vec::new();
    let mut commands = HashSet::new();
    let candidates = [
        (
            root.join("Cargo.toml").is_file(),
            ServerSpec {
                command: "rust-analyzer",
                args: &[],
                language_id: "rust",
            },
        ),
        (
            root.join("tsconfig.json").is_file()
                || root.join("jsconfig.json").is_file()
                || root.join("package.json").is_file(),
            ServerSpec {
                command: "typescript-language-server",
                args: &["--stdio"],
                language_id: "typescript",
            },
        ),
        (
            root.join("pyproject.toml").is_file()
                || root.join("requirements.txt").is_file()
                || root.join("setup.py").is_file(),
            ServerSpec {
                command: "pyright-langserver",
                args: &["--stdio"],
                language_id: "python",
            },
        ),
        (
            root.join("go.mod").is_file(),
            ServerSpec {
                command: "gopls",
                args: &["serve"],
                language_id: "go",
            },
        ),
        (
            root.join("compile_commands.json").is_file() || root.join("CMakeLists.txt").is_file(),
            ServerSpec {
                command: "clangd",
                args: &[],
                language_id: "cpp",
            },
        ),
    ];
    for (enabled, spec) in candidates {
        if enabled && commands.insert(spec.command) {
            specs.push(spec);
        }
    }
    specs
}

fn discover_workspace_servers(root: &Path) -> Result<Vec<WorkspaceServer>, AppError> {
    let mut results = Vec::new();
    let mut seen = HashSet::new();
    let mut directories = vec![(root.to_path_buf(), 0usize)];
    let mut scanned = 0usize;

    while let Some((directory, depth)) = directories.pop() {
        if scanned >= MAX_PROJECT_SCAN_DIRS || results.len() >= MAX_PROJECT_ROOTS {
            break;
        }
        scanned += 1;

        for spec in server_specs_for_directory(&directory) {
            let key = format!("{}|{}", spec.command, directory.to_string_lossy());
            if seen.insert(key) {
                results.push(WorkspaceServer {
                    root: directory.clone(),
                    spec,
                });
                if results.len() >= MAX_PROJECT_ROOTS {
                    break;
                }
            }
        }

        if depth >= MAX_PROJECT_SCAN_DEPTH {
            continue;
        }
        let mut entries = fs::read_dir(&directory)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries.into_iter().rev() {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() || ignored_directory(&path) {
                continue;
            }
            directories.push((path, depth + 1));
        }
    }

    Ok(results)
}

fn nearest_project_root(workspace_root: &Path, file: &Path, spec: ServerSpec) -> PathBuf {
    let mut current = file.parent().unwrap_or(workspace_root).to_path_buf();
    loop {
        if project_marker_present(&current, spec) {
            return current;
        }
        if current == workspace_root {
            break;
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if !parent.starts_with(workspace_root) {
            break;
        }
        current = parent.to_path_buf();
    }
    workspace_root.to_path_buf()
}

fn project_marker_present(root: &Path, spec: ServerSpec) -> bool {
    match spec.command {
        "rust-analyzer" => root.join("Cargo.toml").is_file(),
        "typescript-language-server" => {
            root.join("tsconfig.json").is_file()
                || root.join("jsconfig.json").is_file()
                || root.join("package.json").is_file()
        }
        "pyright-langserver" => {
            root.join("pyproject.toml").is_file()
                || root.join("requirements.txt").is_file()
                || root.join("setup.py").is_file()
        }
        "gopls" => root.join("go.mod").is_file(),
        "clangd" => {
            root.join("compile_commands.json").is_file() || root.join("CMakeLists.txt").is_file()
        }
        _ => false,
    }
}

fn sanitize_lsp_result(root: &Path, result: Value, limit: usize) -> Result<Value, AppError> {
    match result {
        Value::Null => Ok(Value::Null),
        Value::Array(items) => {
            let mut safe = Vec::new();
            for item in items {
                if safe.len() >= limit {
                    break;
                }
                if lsp_item_is_scoped(root, &item)? {
                    safe.push(item);
                }
            }
            Ok(Value::Array(safe))
        }
        Value::Object(_) => {
            if lsp_item_is_scoped(root, &result)? {
                Ok(result)
            } else {
                Ok(Value::Null)
            }
        }
        _ => Ok(Value::Null),
    }
}

fn result_has_items(result: &Value) -> bool {
    match result {
        Value::Array(items) => !items.is_empty(),
        Value::Object(_) => true,
        _ => false,
    }
}

fn lsp_item_is_scoped(root: &Path, item: &Value) -> Result<bool, AppError> {
    let uri = item
        .get("uri")
        .and_then(Value::as_str)
        .or_else(|| item.get("targetUri").and_then(Value::as_str))
        .or_else(|| item.pointer("/location/uri").and_then(Value::as_str));
    let Some(uri) = uri else {
        return Ok(false);
    };
    uri_is_scoped(root, uri)
}

fn uri_is_scoped(root: &Path, raw: &str) -> Result<bool, AppError> {
    let url = match Url::parse(raw) {
        Ok(url) if url.scheme() == "file" => url,
        _ => return Ok(false),
    };
    let path = match url.to_file_path() {
        Ok(path) => path,
        Err(_) => return Ok(false),
    };
    let canonical = match fs::canonicalize(&path) {
        Ok(path) => path,
        Err(_) => return Ok(false),
    };
    if !canonical.starts_with(root) {
        return Ok(false);
    }
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Ok(false);
    }
    Ok(true)
}

fn append_unique_results(
    target: &mut Vec<Value>,
    seen: &mut HashSet<String>,
    result: Value,
    limit: usize,
) {
    let mut values = match result {
        Value::Array(values) => values,
        Value::Null => Vec::new(),
        value => vec![value],
    };
    for value in values.drain(..) {
        if target.len() >= MAX_SYMBOL_RESULTS || limit == 0 {
            break;
        }
        let key = value.to_string();
        if seen.insert(key) {
            target.push(value);
        }
    }
}

fn fallback_workspace_symbols(root: &Path, query: &str) -> Result<Value, AppError> {
    let query = query.to_ascii_lowercase();
    let mut results = Vec::new();
    for file in collect_source_files(root)? {
        let Ok(text) = read_source(&file) else {
            continue;
        };
        for (line_index, line) in text.lines().enumerate() {
            if let Some((kind, symbol)) = declaration_symbol(&file, line) {
                if symbol.to_ascii_lowercase().contains(&query) {
                    results.push(json!({
                        "name": symbol,
                        "kind": kind,
                        "path": relative_display(root, &file),
                        "line": line_index + 1
                    }));
                    if results.len() >= MAX_SYMBOL_RESULTS {
                        return Ok(Value::Array(results));
                    }
                }
            }
        }
    }
    Ok(Value::Array(results))
}

fn fallback_definition(root: &Path, symbol: &str) -> Result<Value, AppError> {
    let mut results = Vec::new();
    for file in collect_source_files(root)? {
        let Ok(text) = read_source(&file) else {
            continue;
        };
        for (line_index, line) in text.lines().enumerate() {
            if let Some((kind, candidate)) = declaration_symbol(&file, line) {
                if candidate == symbol {
                    results.push(json!({
                        "name": candidate,
                        "kind": kind,
                        "path": relative_display(root, &file),
                        "line": line_index + 1
                    }));
                    if results.len() >= MAX_SYMBOL_RESULTS {
                        return Ok(Value::Array(results));
                    }
                }
            }
        }
    }
    Ok(Value::Array(results))
}

fn fallback_references(root: &Path, symbol: &str) -> Result<Value, AppError> {
    let mut results = Vec::new();
    for file in collect_source_files(root)? {
        let Ok(text) = read_source(&file) else {
            continue;
        };
        for (line_index, line) in text.lines().enumerate() {
            for column in word_occurrences(line, symbol) {
                results.push(json!({
                    "path": relative_display(root, &file),
                    "line": line_index + 1,
                    "character": column,
                    "preview": truncate_preview(line.trim(), 320)
                }));
                if results.len() >= MAX_REFERENCE_RESULTS {
                    return Ok(Value::Array(results));
                }
            }
        }
    }
    Ok(Value::Array(results))
}

fn declaration_symbol(file: &Path, line: &str) -> Option<(&'static str, String)> {
    let trimmed = line.trim_start();
    if trimmed.is_empty()
        || trimmed.starts_with("//")
        || trimmed.starts_with('#')
        || trimmed.starts_with("/*")
        || trimmed.starts_with('*')
    {
        return None;
    }

    match extension(file).as_str() {
        "rs" => rust_declaration(trimmed),
        "ts" | "tsx" | "js" | "jsx" => javascript_declaration(trimmed),
        "py" => python_declaration(trimmed),
        "go" => go_declaration(trimmed),
        "php" => php_declaration(trimmed),
        "c" | "h" | "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => c_like_declaration(trimmed),
        "java" | "cs" => java_like_declaration(trimmed),
        "kt" | "kts" => kotlin_declaration(trimmed),
        "rb" => ruby_declaration(trimmed),
        _ => None,
    }
}

fn rust_declaration(line: &str) -> Option<(&'static str, String)> {
    let mut value = strip_rust_visibility(line);
    for prefix in ["async ", "unsafe "] {
        if let Some(rest) = value.strip_prefix(prefix) {
            value = rest;
        }
    }
    for (prefix, kind) in [
        ("const fn ", "function"),
        ("fn ", "function"),
        ("struct ", "struct"),
        ("enum ", "enum"),
        ("trait ", "trait"),
        ("type ", "type"),
        ("const ", "constant"),
        ("static ", "static"),
        ("mod ", "module"),
    ] {
        if let Some(rest) = value.strip_prefix(prefix) {
            return identifier_from_start(rest).map(|symbol| (kind, symbol));
        }
    }
    None
}

fn strip_rust_visibility(line: &str) -> &str {
    if let Some(rest) = line.strip_prefix("pub ") {
        return rest;
    }
    if let Some(rest) = line.strip_prefix("pub(") {
        if let Some(index) = rest.find(')') {
            return rest[index + 1..].trim_start();
        }
    }
    line
}

fn javascript_declaration(line: &str) -> Option<(&'static str, String)> {
    let mut value = line;
    let exported = value.starts_with("export ");
    for prefix in ["export default ", "export declare ", "export ", "declare "] {
        if let Some(rest) = value.strip_prefix(prefix) {
            value = rest;
            break;
        }
    }
    if let Some(rest) = value.strip_prefix("async function ") {
        return identifier_from_start(rest).map(|symbol| ("function", symbol));
    }
    for (prefix, kind) in [
        ("function ", "function"),
        ("class ", "class"),
        ("interface ", "interface"),
        ("type ", "type"),
        ("enum ", "enum"),
        ("namespace ", "namespace"),
    ] {
        if let Some(rest) = value.strip_prefix(prefix) {
            return identifier_from_start(rest).map(|symbol| (kind, symbol));
        }
    }
    for prefix in ["const ", "let ", "var "] {
        if let Some(rest) = value.strip_prefix(prefix) {
            let symbol = identifier_from_start(rest)?;
            let suffix = rest.get(symbol.len()..).unwrap_or_default();
            if exported || suffix.contains("=>") || suffix.contains("= function") {
                return Some(("variable", symbol));
            }
        }
    }
    None
}

fn python_declaration(line: &str) -> Option<(&'static str, String)> {
    for (prefix, kind) in [
        ("async def ", "function"),
        ("def ", "function"),
        ("class ", "class"),
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return identifier_from_start(rest).map(|symbol| (kind, symbol));
        }
    }
    None
}

fn go_declaration(line: &str) -> Option<(&'static str, String)> {
    if let Some(mut rest) = line.strip_prefix("func ") {
        if rest.starts_with('(') {
            let end = rest.find(')')?;
            rest = rest[end + 1..].trim_start();
        }
        return identifier_from_start(rest).map(|symbol| ("function", symbol));
    }
    for (prefix, kind) in [
        ("type ", "type"),
        ("const ", "constant"),
        ("var ", "variable"),
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return identifier_from_start(rest).map(|symbol| (kind, symbol));
        }
    }
    None
}

fn php_declaration(line: &str) -> Option<(&'static str, String)> {
    let mut value = line.trim_start_matches(|character: char| character == '&');
    loop {
        let mut stripped = false;
        for prefix in [
            "public ",
            "protected ",
            "private ",
            "static ",
            "final ",
            "abstract ",
            "readonly ",
        ] {
            if let Some(rest) = value.strip_prefix(prefix) {
                value = rest;
                stripped = true;
                break;
            }
        }
        if !stripped {
            break;
        }
    }
    for (prefix, kind) in [
        ("function ", "function"),
        ("class ", "class"),
        ("interface ", "interface"),
        ("trait ", "trait"),
        ("enum ", "enum"),
    ] {
        if let Some(rest) = value.strip_prefix(prefix) {
            let rest = rest.trim_start_matches('&');
            return identifier_from_start(rest).map(|symbol| (kind, symbol));
        }
    }
    None
}

fn c_like_declaration(line: &str) -> Option<(&'static str, String)> {
    let mut value = line;
    for prefix in ["static ", "inline ", "constexpr ", "virtual ", "extern "] {
        if let Some(rest) = value.strip_prefix(prefix) {
            value = rest;
        }
    }
    for (prefix, kind) in [
        ("struct ", "struct"),
        ("class ", "class"),
        ("enum ", "enum"),
        ("namespace ", "namespace"),
    ] {
        if let Some(rest) = value.strip_prefix(prefix) {
            return identifier_from_start(rest).map(|symbol| (kind, symbol));
        }
    }
    function_name_before_paren(value).map(|symbol| ("function", symbol))
}

fn java_like_declaration(line: &str) -> Option<(&'static str, String)> {
    let mut value = line;
    loop {
        let mut stripped = false;
        for prefix in [
            "public ",
            "protected ",
            "private ",
            "static ",
            "final ",
            "abstract ",
            "sealed ",
            "partial ",
            "async ",
        ] {
            if let Some(rest) = value.strip_prefix(prefix) {
                value = rest;
                stripped = true;
                break;
            }
        }
        if !stripped {
            break;
        }
    }
    for (prefix, kind) in [
        ("class ", "class"),
        ("interface ", "interface"),
        ("enum ", "enum"),
        ("record ", "record"),
        ("struct ", "struct"),
    ] {
        if let Some(rest) = value.strip_prefix(prefix) {
            return identifier_from_start(rest).map(|symbol| (kind, symbol));
        }
    }
    function_name_before_paren(value).map(|symbol| ("function", symbol))
}

fn kotlin_declaration(line: &str) -> Option<(&'static str, String)> {
    let mut value = line;
    for prefix in ["public ", "private ", "protected ", "internal ", "suspend "] {
        if let Some(rest) = value.strip_prefix(prefix) {
            value = rest;
        }
    }
    for (prefix, kind) in [
        ("fun ", "function"),
        ("class ", "class"),
        ("interface ", "interface"),
        ("object ", "object"),
        ("typealias ", "type"),
    ] {
        if let Some(rest) = value.strip_prefix(prefix) {
            return identifier_from_start(rest).map(|symbol| (kind, symbol));
        }
    }
    if let Some(rest) = value.strip_prefix("data class ") {
        return identifier_from_start(rest).map(|symbol| ("class", symbol));
    }
    None
}

fn ruby_declaration(line: &str) -> Option<(&'static str, String)> {
    for (prefix, kind) in [
        ("def ", "function"),
        ("class ", "class"),
        ("module ", "module"),
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return identifier_from_start(rest).map(|symbol| (kind, symbol));
        }
    }
    None
}

fn function_name_before_paren(line: &str) -> Option<String> {
    let paren = line.find('(')?;
    let before = line[..paren].trim_end();
    if before.is_empty() {
        return None;
    }
    let name = before
        .split(|character: char| character.is_whitespace() || character == '*' || character == '&')
        .filter(|part| !part.is_empty())
        .next_back()?;
    if matches!(
        name,
        "if" | "for" | "while" | "switch" | "catch" | "return" | "new"
    ) {
        return None;
    }
    if name.chars().all(is_identifier_char) {
        Some(name.to_string())
    } else {
        None
    }
}

fn identifier_from_start(value: &str) -> Option<String> {
    let value = value.trim_start_matches(|character: char| ['*', '&', '('].contains(&character));
    let symbol = value
        .chars()
        .take_while(|character| is_identifier_char(*character))
        .collect::<String>();
    if symbol.is_empty() {
        None
    } else {
        Some(symbol)
    }
}

fn symbol_at_position(text: &str, line: usize, character: usize) -> Result<String, AppError> {
    let line_text = source_line(text, line)?;
    validate_character_position(line_text, character)?;
    let chars = line_text.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return Err(AppError::internal("no symbol at the requested position"));
    }
    let mut index = if character == chars.len() {
        chars.len().saturating_sub(1)
    } else {
        character
    };
    if !is_identifier_char(chars[index]) && index > 0 && is_identifier_char(chars[index - 1]) {
        index -= 1;
    }
    if !is_identifier_char(chars[index]) {
        return Err(AppError::internal("no symbol at the requested position"));
    }
    let mut start = index;
    while start > 0 && is_identifier_char(chars[start - 1]) {
        start -= 1;
    }
    let mut end = index + 1;
    while end < chars.len() && is_identifier_char(chars[end]) {
        end += 1;
    }
    Ok(chars[start..end].iter().collect())
}

fn source_line(text: &str, line: usize) -> Result<&str, AppError> {
    text.lines()
        .nth(line.saturating_sub(1))
        .ok_or_else(|| AppError::internal("symbol navigation line is outside the file"))
}

fn validate_character_position(line: &str, character: usize) -> Result<(), AppError> {
    let count = line.chars().count();
    if character > count {
        return Err(AppError::internal(
            "symbol navigation character is outside the line",
        ));
    }
    Ok(())
}

fn utf16_character_offset(line: &str, character: usize) -> Result<usize, AppError> {
    validate_character_position(line, character)?;
    Ok(line
        .chars()
        .take(character)
        .map(char::len_utf16)
        .sum::<usize>())
}

fn word_occurrences(line: &str, symbol: &str) -> Vec<usize> {
    let mut results = Vec::new();
    for (byte_index, _) in line.match_indices(symbol) {
        let before = line[..byte_index].chars().next_back();
        let after = line[byte_index + symbol.len()..].chars().next();
        if before.map_or(true, |value| !is_identifier_char(value))
            && after.map_or(true, |value| !is_identifier_char(value))
        {
            results.push(line[..byte_index].chars().count());
        }
    }
    results
}

fn collect_source_files(root: &Path) -> Result<Vec<PathBuf>, AppError> {
    let mut files = Vec::new();
    let mut directories = vec![(root.to_path_buf(), 0usize)];
    while let Some((directory, depth)) = directories.pop() {
        if depth > 10 || files.len() >= MAX_FALLBACK_FILES {
            continue;
        }
        let mut entries = fs::read_dir(&directory)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if files.len() >= MAX_FALLBACK_FILES {
                break;
            }
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                if !ignored_directory(&path) {
                    directories.push((path, depth + 1));
                }
            } else if metadata.is_file()
                && supported_source(&path)
                && metadata.len() <= MAX_SOURCE_BYTES
            {
                files.push(path);
            }
        }
    }
    Ok(files)
}

fn resolve_source_file(root: &Path, relative_path: &str) -> Result<PathBuf, AppError> {
    let relative = Path::new(relative_path.trim());
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(AppError::internal(
            "symbol navigation requires a workspace-relative file path",
        ));
    }
    let joined = root.join(relative);
    if joined.exists() && fs::symlink_metadata(&joined)?.file_type().is_symlink() {
        return Err(AppError::internal(
            "symbol navigation refuses symlink source files",
        ));
    }
    let file = fs::canonicalize(joined)?;
    if !file.starts_with(root) || !file.is_file() {
        return Err(AppError::internal(
            "symbol navigation file is outside the workspace or unsafe",
        ));
    }
    Ok(file)
}

fn read_source(path: &Path) -> Result<String, AppError> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_SOURCE_BYTES {
        return Err(AppError::internal(
            "symbol navigation file exceeds the read limit",
        ));
    }
    fs::read_to_string(path)
        .map_err(|_| AppError::internal("symbol navigation supports UTF-8 source files only"))
}

fn canonical_root(root: &Path) -> Result<PathBuf, AppError> {
    let root = fs::canonicalize(root)?;
    if !root.is_dir() {
        return Err(AppError::internal(
            "symbol navigation root is not a directory",
        ));
    }
    Ok(root)
}

fn directory_uri(path: &Path) -> Result<String, AppError> {
    Url::from_directory_path(path)
        .map(|url| url.to_string())
        .map_err(|_| AppError::internal("failed to construct workspace URI for language server"))
}

fn file_uri(path: &Path) -> Result<String, AppError> {
    Url::from_file_path(path)
        .map(|url| url.to_string())
        .map_err(|_| AppError::internal("failed to construct file URI for language server"))
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn supported_source(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "rs" | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "py"
            | "go"
            | "php"
            | "c"
            | "h"
            | "cc"
            | "cpp"
            | "cxx"
            | "hh"
            | "hpp"
            | "hxx"
            | "java"
            | "cs"
            | "kt"
            | "kts"
            | "rb"
    )
}

fn ignored_directory(path: &Path) -> bool {
    matches!(
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default(),
        ".git"
            | "node_modules"
            | "target"
            | "dist"
            | "build"
            | ".next"
            | ".venv"
            | "vendor"
            | ".openmindai-patch-transactions"
            | ".idea"
            | ".gradle"
            | ".cache"
            | "coverage"
            | "Pods"
            | "DerivedData"
    )
}

fn is_identifier_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn truncate_preview(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let mut output = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        output.push('…');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_finds_definitions_and_references() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::write(
            temp.path().join("src/lib.rs"),
            "pub struct Router {}\nfn build() { let _ = Router {}; }\n",
        )
        .unwrap();
        let symbols = fallback_workspace_symbols(temp.path(), "Router").unwrap();
        assert!(symbols.to_string().contains("Router"));
        let definitions = fallback_definition(temp.path(), "Router").unwrap();
        assert!(definitions.to_string().contains("src/lib.rs"));
        let references = fallback_references(temp.path(), "Router").unwrap();
        assert!(references.as_array().unwrap().len() >= 2);
    }

    #[test]
    fn fallback_understands_modern_declarations() {
        let rust = Path::new("lib.rs");
        let ts = Path::new("api.ts");
        let go = Path::new("router.go");
        assert_eq!(
            declaration_symbol(rust, "pub(crate) async fn execute() {}"),
            Some(("function", "execute".to_string()))
        );
        assert_eq!(
            declaration_symbol(ts, "export const loadUser = async () => {}"),
            Some(("variable", "loadUser".to_string()))
        );
        assert_eq!(
            declaration_symbol(go, "func (r *Router) Dispatch() {}"),
            Some(("function", "Dispatch".to_string()))
        );
    }

    #[test]
    fn symbol_at_position_extracts_identifier() {
        assert_eq!(
            symbol_at_position("let value = Router::new();", 1, 14).unwrap(),
            "Router"
        );
    }

    #[test]
    fn utf16_position_accounts_for_surrogate_pairs() {
        let line = "🙂Router";
        assert_eq!(utf16_character_offset(line, 1).unwrap(), 2);
        assert_eq!(utf16_character_offset(line, 7).unwrap(), 8);
    }

    #[test]
    fn polyglot_workspace_selects_multiple_servers() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("Cargo.toml"), "[package]\nname='x'\n").unwrap();
        fs::write(temp.path().join("package.json"), "{}").unwrap();
        let specs = server_specs_for_directory(temp.path());
        let commands = specs.iter().map(|spec| spec.command).collect::<Vec<_>>();
        assert!(commands.contains(&"rust-analyzer"));
        assert!(commands.contains(&"typescript-language-server"));
    }

    #[test]
    fn workspace_server_discovery_finds_nested_projects() {
        let temp = tempfile::tempdir().unwrap();
        let nested = temp.path().join("apps/web");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("package.json"), "{}").unwrap();
        let servers = discover_workspace_servers(temp.path()).unwrap();
        assert!(servers.iter().any(|server| {
            server.spec.command == "typescript-language-server" && server.root == nested
        }));
    }

    #[test]
    fn nearest_project_root_prefers_nested_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let nested = temp.path().join("crates/core");
        let source = nested.join("src/lib.rs");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(nested.join("Cargo.toml"), "[package]\nname='core'\n").unwrap();
        fs::write(&source, "pub fn run() {}\n").unwrap();
        let spec = select_server_for_file(&source).unwrap();
        assert_eq!(nearest_project_root(temp.path(), &source, spec), nested);
    }

    #[test]
    fn empty_lsp_result_is_not_treated_as_successful_navigation() {
        assert!(!result_has_items(&Value::Null));
        assert!(!result_has_items(&Value::Array(Vec::new())));
        assert!(result_has_items(&json!({"uri":"file:///tmp/a.rs"})));
    }

    #[test]
    fn lsp_results_are_scoped_to_workspace() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let inside_file = root.path().join("inside.rs");
        let outside_file = outside.path().join("outside.rs");
        fs::write(&inside_file, "fn inside() {}\n").unwrap();
        fs::write(&outside_file, "fn outside() {}\n").unwrap();
        let inside_uri = file_uri(&inside_file).unwrap();
        let outside_uri = file_uri(&outside_file).unwrap();
        let result = json!([
            {"uri": inside_uri, "range": {}},
            {"uri": outside_uri, "range": {}}
        ]);
        let sanitized = sanitize_lsp_result(root.path(), result, 10).unwrap();
        assert_eq!(sanitized.as_array().unwrap().len(), 1);
    }
}
