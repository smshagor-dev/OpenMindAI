#!/usr/bin/env python3
from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text(encoding="utf-8")
    if old not in text:
        raise SystemExit(f"anchor missing in {path}: {old[:120]!r}")
    if text.count(old) != 1:
        raise SystemExit(f"anchor not unique in {path}: {old[:120]!r}")
    path.write_text(text.replace(old, new, 1), encoding="utf-8")


coding = Path("src-tauri/src/coding_lsp.rs")

replace_once(
    coding,
    '    sync::Mutex,\n};',
    '    sync::{mpsc, Mutex},\n    task::JoinHandle,\n};',
)

replace_once(
    coding,
    'const LSP_SESSION_IDLE_SECS: u64 = 300;\n',
    '''const LSP_SESSION_IDLE_SECS: u64 = 300;
const MAX_LSP_CONTROL_MESSAGES: usize = 64;
const MAX_DIAGNOSTIC_FILES_PER_SESSION: usize = 128;
const MAX_DIAGNOSTICS_PER_FILE: usize = 200;
const MAX_DIAGNOSTIC_MESSAGE_CHARS: usize = 4_000;
const DIAGNOSTIC_WAIT_MILLIS: u64 = 600;
''',
)

replace_once(
    coding,
    '''struct DocumentState {
    version: i64,
    fingerprint: [u8; 32],
    end_position: LspPosition,
    last_used: Instant,
}
''',
    '''struct DocumentState {
    version: i64,
    fingerprint: [u8; 32],
    end_position: LspPosition,
    synced_at: Instant,
    last_used: Instant,
}

#[derive(Debug, Clone)]
struct DiagnosticSnapshot {
    version: Option<i64>,
    diagnostics: Vec<Value>,
    truncated: bool,
    updated_at: Instant,
}

#[derive(Debug, Default)]
struct DiagnosticCache {
    by_uri: HashMap<String, DiagnosticSnapshot>,
}
''',
)

replace_once(
    coding,
    '''struct LspSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    server_name: String,
    root_uri: String,
    root_name: String,
    capabilities: ServerCapabilities,
    open_documents: HashMap<String, DocumentState>,
}
''',
    '''struct LspSession {
    child: Child,
    stdin: ChildStdin,
    inbound: mpsc::Receiver<Result<Value, String>>,
    reader_task: JoinHandle<()>,
    diagnostics: Arc<Mutex<DiagnosticCache>>,
    next_id: u64,
    server_name: String,
    root_uri: String,
    root_name: String,
    capabilities: ServerCapabilities,
    open_documents: HashMap<String, DocumentState>,
}
''',
)

replace_once(
    coding,
    '''pub async fn hover(
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
        NavigationKind::Hover,
        allow_language_server,
    )
    .await
}

#[derive(Debug, Clone, Copy)]
enum NavigationKind {
''',
    '''pub async fn hover(
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
        NavigationKind::Hover,
        allow_language_server,
    )
    .await
}

pub async fn diagnostics(
    root: &Path,
    relative_path: &str,
    allow_language_server: bool,
) -> Result<NavigationResult, AppError> {
    let root = canonical_root(root)?;
    let file = resolve_source_file(&root, relative_path)?;
    let display_path = relative_display(&root, &file);
    let text = read_source(&file)?;

    if !allow_language_server {
        return Ok(diagnostics_unavailable(
            display_path,
            "language-server diagnostics require Full PC + Terminal access",
        ));
    }
    let Some(spec) = select_server_for_file(&file) else {
        return Ok(diagnostics_unavailable(
            display_path,
            "no supported language server is configured for this file type",
        ));
    };
    let server_root = nearest_project_root(&root, &file, spec);
    let Ok(lease) = acquire_healthy_lsp_session(&root, &server_root, spec).await else {
        return Ok(diagnostics_unavailable(
            display_path,
            "a trusted language server is not available",
        ));
    };
    let uri = file_uri(&file)?;
    let sync = {
        let mut session = lease.session.lock().await;
        session
            .sync_document(&file, &text, spec.language_id)
            .await
            .map(|_| {
                let state = session.open_documents.get(&uri);
                (
                    Arc::clone(&session.diagnostics),
                    state.map(|state| state.version),
                    state.map(|state| state.synced_at),
                )
            })
    };
    let (cache, expected_version, synced_at) = match sync {
        Ok(state) => state,
        Err(_) => {
            invalidate_pooled_session(&lease).await;
            return Ok(diagnostics_unavailable(
                display_path,
                "language-server document synchronization failed",
            ));
        }
    };
    let snapshot = wait_for_diagnostics(
        &cache,
        &uri,
        expected_version,
        synced_at,
        Duration::from_millis(DIAGNOSTIC_WAIT_MILLIS),
    )
    .await;
    let server = Some(format!(
        "{}@{}",
        spec.command,
        relative_display(&root, &server_root)
    ));

    if let Some(snapshot) = snapshot {
        return Ok(NavigationResult {
            engine: "lsp".to_string(),
            server,
            result: json!({
                "path": display_path,
                "published": true,
                "version": snapshot.version,
                "truncated": snapshot.truncated,
                "diagnostics": snapshot.diagnostics,
            }),
        });
    }

    Ok(NavigationResult {
        engine: "lsp-pending".to_string(),
        server,
        result: json!({
            "path": display_path,
            "published": false,
            "diagnostics": [],
            "reason": "the language server has not published a current diagnostic snapshot yet",
        }),
    })
}

fn diagnostics_unavailable(path: String, reason: &str) -> NavigationResult {
    NavigationResult {
        engine: "lsp-unavailable".to_string(),
        server: None,
        result: json!({
            "path": path,
            "published": false,
            "diagnostics": [],
            "reason": reason,
        }),
    }
}

#[derive(Debug, Clone, Copy)]
enum NavigationKind {
''',
)

