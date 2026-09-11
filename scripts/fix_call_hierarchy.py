from pathlib import Path

lsp_path = Path("src-tauri/src/coding_lsp.rs")
lsp = lsp_path.read_text(encoding="utf-8")
agent_path = Path("src-tauri/src/local_agent.rs")
agent = agent_path.read_text(encoding="utf-8")


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return text.replace(old, new, 1)


lsp = replace_once(
    lsp,
    '                            "documentSymbol": {\n                                "dynamicRegistration": false,\n                                "hierarchicalDocumentSymbolSupport": true,\n                                "tagSupport": {"valueSet": [1]}\n                            },\n                            "publishDiagnostics": {',
    '                            "documentSymbol": {\n                                "dynamicRegistration": false,\n                                "hierarchicalDocumentSymbolSupport": true,\n                                "tagSupport": {"valueSet": [1]}\n                            },\n                            "callHierarchy": {"dynamicRegistration": false},\n                            "publishDiagnostics": {',
    "client callHierarchy capability",
)

call_hierarchy_prefix = '''async fn call_hierarchy(
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

    if !allow_language_server {'''
call_hierarchy_prefix_hardened = '''async fn call_hierarchy(
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
    let lsp_position = validated_call_hierarchy_position(&text, line, character)?;

    if !allow_language_server {'''
lsp = replace_once(
    lsp,
    call_hierarchy_prefix,
    call_hierarchy_prefix_hardened,
    "call hierarchy position validation",
)

lsp = replace_once(
    lsp,
    '                        "textDocument": {"uri": uri},\n                        "position": {"line": line, "character": character}\n',
    '                        "textDocument": {"uri": uri},\n                        "position": {\n                            "line": lsp_position.line,\n                            "character": lsp_position.character\n                        }\n',
    "call hierarchy LSP position",
)

marker = 'fn call_hierarchy_unavailable(server: Option<String>, reason: &str) -> NavigationResult {\n'
helper = '''fn validated_call_hierarchy_position(\n    text: &str,\n    line: u64,\n    character: u64,\n) -> Result<LspPosition, AppError> {\n    if line == 0 {\n        return Err(AppError::internal(\n            "call hierarchy line is 1-based and must be >= 1",\n        ));\n    }\n    let line_index = usize::try_from(line)\n        .map_err(|_| AppError::internal("call hierarchy line is too large"))?;\n    let character_index = usize::try_from(character)\n        .map_err(|_| AppError::internal("call hierarchy character is too large"))?;\n    let line_text = source_line(text, line_index)?;\n    validate_character_position(line_text, character_index)?;\n    let utf16_character = utf16_character_offset(line_text, character_index)?;\n    let utf16_character = u64::try_from(utf16_character)\n        .map_err(|_| AppError::internal("call hierarchy UTF-16 character offset is too large"))?;\n    Ok(LspPosition {\n        line: line - 1,\n        character: utf16_character,\n    })\n}\n\n'''
if lsp.count(marker) != 1:
    raise SystemExit(f"call hierarchy helper marker count={lsp.count(marker)}")
lsp = lsp.replace(marker, helper + marker, 1)

lsp = replace_once(
    lsp,
    '    for item in items {\n        let Some(uri) = item.get("uri").and_then(Value::as_str) else {\n            continue;\n        };\n        if uri_is_scoped(root, uri)? && uri_matches_file(uri, file) {\n            return Ok(Some(item.clone()));\n        }\n    }',
    '    for item in items.iter().take(MAX_CALL_HIERARCHY_RESULTS) {\n        let Some(uri) = item.get("uri").and_then(Value::as_str) else {\n            continue;\n        };\n        if uri_is_scoped(root, uri)?\n            && uri_matches_file(uri, file)\n            && sanitize_call_hierarchy_item(root, item)?.is_some()\n        {\n            return Ok(Some(item.clone()));\n        }\n    }',
    "prepared item validation bound",
)

lsp = replace_once(
    lsp,
    '    if let Some(detail) = value.get("detail").and_then(Value::as_str) {\n        safe.insert(\n            "detail".to_string(),\n            Value::String(truncate_preview(detail, MAX_SYMBOL_DETAIL_CHARS)),\n        );\n    }\n    Ok(Some(Value::Object(safe)))\n}\n\npub async fn definition(',
    '    if let Some(detail) = value.get("detail").and_then(Value::as_str) {\n        safe.insert(\n            "detail".to_string(),\n            Value::String(truncate_preview(detail, MAX_SYMBOL_DETAIL_CHARS)),\n        );\n    }\n    if let Some(tags) = sanitize_symbol_tags(value.get("tags")) {\n        safe.insert("tags".to_string(), tags);\n    }\n    Ok(Some(Value::Object(safe)))\n}\n\npub async fn definition(',
    "call hierarchy safe tags",
)

