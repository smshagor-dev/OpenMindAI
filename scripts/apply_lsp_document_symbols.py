from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        raise SystemExit(f"missing anchor: {label}")
    if text.count(old) != 1:
        raise SystemExit(f"non-unique anchor: {label} ({text.count(old)})")
    return text.replace(old, new, 1)


coding_path = Path("src-tauri/src/coding_lsp.rs")
local_path = Path("src-tauri/src/local_agent.rs")
security_path = Path("src-tauri/src/openagent_security.rs")

coding = coding_path.read_text()
coding = replace_once(
    coding,
    "const MAX_SYMBOL_RESULTS: usize = 100;\nconst MAX_REFERENCE_RESULTS: usize = 200;",
    "const MAX_SYMBOL_RESULTS: usize = 100;\nconst MAX_DOCUMENT_SYMBOL_RESULTS: usize = 200;\nconst MAX_DOCUMENT_SYMBOL_DEPTH: usize = 16;\nconst MAX_SYMBOL_NAME_CHARS: usize = 512;\nconst MAX_SYMBOL_DETAIL_CHARS: usize = 2_000;\nconst MAX_REFERENCE_RESULTS: usize = 200;",
    "document symbol constants",
)
coding = replace_once(
    coding,
    "struct ServerCapabilities {\n    workspace_symbols: bool,\n    definition: bool,",
    "struct ServerCapabilities {\n    workspace_symbols: bool,\n    document_symbols: bool,\n    definition: bool,",
    "server capabilities field",
)

document_symbols_fn = r'''pub async fn document_symbols(
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

'''
coding = replace_once(
    coding,
    "pub async fn definition(\n",
    document_symbols_fn + "pub async fn definition(\n",
    "document symbols public function",
)

coding = replace_once(
    coding,
    '                            "hover": {"dynamicRegistration": false, "contentFormat": ["markdown", "plaintext"]},\n                            "publishDiagnostics": {',
    '                            "hover": {"dynamicRegistration": false, "contentFormat": ["markdown", "plaintext"]},\n                            "documentSymbol": {\n                                "dynamicRegistration": false,\n                                "hierarchicalDocumentSymbolSupport": true,\n                                "tagSupport": {"valueSet": [1]}\n                            },\n                            "publishDiagnostics": {',
    "document symbol client capability",
)

coding = replace_once(
    coding,
    '        workspace_symbols: capability_enabled(capabilities, "workspaceSymbolProvider"),\n        definition: capability_enabled(capabilities, "definitionProvider"),',
    '        workspace_symbols: capability_enabled(capabilities, "workspaceSymbolProvider"),\n        document_symbols: capability_enabled(capabilities, "documentSymbolProvider"),\n        definition: capability_enabled(capabilities, "definitionProvider"),',
    "document symbol server capability",
)

symbol_sanitizers = r'''fn sanitize_document_symbol_result(
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
        if let Some(symbol) =
            sanitize_document_symbol_item(root, file, &value, 0, &mut remaining)?
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

'''
coding = replace_once(
    coding,
    "fn sanitize_publish_diagnostics(\n",
    symbol_sanitizers + "fn sanitize_publish_diagnostics(\n",
    "document symbol sanitizers",
)

fallback_outline = r'''fn fallback_document_symbols(root: &Path, file: &Path, text: &str) -> Value {
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

'''
coding = replace_once(
    coding,
    "fn fallback_workspace_symbols(root: &Path, query: &str) -> Result<Value, AppError> {\n",
    fallback_outline + "fn fallback_workspace_symbols(root: &Path, query: &str) -> Result<Value, AppError> {\n",
    "document symbol lexical fallback",
)

coding = replace_once(
    coding,
    '                "workspaceSymbolProvider": true,\n                "definitionProvider": {"workDoneProgress": true},',
    '                "workspaceSymbolProvider": true,\n                "documentSymbolProvider": {"label": "outline"},\n                "definitionProvider": {"workDoneProgress": true},',
    "document symbol capability test fixture",
)
coding = replace_once(
    coding,
    "        assert!(parsed.workspace_symbols);\n        assert!(parsed.definition);",
    "        assert!(parsed.workspace_symbols);\n        assert!(parsed.document_symbols);\n        assert!(parsed.definition);",
    "document symbol capability assertion",
)

