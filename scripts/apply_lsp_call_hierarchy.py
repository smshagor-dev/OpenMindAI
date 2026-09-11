from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        raise SystemExit(f"missing anchor: {label}")
    return text.replace(old, new, 1)


def insert_before(text: str, anchor: str, addition: str, label: str) -> str:
    if anchor not in text:
        raise SystemExit(f"missing anchor: {label}")
    return text.replace(anchor, addition + anchor, 1)


def insert_after_line_containing(text: str, needles: tuple[str, ...], addition: str, label: str) -> str:
    lines = text.splitlines(keepends=True)
    for index, line in enumerate(lines):
        if all(needle in line for needle in needles):
            lines.insert(index + 1, addition)
            return "".join(lines)
    raise SystemExit(f"missing line anchor: {label}")


lsp_path = Path("src-tauri/src/coding_lsp.rs")
lsp = lsp_path.read_text(encoding="utf-8")

lsp = replace_once(
    lsp,
    "const MAX_DOCUMENT_SYMBOL_DEPTH: usize = 16;\n",
    "const MAX_DOCUMENT_SYMBOL_DEPTH: usize = 16;\nconst MAX_CALL_HIERARCHY_RESULTS: usize = 100;\nconst MAX_CALL_HIERARCHY_RANGES: usize = 64;\n",
    "call hierarchy constants",
)

lsp = replace_once(
    lsp,
    "    document_symbols: bool,\n    definition: bool,\n",
    "    document_symbols: bool,\n    call_hierarchy: bool,\n    definition: bool,\n",
    "server capability field",
)

