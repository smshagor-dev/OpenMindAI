from __future__ import annotations

from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{path}: expected one replacement, found {count}: {old[:120]!r}")
    file.write_text(text.replace(old, new, 1), encoding="utf-8")


def insert_prompt_tool() -> None:
    path = Path("src-tauri/src/local_agent.rs")
    text = path.read_text(encoding="utf-8")
    marker = '\\"tool\\":\\"symbol_references\\"'
    lines = text.splitlines(keepends=True)
    matches = [index for index, line in enumerate(lines) if marker in line]
    if len(matches) != 1:
        raise RuntimeError(f"{path}: expected one symbol_references prompt line, found {len(matches)}")
    index = matches[0]
    hover_line = lines[index].replace("symbol_references", "symbol_hover")
    lines.insert(index + 1, hover_line)
    path.write_text("".join(lines), encoding="utf-8")


def patch_coding_lsp() -> None:
    path = "src-tauri/src/coding_lsp.rs"

    replace_once(
        path,
        "const MAX_REFERENCE_RESULTS: usize = 200;\nconst MAX_LSP_MESSAGE_BYTES: usize = 8 * 1024 * 1024;",
        "const MAX_REFERENCE_RESULTS: usize = 200;\nconst MAX_HOVER_CHARS: usize = 12_000;\nconst MAX_LSP_MESSAGE_BYTES: usize = 8 * 1024 * 1024;",
    )

    replace_once(
        path,
        "#[derive(Debug, Clone)]\nstruct WorkspaceServer {\n    root: PathBuf,\n    spec: ServerSpec,\n}\n\nstruct LspSession {",
        "#[derive(Debug, Clone)]\nstruct WorkspaceServer {\n    root: PathBuf,\n    spec: ServerSpec,\n}\n\n#[derive(Debug, Clone, Copy, Default)]\nstruct ServerCapabilities {\n    workspace_symbols: bool,\n    definition: bool,\n    references: bool,\n    hover: bool,\n}\n\nstruct LspSession {",
    )

    replace_once(
        path,
        "    root_uri: String,\n    root_name: String,\n}",
        "    root_uri: String,\n    root_name: String,\n    capabilities: ServerCapabilities,\n}",
    )

    replace_once(
        path,
        "            let Ok(mut session) = LspSession::start(&root, &candidate.root, candidate.spec).await\n            else {\n                continue;\n            };\n            let request = session\n                .request(\"workspace/symbol\", json!({\"query\": query}))\n                .await;",
        "            let Ok(mut session) = LspSession::start(&root, &candidate.root, candidate.spec).await\n            else {\n                continue;\n            };\n            if !session.capabilities.workspace_symbols {\n                session.close().await;\n                continue;\n            }\n            let request = session\n                .request(\"workspace/symbol\", json!({\"query\": query}))\n                .await;",
    )

    replace_once(
        path,
        "pub async fn references(\n    root: &Path,\n    relative_path: &str,\n    line: u64,\n    character: u64,\n    allow_language_server: bool,\n) -> Result<NavigationResult, AppError> {\n    position_navigation(\n        root,\n        relative_path,\n        line,\n        character,\n        NavigationKind::References,\n        allow_language_server,\n    )\n    .await\n}\n\n#[derive(Debug, Clone, Copy)]\nenum NavigationKind {\n    Definition,\n    References,\n}",
        "pub async fn references(\n    root: &Path,\n    relative_path: &str,\n    line: u64,\n    character: u64,\n    allow_language_server: bool,\n) -> Result<NavigationResult, AppError> {\n    position_navigation(\n        root,\n        relative_path,\n        line,\n        character,\n        NavigationKind::References,\n        allow_language_server,\n    )\n    .await\n}\n\npub async fn hover(\n    root: &Path,\n    relative_path: &str,\n    line: u64,\n    character: u64,\n    allow_language_server: bool,\n) -> Result<NavigationResult, AppError> {\n    position_navigation(\n        root,\n        relative_path,\n        line,\n        character,\n        NavigationKind::Hover,\n        allow_language_server,\n    )\n    .await\n}\n\n#[derive(Debug, Clone, Copy)]\nenum NavigationKind {\n    Definition,\n    References,\n    Hover,\n}",
    )

    old_navigation = '''    if let (true, Some(spec)) = (allow_language_server, select_server_for_file(&file)) {
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
    };'''
    new_navigation = '''    if let (true, Some(spec)) = (allow_language_server, select_server_for_file(&file)) {
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
    }

    let symbol = symbol_at_position(&text, line_index, character_index)?;
    let result = match kind {
        NavigationKind::Definition => fallback_definition(&root, &symbol)?,
        NavigationKind::References => fallback_references(&root, &symbol)?,
        NavigationKind::Hover => fallback_hover(&root, &symbol)?,
    };'''
    replace_once(path, old_navigation, new_navigation)

    replace_once(
        path,
        "            root_uri: root_uri.clone(),\n            root_name: root_name.clone(),\n        };\n        session\n            .request(\n                \"initialize\",",
        "            root_uri: root_uri.clone(),\n            root_name: root_name.clone(),\n            capabilities: ServerCapabilities::default(),\n        };\n        let initialize_result = session\n            .request(\n                \"initialize\",",
    )

    replace_once(
        path,
        '                            "definition": {"dynamicRegistration": false, "linkSupport": true},\n                            "references": {"dynamicRegistration": false},\n                            "synchronization": {"dynamicRegistration": false, "didOpen": true}',
        '                            "definition": {"dynamicRegistration": false, "linkSupport": true},\n                            "references": {"dynamicRegistration": false},\n                            "hover": {"dynamicRegistration": false, "contentFormat": ["markdown", "plaintext"]},\n                            "synchronization": {"dynamicRegistration": false, "didOpen": true}',
    )

    replace_once(
        path,
        "            )\n            .await?;\n        session.notify(\"initialized\", json!({})).await?;",
        "            )\n            .await?;\n        session.capabilities = parse_server_capabilities(&initialize_result);\n        session.notify(\"initialized\", json!({})).await?;",
    )

    replace_once(
        path,
        "    async fn close(&mut self) {\n        let _ = self.request(\"shutdown\", Value::Null).await;",
        "    fn supports_navigation(&self, kind: NavigationKind) -> bool {\n        match kind {\n            NavigationKind::Definition => self.capabilities.definition,\n            NavigationKind::References => self.capabilities.references,\n            NavigationKind::Hover => self.capabilities.hover,\n        }\n    }\n\n    async fn close(&mut self) {\n        let _ = self.request(\"shutdown\", Value::Null).await;",
    )

    replace_once(
        path,
        "}\n\nasync fn read_lsp_message(reader: &mut BufReader<ChildStdout>) -> Result<Value, AppError> {",
        '''}

fn parse_server_capabilities(initialize_result: &Value) -> ServerCapabilities {
    let capabilities = initialize_result
        .get("capabilities")
        .unwrap_or(&Value::Null);
    ServerCapabilities {
        workspace_symbols: capability_enabled(capabilities, "workspaceSymbolProvider"),
        definition: capability_enabled(capabilities, "definitionProvider"),
        references: capability_enabled(capabilities, "referencesProvider"),
        hover: capability_enabled(capabilities, "hoverProvider"),
    }
}

fn capability_enabled(capabilities: &Value, name: &str) -> bool {
    match capabilities.get(name) {
        Some(Value::Bool(value)) => *value,
        Some(Value::Object(_)) => true,
        _ => false,
    }
}

async fn read_lsp_message(reader: &mut BufReader<ChildStdout>) -> Result<Value, AppError> {''',
    )

    replace_once(
        path,
        "fn result_has_items(result: &Value) -> bool {",
        '''fn sanitize_hover_result(result: Value) -> Result<Value, AppError> {
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

fn result_has_items(result: &Value) -> bool {''',
    )

    replace_once(
        path,
        "fn fallback_references(root: &Path, symbol: &str) -> Result<Value, AppError> {",
        '''fn fallback_hover(root: &Path, symbol: &str) -> Result<Value, AppError> {
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

fn fallback_references(root: &Path, symbol: &str) -> Result<Value, AppError> {''',
    )

    replace_once(
        path,
        "    #[test]\n    fn lsp_results_are_scoped_to_workspace() {",
        '''    #[test]
    fn server_capabilities_are_parsed_from_initialize_result() {
        let parsed = parse_server_capabilities(&json!({
            "capabilities": {
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

    #[test]
    fn hover_sanitization_bounds_untrusted_markup() {
        let long = "x".repeat(MAX_HOVER_CHARS + 50);
        let safe = sanitize_hover_result(json!({
            "contents": {"kind": "markdown", "value": long},
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
            "unsafe": "discard me"
        }))
        .unwrap();
        let value = safe.pointer("/contents/value").and_then(Value::as_str).unwrap();
        assert!(value.chars().count() <= MAX_HOVER_CHARS);
        assert!(safe.get("unsafe").is_none());
    }

    #[test]
    fn fallback_hover_reports_declaration_metadata() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("lib.rs"), "pub struct Router {}\\n").unwrap();
        let hover = fallback_hover(temp.path(), "Router").unwrap();
        let item = hover.as_array().unwrap().first().unwrap();
        assert_eq!(item.get("kind").and_then(Value::as_str), Some("struct"));
        assert_eq!(item.get("path").and_then(Value::as_str), Some("lib.rs"));
    }

    #[test]
    fn lsp_results_are_scoped_to_workspace() {''',
    )


