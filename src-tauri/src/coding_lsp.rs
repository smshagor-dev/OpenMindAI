use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{mpsc, Mutex},
    task::JoinHandle,
};
use url::Url;

use crate::app_error::AppError;

const LSP_TIMEOUT_SECS: u64 = 20;
const MAX_SOURCE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_FALLBACK_FILES: usize = 1_200;
const MAX_SYMBOL_RESULTS: usize = 100;
const MAX_DOCUMENT_SYMBOL_RESULTS: usize = 200;
const MAX_DOCUMENT_SYMBOL_DEPTH: usize = 16;
const MAX_SYMBOL_NAME_CHARS: usize = 512;
const MAX_SYMBOL_DETAIL_CHARS: usize = 2_000;
const MAX_REFERENCE_RESULTS: usize = 200;
const MAX_HOVER_CHARS: usize = 12_000;
const MAX_LSP_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_LSP_HEADER_BYTES: usize = 16 * 1024;
const MAX_PROJECT_ROOTS: usize = 32;
const MAX_PROJECT_SCAN_DIRS: usize = 250;
const MAX_PROJECT_SCAN_DEPTH: usize = 5;
const MAX_WORKSPACE_LSP_SESSIONS: usize = 8;
const MAX_POOLED_LSP_SESSIONS: usize = 8;
const MAX_OPEN_DOCUMENTS_PER_SESSION: usize = 128;
const LSP_SESSION_IDLE_SECS: u64 = 300;
const MAX_LSP_CONTROL_MESSAGES: usize = 64;
const MAX_DIAGNOSTIC_FILES_PER_SESSION: usize = 128;
const MAX_DIAGNOSTICS_PER_FILE: usize = 200;
const MAX_DIAGNOSTIC_MESSAGE_CHARS: usize = 4_000;
const DIAGNOSTIC_WAIT_MILLIS: u64 = 600;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum TextDocumentSyncKind {
    #[default]
    None,
    Full,
    Incremental,
}

#[derive(Debug, Clone, Copy)]
struct LspPosition {
    line: u64,
    character: u64,
}