call_hierarchy_code = r'''
#[derive(Debug, Clone, Copy)]
enum CallHierarchyDirection {
    Incoming,
    Outgoing,
}

pub async fn incoming_calls(
    root: &Path,
    relative_path: &str,
    line: u64,
    character: u64,
    allow_language_server: bool,
) -> Result<NavigationResult, AppError> {
    call_hierarchy(
        root,
        relative_path,
        line,
        character,
        CallHierarchyDirection::Incoming,
        allow_language_server,
    )
    .await
}

pub async fn outgoing_calls(
    root: &Path,
    relative_path: &str,
    line: u64,
    character: u64,
    allow_language_server: bool,
) -> Result<NavigationResult, AppError> {
    call_hierarchy(
        root,
        relative_path,
        line,
        character,
        CallHierarchyDirection::Outgoing,
        allow_language_server,
    )
    .await
}

async fn call_hierarchy(
    root: &Path,
    relative_path: &str,
    line: u64,
    character: u64,
    direction: CallHierarchyDirection,
    allow_language_server: bool,
) -> Result<NavigationResult, AppError> {
    let root = canonical_root(root)?;
    let file = resolve_source_file(&root, relative_path)?;
    let text = read_source(&file)?;

    if !allow_language_server {
        return Ok(call_hierarchy_unavailable(
            None,
            "language-server call hierarchy requires Full PC + Terminal access",
        ));
    }

    let Some(spec) = select_server_for_file(&file) else {
        return Ok(call_hierarchy_unavailable(
            None,
            "no supported language server is configured for this file type",
        ));
    };
    let server_root = nearest_project_root(&root, &file, spec);
    let server = Some(format!(
        "{}@{}",
        spec.command,
        relative_display(&root, &server_root)
    ));

    let Ok(lease) = acquire_healthy_lsp_session(&root, &server_root, spec).await else {
        return Ok(call_hierarchy_unavailable(
            server,
            "trusted language server is unavailable",
        ));
    };

    let response = {
        let mut session = lease.session.lock().await;
        if !session.capabilities.call_hierarchy {
            return Ok(call_hierarchy_unavailable(
                server,
                "language server does not advertise callHierarchyProvider",
            ));
        }
        if let Err(error) = session.sync_document(&file, &text, spec.language_id).await {
            Err(error)
        } else {
            let uri = file_uri(&file)?;
            match session
                .request(
                    "textDocument/prepareCallHierarchy",
                    json!({
                        "textDocument": {"uri": uri},
                        "position": {"line": line, "character": character}
                    }),
                )
                .await
            {
                Ok(prepared) => match first_scoped_call_hierarchy_item(&root, &file, &prepared)? {
                    Some(item) => {
                        let method = match direction {
                            CallHierarchyDirection::Incoming => "callHierarchy/incomingCalls",
                            CallHierarchyDirection::Outgoing => "callHierarchy/outgoingCalls",
                        };
                        session.request(method, json!({"item": item})).await
                    }
                    None => {
                        return Ok(NavigationResult {
                            engine: "lsp".to_string(),
                            server,
                            result: Value::Array(Vec::new()),
                        });
                    }
                },
                Err(error) => Err(error),
            }
        }
    };

    match response {
        Ok(result) => Ok(NavigationResult {
            engine: "lsp".to_string(),
            server,
            result: sanitize_call_hierarchy_result(&root, result, direction)?,
        }),
        Err(_) => {
            invalidate_pooled_session(&lease).await;
            Ok(call_hierarchy_unavailable(
                server,
                "language server call-hierarchy request failed",
            ))
        }
    }
}

fn call_hierarchy_unavailable(server: Option<String>, reason: &str) -> NavigationResult {
    NavigationResult {
        engine: "lsp-unavailable".to_string(),
        server,
        result: json!({
            "available": false,
            "reason": reason,
            "calls": []
        }),
    }
}

fn first_scoped_call_hierarchy_item(
    root: &Path,
    file: &Path,
    result: &Value,
) -> Result<Option<Value>, AppError> {
    let Some(items) = result.as_array() else {
        return Ok(None);
    };
    for item in items {
        let Some(uri) = item.get("uri").and_then(Value::as_str) else {
            continue;
        };
        if uri_is_scoped(root, uri)? && uri_matches_file(uri, file) {
            return Ok(Some(item.clone()));
        }
    }
    Ok(None)
}

fn sanitize_call_hierarchy_result(
    root: &Path,
    result: Value,
    direction: CallHierarchyDirection,
) -> Result<Value, AppError> {
    let Some(entries) = result.as_array() else {
        return Ok(Value::Array(Vec::new()));
    };
    let item_key = match direction {
        CallHierarchyDirection::Incoming => "from",
        CallHierarchyDirection::Outgoing => "to",
    };
    let mut output = Vec::new();
    for entry in entries.iter().take(MAX_CALL_HIERARCHY_RESULTS) {
        let Some(item) = entry.get(item_key) else {
            continue;
        };
        let Some(item) = sanitize_call_hierarchy_item(root, item)? else {
            continue;
        };
        let ranges = entry
            .get("fromRanges")
            .and_then(Value::as_array)
            .map(|ranges| {
                ranges
                    .iter()
                    .take(MAX_CALL_HIERARCHY_RANGES)
                    .filter_map(sanitize_diagnostic_range)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut safe = serde_json::Map::new();
        safe.insert(item_key.to_string(), item);
        safe.insert("fromRanges".to_string(), Value::Array(ranges));
        output.push(Value::Object(safe));
    }
    Ok(Value::Array(output))
}

fn sanitize_call_hierarchy_item(root: &Path, value: &Value) -> Result<Option<Value>, AppError> {
    let Some(name) = value.get("name").and_then(Value::as_str) else {
        return Ok(None);
    };
    if name.trim().is_empty() {
        return Ok(None);
    }
    let Some(kind) = value.get("kind").and_then(Value::as_u64) else {
        return Ok(None);
    };
    if !(1..=26).contains(&kind) {
        return Ok(None);
    }
    let Some(uri) = value.get("uri").and_then(Value::as_str) else {
        return Ok(None);
    };
    if !uri_is_scoped(root, uri)? {
        return Ok(None);
    }
    let Ok(url) = Url::parse(uri) else {
        return Ok(None);
    };
    let Ok(path) = url.to_file_path() else {
        return Ok(None);
    };
    let Ok(path) = fs::canonicalize(path) else {
        return Ok(None);
    };
    if !path.starts_with(root) {
        return Ok(None);
    }
    let Some(range) = value.get("range").and_then(sanitize_diagnostic_range) else {
        return Ok(None);
    };
    let Some(selection_range) = value
        .get("selectionRange")
        .and_then(sanitize_diagnostic_range)
    else {
        return Ok(None);
    };

    let mut safe = serde_json::Map::new();
    safe.insert(
        "name".to_string(),
        Value::String(truncate_preview(name, MAX_SYMBOL_NAME_CHARS)),
    );
    safe.insert("kind".to_string(), json!(kind));
    safe.insert(
        "path".to_string(),
        Value::String(relative_display(root, &path)),
    );
    safe.insert("range".to_string(), range);
    safe.insert("selectionRange".to_string(), selection_range);
    if let Some(detail) = value.get("detail").and_then(Value::as_str) {
        safe.insert(
            "detail".to_string(),
            Value::String(truncate_preview(detail, MAX_SYMBOL_DETAIL_CHARS)),
        );
    }
    Ok(Some(Value::Object(safe)))
}

'''