def patch_local_agent() -> None:
    path = "src-tauri/src/local_agent.rs"
    insert_prompt_tool()

    replace_once(
        path,
        "Prefer symbol_search/symbol_definition/symbol_references for identifier navigation.",
        "Prefer symbol_search/symbol_definition/symbol_references/symbol_hover for identifier navigation.",
    )

    old = '''        "symbol_definition" | "symbol_references" => {
            let root_id = optional_string(action, "rootId");
            let path = required_string(action, "path")?;
            let line = action
                .get("line")
                .and_then(Value::as_u64)
                .ok_or_else(|| AppError::internal("symbol navigation requires `line`"))?;
            let character = action.get("character").and_then(Value::as_u64).unwrap_or(0);
            let root = selected_root_path(config, root_id.as_deref())?;
            let navigation = if tool == "symbol_definition" {
                coding_lsp::definition(&root, &path, line, character, config.full_pc_access).await?
            } else {
                coding_lsp::references(&root, &path, line, character, config.full_pc_access).await?
            };
            let result = serde_json::to_string(&navigation).map_err(|error| {
                AppError::internal(format!(
                    "failed to encode symbol navigation result: {error}"
                ))
            })?;
            Ok(AgentTurnResult {
                trace_label: format!(
                    "{} {}:{}:{}",
                    if tool == "symbol_definition" {
                        "Resolved definition at"
                    } else {
                        "Found references from"
                    },
                    path,
                    line,
                    character
                ),
                transcript_result: bounded(&result, MAX_TOOL_RESULT_CHARS),
            })
        }'''
    new = '''        "symbol_definition" | "symbol_references" | "symbol_hover" => {
            let root_id = optional_string(action, "rootId");
            let path = required_string(action, "path")?;
            let line = action
                .get("line")
                .and_then(Value::as_u64)
                .ok_or_else(|| AppError::internal("symbol navigation requires `line`"))?;
            let character = action.get("character").and_then(Value::as_u64).unwrap_or(0);
            let root = selected_root_path(config, root_id.as_deref())?;
            let navigation = if tool == "symbol_definition" {
                coding_lsp::definition(&root, &path, line, character, config.full_pc_access).await?
            } else if tool == "symbol_references" {
                coding_lsp::references(&root, &path, line, character, config.full_pc_access).await?
            } else {
                coding_lsp::hover(&root, &path, line, character, config.full_pc_access).await?
            };
            let result = serde_json::to_string(&navigation).map_err(|error| {
                AppError::internal(format!(
                    "failed to encode symbol navigation result: {error}"
                ))
            })?;
            Ok(AgentTurnResult {
                trace_label: format!(
                    "{} {}:{}:{}",
                    if tool == "symbol_definition" {
                        "Resolved definition at"
                    } else if tool == "symbol_references" {
                        "Found references from"
                    } else {
                        "Resolved hover metadata at"
                    },
                    path,
                    line,
                    character
                ),
                transcript_result: bounded(&result, MAX_TOOL_RESULT_CHARS),
            })
        }'''
    replace_once(path, old, new)


def patch_security() -> None:
    replace_once(
        "src-tauri/src/openagent_security.rs",
        '        | "symbol_references" | "git_status" | "git_diff" => RiskLevel::ReadOnly,',
        '        | "symbol_references" | "symbol_hover" | "git_status" | "git_diff" => RiskLevel::ReadOnly,',
    )


if __name__ == "__main__":
    patch_coding_lsp()
    patch_local_agent()
    patch_security()
