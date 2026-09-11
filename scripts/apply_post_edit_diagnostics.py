from pathlib import Path

path = Path("src-tauri/src/local_agent.rs")
text = path.read_text(encoding="utf-8")


def replace_once(old: str, new: str) -> None:
    global text
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"expected exactly one match, found {count}: {old[:120]!r}")
    text = text.replace(old, new, 1)


replace_once(
    'const MAX_TERMINAL_OUTPUT_CHARS: usize = 20_000;\nconst DEFAULT_TERMINAL_TIMEOUT_SECS: u64 = 180;\n',
    'const MAX_TERMINAL_OUTPUT_CHARS: usize = 20_000;\nconst MAX_POST_EDIT_DIAGNOSTIC_FILES: usize = 8;\nconst MAX_POST_EDIT_DIAGNOSTIC_PREVIEW: usize = 6;\nconst MAX_POST_EDIT_DIAGNOSTIC_CHARS: usize = 6_000;\nconst DEFAULT_TERMINAL_TIMEOUT_SECS: u64 = 180;\n',
)

replace_once(
    '            Ok(AgentTurnResult {\n                trace_label: format!("Wrote {} transactionally", display_path(&file)),\n                transcript_result: format!(\n                    "ok path={} chars={} transaction={}",\n                    display_path(&file),\n                    content.chars().count(),\n                    outcome.transaction_id\n                ),\n            })\n',
    '            let diagnostics = collect_post_edit_diagnostics(\n                config,\n                root_id.as_deref(),\n                "write_file",\n                action,\n            )\n            .await;\n            Ok(AgentTurnResult {\n                trace_label: format!("Wrote {} transactionally", display_path(&file)),\n                transcript_result: format!(\n                    "ok path={} chars={} transaction={}\\npost_edit_diagnostics={}",\n                    display_path(&file),\n                    content.chars().count(),\n                    outcome.transaction_id,\n                    diagnostics\n                ),\n            })\n',
)

replace_once(
    '            Ok(AgentTurnResult {\n                trace_label: format!("Updated {} transactionally", display_path(&file)),\n                transcript_result: format!(\n                    "ok path={} exact_replacements=1 transaction={}",\n                    display_path(&file),\n                    outcome.transaction_id\n                ),\n            })\n',
    '            let diagnostics = collect_post_edit_diagnostics(\n                config,\n                root_id.as_deref(),\n                "replace_text",\n                action,\n            )\n            .await;\n            Ok(AgentTurnResult {\n                trace_label: format!("Updated {} transactionally", display_path(&file)),\n                transcript_result: format!(\n                    "ok path={} exact_replacements=1 transaction={}\\npost_edit_diagnostics={}",\n                    display_path(&file),\n                    outcome.transaction_id,\n                    diagnostics\n                ),\n            })\n',
)

replace_once(
    '            let result = serde_json::to_string(&outcome).map_err(|error| {\n                AppError::internal(format!(\n                    "failed to encode patch_transaction result: {error}"\n                ))\n            })?;\n            Ok(AgentTurnResult {\n                trace_label: format!(\n                    "Applied patch transaction {} across {} files",\n                    outcome.transaction_id, outcome.changed_files\n                ),\n                transcript_result: bounded(&result, MAX_TOOL_RESULT_CHARS),\n            })\n',
    '            let result = serde_json::to_string(&outcome).map_err(|error| {\n                AppError::internal(format!(\n                    "failed to encode patch_transaction result: {error}"\n                ))\n            })?;\n            let diagnostics = collect_post_edit_diagnostics(\n                config,\n                root_id.as_deref(),\n                "patch_transaction",\n                action,\n            )\n            .await;\n            Ok(AgentTurnResult {\n                trace_label: format!(\n                    "Applied patch transaction {} across {} files",\n                    outcome.transaction_id, outcome.changed_files\n                ),\n                transcript_result: bounded(\n                    &format!("{result}\\npost_edit_diagnostics={diagnostics}"),\n                    MAX_TOOL_RESULT_CHARS,\n                ),\n            })\n',
)