new_tests = r'''    #[test]
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

'''
coding = replace_once(
    coding,
    "    #[test]\n    fn fallback_finds_definitions_and_references() {\n",
    new_tests + "    #[test]\n    fn fallback_finds_definitions_and_references() {\n",
    "document symbol tests",
)
coding_path.write_text(coding)

local = local_path.read_text()
local = replace_once(
    local,
    r'''{{\"type\":\"tool\",\"tool\":\"symbol_search\",\"rootId\":\"ID\",\"query\":\"symbol name\"}}\n\
{{\"type\":\"tool\",\"tool\":\"symbol_definition\",\"rootId\":\"ID\",\"path\":\"file\",\"line\":1,\"character\":0}}\n\''',
    r'''{{\"type\":\"tool\",\"tool\":\"symbol_search\",\"rootId\":\"ID\",\"query\":\"symbol name\"}}\n\
{{\"type\":\"tool\",\"tool\":\"symbol_outline\",\"rootId\":\"ID\",\"path\":\"file\"}}\n\
{{\"type\":\"tool\",\"tool\":\"symbol_definition\",\"rootId\":\"ID\",\"path\":\"file\",\"line\":1,\"character\":0}}\n\''',
    "symbol outline prompt shape",
)
local = replace_once(
    local,
    "- Prefer symbol_search/symbol_definition/symbol_references/symbol_hover for identifier navigation and symbol_diagnostics for file diagnostics.",
    "- Prefer symbol_outline for a bounded file-level outline, symbol_search/symbol_definition/symbol_references/symbol_hover for identifier navigation, and symbol_diagnostics for file diagnostics.",
    "symbol outline prompt rule",
)
outline_tool = r'''        "symbol_outline" => {
            let root_id = optional_string(action, "rootId");
            let path = required_string(action, "path")?;
            let root = selected_root_path(config, root_id.as_deref())?;
            let outline =
                coding_lsp::document_symbols(&root, &path, config.full_pc_access).await?;
            let result = serde_json::to_string(&outline).map_err(|error| {
                AppError::internal(format!("failed to encode symbol_outline result: {error}"))
            })?;
            Ok(AgentTurnResult {
                trace_label: format!("Outlined symbols in {path}"),
                transcript_result: bounded(&result, MAX_TOOL_RESULT_CHARS),
            })
        }
'''
local = replace_once(
    local,
    '        "symbol_diagnostics" => {\n',
    outline_tool + '        "symbol_diagnostics" => {\n',
    "symbol outline tool executor",
)
local_path.write_text(local)

security = security_path.read_text()
security = replace_once(
    security,
    '        | "symbol_references" | "symbol_hover" | "symbol_diagnostics" | "git_status"\n        | "git_diff" => RiskLevel::ReadOnly,',
    '        | "symbol_references" | "symbol_hover" | "symbol_outline" | "symbol_diagnostics"\n        | "git_status" | "git_diff" => RiskLevel::ReadOnly,',
    "symbol outline security classification",
)
security = replace_once(
    security,
    '''        let (decision, _) =
            authorize_tool("symbol_diagnostics", &json!({}), ApprovalMode::AlwaysAsk);
        assert_eq!(decision, PolicyDecision::Allow);''',
    '''        let (decision, _) =
            authorize_tool("symbol_diagnostics", &json!({}), ApprovalMode::AlwaysAsk);
        assert_eq!(decision, PolicyDecision::Allow);
        let (decision, _) =
            authorize_tool("symbol_outline", &json!({}), ApprovalMode::AlwaysAsk);
        assert_eq!(decision, PolicyDecision::Allow);''',
    "symbol outline security test",
)
security_path.write_text(security)