replace_once(
    coding,
    '''        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::internal("language server stdout unavailable"))?;
        let root_uri = directory_uri(server_root)?;
''',
    '''        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::internal("language server stdout unavailable"))?;
        let diagnostics = Arc::new(Mutex::new(DiagnosticCache::default()));
        let (inbound_tx, inbound) = mpsc::channel(MAX_LSP_CONTROL_MESSAGES);
        let reader_task = tokio::spawn(read_lsp_stream(
            BufReader::new(stdout),
            inbound_tx,
            Arc::clone(&diagnostics),
            workspace_root.to_path_buf(),
        ));
        let root_uri = directory_uri(server_root)?;
''',
)

replace_once(
    coding,
    '''        let mut session = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
''',
    '''        let mut session = Self {
            child,
            stdin,
            inbound,
            reader_task,
            diagnostics,
            next_id: 1,
''',
)

replace_once(
    coding,
    '''                            "hover": {"dynamicRegistration": false, "contentFormat": ["markdown", "plaintext"]},
                            "synchronization": {"dynamicRegistration": false, "didOpen": true}
''',
    '''                            "hover": {"dynamicRegistration": false, "contentFormat": ["markdown", "plaintext"]},
                            "publishDiagnostics": {
                                "relatedInformation": false,
                                "tagSupport": {"valueSet": [1, 2]},
                                "versionSupport": true,
                                "codeDescriptionSupport": false,
                                "dataSupport": false
                            },
                            "synchronization": {"dynamicRegistration": false, "didOpen": true}
''',
)

replace_once(
    coding,
    '''                DocumentState {
                    version,
                    fingerprint,
                    end_position,
                    last_used: now,
                },
''',
    '''                DocumentState {
                    version,
                    fingerprint,
                    end_position,
                    synced_at: now,
                    last_used: now,
                },
''',
)

replace_once(
    coding,
    '''            DocumentState {
                version: 1,
                fingerprint,
                end_position,
                last_used: now,
            },
''',
    '''            DocumentState {
                version: 1,
                fingerprint,
                end_position,
                synced_at: now,
                last_used: now,
            },
''',
)

replace_once(
    coding,
    '''    async fn read_response(&mut self, id: u64) -> Result<Value, AppError> {
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
''',
    '''    async fn read_response(&mut self, id: u64) -> Result<Value, AppError> {
        loop {
            let inbound = self.inbound.recv().await.ok_or_else(|| {
                AppError::internal("language server response channel closed unexpectedly")
            })?;
            let value = inbound.map_err(|error| {
                AppError::internal(format!("language server output failed: {error}"))
            })?;
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
''',
)

replace_once(
    coding,
    '''        let _ = self.request("shutdown", Value::Null).await;
        let _ = self.notify("exit", Value::Null).await;
        let _ = self.child.kill().await;
    }
}
''',
    '''        let _ = self.request("shutdown", Value::Null).await;
        let _ = self.notify("exit", Value::Null).await;
        self.reader_task.abort();
        let _ = self.child.kill().await;
    }
}
''',
)

