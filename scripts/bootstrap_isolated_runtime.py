from pathlib import Path


def read(path: str) -> str:
    return Path(path).read_text(encoding="utf-8")


def write(path: str, value: str) -> None:
    Path(path).write_text(value, encoding="utf-8")


def replace_once(value: str, old: str, new: str, label: str) -> str:
    count = value.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one target, found {count}")
    return value.replace(old, new, 1)


def replace_span(value: str, start_marker: str, end_marker: str, replacement: str, label: str) -> str:
    start = value.find(start_marker)
    if start < 0:
        raise SystemExit(f"{label}: start marker not found")
    end = value.find(end_marker, start)
    if end < 0:
        raise SystemExit(f"{label}: end marker not found")
    return value[:start] + replacement + value[end:]


# Register the shared runtime backend.
lib_path = "src-tauri/src/lib.rs"
lib = read(lib_path)
lib = replace_once(
    lib,
    "mod github_workspace;\n",
    "mod github_workspace;\nmod isolated_runtime;\n",
    "lib module registration",
)
write(lib_path, lib)


# Make the risk engine distinguish isolated workspace execution from explicit host execution,
# and reuse the runtime's real provider capability instead of maintaining duplicate detection.
security_path = "src-tauri/src/openagent_security.rs"
security = read(security_path)
security = replace_once(
    security,
    'use std::{env, path::PathBuf, process::Command};\n\nuse serde::Serialize;\nuse serde_json::Value;\n',
    'use serde_json::Value;\n',
    "security imports",
)
old_terminal_arm = '''        "terminal" => classify_terminal(
            action
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ),
'''
new_terminal_arm = '''        "terminal" => classify_terminal(
            action
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            action
                .get("hostExecution")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ),
'''
security = replace_once(security, old_terminal_arm, new_terminal_arm, "terminal risk arm")
classify_start = "fn classify_terminal(command: &str) -> RiskLevel {"
classify_end = "#[derive(Debug, Clone, Serialize)]"
new_classifier = '''fn classify_terminal(command: &str, host_execution: bool) -> RiskLevel {
    let normalized = command.trim().to_ascii_lowercase();
    if normalized.is_empty()
        || normalized.contains("rm -rf /")
        || normalized.contains("format c:")
        || normalized.contains("remove-item -recurse c:\\\\")
        || normalized.contains("shutdown ")
        || normalized.contains("reboot")
    {
        return RiskLevel::Prohibited;
    }
    if host_execution {
        return RiskLevel::HostExecution;
    }
    if normalized.starts_with("git status")
        || normalized.starts_with("git diff")
        || normalized.starts_with("git log")
        || normalized.starts_with("rg ")
        || normalized.starts_with("grep ")
        || normalized.starts_with("ls")
        || normalized.starts_with("dir")
    {
        return RiskLevel::ReadOnly;
    }
    if normalized.contains("rm -rf ")
        || normalized.starts_with("remove-item ")
        || normalized.starts_with("git reset --hard")
        || normalized.starts_with("git clean -")
    {
        return RiskLevel::Destructive;
    }
    RiskLevel::WorkspaceWrite
}

'''
security = replace_span(security, classify_start, classify_end, new_classifier, "terminal classifier")
sandbox_start = "#[derive(Debug, Clone, Serialize)]"
sandbox_end = "#[cfg(test)]"
new_sandbox_bridge = '''pub use crate::isolated_runtime::SandboxCapability;

#[tauri::command]
pub fn openagent_sandbox_capability() -> SandboxCapability {
    crate::isolated_runtime::sandbox_capability()
}

'''
security = replace_span(
    security,
    sandbox_start,
    sandbox_end,
    new_sandbox_bridge,
    "sandbox capability bridge",
)
security = replace_once(
    security,
    '&json!({"command": "npm install"}),',
    '&json!({"command": "npm install", "hostExecution": true}),',
    "host execution policy test",
)
insert_test_anchor = '''    #[test]
    fn catastrophic_commands_are_denied_even_in_trusted_mode() {
'''
new_isolated_test = '''    #[test]
    fn isolated_terminal_build_is_a_workspace_write() {
        assert_eq!(
            authorize_tool(
                "terminal",
                &json!({"command": "cargo test", "hostExecution": false}),
                ApprovalMode::RiskBased
            )
            .0,
            PolicyDecision::Allow
        );
        assert_eq!(
            authorize_tool(
                "terminal",
                &json!({"command": "cargo test", "hostExecution": false}),
                ApprovalMode::AlwaysAsk
            )
            .0,
            PolicyDecision::RequireApproval
        );
    }

'''
security = replace_once(
    security,
    insert_test_anchor,
    new_isolated_test + insert_test_anchor,
    "isolated policy test",
)
write(security_path, security)


