from __future__ import annotations

from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{path}: expected one replacement, found {count}: {old[:140]!r}")
    file.write_text(text.replace(old, new, 1), encoding="utf-8")


def patch_lsp() -> None:
    path = "src-tauri/src/coding_lsp.rs"

    replace_once(
        path,
        '''use std::{
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
};''',
        '''use std::{
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
    sync::Mutex,
};''',
    )

    replace_once(
        path,
        '''const MAX_PROJECT_SCAN_DEPTH: usize = 5;
const MAX_WORKSPACE_LSP_SESSIONS: usize = 8;''',
        '''const MAX_PROJECT_SCAN_DEPTH: usize = 5;
const MAX_WORKSPACE_LSP_SESSIONS: usize = 8;
const MAX_POOLED_LSP_SESSIONS: usize = 8;
const MAX_OPEN_DOCUMENTS_PER_SESSION: usize = 128;
const LSP_SESSION_IDLE_SECS: u64 = 300;''',
    )

    replace_once(
        path,
        '''#[derive(Debug, Clone, Copy, Default)]
struct ServerCapabilities {
    workspace_symbols: bool,
    definition: bool,
    references: bool,
    hover: bool,
}

struct LspSession {''',
        '''#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
    last_used: Instant,
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
    definition: bool,
    references: bool,
    hover: bool,
    sync_open_close: bool,
    sync_kind: TextDocumentSyncKind,
}

struct LspSession {''',
    )

    replace_once(
        path,
        '''    root_uri: String,
    root_name: String,
    capabilities: ServerCapabilities,
}

pub async fn workspace_symbols(''',
        '''    root_uri: String,
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
    let mut duplicate = None;
    let mut rejected = false;
    {
        let mut pool = lsp_session_pool().lock().await;
        if let Some(entry) = pool.entries.get_mut(&key) {
            entry.last_used = Instant::now();
            duplicate = Some(SessionLease {
                key: key.clone(),
                session: Arc::clone(&entry.session),
            });
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

    if let Some(duplicate) = duplicate {
        close_session_handles(vec![started]).await;
        return Ok(duplicate);
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

pub async fn workspace_symbols(''',
    )

    old_workspace = '''            let Ok(mut session) = LspSession::start(&root, &candidate.root, candidate.spec).await
            else {
                continue;
            };
            if !session.capabilities.workspace_symbols {
                session.close().await;
                continue;
            }
            let request = session
                .request("workspace/symbol", json!({"query": query}))
                .await;
            if let Ok(result) = request {
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
            session.close().await;'''
    new_workspace = '''            let Ok(lease) =
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
            }'''
    replace_once(path, old_workspace, new_workspace)

    old_position = '''    if let (true, Some(spec)) = (allow_language_server, select_server_for_file(&file)) {
        let server_root = nearest_project_root(&root, &file, spec);
        if let Ok(mut session) = LspSession::start(&root, &server_root, spec).await {
            if session.supports_navigation(kind)
                && session
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
                    NavigationKind::Hover => json!({
                        "textDocument": {"uri": uri},
                        "position": {"line": line - 1, "character": lsp_character}
                    }),
                };
                let (method, limit) = match kind {
                    NavigationKind::Definition => ("textDocument/definition", MAX_SYMBOL_RESULTS),
                    NavigationKind::References => {
                        ("textDocument/references", MAX_REFERENCE_RESULTS)
                    }
                    NavigationKind::Hover => ("textDocument/hover", 1),
                };
                if let Ok(result) = session.request(method, params).await {
                    let result = match kind {
                        NavigationKind::Hover => sanitize_hover_result(result)?,
                        NavigationKind::Definition | NavigationKind::References => {
                            sanitize_lsp_result(&root, result, limit)?
                        }
                    };
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
    }'''
    new_position = '''    if let (true, Some(spec)) = (allow_language_server, select_server_for_file(&file)) {
        let server_root = nearest_project_root(&root, &file, spec);
        if let Ok(lease) = acquire_healthy_lsp_session(&root, &server_root, spec).await {
            let request = {
                let mut session = lease.session.lock().await;
                if !session.supports_navigation(kind) {
                    None
                } else if let Err(error) = session
                    .sync_document(&file, &text, spec.language_id)
                    .await
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
    }'''
    replace_once(path, old_position, new_position)

    replace_once(
        path,
        '''            root_name: root_name.clone(),
            capabilities: ServerCapabilities::default(),
        };''',
        '''            root_name: root_name.clone(),
            capabilities: ServerCapabilities::default(),
            open_documents: HashMap::new(),
        };''',
    )

    old_open = '''    async fn open_document(
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
'''
    new_open = '''    async fn sync_document(
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
                last_used: now,
            },
        );
        Ok(())
    }
'''
    replace_once(path, old_open, new_open)

    replace_once(
        path,
        '''    fn supports_navigation(&self, kind: NavigationKind) -> bool {
        match kind {
            NavigationKind::Definition => self.capabilities.definition,
            NavigationKind::References => self.capabilities.references,
            NavigationKind::Hover => self.capabilities.hover,
        }
    }

    async fn close(&mut self) {
        let _ = self.request("shutdown", Value::Null).await;
        let _ = self.notify("exit", Value::Null).await;
        let _ = self.child.kill().await;
    }''',
        '''    fn supports_navigation(&self, kind: NavigationKind) -> bool {
        match kind {
            NavigationKind::Definition => self.capabilities.definition,
            NavigationKind::References => self.capabilities.references,
            NavigationKind::Hover => self.capabilities.hover,
        }
    }

    fn is_alive(&mut self) -> bool {
        self.child
            .try_wait()
            .is_ok_and(|status| status.is_none())
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
        let _ = self.child.kill().await;
    }''',
    )

    replace_once(
        path,
        '''    ServerCapabilities {
        workspace_symbols: capability_enabled(capabilities, "workspaceSymbolProvider"),
        definition: capability_enabled(capabilities, "definitionProvider"),
        references: capability_enabled(capabilities, "referencesProvider"),
        hover: capability_enabled(capabilities, "hoverProvider"),
    }
}

fn capability_enabled(capabilities: &Value, name: &str) -> bool {''',
        '''    let (sync_open_close, sync_kind) = parse_text_document_sync(capabilities);
    ServerCapabilities {
        workspace_symbols: capability_enabled(capabilities, "workspaceSymbolProvider"),
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
            let kind = sync_kind_from_value(
                options
                    .get("change")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            );
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

fn capability_enabled(capabilities: &Value, name: &str) -> bool {''',
    )

    replace_once(
        path,
        '''fn result_has_items(result: &Value) -> bool {''',
        '''fn source_fingerprint(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

fn source_end_position(text: &str) -> LspPosition {
    let line = text.bytes().filter(|byte| *byte == b'\\n').count() as u64;
    let tail = text.rsplit_once('\\n').map(|(_, tail)| tail).unwrap_or(text);
    let character = tail.encode_utf16().count() as u64;
    LspPosition { line, character }
}

fn oldest_document_uri(documents: &HashMap<String, DocumentState>) -> Option<String> {
    documents
        .iter()
        .min_by_key(|(_, state)| state.last_used)
        .map(|(uri, _)| uri.clone())
}

fn result_has_items(result: &Value) -> bool {''',
    )

    replace_once(
        path,
        '''            "capabilities": {
                "workspaceSymbolProvider": true,
                "definitionProvider": {"workDoneProgress": true},
                "referencesProvider": false,
                "hoverProvider": {}
            }
        }));
        assert!(parsed.workspace_symbols);
        assert!(parsed.definition);
        assert!(!parsed.references);
        assert!(parsed.hover);
    }
''',
        '''            "capabilities": {
                "workspaceSymbolProvider": true,
                "definitionProvider": {"workDoneProgress": true},
                "referencesProvider": false,
                "hoverProvider": {},
                "textDocumentSync": {"openClose": true, "change": 2}
            }
        }));
        assert!(parsed.workspace_symbols);
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
        let position = source_end_position("first\\n🙂x");
        assert_eq!(position.line, 1);
        assert_eq!(position.character, 3);
        let trailing = source_end_position("first\\n");
        assert_eq!(trailing.line, 1);
        assert_eq!(trailing.character, 0);
    }

    #[test]
    fn source_fingerprint_changes_with_document_content() {
        assert_ne!(source_fingerprint("before"), source_fingerprint("after"));
        assert_eq!(source_fingerprint("same"), source_fingerprint("same"));
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
                last_used: now,
            },
        );
        documents.insert(
            "file:///old.rs".to_string(),
            DocumentState {
                version: 1,
                fingerprint: source_fingerprint("old"),
                end_position: source_end_position("old"),
                last_used: earlier,
            },
        );
        assert_eq!(
            oldest_document_uri(&documents).as_deref(),
            Some("file:///old.rs")
        );
    }
''',
    )


def patch_agent_prompt() -> None:
    path = "src-tauri/src/local_agent.rs"
    replace_once(
        path,
        "A language server may run only when Full PC + Terminal access is enabled and its executable resolves from a trusted PATH location; otherwise bounded lexical indexing is used.",
        "A language server may run only when Full PC + Terminal access is enabled and its executable resolves from a trusted PATH location; compatible servers are reused through a bounded idle-evicted session pool with document synchronization, otherwise bounded lexical indexing is used.",
    )


if __name__ == "__main__":
    patch_lsp()
    patch_agent_prompt()