replace_once(
    coding,
    '''async fn read_lsp_message(reader: &mut BufReader<ChildStdout>) -> Result<Value, AppError> {
''',
    '''async fn read_lsp_stream(
    mut reader: BufReader<ChildStdout>,
    inbound: mpsc::Sender<Result<Value, String>>,
    diagnostics: Arc<Mutex<DiagnosticCache>>,
    workspace_root: PathBuf,
) {
    loop {
        match read_lsp_message(&mut reader).await {
            Ok(value) => {
                let method = value.get("method").and_then(Value::as_str);
                let is_notification = method.is_some() && value.get("id").is_none();
                if method == Some("textDocument/publishDiagnostics") && is_notification {
                    if let Some((uri, snapshot)) =
                        sanitize_publish_diagnostics(&workspace_root, &value)
                    {
                        let mut cache = diagnostics.lock().await;
                        insert_diagnostic_snapshot(&mut cache, uri, snapshot);
                    }
                    continue;
                }
                if is_notification {
                    continue;
                }
                if inbound.send(Ok(value)).await.is_err() {
                    break;
                }
            }
            Err(error) => {
                let _ = inbound.send(Err(error.to_string())).await;
                break;
            }
        }
    }
}

fn sanitize_publish_diagnostics(
    root: &Path,
    message: &Value,
) -> Option<(String, DiagnosticSnapshot)> {
    let params = message.get("params")?;
    let uri = params.get("uri")?.as_str()?;
    if !uri_is_scoped(root, uri).ok()? {
        return None;
    }
    let raw = params.get("diagnostics")?.as_array()?;
    let diagnostics = raw
        .iter()
        .take(MAX_DIAGNOSTICS_PER_FILE)
        .filter_map(sanitize_diagnostic)
        .collect::<Vec<_>>();
    Some((
        uri.to_string(),
        DiagnosticSnapshot {
            version: params.get("version").and_then(Value::as_i64),
            diagnostics,
            truncated: raw.len() > MAX_DIAGNOSTICS_PER_FILE,
            updated_at: Instant::now(),
        },
    ))
}

fn sanitize_diagnostic(value: &Value) -> Option<Value> {
    let range = sanitize_diagnostic_range(value.get("range")?)?;
    let message = value.get("message")?.as_str()?;
    let mut output = serde_json::Map::new();
    output.insert("range".to_string(), range);
    output.insert(
        "message".to_string(),
        Value::String(truncate_preview(message, MAX_DIAGNOSTIC_MESSAGE_CHARS)),
    );
    if let Some(severity) = value
        .get("severity")
        .and_then(Value::as_u64)
        .filter(|severity| (1..=4).contains(severity))
    {
        output.insert("severity".to_string(), json!(severity));
    }
    if let Some(code) = value.get("code") {
        match code {
            Value::String(code) => {
                output.insert(
                    "code".to_string(),
                    Value::String(truncate_preview(code, 256)),
                );
            }
            Value::Number(_) => {
                output.insert("code".to_string(), code.clone());
            }
            _ => {}
        }
    }
    if let Some(source) = value.get("source").and_then(Value::as_str) {
        output.insert(
            "source".to_string(),
            Value::String(truncate_preview(source, 128)),
        );
    }
    if let Some(tags) = value.get("tags").and_then(Value::as_array) {
        let tags = tags
            .iter()
            .filter_map(Value::as_u64)
            .filter(|tag| matches!(tag, 1 | 2))
            .take(8)
            .map(Value::from)
            .collect::<Vec<_>>();
        if !tags.is_empty() {
            output.insert("tags".to_string(), Value::Array(tags));
        }
    }
    Some(Value::Object(output))
}

fn sanitize_diagnostic_range(value: &Value) -> Option<Value> {
    Some(json!({
        "start": sanitize_diagnostic_position(value.get("start")?)?,
        "end": sanitize_diagnostic_position(value.get("end")?)?,
    }))
}

fn sanitize_diagnostic_position(value: &Value) -> Option<Value> {
    Some(json!({
        "line": value.get("line")?.as_u64()?,
        "character": value.get("character")?.as_u64()?,
    }))
}

fn insert_diagnostic_snapshot(
    cache: &mut DiagnosticCache,
    uri: String,
    snapshot: DiagnosticSnapshot,
) {
    if !cache.by_uri.contains_key(&uri) && cache.by_uri.len() >= MAX_DIAGNOSTIC_FILES_PER_SESSION {
        if let Some(oldest) = cache
            .by_uri
            .iter()
            .min_by_key(|(_, snapshot)| snapshot.updated_at)
            .map(|(uri, _)| uri.clone())
        {
            cache.by_uri.remove(&oldest);
        }
    }
    cache.by_uri.insert(uri, snapshot);
}

async fn wait_for_diagnostics(
    cache: &Arc<Mutex<DiagnosticCache>>,
    uri: &str,
    expected_version: Option<i64>,
    synced_at: Option<Instant>,
    timeout: Duration,
) -> Option<DiagnosticSnapshot> {
    let deadline = Instant::now() + timeout;
    loop {
        {
            let cache = cache.lock().await;
            if let Some(snapshot) = cache.by_uri.get(uri) {
                if diagnostic_snapshot_matches(snapshot, expected_version, synced_at) {
                    return Some(snapshot.clone());
                }
            }
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn diagnostic_snapshot_matches(
    snapshot: &DiagnosticSnapshot,
    expected_version: Option<i64>,
    synced_at: Option<Instant>,
) -> bool {
    match (expected_version, snapshot.version) {
        (Some(expected), Some(actual)) => expected == actual,
        (Some(_), None) => synced_at.is_some_and(|synced_at| snapshot.updated_at >= synced_at),
        (None, _) => true,
    }
}

async fn read_lsp_message(reader: &mut BufReader<ChildStdout>) -> Result<Value, AppError> {
''',
)