# Route coding-loop terminal actions through fail-closed isolation by default.
agent_path = "src-tauri/src/local_agent.rs"
agent = read(agent_path)
agent = replace_once(
    agent,
    '    inference::{StreamChunkEvent, StreamDoneEvent, StreamStartedEvent},\n',
    '    inference::{StreamChunkEvent, StreamDoneEvent, StreamStartedEvent},\n    isolated_runtime,\n',
    "agent runtime import",
)
agent = replace_once(
    agent,
    "        terminal_enabled: workspace.full_pc_access,\n",
    "        terminal_enabled: attached_roots > 0\n            && (workspace.full_pc_access || isolated_runtime::sandbox_capability().available),\n",
    "agent terminal status",
)
prompt_start = "    let terminal_rule = if context.workspace.full_pc_access {"
prompt_end = "    let system = format!("
new_prompt_prelude = '''    let sandbox = isolated_runtime::sandbox_capability();
    let terminal_rule = if sandbox.available {
        format!(
            "terminal is AVAILABLE. Default terminal actions run inside {} with the attached workspace as the only writable project mount and networking disabled. Set hostExecution=true only when host access is genuinely required; hostExecution requires Full PC + Terminal permission.",
            sandbox.provider.as_deref().unwrap_or("the local isolation backend")
        )
    } else if context.workspace.full_pc_access {
        "No strong isolation provider is available. terminal may only run when hostExecution=true, using the explicit Full PC + Terminal grant; never assume an isolated action will fall back to the host.".to_string()
    } else {
        "terminal is NOT AVAILABLE because no strong isolation provider is installed and Full PC + Terminal access is disabled. Use filesystem tools only.".to_string()
    };
    let platform_rule = if cfg!(target_os = "windows") {
        "Host OS: Windows. Isolated execution uses the Windows Sandbox disposable microVM when available. Explicit host execution uses non-interactive Windows PowerShell."
    } else if cfg!(target_os = "macos") {
        "Host OS: macOS. Isolated execution uses sandbox-exec with network disabled. Explicit host execution uses /bin/sh -lc."
    } else {
        "Host OS: Linux. Isolated execution uses bubblewrap with network disabled. Explicit host execution uses /bin/sh -lc."
    };

'''
agent = replace_span(agent, prompt_start, prompt_end, new_prompt_prelude, "agent terminal prompt prelude")
agent = replace_once(
    agent,
    '{{\\"type\\":\\"tool\\",\\"tool\\":\\"terminal\\",\\"rootId\\":\\"ID\\",\\"cwd\\":\\"relative/or/absolute\\",\\"command\\":\\"command\\",\\"timeoutSec\\":180}}\\n\\\n',
    '{{\\"type\\":\\"tool\\",\\"tool\\":\\"terminal\\",\\"rootId\\":\\"ID\\",\\"cwd\\":\\"relative/path\\",\\"command\\":\\"command\\",\\"timeoutSec\\":180,\\"hostExecution\\":false}}\\n\\\n',
    "agent terminal schema",
)
agent = replace_once(
    agent,
    '- After edits, validate with appropriate tests/build/lint when terminal is available. Run validation commands one at a time so each exit code is authoritative. If validation fails, inspect the error, change approach, fix, and rerun until green or a concrete blocker is established.\\n\\\n',
    '- terminal defaults to strong isolated workspace execution with networking disabled and no inherited host secrets. Never request hostExecution unless isolation cannot satisfy a task that the user explicitly authorized.\\n\\\n- After edits, validate with appropriate tests/build/lint when terminal is available. Run validation commands one at a time so each exit code is authoritative. If validation fails, inspect the error, change approach, fix, and rerun until green or a concrete blocker is established.\\n\\\n',
    "agent isolation rule",
)
agent = replace_once(
    agent,
    '- Absolute paths are only allowed when Full PC access is enabled.\\n\\\n',
    agent_rule := '- Absolute file paths are only allowed when Full PC access is enabled. Isolated terminal cwd must stay inside its attached root; absolute host cwd requires hostExecution=true and Full PC permission.\\n\\\n',
    "agent absolute path rule",
)
execute_start = agent.find('async fn execute_tool(')
if execute_start < 0:
    raise SystemExit("agent execute_tool not found")