#[derive(Debug, Clone)]
struct DocumentState {
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

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct SessionKey {
    workspace_root: PathBuf,
    server_root: PathBuf,
    command: &'static str,
}

struct PooledSession {
    session: Arc<Mutex<LspSession>>,
    last_used: Instant,
}

#[derive(Default)]
struct LspSessionPool {
    entries: HashMap<SessionKey, PooledSession>,
}

#[derive(Clone)]
struct SessionLease {
    key: SessionKey,
    session: Arc<Mutex<LspSession>>,
}

static LSP_SESSION_POOL: OnceLock<Mutex<LspSessionPool>> = OnceLock::new();

#[derive(Debug, Clone, Copy, Default)]
struct ServerCapabilities {
    workspace_symbols: bool,
    document_symbols: bool,
    definition: bool,
    references: bool,
    hover: bool,
    sync_open_close: bool,
    sync_kind: TextDocumentSyncKind,
}

struct LspSession {
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

fn lsp_session_pool() -> &'static Mutex<LspSessionPool> {
    LSP_SESSION_POOL.get_or_init(|| Mutex::new(LspSessionPool::default()))
}

fn session_key(workspace_root: &Path, server_root: &Path, spec: ServerSpec) -> SessionKey {
    SessionKey {
        workspace_root: workspace_root.to_path_buf(),
        server_root: server_root.to_path_buf(),
        command: spec.command,
    }
}

async fn close_session_handles(handles: Vec<Arc<Mutex<LspSession>>>) {
    for handle in handles {
        let mut session = handle.lock().await;
        session.close().await;
    }
}

async fn acquire_lsp_session(
    workspace_root: &Path,
    server_root: &Path,
    spec: ServerSpec,
) -> Result<SessionLease, AppError> {
    let key = session_key(workspace_root, server_root, spec);
    let now = Instant::now();
    let mut evicted = Vec::new();
    let mut at_capacity = false;
    let existing = {
        let mut pool = lsp_session_pool().lock().await;
        let idle_keys = pool
            .entries
            .iter()
            .filter(|(_, entry)| {
                Arc::strong_count(&entry.session) == 1
                    && now.duration_since(entry.last_used)
                        >= Duration::from_secs(LSP_SESSION_IDLE_SECS)
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for idle_key in idle_keys {
            if let Some(entry) = pool.entries.remove(&idle_key) {
                evicted.push(entry.session);
            }
        }

        if let Some(entry) = pool.entries.get_mut(&key) {
            entry.last_used = now;
            Some(SessionLease {
                key: key.clone(),
                session: Arc::clone(&entry.session),
            })
        } else {
            if pool.entries.len() >= MAX_POOLED_LSP_SESSIONS {
                let oldest = pool
                    .entries
                    .iter()
                    .filter(|(_, entry)| Arc::strong_count(&entry.session) == 1)
                    .min_by_key(|(_, entry)| entry.last_used)
                    .map(|(key, _)| key.clone());
                if let Some(oldest) = oldest {
                    if let Some(entry) = pool.entries.remove(&oldest) {
                        evicted.push(entry.session);
                    }
                } else {
                    at_capacity = true;
                }
            }
            None
        }
    };

    close_session_handles(evicted).await;
    if let Some(existing) = existing {
        return Ok(existing);
    }
    if at_capacity {
        return Err(AppError::internal(
            "language-server session pool is at capacity with active sessions",
        ));
    }

    let started = Arc::new(Mutex::new(
        LspSession::start(workspace_root, server_root, spec).await?,
    ));
    let mut duplicate_session = None;
    let mut rejected = false;
    {
        let mut pool = lsp_session_pool().lock().await;
        if let Some(entry) = pool.entries.get_mut(&key) {
            entry.last_used = Instant::now();
            duplicate_session = Some(Arc::clone(&entry.session));
        } else if pool.entries.len() < MAX_POOLED_LSP_SESSIONS {
            pool.entries.insert(
                key.clone(),
                PooledSession {
                    session: Arc::clone(&started),
                    last_used: Instant::now(),
                },
            );
        } else {
            rejected = true;
        }
    }

    if let Some(session) = duplicate_session {
        close_session_handles(vec![started]).await;
        return Ok(SessionLease { key, session });
    }
    if rejected {
        close_session_handles(vec![started]).await;
        return Err(AppError::internal(
            "language-server session pool filled while starting a server",
        ));
    }

    Ok(SessionLease {
        key,
        session: started,
    })
}

async fn invalidate_pooled_session(lease: &SessionLease) {
    let removed = {
        let mut pool = lsp_session_pool().lock().await;
        let matches = pool
            .entries
            .get(&lease.key)
            .is_some_and(|entry| Arc::ptr_eq(&entry.session, &lease.session));
        if matches {
            pool.entries.remove(&lease.key).map(|entry| entry.session)
        } else {
            None
        }
    };
    if let Some(session) = removed {
        close_session_handles(vec![session]).await;
    }
}

async fn acquire_healthy_lsp_session(
    workspace_root: &Path,
    server_root: &Path,
    spec: ServerSpec,
) -> Result<SessionLease, AppError> {
    for _ in 0..2 {
        let lease = acquire_lsp_session(workspace_root, server_root, spec).await?;
        let alive = {
            let mut session = lease.session.lock().await;
            session.is_alive()
        };
        if alive {
            return Ok(lease);
        }
        invalidate_pooled_session(&lease).await;
    }
    Err(AppError::internal(format!(
        "{} could not be restarted after an unexpected exit",
        spec.command
    )))
}

pub async fn shutdown_pooled_sessions() {
    let sessions = {
        let mut pool = lsp_session_pool().lock().await;
        pool.entries
            .drain()
            .map(|(_, entry)| entry.session)
            .collect::<Vec<_>>()
    };
    close_session_handles(sessions).await;
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
            let Ok(lease) =
                acquire_healthy_lsp_session(&root, &candidate.root, candidate.spec).await
            else {
                continue;
            };
            let request = {
                let mut session = lease.session.lock().await;
                if !session.capabilities.workspace_symbols {
                    None
                } else {
                    Some(
                        session
                            .request("workspace/symbol", json!({"query": query}))
                            .await,
                    )
                }
            };
            let Some(request) = request else {
                continue;
            };
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
                Err(_) => {
                    invalidate_pooled_session(&lease).await;
                }
            }
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

pub async fn document_symbols(
    root: &Path,
    relative_path: &str,
    allow_language_server: bool,
) -> Result<NavigationResult, AppError> {
    let root = canonical_root(root)?;
    let file = resolve_source_file(&root, relative_path)?;
    let text = read_source(&file)?;

    if let (true, Some(spec)) = (allow_language_server, select_server_for_file(&file)) {
        let server_root = nearest_project_root(&root, &file, spec);
        if let Ok(lease) = acquire_healthy_lsp_session(&root, &server_root, spec).await {
            let request = {
                let mut session = lease.session.lock().await;
                if !session.capabilities.document_symbols {
                    None
                } else if let Err(error) =
                    session.sync_document(&file, &text, spec.language_id).await
                {
                    Some(Err(error))
                } else {
                    let uri = file_uri(&file)?;
                    Some(
                        session
                            .request(
                                "textDocument/documentSymbol",
                                json!({"textDocument": {"uri": uri}}),
                            )
                            .await,
                    )
                }
            };

            if let Some(request) = request {
                match request {
                    Ok(result) => {
                        let result = sanitize_document_symbol_result(&root, &file, result)?;
                        let server = Some(format!(
                            "{}@{}",
                            spec.command,
                            relative_display(&root, &server_root)
                        ));
                        if result_has_items(&result) {
                            return Ok(NavigationResult {
                                engine: "lsp".to_string(),
                                server,
                                result,
                            });
                        }
                        return Ok(NavigationResult {
                            engine: "lsp+lexical-fallback".to_string(),
                            server,
                            result: fallback_document_symbols(&root, &file, &text),
                        });
                    }
                    Err(_) => {
                        invalidate_pooled_session(&lease).await;
                    }
                }
            }
        }
    }

    Ok(NavigationResult {
        engine: "lexical-fallback".to_string(),
        server: None,
        result: fallback_document_symbols(&root, &file, &text),
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

pub async fn hover(
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
    Definition,
    References,
    Hover,
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
        if let Ok(lease) = acquire_healthy_lsp_session(&root, &server_root, spec).await {
            let request = {
                let mut session = lease.session.lock().await;
                if !session.supports_navigation(kind) {
                    None
                } else if let Err(error) =
                    session.sync_document(&file, &text, spec.language_id).await
                {
                    Some(Err(error))
                } else {
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
                        NavigationKind::Hover => json!({
                            "textDocument": {"uri": uri},
                            "position": {"line": line - 1, "character": lsp_character}
                        }),
                    };
                    let method = match kind {
                        NavigationKind::Definition => "textDocument/definition",
                        NavigationKind::References => "textDocument/references",
                        NavigationKind::Hover => "textDocument/hover",
                    };
                    Some(session.request(method, params).await)
                }
            };

            if let Some(request) = request {
                match request {
                    Ok(result) => {
                        let limit = match kind {
                            NavigationKind::Definition => MAX_SYMBOL_RESULTS,
                            NavigationKind::References => MAX_REFERENCE_RESULTS,
                            NavigationKind::Hover => 1,
                        };
                        let result = match kind {
                            NavigationKind::Hover => sanitize_hover_result(result)?,
                            NavigationKind::Definition | NavigationKind::References => {
                                sanitize_lsp_result(&root, result, limit)?
                            }
                        };
                        if result_has_items(&result) {
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
                    Err(_) => {
                        invalidate_pooled_session(&lease).await;
                    }
                }
            }
        }
    }

    let symbol = symbol_at_position(&text, line_index, character_index)?;
    let result = match kind {
        NavigationKind::Definition => fallback_definition(&root, &symbol)?,
        NavigationKind::References => fallback_references(&root, &symbol)?,
        NavigationKind::Hover => fallback_hover(&root, &symbol)?,
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
        let diagnostics = Arc::new(Mutex::new(DiagnosticCache::default()));
        let (inbound_tx, inbound) = mpsc::channel(MAX_LSP_CONTROL_MESSAGES);
        let reader_task = tokio::spawn(read_lsp_stream(
            BufReader::new(stdout),
            inbound_tx,
            Arc::clone(&diagnostics),
            workspace_root.to_path_buf(),
        ));
        let root_uri = directory_uri(server_root)?;
        let root_name = server_root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("workspace")
            .to_string();
        let mut session = Self {
            child,
            stdin,
            inbound,
            reader_task,
            diagnostics,
            next_id: 1,
            server_name: spec.command.to_string(),
            root_uri: root_uri.clone(),
            root_name: root_name.clone(),
            capabilities: ServerCapabilities::default(),
            open_documents: HashMap::new(),
        };
        let initialize_result = session
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
                            "hover": {"dynamicRegistration": false, "contentFormat": ["markdown", "plaintext"]},
                            "documentSymbol": {
                                "dynamicRegistration": false,
                                "hierarchicalDocumentSymbolSupport": true,
                                "tagSupport": {"valueSet": [1]}
                            },
                            "publishDiagnostics": {
                                "relatedInformation": false,
                                "tagSupport": {"valueSet": [1, 2]},
                                "versionSupport": true,
                                "codeDescriptionSupport": false,
                                "dataSupport": false
                            },
                            "synchronization": {"dynamicRegistration": false, "didOpen": true}
                        }
                    }
                }),
            )
            .await?;
        session.capabilities = parse_server_capabilities(&initialize_result);
        session.notify("initialized", json!({})).await?;
        Ok(session)
    }

    async fn sync_document(
        &mut self,
        file: &Path,
        text: &str,
        language_id: &str,
    ) -> Result<(), AppError> {
        if !self.capabilities.sync_open_close {
            return Ok(());
        }

        let uri = file_uri(file)?;
        let fingerprint = source_fingerprint(text);
        let end_position = source_end_position(text);
        let now = Instant::now();
        let existing = self.open_documents.get(&uri).cloned();

        if let Some(existing) = existing {
            if existing.fingerprint == fingerprint {
                if let Some(state) = self.open_documents.get_mut(&uri) {
                    state.last_used = now;
                }
                return Ok(());
            }

            let version = existing.version.saturating_add(1);
            match self.capabilities.sync_kind {
                TextDocumentSyncKind::Full => {
                    self.notify(
                        "textDocument/didChange",
                        json!({
                            "textDocument": {"uri": uri.clone(), "version": version},
                            "contentChanges": [{"text": text}]
                        }),
                    )
                    .await?;
                }
                TextDocumentSyncKind::Incremental => {
                    self.notify(
                        "textDocument/didChange",
                        json!({
                            "textDocument": {"uri": uri.clone(), "version": version},
                            "contentChanges": [{
                                "range": {
                                    "start": {"line": 0, "character": 0},
                                    "end": {
                                        "line": existing.end_position.line,
                                        "character": existing.end_position.character
                                    }
                                },
                                "text": text
                            }]
                        }),
                    )
                    .await?;
                }
                TextDocumentSyncKind::None => {
                    self.notify(
                        "textDocument/didClose",
                        json!({"textDocument": {"uri": uri.clone()}}),
                    )
                    .await?;
                    self.notify(
                        "textDocument/didOpen",
                        json!({
                            "textDocument": {
                                "uri": uri.clone(),
                                "languageId": language_id,
                                "version": version,
                                "text": text
                            }
                        }),
                    )
                    .await?;
                }
            }

            self.open_documents.insert(
                uri,
                DocumentState {
                    version,
                    fingerprint,
                    end_position,
                    synced_at: now,
                    last_used: now,
                },
            );
            return Ok(());
        }

        if self.open_documents.len() >= MAX_OPEN_DOCUMENTS_PER_SESSION {
            if let Some(oldest_uri) = oldest_document_uri(&self.open_documents) {
                self.notify(
                    "textDocument/didClose",
                    json!({"textDocument": {"uri": oldest_uri.clone()}}),
                )
                .await?;
                self.open_documents.remove(&oldest_uri);
            }
        }

        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri.clone(),
                    "languageId": language_id,
                    "version": 1,
                    "text": text
                }
            }),
        )
        .await?;
        self.open_documents.insert(
            uri,
            DocumentState {
                version: 1,
                fingerprint,
                end_position,
                synced_at: now,
                last_used: now,
            },
        );
        Ok(())
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

    fn supports_navigation(&self, kind: NavigationKind) -> bool {
        match kind {
            NavigationKind::Definition => self.capabilities.definition,
            NavigationKind::References => self.capabilities.references,
            NavigationKind::Hover => self.capabilities.hover,
        }
    }

    fn is_alive(&mut self) -> bool {
        self.child.try_wait().is_ok_and(|status| status.is_none())
    }

    async fn close(&mut self) {
        let open_uris = self.open_documents.keys().cloned().collect::<Vec<_>>();
        for uri in open_uris {
            let _ = self
                .notify(
                    "textDocument/didClose",
                    json!({"textDocument": {"uri": uri}}),
                )
                .await;
        }
        self.open_documents.clear();
        let _ = self.request("shutdown", Value::Null).await;
        let _ = self.notify("exit", Value::Null).await;
        self.reader_task.abort();
        let _ = self.child.kill().await;
    }
}