test_marker = '    #[test]\n    fn call_hierarchy_capability_is_parsed() {\n'
tests = r'''    #[test]
    fn call_hierarchy_position_is_one_based_and_utf16_safe() {
        let position = validated_call_hierarchy_position("🙂Router\n", 1, 1).unwrap();
        assert_eq!(position.line, 0);
        assert_eq!(position.character, 2);
        assert!(validated_call_hierarchy_position("Router\n", 0, 0).is_err());
        assert!(validated_call_hierarchy_position("Router\n", 1, 99).is_err());
    }

    #[test]
    fn call_hierarchy_results_and_ranges_are_bounded() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let file = root.join("inside.rs");
        fs::write(&file, "fn caller() {}\n").unwrap();
        let uri = file_uri(&file).unwrap();
        let ranges = (0..(MAX_CALL_HIERARCHY_RANGES + 5))
            .map(|index| {
                json!({
                    "start": {"line": index, "character": 0},
                    "end": {"line": index, "character": 1}
                })
            })
            .collect::<Vec<_>>();
        let calls = (0..(MAX_CALL_HIERARCHY_RESULTS + 5))
            .map(|index| {
                json!({
                    "to": {
                        "name": format!("callee_{index}"),
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
                        "data": {"opaque": index}
                    },
                    "fromRanges": ranges
                })
            })
            .collect::<Vec<_>>();
        let safe = sanitize_call_hierarchy_result(
            &root,
            Value::Array(calls),
            CallHierarchyDirection::Outgoing,
        )
        .unwrap();
        let safe = safe.as_array().unwrap();
        assert_eq!(safe.len(), MAX_CALL_HIERARCHY_RESULTS);
        assert_eq!(
            safe[0]["fromRanges"].as_array().unwrap().len(),
            MAX_CALL_HIERARCHY_RANGES
        );
        assert!(safe[0]["to"].get("data").is_none());
    }

    #[test]
    fn prepared_call_hierarchy_data_is_internal_only_but_round_trippable() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let file = root.join("requested.rs");
        fs::write(&file, "fn requested() {}\n").unwrap();
        let result = json!([{
            "name": "requested",
            "kind": 12,
            "uri": file_uri(&file).unwrap(),
            "range": {
                "start": {"line": 0, "character": 0},
                "end": {"line": 0, "character": 15}
            },
            "selectionRange": {
                "start": {"line": 0, "character": 3},
                "end": {"line": 0, "character": 12}
            },
            "data": {"opaque": "server-token"}
        }]);
        let prepared = first_scoped_call_hierarchy_item(&root, &file, &result)
            .unwrap()
            .unwrap();
        assert_eq!(
            prepared.pointer("/data/opaque").and_then(Value::as_str),
            Some("server-token")
        );
        let visible = sanitize_call_hierarchy_item(&root, &prepared)
            .unwrap()
            .unwrap();
        assert!(visible.get("data").is_none());
    }

'''
if lsp.count(test_marker) != 1:
    raise SystemExit(f"test marker count={lsp.count(test_marker)}")
lsp = lsp.replace(test_marker, tests + test_marker, 1)

agent = replace_once(
    agent,
    'symbol_incoming_calls/symbol_outgoing_calls for language-server call relationships, and symbol_diagnostics for file diagnostics. A language server may run only when Full PC + Terminal access is enabled and its executable resolves from a trusted PATH location; compatible servers are reused through a bounded idle-evicted session pool with document synchronization and bounded background notification draining, otherwise bounded lexical indexing is used. Treat diagnostics with published=false as non-authoritative.',
    'symbol_incoming_calls/symbol_outgoing_calls for language-server call relationships, and symbol_diagnostics for file diagnostics. A language server may run only when Full PC + Terminal access is enabled and its executable resolves from a trusted PATH location; compatible servers are reused through a bounded idle-evicted session pool with document synchronization and bounded background notification draining. Call relationships are semantic-only and report LSP unavailability rather than pretending lexical search is equivalent. Other symbol navigation may use bounded lexical indexing when appropriate. Treat diagnostics with published=false as non-authoritative.',
    "call hierarchy prompt semantics",
)

agent = replace_once(
    agent,
    '                    line + 1,\n                    character\n',
    '                    line,\n                    character\n',
    "call hierarchy trace line",
)

lsp_path.write_text(lsp, encoding="utf-8")
agent_path.write_text(agent, encoding="utf-8")