marker = 'async fn run_terminal(\n'
insert = r'''fn post_edit_candidate_paths(tool: &str, action: &Value) -> Result<Vec<String>, AppError> {
    let raw = match tool {
        "write_file" | "replace_text" => vec![required_string(action, "path")?],
        "patch_transaction" => coding_patch::transaction_paths(action)?,
        _ => Vec::new(),
    };
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    for path in raw {
        if Path::new(&path).is_absolute() || !seen.insert(path.clone()) {
            continue;
        }
        paths.push(path);
    }
    Ok(paths)
}

async fn collect_post_edit_diagnostics(
    config: &AgentWorkspaceConfig,
    root_id: Option<&str>,
    tool: &str,
    action: &Value,
) -> String {
    let candidates = match post_edit_candidate_paths(tool, action) {
        Ok(paths) => paths,
        Err(error) => {
            return json!({
                "status": "unavailable",
                "published": false,
                "reason": one_line(&error.to_string(), 240),
            })
            .to_string();
        }
    };
    if candidates.is_empty() {
        return json!({
            "status": "skipped",
            "published": false,
            "reason": "no relative file paths were changed",
        })
        .to_string();
    }
    let root = match selected_root_path(config, root_id) {
        Ok(root) => root,
        Err(error) => {
            return json!({
                "status": "unavailable",
                "published": false,
                "reason": one_line(&error.to_string(), 240),
            })
            .to_string();
        }
    };
    let requested = candidates.len();
    let truncated_files = requested > MAX_POST_EDIT_DIAGNOSTIC_FILES;
    let mut results = Vec::new();
    for relative_path in candidates
        .into_iter()
        .take(MAX_POST_EDIT_DIAGNOSTIC_FILES)
    {
        let resolved = match resolve_agent_path(config, root_id, &relative_path, false) {
            Ok(path) => path,
            Err(error) => {
                results.push(json!({
                    "path": relative_path,
                    "status": "unavailable",
                    "published": false,
                    "reason": one_line(&error.to_string(), 240),
                }));
                continue;
            }
        };
        if !resolved.is_file() {
            results.push(json!({
                "path": relative_path,
                "status": "skipped",
                "published": false,
                "reason": "changed path is not a regular file after the mutation",
            }));
            continue;
        }
        match coding_lsp::diagnostics(&root, &relative_path, config.full_pc_access).await {
            Ok(navigation) => {
                let published = navigation
                    .result
                    .get("published")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let diagnostics = navigation
                    .result
                    .get("diagnostics")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let diagnostic_count = diagnostics.len();
                let preview = diagnostics
                    .into_iter()
                    .take(MAX_POST_EDIT_DIAGNOSTIC_PREVIEW)
                    .collect::<Vec<_>>();
                let reason = navigation
                    .result
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(|value| one_line(value, 240));
                results.push(json!({
                    "path": relative_path,
                    "status": if published { "published" } else { "non_authoritative" },
                    "engine": navigation.engine,
                    "server": navigation.server,
                    "published": published,
                    "diagnosticCount": diagnostic_count,
                    "diagnosticsTruncated": diagnostic_count > MAX_POST_EDIT_DIAGNOSTIC_PREVIEW,
                    "diagnostics": preview,
                    "reason": reason,
                }));
            }
            Err(error) => {
                results.push(json!({
                    "path": relative_path,
                    "status": "unavailable",
                    "published": false,
                    "reason": one_line(&error.to_string(), 240),
                }));
            }
        }
    }
    bounded(
        &json!({
            "status": "completed",
            "requestedFiles": requested,
            "checkedFiles": results.len(),
            "filesTruncated": truncated_files,
            "results": results,
        })
        .to_string(),
        MAX_POST_EDIT_DIAGNOSTIC_CHARS,
    )
}

'''
if text.count(marker) != 1:
    raise SystemExit(f"expected exactly one run_terminal marker, found {text.count(marker)}")
text = text.replace(marker, insert + marker, 1)

replace_once(
    '        assert!(first.transcript_result.contains("transaction="));\n',
    '        assert!(first.transcript_result.contains("transaction="));\n        assert!(first.transcript_result.contains("post_edit_diagnostics="));\n        assert!(first.transcript_result.contains("\\\"published\\\":false"));\n',
)

replace_once(
    '        assert!(result.transcript_result.contains("transaction="));\n        assert_eq!(\n            fs::read_to_string(temp.path().join("file.txt")).unwrap(),\n',
    '        assert!(result.transcript_result.contains("transaction="));\n        assert!(result.transcript_result.contains("post_edit_diagnostics="));\n        assert!(result.transcript_result.contains("\\\"published\\\":false"));\n        assert_eq!(\n            fs::read_to_string(temp.path().join("file.txt")).unwrap(),\n',
)

needle = '    #[test]\n    fn openagent_prefers_nemotron_35_lightning() {\n'
tests = r'''    #[test]
    fn post_edit_candidate_paths_are_deduplicated_and_leave_bounding_to_collection() {
        let action = json!({
            "operations": [
                {"op": "create", "path": "a.rs", "content": "fn a() {}"},
                {"op": "write", "path": "a.rs", "content": "fn a() { println!(\"a\"); }"},
                {"op": "create", "path": "b.rs", "content": "fn b() {}"}
            ]
        });
        let paths = post_edit_candidate_paths("patch_transaction", &action).unwrap();
        assert_eq!(paths, vec!["a.rs".to_string(), "b.rs".to_string()]);
    }

    #[tokio::test]
    async fn post_edit_diagnostics_are_bounded_and_non_fatal_without_lsp_access() {
        let temp = tempfile::tempdir().unwrap();
        let config = test_workspace_config(temp.path());
        let operations = (0..(MAX_POST_EDIT_DIAGNOSTIC_FILES + 2))
            .map(|index| {
                json!({
                    "op": "create",
                    "path": format!("file-{index}.rs"),
                    "content": format!("fn item_{index}() {{}}"),
                })
            })
            .collect::<Vec<_>>();
        let action = json!({"rootId": "root", "operations": operations});
        let outcome = execute_tool("patch_transaction", &action, &config, "auto")
            .await
            .unwrap();
        assert!(outcome.transcript_result.contains("post_edit_diagnostics="));
        assert!(outcome.transcript_result.contains("\"filesTruncated\":true"));
        assert!(outcome.transcript_result.contains("\"published\":false"));
    }

'''
if text.count(needle) != 1:
    raise SystemExit(f"expected exactly one test insertion point, found {text.count(needle)}")
text = text.replace(needle, tests + needle, 1)

path.write_text(text, encoding="utf-8")