fn parse_server_capabilities(initialize_result: &Value) -> ServerCapabilities {
    let capabilities = initialize_result
        .get("capabilities")
        .unwrap_or(&Value::Null);
    let (sync_open_close, sync_kind) = parse_text_document_sync(capabilities);
    ServerCapabilities {
        workspace_symbols: capability_enabled(capabilities, "workspaceSymbolProvider"),
        document_symbols: capability_enabled(capabilities, "documentSymbolProvider"),
        definition: capability_enabled(capabilities, "definitionProvider"),
        references: capability_enabled(capabilities, "referencesProvider"),
        hover: capability_enabled(capabilities, "hoverProvider"),
        sync_open_close,
        sync_kind,
    }
}

fn parse_text_document_sync(capabilities: &Value) -> (bool, TextDocumentSyncKind) {
    let Some(sync) = capabilities.get("textDocumentSync") else {
        return (false, TextDocumentSyncKind::None);
    };
    match sync {
        Value::Number(value) => {
            let kind = sync_kind_from_value(value.as_u64().unwrap_or(0));
            (kind != TextDocumentSyncKind::None, kind)
        }
        Value::Object(options) => {
            let open_close = options
                .get("openClose")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let kind =
                sync_kind_from_value(options.get("change").and_then(Value::as_u64).unwrap_or(0));
            (open_close, kind)
        }
        _ => (false, TextDocumentSyncKind::None),
    }
}