replace_once(
    coding,
    '''                fingerprint: source_fingerprint("new"),
                end_position: source_end_position("new"),
                last_used: now,
''',
    '''                fingerprint: source_fingerprint("new"),
                end_position: source_end_position("new"),
                synced_at: now,
                last_used: now,
''',
)
replace_once(
    coding,
    '''                fingerprint: source_fingerprint("old"),
                end_position: source_end_position("old"),
                last_used: earlier,
''',
    '''                fingerprint: source_fingerprint("old"),
                end_position: source_end_position("old"),
                synced_at: earlier,
                last_used: earlier,
''',
)

replace_once(
    coding,
    '''    #[test]
    fn oldest_document_selection_is_lru_ordered() {
''',
    '''    #[test]
    fn published_diagnostics_are_scoped_sanitized_and_bounded() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let file = root.join("lib.rs");
        fs::write(&file, "fn main() {}\\n").unwrap();
        let uri = file_uri(&file).unwrap();
        let oversized = "x".repeat(MAX_DIAGNOSTIC_MESSAGE_CHARS + 32);
        let message = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": uri,
                "version": 7,
                "diagnostics": [{
                    "range": {
                        "start": {"line": 0, "character": 0},
                        "end": {"line": 0, "character": 2}
                    },
                    "severity": 1,
                    "code": "E0123",
                    "source": "rust-analyzer",
                    "message": oversized,
                    "data": {"secret": "drop-me"},
                    "relatedInformation": [{"message": "drop-me"}]
                }]
            }
        });
        let (_, snapshot) = sanitize_publish_diagnostics(&root, &message).unwrap();
        assert_eq!(snapshot.version, Some(7));
        assert_eq!(snapshot.diagnostics.len(), 1);
        let diagnostic = &snapshot.diagnostics[0];
        assert!(diagnostic.get("data").is_none());
        assert!(diagnostic.get("relatedInformation").is_none());
        assert!(
            diagnostic["message"].as_str().unwrap().chars().count()
                <= MAX_DIAGNOSTIC_MESSAGE_CHARS + 1
        );

        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("outside.rs");
        fs::write(&outside_file, "fn outside() {}\\n").unwrap();
        let outside_message = json!({
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": file_uri(&outside_file).unwrap(),
                "diagnostics": []
            }
        });
        assert!(sanitize_publish_diagnostics(&root, &outside_message).is_none());
    }

    #[test]
    fn diagnostic_cache_is_bounded_and_version_aware() {
        let mut cache = DiagnosticCache::default();
        let now = Instant::now();
        for index in 0..=MAX_DIAGNOSTIC_FILES_PER_SESSION {
            insert_diagnostic_snapshot(
                &mut cache,
                format!("file:///workspace/{index}.rs"),
                DiagnosticSnapshot {
                    version: Some(1),
                    diagnostics: Vec::new(),
                    truncated: false,
                    updated_at: now + Duration::from_millis(index as u64),
                },
            );
        }
        assert_eq!(cache.by_uri.len(), MAX_DIAGNOSTIC_FILES_PER_SESSION);
        assert!(!cache.by_uri.contains_key("file:///workspace/0.rs"));

        let snapshot = DiagnosticSnapshot {
            version: Some(3),
            diagnostics: Vec::new(),
            truncated: false,
            updated_at: now,
        };
        assert!(diagnostic_snapshot_matches(&snapshot, Some(3), Some(now)));
        assert!(!diagnostic_snapshot_matches(&snapshot, Some(4), Some(now)));
    }

    #[test]
    fn oldest_document_selection_is_lru_ordered() {
''',
)