lsp = insert_before(
    lsp,
    "pub async fn definition(\n",
    call_hierarchy_code,
    "definition function",
)

lsp = replace_once(
    lsp,
    '        document_symbols: capability_enabled(capabilities, "documentSymbolProvider"),\n        definition: capability_enabled(capabilities, "definitionProvider"),\n',
    '        document_symbols: capability_enabled(capabilities, "documentSymbolProvider"),\n        call_hierarchy: capability_enabled(capabilities, "callHierarchyProvider"),\n        definition: capability_enabled(capabilities, "definitionProvider"),\n',
    "capability parsing",
)

call_tests = r'''
    #[test]
    fn call_hierarchy_capability_is_parsed() {
        let parsed = parse_server_capabilities(&json!({
            "capabilities": {
                "callHierarchyProvider": {"workDoneProgress": true}
            }
        }));
        assert!(parsed.call_hierarchy);
    }

    #[test]
    fn call_hierarchy_sanitization_is_scoped_and_drops_opaque_data() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let inside = root.join("inside.rs");
        fs::write(&inside, "fn caller() {}\n").unwrap();
        let inside_uri = file_uri(&inside).unwrap();

        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("outside.rs");
        fs::write(&outside_file, "fn outside() {}\n").unwrap();
        let outside_uri = file_uri(&outside_file).unwrap();

        let entry = |uri: String, name: &str| json!({
            "from": {
                "name": name,
                "kind": 12,
                "uri": uri,
                "range": {
                    "start": {"line": 0, "character": 0},
                    "end": {"line": 0, "character": 10}
                },
                "selectionRange": {
                    "start": {"line": 0, "character": 3},
                    "end": {"line": 0, "character": 9}
                },
                "detail": "safe detail",
                "data": {"secret": "drop-me"},
                "unsafe": "drop-me"
            },
            "fromRanges": [
                {
                    "start": {"line": 1, "character": 0},
                    "end": {"line": 1, "character": 5}
                }
            ]
        });

        let safe = sanitize_call_hierarchy_result(
            &root,
            json!([
                entry(inside_uri, "caller"),
                entry(outside_uri, "outside")
            ]),
            CallHierarchyDirection::Incoming,
        )
        .unwrap();
        let items = safe.as_array().unwrap();
        assert_eq!(items.len(), 1);
        let from = items[0].get("from").unwrap();
        assert_eq!(from.get("path").and_then(Value::as_str), Some("inside.rs"));
        assert!(from.get("data").is_none());
        assert!(from.get("unsafe").is_none());
        assert_eq!(items[0]["fromRanges"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn prepared_call_hierarchy_item_must_match_requested_file() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let requested = root.join("requested.rs");
        let other = root.join("other.rs");
        fs::write(&requested, "fn requested() {}\n").unwrap();
        fs::write(&other, "fn other() {}\n").unwrap();
        let result = json!([
            {
                "name": "other",
                "kind": 12,
                "uri": file_uri(&other).unwrap(),
                "range": {
                    "start": {"line": 0, "character": 0},
                    "end": {"line": 0, "character": 5}
                },
                "selectionRange": {
                    "start": {"line": 0, "character": 3},
                    "end": {"line": 0, "character": 5}
                },
                "data": {"opaque": true}
            }
        ]);
        assert!(first_scoped_call_hierarchy_item(&root, &requested, &result)
            .unwrap()
            .is_none());
    }

'''

lsp = insert_before(
    lsp,
    "    #[test]\n    fn server_capabilities_are_parsed_from_initialize_result()",
    call_tests,
    "server capability test",
)

lsp_path.write_text(lsp, encoding="utf-8")