fn sync_kind_from_value(value: u64) -> TextDocumentSyncKind {
    match value {
        1 => TextDocumentSyncKind::Full,
        2 => TextDocumentSyncKind::Incremental,
        _ => TextDocumentSyncKind::None,
    }
}

fn capability_enabled(capabilities: &Value, name: &str) -> bool {
    match capabilities.get(name) {
        Some(Value::Bool(value)) => *value,
        Some(Value::Object(_)) => true,
        _ => false,
    }
}

async fn read_lsp_stream(
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

fn sanitize_document_symbol_result(
    root: &Path,
    file: &Path,
    result: Value,
) -> Result<Value, AppError> {
    let values = match result {
        Value::Array(values) => values,
        Value::Null => Vec::new(),
        value => vec![value],
    };
    let mut remaining = MAX_DOCUMENT_SYMBOL_RESULTS;
    let mut output = Vec::new();
    for value in values {
        if remaining == 0 {
            break;
        }
        if let Some(symbol) = sanitize_document_symbol_item(root, file, &value, 0, &mut remaining)?
        {
            output.push(symbol);
        }
    }
    Ok(Value::Array(output))
}

fn sanitize_document_symbol_item(
    root: &Path,
    file: &Path,
    value: &Value,
    depth: usize,
    remaining: &mut usize,
) -> Result<Option<Value>, AppError> {
    if depth >= MAX_DOCUMENT_SYMBOL_DEPTH || *remaining == 0 {
        return Ok(None);
    }
    if value.get("location").is_some() {
        return sanitize_symbol_information(root, file, value, remaining);
    }

    let Some(name) = value.get("name").and_then(Value::as_str) else {
        return Ok(None);
    };
    let Some(kind) = value
        .get("kind")
        .and_then(Value::as_u64)
        .filter(|kind| (1..=26).contains(kind))
    else {
        return Ok(None);
    };
    let Some(range) = value.get("range").and_then(sanitize_diagnostic_range) else {
        return Ok(None);
    };
    let Some(selection_range) = value
        .get("selectionRange")
        .and_then(sanitize_diagnostic_range)
    else {
        return Ok(None);
    };

    *remaining = remaining.saturating_sub(1);
    let mut safe = serde_json::Map::new();
    safe.insert(
        "name".to_string(),
        Value::String(truncate_preview(name, MAX_SYMBOL_NAME_CHARS)),
    );
    safe.insert("kind".to_string(), json!(kind));
    safe.insert("range".to_string(), range);
    safe.insert("selectionRange".to_string(), selection_range);

    if let Some(detail) = value.get("detail").and_then(Value::as_str) {
        safe.insert(
            "detail".to_string(),
            Value::String(truncate_preview(detail, MAX_SYMBOL_DETAIL_CHARS)),
        );
    }
    if let Some(tags) = sanitize_symbol_tags(value.get("tags")) {
        safe.insert("tags".to_string(), tags);
    }

    if let Some(children) = value.get("children").and_then(Value::as_array) {
        let mut safe_children = Vec::new();
        for child in children {
            if *remaining == 0 {
                break;
            }
            if let Some(child) =
                sanitize_document_symbol_item(root, file, child, depth + 1, remaining)?
            {
                safe_children.push(child);
            }
        }
        if !safe_children.is_empty() {
            safe.insert("children".to_string(), Value::Array(safe_children));
        }
    }

    Ok(Some(Value::Object(safe)))
}

fn sanitize_symbol_information(
    root: &Path,
    file: &Path,
    value: &Value,
    remaining: &mut usize,
) -> Result<Option<Value>, AppError> {
    if *remaining == 0 {
        return Ok(None);
    }
    let Some(name) = value.get("name").and_then(Value::as_str) else {
        return Ok(None);
    };
    let Some(kind) = value
        .get("kind")
        .and_then(Value::as_u64)
        .filter(|kind| (1..=26).contains(kind))
    else {
        return Ok(None);
    };
    let Some(location) = value.get("location") else {
        return Ok(None);
    };
    let Some(uri) = location.get("uri").and_then(Value::as_str) else {
        return Ok(None);
    };
    if !uri_is_scoped(root, uri)? || !uri_matches_file(uri, file) {
        return Ok(None);
    }
    let Some(range) = location.get("range").and_then(sanitize_diagnostic_range) else {
        return Ok(None);
    };

    *remaining = remaining.saturating_sub(1);
    let mut safe = serde_json::Map::new();
    safe.insert(
        "name".to_string(),
        Value::String(truncate_preview(name, MAX_SYMBOL_NAME_CHARS)),
    );
    safe.insert("kind".to_string(), json!(kind));
    safe.insert(
        "location".to_string(),
        json!({
            "uri": uri,
            "range": range,
        }),
    );
    if let Some(container_name) = value.get("containerName").and_then(Value::as_str) {
        safe.insert(
            "containerName".to_string(),
            Value::String(truncate_preview(container_name, MAX_SYMBOL_NAME_CHARS)),
        );
    }
    if let Some(tags) = sanitize_symbol_tags(value.get("tags")) {
        safe.insert("tags".to_string(), tags);
    }
    Ok(Some(Value::Object(safe)))
}

fn sanitize_symbol_tags(value: Option<&Value>) -> Option<Value> {
    let tags = value?
        .as_array()?
        .iter()
        .filter_map(Value::as_u64)
        .filter(|tag| *tag == 1)
        .take(8)
        .map(Value::from)
        .collect::<Vec<_>>();
    (!tags.is_empty()).then_some(Value::Array(tags))
}

fn uri_matches_file(raw: &str, file: &Path) -> bool {
    let Ok(url) = Url::parse(raw) else {
        return false;
    };
    if url.scheme() != "file" {
        return false;
    }
    let Ok(path) = url.to_file_path() else {
        return false;
    };
    fs::canonicalize(path).is_ok_and(|path| path == file)
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
    let root = fs::canonicalize(root)?;
    match result {
        Value::Null => Ok(Value::Null),
        Value::Array(items) => {
            let mut safe = Vec::new();
            for item in items {
                if safe.len() >= limit {
                    break;
                }
                if lsp_item_is_scoped(&root, &item)? {
                    safe.push(item);
                }
            }
            Ok(Value::Array(safe))
        }
        Value::Object(_) => {
            if lsp_item_is_scoped(&root, &result)? {
                Ok(result)
            } else {
                Ok(Value::Null)
            }
        }
        _ => Ok(Value::Null),
    }
}

fn sanitize_hover_result(result: Value) -> Result<Value, AppError> {
    let Value::Object(mut object) = result else {
        return Ok(Value::Null);
    };
    let Some(contents) = object.remove("contents") else {
        return Ok(Value::Null);
    };
    let mut remaining = MAX_HOVER_CHARS;
    let contents = sanitize_hover_contents(contents, &mut remaining);
    if !hover_contents_has_data(&contents) {
        return Ok(Value::Null);
    }

    let mut safe = serde_json::Map::new();
    safe.insert("contents".to_string(), contents);
    if let Some(range) = object.remove("range").filter(Value::is_object) {
        safe.insert("range".to_string(), range);
    }
    Ok(Value::Object(safe))
}

fn sanitize_hover_contents(value: Value, remaining: &mut usize) -> Value {
    if *remaining == 0 {
        return Value::Null;
    }
    match value {
        Value::String(text) => {
            let bounded = text.chars().take(*remaining).collect::<String>();
            *remaining = (*remaining).saturating_sub(bounded.chars().count());
            Value::String(bounded)
        }
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .take(32)
                .map(|value| sanitize_hover_contents(value, remaining))
                .filter(hover_contents_has_data)
                .collect(),
        ),
        Value::Object(mut object) => {
            let mut safe = serde_json::Map::new();
            for key in ["kind", "language", "value"] {
                if let Some(value) = object.remove(key) {
                    let value = sanitize_hover_contents(value, remaining);
                    if hover_contents_has_data(&value) {
                        safe.insert(key.to_string(), value);
                    }
                }
            }
            Value::Object(safe)
        }
        _ => Value::Null,
    }
}

