use std::{
    fs,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationResult {
    pub engine: String,
    pub server: Option<String>,
    pub result: Value,
}

#[derive(Debug, Clone, Copy)]
struct ServerSpec {
    command: &'static str,
    args: &'static [&'static str],
    language_id: &'static str,
}

struct LspSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    server_name: String,
}

pub async fn workspace_symbols(root: &Path, query: &str) -> Result<NavigationResult, AppError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(AppError::internal("symbol_search query cannot be empty"));
    }
    let root = canonical_root(root)?;
    if let Some(spec) = select_server(&root, None) {
        if let Ok(mut session) = LspSession::start(&root, spec).await {
            if let Ok(result) = session
                .request("workspace/symbol", json!({"query": query}))
                .await
            {
                session.close().await;
                return Ok(NavigationResult {
                    engine: "lsp".to_string(),
                    server: Some(spec.command.to_string()),
                    result,
                });
            }
            session.close().await;
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
) -> Result<NavigationResult, AppError> {
    position_navigation(
        root,
        relative_path,
        line,
        character,
        NavigationKind::Definition,
    )
    .await
}

pub async fn references(
    root: &Path,
    relative_path: &str,
    line: u64,
    character: u64,
) -> Result<NavigationResult, AppError> {
    position_navigation(
        root,
        relative_path,
        line,
        character,
        NavigationKind::References,
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
) -> Result<NavigationResult, AppError> {
    if line == 0 {
        return Err(AppError::internal(
            "symbol navigation line is 1-based and must be >= 1",
        ));
    }
    let root = canonical_root(root)?;
    let file = resolve_source_file(&root, relative_path)?;
    let text = read_source(&file)?;
    if let Some(spec) = select_server(&root, Some(&file)) {
        if let Ok(mut session) = LspSession::start(&root, spec).await {
            if session
                .open_document(&file, &text, spec.language_id)
                .await
                .is_ok()
            {
                let uri = file_uri(&file)?;
                let params = match kind {
                    NavigationKind::Definition => json!({
                        "textDocument": {"uri": uri},
                        "position": {"line": line - 1, "character": character}
                    }),
                    NavigationKind::References => json!({
                        "textDocument": {"uri": uri},
                        "position": {"line": line - 1, "character": character},
                        "context": {"includeDeclaration": true}
                    }),
                };
                let method = match kind {
                    NavigationKind::Definition => "textDocument/definition",
                    NavigationKind::References => "textDocument/references",
                };
                if let Ok(result) = session.request(method, params).await {
                    session.close().await;
                    return Ok(NavigationResult {
                        engine: "lsp".to_string(),
                        server: Some(spec.command.to_string()),
                        result,
                    });
                }
            }
            session.close().await;
        }
    }

    let symbol = symbol_at_position(&text, line as usize, character as usize)?;
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

impl LspSession {
    async fn start(root: &Path, spec: ServerSpec) -> Result<Self, AppError> {
        let mut process = Command::new(spec.command);
        process
            .args(spec.args)
            .current_dir(root)
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
        let mut session = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            server_name: spec.command.to_string(),
        };
        let root_uri = directory_uri(root)?;
        session
            .request(
                "initialize",
                json!({
                    "processId": Value::Null,
                    "clientInfo": {"name":"OpenMindAI OpenAgent","version":"1"},
                    "rootUri": root_uri,
                    "capabilities": {
                        "workspace": {"symbol": {}},
                        "textDocument": {
                            "definition": {},
                            "references": {},
                            "synchronization": {"didOpen": true}
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
        let body = serde_json::to_vec(&value)
            .map_err(|error| AppError::internal(format!("failed to encode LSP request: {error}")))?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.stdin.write_all(header.as_bytes()).await?;
        self.stdin.write_all(&body).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn read_response(&mut self, id: u64) -> Result<Value, AppError> {
        loop {
            let value = read_lsp_message(&mut self.stdout).await?;
            if value.get("id").and_then(Value::as_u64) == Some(id) {
                return Ok(value);
            }
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
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).await?;
        if read == 0 {
            return Err(AppError::internal("language server closed its output"));
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| AppError::internal("invalid LSP Content-Length header"))?,
            );
        }
    }
    let length = content_length.ok_or_else(|| AppError::internal("missing LSP Content-Length"))?;
    if length > 8 * 1024 * 1024 {
        return Err(AppError::internal(
            "language server response exceeds safety limit",
        ));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).await?;
    serde_json::from_slice(&body)
        .map_err(|error| AppError::internal(format!("invalid LSP JSON response: {error}")))
}

fn select_server(root: &Path, file: Option<&Path>) -> Option<ServerSpec> {
    if let Some(file) = file {
        match extension(file).as_str() {
            "rs" => {
                return Some(ServerSpec {
                    command: "rust-analyzer",
                    args: &[],
                    language_id: "rust",
                })
            }
            "ts" | "tsx" => {
                return Some(ServerSpec {
                    command: "typescript-language-server",
                    args: &["--stdio"],
                    language_id: "typescript",
                })
            }
            "js" | "jsx" => {
                return Some(ServerSpec {
                    command: "typescript-language-server",
                    args: &["--stdio"],
                    language_id: "javascript",
                })
            }
            "py" => {
                return Some(ServerSpec {
                    command: "pyright-langserver",
                    args: &["--stdio"],
                    language_id: "python",
                })
            }
            "go" => {
                return Some(ServerSpec {
                    command: "gopls",
                    args: &["serve"],
                    language_id: "go",
                })
            }
            _ => {}
        }
    }
    if root.join("Cargo.toml").is_file() {
        Some(ServerSpec {
            command: "rust-analyzer",
            args: &[],
            language_id: "rust",
        })
    } else if root.join("tsconfig.json").is_file() || root.join("package.json").is_file() {
        Some(ServerSpec {
            command: "typescript-language-server",
            args: &["--stdio"],
            language_id: "typescript",
        })
    } else if root.join("pyproject.toml").is_file() || root.join("requirements.txt").is_file() {
        Some(ServerSpec {
            command: "pyright-langserver",
            args: &["--stdio"],
            language_id: "python",
        })
    } else if root.join("go.mod").is_file() {
        Some(ServerSpec {
            command: "gopls",
            args: &["serve"],
            language_id: "go",
        })
    } else {
        None
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
                    "preview": line.trim()
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
    let ext = extension(file);
    let patterns: &[(&str, &str)] = match ext.as_str() {
        "rs" => &[
            ("pub fn ", "function"),
            ("fn ", "function"),
            ("pub struct ", "struct"),
            ("struct ", "struct"),
            ("pub enum ", "enum"),
            ("enum ", "enum"),
            ("pub trait ", "trait"),
            ("trait ", "trait"),
        ],
        "ts" | "tsx" | "js" | "jsx" => &[
            ("export function ", "function"),
            ("function ", "function"),
            ("export class ", "class"),
            ("class ", "class"),
            ("export interface ", "interface"),
            ("interface ", "interface"),
            ("export type ", "type"),
            ("type ", "type"),
        ],
        "py" => &[
            ("async def ", "function"),
            ("def ", "function"),
            ("class ", "class"),
        ],
        "go" => &[("func ", "function"), ("type ", "type")],
        "php" => &[
            ("function ", "function"),
            ("class ", "class"),
            ("interface ", "interface"),
            ("trait ", "trait"),
        ],
        _ => &[],
    };
    for (prefix, kind) in patterns {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let rest = rest.trim_start_matches(|c: char| c == '*' || c == '&' || c == '(');
            let symbol = rest
                .chars()
                .take_while(|character| is_identifier_char(*character))
                .collect::<String>();
            if !symbol.is_empty() {
                return Some((*kind, symbol));
            }
        }
    }
    None
}

fn symbol_at_position(text: &str, line: usize, character: usize) -> Result<String, AppError> {
    let line_text = text
        .lines()
        .nth(line.saturating_sub(1))
        .ok_or_else(|| AppError::internal("symbol navigation line is outside the file"))?;
    let chars = line_text.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return Err(AppError::internal(
            "no symbol at the requested position",
        ));
    }
    let mut index = character.min(chars.len().saturating_sub(1));
    if !is_identifier_char(chars[index]) && index > 0 && is_identifier_char(chars[index - 1]) {
        index -= 1;
    }
    if !is_identifier_char(chars[index]) {
        return Err(AppError::internal(
            "no symbol at the requested position",
        ));
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
    let file = fs::canonicalize(root.join(relative))?;
    if !file.starts_with(root)
        || !file.is_file()
        || fs::symlink_metadata(&file)?.file_type().is_symlink()
    {
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
    fs::read_to_string(path).map_err(|_| {
        AppError::internal("symbol navigation supports UTF-8 source files only")
    })
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
        "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "php"
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
    fn symbol_at_position_extracts_identifier() {
        assert_eq!(
            symbol_at_position("let value = Router::new();", 1, 14).unwrap(),
            "Router"
        );
    }
}