local_agent = Path("src-tauri/src/local_agent.rs")
replace_once(
    local_agent,
    '''{{\\"type\\":\\"tool\\",\\"tool\\":\\"symbol_hover\\",\\"rootId\\":\\"ID\\",\\"path\\":\\"file\\",\\"line\\":1,\\"character\\":0}}\\n\\
{{\\"type\\":\\"tool\\",\\"tool\\":\\"write_file\\",\\"rootId\\":\\"ID\\",\\"path\\":\\"file\\",\\"content\\":\\"complete content\\"}}\\n\\
''',
    '''{{\\"type\\":\\"tool\\",\\"tool\\":\\"symbol_hover\\",\\"rootId\\":\\"ID\\",\\"path\\":\\"file\\",\\"line\\":1,\\"character\\":0}}\\n\\
{{\\"type\\":\\"tool\\",\\"tool\\":\\"symbol_diagnostics\\",\\"rootId\\":\\"ID\\",\\"path\\":\\"file\\"}}\\n\\
{{\\"type\\":\\"tool\\",\\"tool\\":\\"write_file\\",\\"rootId\\":\\"ID\\",\\"path\\":\\"file\\",\\"content\\":\\"complete content\\"}}\\n\\
''',
)

replace_once(
    local_agent,
    '- Prefer symbol_search/symbol_definition/symbol_references/symbol_hover for identifier navigation. A language server may run only when Full PC + Terminal access is enabled and its executable resolves from a trusted PATH location; compatible servers are reused through a bounded idle-evicted session pool with document synchronization, otherwise bounded lexical indexing is used.\\n\\\n',
    '- Prefer symbol_search/symbol_definition/symbol_references/symbol_hover for identifier navigation and symbol_diagnostics for file diagnostics. A language server may run only when Full PC + Terminal access is enabled and its executable resolves from a trusted PATH location; compatible servers are reused through a bounded idle-evicted session pool with document synchronization and bounded background notification draining, otherwise bounded lexical indexing is used. Treat diagnostics with published=false as non-authoritative.\\n\\\n',
)

replace_once(
    local_agent,
    '''        "symbol_definition" | "symbol_references" | "symbol_hover" => {
''',
    '''        "symbol_diagnostics" => {
            let root_id = optional_string(action, "rootId");
            let path = required_string(action, "path")?;
            let root = selected_root_path(config, root_id.as_deref())?;
            let diagnostics = coding_lsp::diagnostics(&root, &path, config.full_pc_access).await?;
            let result = serde_json::to_string(&diagnostics).map_err(|error| {
                AppError::internal(format!(
                    "failed to encode symbol_diagnostics result: {error}"
                ))
            })?;
            Ok(AgentTurnResult {
                trace_label: format!("Collected diagnostics for {path}"),
                transcript_result: bounded(&result, MAX_TOOL_RESULT_CHARS),
            })
        }
        "symbol_definition" | "symbol_references" | "symbol_hover" => {
''',
)

security = Path("src-tauri/src/openagent_security.rs")
replace_once(
    security,
    '''        "list_dir" | "read_file" | "search_text" | "symbol_search" | "symbol_definition"
        | "symbol_references" | "symbol_hover" | "git_status" | "git_diff" => RiskLevel::ReadOnly,
''',
    '''        "list_dir" | "read_file" | "search_text" | "symbol_search" | "symbol_definition"
        | "symbol_references" | "symbol_hover" | "symbol_diagnostics" | "git_status"
        | "git_diff" => RiskLevel::ReadOnly,
''',
)
replace_once(
    security,
    '''    fn read_only_tools_never_prompt() {
        let (decision, _) = authorize_tool("read_file", &json!({}), ApprovalMode::AlwaysAsk);
        assert_eq!(decision, PolicyDecision::Allow);
    }
''',
    '''    fn read_only_tools_never_prompt() {
        let (decision, _) = authorize_tool("read_file", &json!({}), ApprovalMode::AlwaysAsk);
        assert_eq!(decision, PolicyDecision::Allow);
        let (decision, _) =
            authorize_tool("symbol_diagnostics", &json!({}), ApprovalMode::AlwaysAsk);
        assert_eq!(decision, PolicyDecision::Allow);
    }
''',
)