terminal_start = agent.find('        "terminal" => {\n', execute_start)
terminal_end = agent.find('        other => Err(', terminal_start)
if terminal_start < 0 or terminal_end < 0:
    raise SystemExit("agent terminal tool arm not found")
new_terminal_arm = '''        "terminal" => {
            let host_execution = action
                .get("hostExecution")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if host_execution && !config.full_pc_access {
                return Err(AppError::internal(
                    "hostExecution requires Full PC + Terminal access for this project",
                ));
            }
            let root_id = optional_string(action, "rootId");
            let cwd = optional_string(action, "cwd").unwrap_or_default();
            let command = required_string(action, "command")?;
            let timeout_secs = terminal_timeout_secs(action);
            let result = run_terminal(
                config,
                root_id.as_deref(),
                &cwd,
                &command,
                timeout_secs,
                host_execution,
            )
            .await?;
            if result.timed_out || result.exit_code != 0 {
                return Err(AppError::internal(format!(
                    "terminal command failed via {} in {} (exit {}, timed_out={}, isolated={}, network_disabled={}):\\nstdout:\\n{}\\nstderr:\\n{}",
                    result.backend,
                    result.cwd,
                    result.exit_code,
                    result.timed_out,
                    result.isolated,
                    result.network_disabled,
                    bounded(&result.stdout, 6_000),
                    bounded(&result.stderr, 6_000)
                )));
            }
            Ok(AgentTurnResult {
                trace_label: format!(
                    "Ran `{}` via {} (exit {})",
                    one_line(&command, 120),
                    result.backend,
                    result.exit_code
                ),
                transcript_result: bounded(
                    &format!(
                        "backend={}\\nisolated={}\\nnetwork_disabled={}\\ncwd={}\\nexit_code={}\\ntimed_out={}\\nstdout:\\n{}\\nstderr:\\n{}",
                        result.backend,
                        result.isolated,
                        result.network_disabled,
                        result.cwd,
                        result.exit_code,
                        result.timed_out,
                        result.stdout,
                        result.stderr
                    ),
                    MAX_TOOL_RESULT_CHARS,
                ),
            })
        }
'''
agent = agent[:terminal_start] + new_terminal_arm + agent[terminal_end:]
run_start = agent.find("#[derive(Debug)]\nstruct AgentTerminalResult")
run_end = agent.find("async fn run_git_command", run_start)
if run_start < 0 or run_end < 0:
    raise SystemExit("agent run_terminal helper span not found")
