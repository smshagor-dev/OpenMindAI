from pathlib import Path


def replace_once(source: str, old: str, new: str, label: str) -> str:
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return source.replace(old, new, 1)


# Harden the language-server bridge copied into the clean branch.
lsp_path = Path("src-tauri/src/coding_lsp.rs")
lsp = lsp_path.read_text(encoding="utf-8")
lsp = replace_once(
    lsp,
    "use std::{\n    fs,",
    "use std::{\n    env, fs,",
    "lsp env import",
)
lsp = replace_once(
    lsp,
    "pub async fn workspace_symbols(root: &Path, query: &str) -> Result<NavigationResult, AppError> {",
    "pub async fn workspace_symbols(\n    root: &Path,\n    query: &str,\n    allow_language_server: bool,\n) -> Result<NavigationResult, AppError> {",
    "workspace symbol permission",
)
lsp = replace_once(
    lsp,
    "    if let Some(spec) = select_server(&root, None) {",
    "    if allow_language_server && let Some(spec) = select_server(&root, None) {",
    "workspace server gate",
)
lsp = replace_once(
    lsp,
    "    character: u64,\n) -> Result<NavigationResult, AppError> {\n    position_navigation(\n        root,\n        relative_path,\n        line,\n        character,\n        NavigationKind::Definition,",
    "    character: u64,\n    allow_language_server: bool,\n) -> Result<NavigationResult, AppError> {\n    position_navigation(\n        root,\n        relative_path,\n        line,\n        character,\n        NavigationKind::Definition,\n        allow_language_server,",
    "definition server permission",
)
lsp = replace_once(
    lsp,
    "    character: u64,\n) -> Result<NavigationResult, AppError> {\n    position_navigation(\n        root,\n        relative_path,\n        line,\n        character,\n        NavigationKind::References,",
    "    character: u64,\n    allow_language_server: bool,\n) -> Result<NavigationResult, AppError> {\n    position_navigation(\n        root,\n        relative_path,\n        line,\n        character,\n        NavigationKind::References,\n        allow_language_server,",
    "references server permission",
)
lsp = replace_once(
    lsp,
    "    kind: NavigationKind,\n) -> Result<NavigationResult, AppError> {",
    "    kind: NavigationKind,\n    allow_language_server: bool,\n) -> Result<NavigationResult, AppError> {",
    "position server permission",
)
lsp = replace_once(
    lsp,
    "    if let Some(spec) = select_server(&root, Some(&file)) {",
    "    if allow_language_server && let Some(spec) = select_server(&root, Some(&file)) {",
    "position server gate",
)
lsp = replace_once(
    lsp,
    "        let mut process = Command::new(spec.command);",
    "        let executable = resolve_server_executable(root, spec.command)?;\n        let mut process = Command::new(executable);",
    "trusted server executable",
)
lsp = replace_once(
    lsp,
    '"clientInfo": {"name":"OpenMindAI OpenAgent","version":"1"},',
    '"clientInfo": {"name":"OpenMindAI Coding Workspace","version":"1"},',
    "lsp client identity",
)
helper = r'''
fn resolve_server_executable(root: &Path, name: &str) -> Result<PathBuf, AppError> {
    let path = env::var_os("PATH")
        .ok_or_else(|| AppError::internal("PATH is unavailable for language server discovery"))?;
    for directory in env::split_paths(&path) {
        if !directory.is_absolute() {
            continue;
        }
        let mut candidates = vec![directory.join(name)];
        if cfg!(windows) {
            candidates.push(directory.join(format!("{name}.exe")));
        }
        for candidate in candidates {
            if !candidate.is_file() {
                continue;
            }
            let Ok(executable) = fs::canonicalize(candidate) else {
                continue;
            };
            if executable.starts_with(root) {
                continue;
            }
            return Ok(executable);
        }
    }
    Err(AppError::internal(format!(
        "language server `{name}` is not installed in a trusted PATH location"
    )))
}

'''
lsp = replace_once(
    lsp,
    "impl LspSession {\n",
    helper + "impl LspSession {\n",
    "trusted server resolver",
)
lsp_path.write_text(lsp, encoding="utf-8")

# Register the two new coding modules.
lib_path = Path("src-tauri/src/lib.rs")
lib = lib_path.read_text(encoding="utf-8")
lib = replace_once(
    lib,
    "mod chat;\n",
    "mod chat;\nmod coding_lsp;\nmod coding_patch;\n",
    "module registration",
)
lib_path.write_text(lib, encoding="utf-8")

# Classify new tools under the existing risk engine.
security_path = Path("src-tauri/src/openagent_security.rs")
security = security_path.read_text(encoding="utf-8")
security = replace_once(
    security,
    '        "list_dir" | "read_file" | "search_text" | "git_status" | "git_diff" => RiskLevel::ReadOnly,\n        "write_file" | "replace_text" | "create_dir" => RiskLevel::WorkspaceWrite,',
    '        "list_dir"\n        | "read_file"\n        | "search_text"\n        | "symbol_search"\n        | "symbol_definition"\n        | "symbol_references"\n        | "git_status"\n        | "git_diff" => RiskLevel::ReadOnly,\n        "write_file" | "replace_text" | "patch_transaction" | "create_dir" => {\n            RiskLevel::WorkspaceWrite\n        }',
    "risk classification",
)
security_path.write_text(security, encoding="utf-8")

# Integrate the tools into the existing coding loop.
path = Path("src-tauri/src/local_agent.rs")
text = path.read_text(encoding="utf-8")
text = replace_once(
    text,
    "    openagent_context::build_repository_context,\n    openagent_runs::OpenAgentRunRepository,",
    "    coding_lsp, coding_patch,\n    openagent_context::build_repository_context,\n    openagent_runs::OpenAgentRunRepository,",
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
    "- Prefer symbol_search/symbol_definition/symbol_references for identifier navigation. A language server may run only when Full PC + Terminal access is enabled and its executable resolves from a trusted PATH location; otherwise bounded lexical indexing is used.\\n\\\n"
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
    '        "patch_transaction" => coding_patch::transaction_paths(action)?,\n        "move_path" => vec![\n',
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
            let navigation =
                coding_lsp::workspace_symbols(&root, &query, config.full_pc_access).await?;
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
                coding_lsp::definition(
                    &root,
                    &path,
                    line,
                    character,
                    config.full_pc_access,
                )
                .await?
            } else {
                coding_lsp::references(
                    &root,
                    &path,
                    line,
                    character,
                    config.full_pc_access,
                )
                .await?
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
            let outcome = coding_patch::apply_patch_transaction(&root, action)?;
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
print("patch and symbol integration applied")
