from pathlib import Path

path = Path("src-tauri/src/local_agent.rs")
text = path.read_text(encoding="utf-8")


def replace_once(source: str, old: str, new: str, label: str) -> str:
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return source.replace(old, new, 1)


text = replace_once(
    text,
    "    openagent_context::build_repository_context,\n    openagent_runs::OpenAgentRunRepository,",
    "    openagent_context::build_repository_context,\n    openagent_lsp, openagent_patch,\n    openagent_runs::OpenAgentRunRepository,",
    "imports",
)

text = replace_once(
    text,
    '{{\\"type\\":\\"tool\\",\\"tool\\":\\"search_text\\",\\"rootId\\":\\"ID\\",\\"path\\":\\"optional/subdir\\",\\"query\\":\\"needle\\"}}\\n\\\n',
    '{{\\"type\\":\\"tool\\",\\"tool\\":\\"search_text\\",\\"rootId\\":\\"ID\\",\\"path\\":\\"optional/subdir\\",\\"query\\":\\"needle\\"}}\\n\\\n'
    '{{\\"type\\":\\"tool\\",\\"tool\\":\\"symbol_search\\",\\"rootId\\":\\"ID\\",\\"query\\":\\"symbol name\\"}}\\n\\\n'
    '{{\\"type\\":\\"tool\\",\\"tool\\":\\"symbol_definition\\",\\"rootId\\":\\"ID\\",\\"path\\":\\"file\\",\\"line\\":1,\\"character\\":0}}\\n\\\n'
    '{{\\"type\\":\\"tool\\",\\"tool\\":\\"symbol_references\\",\\"rootId\\":\\"ID\\",\\"path\\":\\"file\\",\\"line\\":1,\\"character\\":0}}\\n\\\n',
    "prompt symbol tools",
)

text = replace_once(
    text,
    '{{\\"type\\":\\"tool\\",\\"tool\\":\\"replace_text\\",\\"rootId\\":\\"ID\\",\\"path\\":\\"file\\",\\"old\\":\\"exact old text\\",\\"new\\":\\"replacement\\"}}\\n\\\n',
    '{{\\"type\\":\\"tool\\",\\"tool\\":\\"replace_text\\",\\"rootId\\":\\"ID\\",\\"path\\":\\"file\\",\\"old\\":\\"exact old text\\",\\"new\\":\\"replacement\\"}}\\n\\\n'
    '{{\\"type\\":\\"tool\\",\\"tool\\":\\"patch_transaction\\",\\"rootId\\":\\"ID\\",\\"operations\\":[{\\"op\\":\\"replace\\",\\"path\\":\\"file\\",\\"old\\":\\"exact old text\\",\\"new\\":\\"replacement\\"},{\\"op\\":\\"create\\",\\"path\\":\\"new/file\\",\\"content\\":\\"complete content\\"}]}}\\n\\\n',
    "prompt patch tool",
)

text = replace_once(
    text,
    "- Prefer replace_text for targeted edits and write_file for new/small files.\\n\\\n",
    "- Prefer symbol_search/symbol_definition/symbol_references for identifier navigation; they use an installed language server when available and fall back to bounded lexical indexing.\\n\\\n"
    "- Prefer patch_transaction for coordinated edits across multiple files. Every operation is preflighted before commit and the host rolls the entire batch back on failure.\\n\\\n"
    "- Prefer replace_text for a single targeted edit and write_file for new/small files.\\n\\\n",
    "prompt rules",
)

capture_start = text.index("fn capture_checkpoint_entries(")
capture_end = text.index("\nfn capture_checkpoint_path(", capture_start)
capture = text[capture_start:capture_end]
capture = replace_once(
    capture,
    '        "move_path" => vec![\n',
    '        "patch_transaction" => openagent_patch::transaction_paths(action)?,\n        "move_path" => vec![\n',
    "checkpoint transaction paths",
)
text = text[:capture_start] + capture + text[capture_end:]

execute_start = text.index("async fn execute_tool(")
execute_end = text.index("\n#[derive(Debug)]\nstruct AgentTerminalResult", execute_start)
execute = text[execute_start:execute_end]

symbol_arms = r'''        "symbol_search" => {
            let root_id = optional_string(action, "rootId");
            let query = required_string(action, "query")?;
            let root = selected_root_path(config, root_id.as_deref())?;
            let navigation = openagent_lsp::workspace_symbols(&root, &query).await?;
            let result = serde_json::to_string(&navigation).map_err(|error| {
                AppError::internal(format!("failed to encode symbol_search result: {error}"))
            })?;
            Ok(AgentTurnResult {
                trace_label: format!("Searched symbols for `{}`", one_line(&query, 80)),
                transcript_result: bounded(&result, MAX_TOOL_RESULT_CHARS),
            })
        }
        "symbol_definition" | "symbol_references" => {
            let root_id = optional_string(action, "rootId");
            let path = required_string(action, "path")?;
            let line = action
                .get("line")
                .and_then(Value::as_u64)
                .ok_or_else(|| AppError::internal("symbol navigation requires `line`"))?;
            let character = action
                .get("character")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let root = selected_root_path(config, root_id.as_deref())?;
            let navigation = if tool == "symbol_definition" {
                openagent_lsp::definition(&root, &path, line, character).await?
            } else {
                openagent_lsp::references(&root, &path, line, character).await?
            };
            let result = serde_json::to_string(&navigation).map_err(|error| {
                AppError::internal(format!("failed to encode symbol navigation result: {error}"))
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
        }
'''
execute = replace_once(
    execute,
    '        "write_file" => {\n',
    symbol_arms + '        "write_file" => {\n',
    "execute symbol arms",
)

patch_arm = r'''        "patch_transaction" => {
            let root_id = optional_string(action, "rootId");
            let root = selected_root_path(config, root_id.as_deref())?;
            let outcome = openagent_patch::apply_patch_transaction(&root, action)?;
            let result = serde_json::to_string(&outcome).map_err(|error| {
                AppError::internal(format!("failed to encode patch_transaction result: {error}"))
            })?;
            Ok(AgentTurnResult {
                trace_label: format!(
                    "Applied patch transaction {} across {} files",
                    outcome.transaction_id, outcome.changed_files
                ),
                transcript_result: bounded(&result, MAX_TOOL_RESULT_CHARS),
            })
        }
'''
execute = replace_once(
    execute,
    '        "create_dir" => {\n',
    patch_arm + '        "create_dir" => {\n',
    "execute patch arm",
)
text = text[:execute_start] + execute + text[execute_end:]

text = replace_once(
    text,
    '        "write_file" | "replace_text" | "create_dir" | "move_path" | "delete_path"\n',
    '        "write_file"\n            | "replace_text"\n            | "patch_transaction"\n            | "create_dir"\n            | "move_path"\n            | "delete_path"\n',
    "mutation classifier",
)

path.write_text(text, encoding="utf-8")
print("local_agent.rs patched successfully")