new_run_terminal = '''async fn run_terminal(
    config: &AgentWorkspaceConfig,
    root_id: Option<&str>,
    cwd: &str,
    command: &str,
    timeout_secs: u64,
    host_execution: bool,
) -> Result<isolated_runtime::ShellExecutionResult, AppError> {
    let command = command.trim();
    if command.is_empty() {
        return Err(AppError::internal("terminal command cannot be empty"));
    }
    if command.chars().count() > MAX_TERMINAL_COMMAND_CHARS {
        return Err(AppError::internal(
            "terminal command exceeds the safety limit",
        ));
    }
    reject_catastrophic_command(command)?;

    if host_execution {
        if !config.full_pc_access {
            return Err(AppError::internal(
                "hostExecution requires Full PC + Terminal access",
            ));
        }
        let start_dir = if cwd.trim().is_empty() {
            selected_root_path(config, root_id)?
        } else {
            resolve_agent_path(config, root_id, cwd, true)?
        };
        return isolated_runtime::run_host_shell(
            &start_dir,
            command,
            timeout_secs,
            MAX_TERMINAL_OUTPUT_CHARS,
        )
        .await;
    }

    let workspace_root = selected_root_path(config, root_id)?;
    let start_dir = if cwd.trim().is_empty() {
        workspace_root.clone()
    } else {
        let supplied = Path::new(cwd.trim());
        if supplied.is_absolute() {
            let canonical = fs::canonicalize(supplied)?;
            if !canonical.starts_with(&workspace_root) {
                return Err(AppError::internal(
                    "isolated terminal cwd cannot leave the selected workspace root",
                ));
            }
            canonical
        } else {
            let candidate = fs::canonicalize(workspace_root.join(supplied))?;
            if !candidate.starts_with(&workspace_root) {
                return Err(AppError::internal(
                    "isolated terminal cwd escaped the selected workspace root",
                ));
            }
            candidate
        }
    };
    if !start_dir.is_dir() {
        return Err(AppError::internal(
            "terminal working directory is not a directory",
        ));
    }
    isolated_runtime::run_isolated_shell(
        &workspace_root,
        &start_dir,
        command,
        timeout_secs,
        MAX_TERMINAL_OUTPUT_CHARS,
    )
    .await
}

'''
agent = agent[:run_start] + new_run_terminal + agent[run_end:]
process_start = agent.find("fn terminal_process(command: &str, cwd: &Path) -> Command")
process_end = agent.find("fn emit_agent_chunk", process_start)
if process_start < 0 or process_end < 0:
    raise SystemExit("agent legacy terminal process span not found")
agent = agent[:process_start] + agent[process_end:]
write(agent_path, agent)