fn hover_contents_has_data(value: &Value) -> bool {
    match value {
        Value::String(value) => !value.is_empty(),
        Value::Array(values) => values.iter().any(hover_contents_has_data),
        Value::Object(values) => values.values().any(hover_contents_has_data),
        _ => false,
    }
}

fn source_fingerprint(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

fn source_end_position(text: &str) -> LspPosition {
    let line = text.bytes().filter(|byte| *byte == b'\n').count() as u64;
    let tail = text.rsplit_once('\n').map(|(_, tail)| tail).unwrap_or(text);
    let character = tail.encode_utf16().count() as u64;
    LspPosition { line, character }
}

fn oldest_document_uri(documents: &HashMap<String, DocumentState>) -> Option<String> {
    documents
        .iter()
        .min_by_key(|(_, state)| state.last_used)
        .map(|(uri, _)| uri.clone())
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

fn fallback_document_symbols(root: &Path, file: &Path, text: &str) -> Value {
    let mut results = Vec::new();
    for (line_index, line) in text.lines().enumerate() {
        if let Some((kind, symbol)) = declaration_symbol(file, line) {
            results.push(json!({
                "name": symbol,
                "kind": kind,
                "path": relative_display(root, file),
                "line": line_index + 1,
                "character": line.chars().take_while(|character| character.is_whitespace()).count(),
            }));
            if results.len() >= MAX_DOCUMENT_SYMBOL_RESULTS {
                break;
            }
        }
    }
    Value::Array(results)
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

fn fallback_hover(root: &Path, symbol: &str) -> Result<Value, AppError> {
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
                        "line": line_index + 1,
                        "preview": truncate_preview(line.trim(), 320)
                    }));
                    if results.len() >= 10 {
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
    let mut value = line.trim_start_matches('&');
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
        .rfind(|part| !part.is_empty())?;
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
        if before.is_none_or(|value| !is_identifier_char(value))
            && after.is_none_or(|value| !is_identifier_char(value))
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
    fn fallback_document_outline_is_bounded_to_one_file() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let file = root.join("lib.rs");
        let text = "pub struct Router {}\nfn build() {}\n";
        fs::write(&file, text).unwrap();
        let outline = fallback_document_symbols(&root, &file, text);
        let items = outline.as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].get("name").and_then(Value::as_str), Some("Router"));
        assert_eq!(items[1].get("name").and_then(Value::as_str), Some("build"));
        assert!(items
            .iter()
            .all(|item| item.get("path").and_then(Value::as_str) == Some("lib.rs")));
    }

    #[test]
    fn document_symbol_sanitization_preserves_safe_hierarchy_and_scope() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let file = root.join("lib.rs");
        fs::write(&file, "pub struct Router {}\n").unwrap();
        let uri = file_uri(&file).unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("outside.rs");
        fs::write(&outside_file, "fn outside() {}\n").unwrap();

        let result = json!([
            {
                "name": "Router",
                "detail": "safe detail",
                "kind": 23,
                "range": {
                    "start": {"line": 0, "character": 0},
                    "end": {"line": 0, "character": 20}
                },
                "selectionRange": {
                    "start": {"line": 0, "character": 11},
                    "end": {"line": 0, "character": 17}
                },
                "data": {"secret": "drop-me"},
                "children": [{
                    "name": "new",
                    "kind": 6,
                    "range": {
                        "start": {"line": 1, "character": 0},
                        "end": {"line": 1, "character": 10}
                    },
                    "selectionRange": {
                        "start": {"line": 1, "character": 3},
                        "end": {"line": 1, "character": 6}
                    },
                    "unsafe": "drop-me"
                }]
            },
            {
                "name": "build",
                "kind": 12,
                "location": {
                    "uri": uri,
                    "range": {
                        "start": {"line": 2, "character": 0},
                        "end": {"line": 2, "character": 8}
                    }
                },
                "containerName": "Router",
                "data": {"secret": "drop-me"}
            },
            {
                "name": "outside",
                "kind": 12,
                "location": {
                    "uri": file_uri(&outside_file).unwrap(),
                    "range": {
                        "start": {"line": 0, "character": 0},
                        "end": {"line": 0, "character": 7}
                    }
                }
            }
        ]);

        let safe = sanitize_document_symbol_result(&root, &file, result).unwrap();
        let items = safe.as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert!(items[0].get("data").is_none());
        assert!(items[0].pointer("/children/0/unsafe").is_none());
        assert_eq!(
            items[1].pointer("/location/uri").and_then(Value::as_str),
            Some(uri.as_str())
        );
        assert!(items[1].get("data").is_none());
    }

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
    fn server_capabilities_are_parsed_from_initialize_result() {
        let parsed = parse_server_capabilities(&json!({
            "capabilities": {
                "workspaceSymbolProvider": true,
                "documentSymbolProvider": {"label": "outline"},
                "definitionProvider": {"workDoneProgress": true},
                "referencesProvider": false,
                "hoverProvider": {},
                "textDocumentSync": {"openClose": true, "change": 2}
            }
        }));
        assert!(parsed.workspace_symbols);
        assert!(parsed.document_symbols);
        assert!(parsed.definition);
        assert!(!parsed.references);
        assert!(parsed.hover);
        assert!(parsed.sync_open_close);
        assert_eq!(parsed.sync_kind, TextDocumentSyncKind::Incremental);
    }

    #[test]
    fn legacy_text_document_sync_kind_is_supported() {
        let capabilities = json!({"textDocumentSync": 1});
        assert_eq!(
            parse_text_document_sync(&capabilities),
            (true, TextDocumentSyncKind::Full)
        );
    }

    #[test]
    fn source_end_position_uses_utf16_columns() {
        let position = source_end_position("first\n🙂x");
        assert_eq!(position.line, 1);
        assert_eq!(position.character, 3);
        let trailing = source_end_position("first\n");
        assert_eq!(trailing.line, 1);
        assert_eq!(trailing.character, 0);
    }

    #[test]
    fn source_fingerprint_changes_with_document_content() {
        assert_ne!(source_fingerprint("before"), source_fingerprint("after"));
        assert_eq!(source_fingerprint("same"), source_fingerprint("same"));
    }

    #[test]
    fn published_diagnostics_are_scoped_sanitized_and_bounded() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let file = root.join("lib.rs");
        fs::write(&file, "fn main() {}\n").unwrap();
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
        fs::write(&outside_file, "fn outside() {}\n").unwrap();
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
        let now = Instant::now();
        let earlier = now.checked_sub(Duration::from_secs(5)).unwrap();
        let mut documents = HashMap::new();
        documents.insert(
            "file:///new.rs".to_string(),
            DocumentState {
                version: 1,
                fingerprint: source_fingerprint("new"),
                end_position: source_end_position("new"),
                synced_at: now,
                last_used: now,
            },
        );
        documents.insert(
            "file:///old.rs".to_string(),
            DocumentState {
                version: 1,
                fingerprint: source_fingerprint("old"),
                end_position: source_end_position("old"),
                synced_at: earlier,
                last_used: earlier,
            },
        );
        assert_eq!(
            oldest_document_uri(&documents).as_deref(),
            Some("file:///old.rs")
        );
    }

    #[test]
    fn hover_sanitization_bounds_untrusted_markup() {
        let long = "x".repeat(MAX_HOVER_CHARS + 50);
        let safe = sanitize_hover_result(json!({
            "contents": {"kind": "markdown", "value": long},
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
            "unsafe": "discard me"
        }))
        .unwrap();
        let value = safe
            .pointer("/contents/value")
            .and_then(Value::as_str)
            .unwrap();
        assert!(value.chars().count() <= MAX_HOVER_CHARS);
        assert!(safe.get("unsafe").is_none());
    }

    #[test]
    fn fallback_hover_reports_declaration_metadata() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("lib.rs"), "pub struct Router {}\n").unwrap();
        let hover = fallback_hover(temp.path(), "Router").unwrap();
        let item = hover.as_array().unwrap().first().unwrap();
        assert_eq!(item.get("kind").and_then(Value::as_str), Some("struct"));
        assert_eq!(item.get("path").and_then(Value::as_str), Some("lib.rs"));
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