agent_path = Path("src-tauri/src/local_agent.rs")
agent = agent_path.read_text(encoding="utf-8")

agent = insert_after_line_containing(
    agent,
    ("symbol_outline", "rootId", "path"),
    '{{\\"type\\":\\"tool\\",\\"tool\\":\\"symbol_incoming_calls\\",\\"rootId\\":\\"ID\\",\\"path\\":\\"file\\",\\"line\\":1,\\"character\\":0}}\\\n\\\n{{\\"type\\":\\"tool\\",\\"tool\\":\\"symbol_outgoing_calls\\",\\"rootId\\":\\"ID\\",\\"path\\":\\"file\\",\\"line\\":1,\\"character\\":0}}\\\n\\\n',
    "prompt call hierarchy tools",
)

old_rule = "- Prefer symbol_outline for a bounded file-level outline, symbol_search/symbol_definition/symbol_references/symbol_hover for identifier navigation, and symbol_diagnostics for file diagnostics."
new_rule = "- Prefer symbol_outline for a bounded file-level outline, symbol_search/symbol_definition/symbol_references/symbol_hover for identifier navigation, symbol_incoming_calls/symbol_outgoing_calls for language-server call relationships, and symbol_diagnostics for file diagnostics."
agent = replace_once(agent, old_rule, new_rule, "navigation prompt rule")

handler = r'''        "symbol_incoming_calls" | "symbol_outgoing_calls" => {
            let root_id = optional_string(action, "rootId");
            let path = required_string(action, "path")?;
            let line = action
                .get("line")
                .and_then(Value::as_u64)
                .ok_or_else(|| AppError::internal("call hierarchy requires `line`"))?;
            let character = action.get("character").and_then(Value::as_u64).unwrap_or(0);
            let root = selected_root_path(config, root_id.as_deref())?;
            let navigation = if tool == "symbol_incoming_calls" {
                coding_lsp::incoming_calls(&root, &path, line, character, config.full_pc_access)
                    .await?
            } else {
                coding_lsp::outgoing_calls(&root, &path, line, character, config.full_pc_access)
                    .await?
            };
            let result = serde_json::to_string(&navigation).map_err(|error| {
                AppError::internal(format!("failed to encode call hierarchy result: {error}"))
            })?;
            Ok(AgentTurnResult {
                trace_label: format!(
                    "{} calls at {path}:{}:{}",
                    if tool == "symbol_incoming_calls" {
                        "Found incoming"
                    } else {
                        "Found outgoing"
                    },
                    line + 1,
                    character
                ),
                transcript_result: bounded(&result, MAX_TOOL_RESULT_CHARS),
            })
        }
'''

agent = insert_before(
    agent,
    '        "symbol_definition" | "symbol_references" | "symbol_hover" => {\n',
    handler,
    "symbol navigation handler",
)
agent_path.write_text(agent, encoding="utf-8")

security_path = Path("src-tauri/src/openagent_security.rs")
security = security_path.read_text(encoding="utf-8")
security = replace_once(
    security,
    '        | "symbol_references" | "symbol_hover" | "symbol_outline" | "symbol_diagnostics"\n',
    '        | "symbol_references" | "symbol_hover" | "symbol_outline" | "symbol_incoming_calls"\n        | "symbol_outgoing_calls" | "symbol_diagnostics"\n',
    "read-only call hierarchy classification",
)
security = replace_once(
    security,
    '        let (decision, _) = authorize_tool("symbol_outline", &json!({}), ApprovalMode::AlwaysAsk);\n        assert_eq!(decision, PolicyDecision::Allow);\n',
    '        let (decision, _) = authorize_tool("symbol_outline", &json!({}), ApprovalMode::AlwaysAsk);\n        assert_eq!(decision, PolicyDecision::Allow);\n        let (decision, _) =\n            authorize_tool("symbol_incoming_calls", &json!({}), ApprovalMode::AlwaysAsk);\n        assert_eq!(decision, PolicyDecision::Allow);\n        let (decision, _) =\n            authorize_tool("symbol_outgoing_calls", &json!({}), ApprovalMode::AlwaysAsk);\n        assert_eq!(decision, PolicyDecision::Allow);\n',
    "read-only call hierarchy test",
)
security_path.write_text(security, encoding="utf-8")