# Route the visible project terminal through the same backend. Without Full PC permission it
# automatically uses strong isolation; Full PC remains the explicit opt-in host shell path.
workspace_path = "src-tauri/src/local_workspace.rs"
workspace = read(workspace_path)
workspace = replace_once(
    workspace,
    "use tokio::process::Command;\n",
    "",
    "workspace legacy command import",
)
workspace = replace_once(
    workspace,
    "    database::Database,\n",
    "    database::Database,\n    isolated_runtime,\n",
    "workspace runtime import",
)
workspace = replace_once(
    workspace,
    "    pub truncated: bool,\n",
    "    pub truncated: bool,\n    pub backend: String,\n    pub isolated: bool,\n    pub network_disabled: bool,\n",
    "workspace terminal result metadata",
)
workspace = replace_once(
    workspace,
    "        terminal_enabled: config.full_pc_access,\n",
    "        terminal_enabled: !config.roots.is_empty()\n            && (config.full_pc_access || isolated_runtime::sandbox_capability().available),\n",
    "workspace terminal status",
)
workspace_run_start = "#[tauri::command]\npub async fn run_project_terminal_command("
workspace_run_end = "pub(crate) fn workspace_context_for_project"
new_workspace_run = '''#[tauri::command]
pub async fn run_project_terminal_command(
    project_id: String,
    root_id: Option<String>,
    cwd: String,
    command: String,
    approved: bool,
    state: State<'_, AppState>,
) -> Result<TerminalCommandResult, AppError> {
    if !approved {
        return Err(AppError::internal(
            "running a terminal command requires explicit approval",
        ));
    }
    let command_text = command.trim();
    if command_text.is_empty() {
        return Err(AppError::internal("terminal command cannot be empty"));
    }
    if command_text.chars().count() > MAX_TERMINAL_COMMAND_CHARS {
        return Err(AppError::internal(
            "terminal command exceeds the safety limit",
        ));
    }

    let config = load_project_config(&state, &project_id)?;
    let workspace_root = selected_root_path(&config, root_id.as_deref())?;
    let start_dir = if cwd.trim().is_empty() {
        workspace_root.clone()
    } else if !config.full_pc_access && Path::new(cwd.trim()).is_absolute() {
        let canonical = fs::canonicalize(cwd.trim())?;
        if !canonical.starts_with(&workspace_root) {
            return Err(AppError::internal(
                "isolated terminal working directory must stay inside the attached workspace",
            ));
        }
        canonical
    } else {
        resolve_path(&config, root_id.as_deref(), &cwd, true)?
    };
    if !start_dir.is_dir() {
        return Err(AppError::internal(
            "terminal working directory is not a directory",
        ));
    }

    let result = if config.full_pc_access {
        isolated_runtime::run_host_shell(
            &start_dir,
            command_text,
            TERMINAL_TIMEOUT_SECS,
            MAX_TERMINAL_OUTPUT_CHARS,
        )
        .await?
    } else {
        isolated_runtime::run_isolated_shell(
            &workspace_root,
            &start_dir,
            command_text,
            TERMINAL_TIMEOUT_SECS,
            MAX_TERMINAL_OUTPUT_CHARS,
        )
        .await?
    };

    Ok(TerminalCommandResult {
        command: command_text.to_string(),
        cwd: result.cwd,
        exit_code: result.exit_code,
        stdout: result.stdout,
        stderr: result.stderr,
        duration_ms: result.duration_ms,
        timed_out: result.timed_out,
        truncated: result.truncated,
        backend: result.backend,
        isolated: result.isolated,
        network_disabled: result.network_disabled,
    })
}

'''
workspace = replace_span(
    workspace,
    workspace_run_start,
    workspace_run_end,
    new_workspace_run,
    "workspace terminal command",
)
legacy_process_start = workspace.find("fn terminal_process(command: &str, cwd: &Path) -> Command")
legacy_process_end = workspace.find("fn collect_workspace_paths", legacy_process_start)
if legacy_process_start < 0 or legacy_process_end < 0:
    raise SystemExit("workspace legacy terminal process span not found")
workspace = workspace[:legacy_process_start] + workspace[legacy_process_end:]
write(workspace_path, workspace)


# Surface execution provenance in the desktop terminal type and UI.
api_path = "src/lib/localWorkspace.ts"
api = read(api_path)
api = replace_once(
    api,
    "  truncated: boolean;\n}",
    "  truncated: boolean;\n  backend: string;\n  isolated: boolean;\n  networkDisabled: boolean;\n}",
    "terminal TypeScript metadata",
)
write(api_path, api)

ui_path = "src/components/ProjectLocalWorkspace.tsx"
ui = read(ui_path)
ui = replace_once(
    ui,
    "          <div className={status.fullPcAccess ? \"local-terminal unlocked\" : \"local-terminal locked\"}>",
    "          <div className={status.terminalEnabled ? \"local-terminal unlocked\" : \"local-terminal locked\"}>",
    "terminal panel lock state",
)
ui = replace_once(
    ui,
    '<span><strong>Project Terminal</strong><small>{status.fullPcAccess ? terminalCwd || activeRoot?.path : "Full PC access is disabled"}</small></span>',
    '<span><strong>Project Terminal</strong><small>{status.fullPcAccess ? terminalCwd || activeRoot?.path : status.terminalEnabled ? `${terminalCwd || activeRoot?.path} · isolated workspace` : "No strong isolation provider available"}</small></span>',
    "terminal heading mode",
)
ui = replace_once(
    ui,
    '{status.fullPcAccess ? <span className="terminal-access-pill"><ShieldCheck size={12} /> OS permissions</span> : <span className="terminal-access-pill locked"><LockKeyhole size={12} /> Locked</span>}',
    '{status.fullPcAccess ? <span className="terminal-access-pill"><ShieldCheck size={12} /> Explicit host shell</span> : status.terminalEnabled ? <span className="terminal-access-pill"><ShieldCheck size={12} /> Isolated · network off</span> : <span className="terminal-access-pill locked"><LockKeyhole size={12} /> Locked</span>}',
    "terminal access pill",
)
terminal_panel_start = ui.find('          <div className={status.terminalEnabled ? "local-terminal unlocked"')
if terminal_panel_start < 0:
    raise SystemExit("terminal UI panel not found after lock-state update")
condition_index = ui.find("            {status.fullPcAccess ? (\n", terminal_panel_start)
if condition_index < 0:
    raise SystemExit("terminal UI render condition not found")
ui = ui[:condition_index] + ui[condition_index:].replace(
    "            {status.fullPcAccess ? (\n",
    "            {status.terminalEnabled ? (\n",
    1,
)
ui = replace_once(
    ui,
    '<small>exit {item.exitCode} · {item.durationMs} ms{item.truncated ? " · output truncated" : ""}</small>',
    '<small>{item.backend} · {item.isolated ? "isolated" : "host"}{item.networkDisabled ? " · network off" : ""} · exit {item.exitCode} · {item.durationMs} ms{item.truncated ? " · output truncated" : ""}</small>',
    "terminal result provenance",
)
ui = replace_once(
    ui,
    '<div className="terminal-empty">Terminal commands run with the current OS user permissions.</div>',
    '<div className="terminal-empty">{status.fullPcAccess ? "Host commands use the current OS-user permissions." : "Commands run inside the detected strong isolation backend with networking disabled."}</div>',
    "terminal empty copy",
)
ui = replace_once(
    ui,
    '<div><strong>Terminal is intentionally locked by default.</strong><span>Enable Full PC + Terminal access to run arbitrary local commands with your user account permissions.</span></div>',
    '<div><strong>No strong isolation provider is available.</strong><span>Install/enable the platform isolation backend, or explicitly enable Full PC access if you intentionally need an unsandboxed host shell.</span></div>',
    "terminal locked copy",
)
ui = replace_once(
    ui,
    '<p>Attach real PC folders. OpenMindAI can browse and edit them directly; terminal access is a separate explicit grant.</p>',
    '<p>Attach real PC folders. File tools stay folder-scoped; supported systems can run terminal commands inside strong network-off isolation without granting Full PC access.</p>',
    "workspace heading copy",
)
ui = replace_once(
    ui,
    '<LockKeyhole size={15} /> Enable Full PC + Terminal',
    '<LockKeyhole size={15} /> Enable Full PC / Host shell',
    "full access button copy",
)
write(ui_path, ui)


# Opening a folder must stay safe-by-default; host access can still be enabled explicitly later.
open_button_path = "src/components/OpenFolderProjectButton.tsx"
open_button = read(open_button_path)
confirm_start = open_button.find("      const grantFullAccess = window.confirm(\n")
confirm_end_marker = "\n\n      conversation = await props.onCreateProjectChat(project);"
confirm_end = open_button.find(confirm_end_marker, confirm_start)
if confirm_start < 0 or confirm_end < 0:
    raise SystemExit("open-folder full-access prompt span not found")
open_button = open_button[:confirm_start] + "      // Keep newly attached projects scoped by default. Host access remains an explicit later opt-in.\n" + open_button[confirm_end:]
write(open_button_path, open_button)
