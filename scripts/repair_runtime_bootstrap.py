from pathlib import Path


def read(path: str) -> str:
    return Path(path).read_text(encoding="utf-8")


def write(path: str, value: str) -> None:
    Path(path).write_text(value, encoding="utf-8")


def replace_once(value: str, old: str, new: str, label: str) -> str:
    count = value.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one target, found {count}")
    return value.replace(old, new, 1)


# Pre-patch structures that are intentionally formatting-sensitive so the main
# integration script can stay focused on stable runtime changes.
runtime_path = Path("src-tauri/src/isolated_runtime.rs")
lines = runtime_path.read_text(encoding="utf-8").splitlines(keepends=True)

mac_start = next((i for i, line in enumerate(lines) if "async fn run_macos_sandbox(" in line), -1)
if mac_start < 0:
    raise SystemExit("macOS runtime function not found")

read_workspace = next(
    (i for i in range(mac_start, len(lines)) if "{workspace}" in lines[i] and "))" in lines[i]),
    -1,
)
if read_workspace < 0:
    raise SystemExit("macOS workspace read rule not found")
read_line = lines[read_workspace]
lines[read_workspace] = read_line.replace("))", ")", 1)
lines.insert(read_workspace + 1, read_line.replace("{workspace}", "{scratch}"))

write_marker = next(
    (i for i in range(read_workspace + 2, len(lines)) if "(allow file-write*" in lines[i]),
    -1,
)
if write_marker < 0:
    raise SystemExit("macOS write rule not found")
private_tmp = next((i for i in range(write_marker, len(lines)) if "/private/tmp" in lines[i]), -1)
plain_tmp = next(
    (i for i in range(write_marker, len(lines)) if '(subpath \\"/tmp\\"))' in lines[i]),
    -1,
)
if private_tmp < 0 or plain_tmp < 0:
    raise SystemExit("macOS temporary write rules not found")
lines[private_tmp] = lines[plain_tmp].replace("/tmp", "{scratch}")
del lines[plain_tmp]

finish_start = next((i for i, line in enumerate(lines) if "fn finish_isolated_output(" in line), -1)
finish_end = next(
    (i for i in range(finish_start + 1, len(lines)) if lines[i].startswith("fn timeout_result(")),
    -1,
)
if finish_start < 0 or finish_end < 0:
    raise SystemExit("shared isolation output span not found")
status_index = next(
    (
        i
        for i in range(finish_start, finish_end)
        if "timed_out: false," in lines[i]
        and i + 1 < finish_end
        and "truncated: stdout_truncated || stderr_truncated," in lines[i + 1]
    ),
    -1,
)
if status_index < 0:
    raise SystemExit("shared isolation output status not found")
indent = lines[status_index].split("timed_out", 1)[0]
lines[status_index] = f"{indent}timed_out,\n"
lines[status_index + 1] = f"{indent}truncated: pre_truncated || stdout_truncated || stderr_truncated,\n"
runtime_path.write_text("".join(lines), encoding="utf-8")

# Wire the persisted isolation policy into the existing coding loop structurally.
loop_path = "src-tauri/src/local_agent.rs"
loop_code = read(loop_path)
loop_code = replace_once(
    loop_code,
    '''    let approval_mode = {
        let db = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        ApprovalMode::parse(
            &SettingsRepository::new(&db)
                .get_preferences()?
                .openagent_approval_mode,
        )
    };
''',
    '''    let (approval_mode, sandbox_mode) = {
        let db = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        let preferences = SettingsRepository::new(&db).get_preferences()?;
        (
            ApprovalMode::parse(&preferences.openagent_approval_mode),
            preferences.openagent_sandbox_mode,
        )
    };
''',
    "load sandbox preference",
)
loop_code = replace_once(
    loop_code,
    '''                step,
                &model.id,
            ) => result?,
''',
    '''                step,
                &model.id,
                &sandbox_mode,
            ) => result?,
''',
    "decision sandbox mode",
)
loop_code = replace_once(
    loop_code,
    'result = execute_tool(tool, &decision, &agent_context.workspace) => result,',
    'result = execute_tool(tool, &decision, &agent_context.workspace, &sandbox_mode) => result,',
    "tool sandbox mode",
)
loop_code = replace_once(
    loop_code,
    '''    step: usize,
    model_id: &str,
) -> Result<Value, AppError> {
''',
    '''    step: usize,
    model_id: &str,
    sandbox_mode: &str,
) -> Result<Value, AppError> {
''',
    "decision signature",
)
loop_code = replace_once(
    loop_code,
    '    let terminal_rule = if sandbox.available {\n',
    '''    let strict_isolation = sandbox_mode == "isolated_sandbox";
    let terminal_rule = if strict_isolation && !sandbox.available {
        "terminal is NOT AVAILABLE because strict isolation is selected but no qualified isolation provider is available. Never fall back to the host.".to_string()
    } else if sandbox.available {
''',
    "strict terminal prompt",
)
loop_code = replace_once(
    loop_code,
    '        "You are OpenAgent, OpenMindAI\'s local coding agent. You operate directly on a user\'s local project only to fulfill the latest user request.\\n\\\n',
    '        "You are OpenAgent, OpenMindAI\'s local coding agent. You operate directly on a user\'s local project only to fulfill the latest user request.\\n\\\nActive sandbox policy: {sandbox_mode}. When isolated_sandbox is selected, explicit hostExecution is forbidden.\\n\\\n',
    "sandbox policy prompt",
)
loop_code = replace_once(
    loop_code,
    '''async fn execute_tool(
    tool: &str,
    action: &Value,
    config: &AgentWorkspaceConfig,
) -> Result<AgentTurnResult, AppError> {
''',
    '''async fn execute_tool(
    tool: &str,
    action: &Value,
    config: &AgentWorkspaceConfig,
    sandbox_mode: &str,
) -> Result<AgentTurnResult, AppError> {
''',
    "execute signature",
)
loop_code = replace_once(
    loop_code,
    '''            let host_execution = action
                .get("hostExecution")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if host_execution && !config.full_pc_access {
''',
    '''            let host_execution = action
                .get("hostExecution")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if host_execution && sandbox_mode == "isolated_sandbox" {
                return Err(AppError::internal(
                    "strict isolation mode forbids hostExecution; switch the project execution policy explicitly if host access is required",
                ));
            }
            if host_execution && !config.full_pc_access {
''',
    "strict host escape block",
)
write(loop_path, loop_code)

# Extend the frontend capability contract and make the strict mode selectable only
# when the backend reports a qualified isolation provider.
types_path = "src/types.ts"
types = read(types_path)
types = replace_once(
    types,
    '''  available: boolean;
  strongIsolation: boolean;
  message: string;
}''',
    '''  available: boolean;
  strongIsolation: boolean;
  processTreeControl: boolean;
  boundedOutput: boolean;
  disposableScratch: boolean;
  resourceLimits: string[];
  message: string;
}''',
    "capability TypeScript fields",
)
write(types_path, types)

ui_path = "src/components/AgentSettings.tsx"
ui = read(ui_path)
ui = replace_once(
    ui,
    '<option value="isolated_sandbox" disabled>Isolated sandbox (installation pending)</option>',
    '<option value="isolated_sandbox" disabled={!sandbox?.available}>Strong isolation · no host escape</option>',
    "qualified isolation option",
)
ui = replace_once(
    ui,
    '''          Isolation provider: {sandbox?.provider ?? "none"} · {sandbox?.message ?? "detecting"}.
          Only qualified execution modes are selectable.
''',
    '''          Isolation provider: {sandbox?.provider ?? "none"} · {sandbox?.message ?? "detecting"}.
          {sandbox?.resourceLimits?.length ? ` Limits: ${sandbox.resourceLimits.join(" · ")}.` : ""}
          {sandbox?.processTreeControl ? " Process-tree cleanup enabled." : ""}
          {sandbox?.boundedOutput ? " Output capture is bounded." : ""}
''',
    "capability UI details",
)
write(ui_path, ui)

# Remove the formatting-sensitive blocks from the secondary integration script;
# they have already been applied above. Keep the remaining stable runtime edits.
bootstrap_path = Path("scripts/bootstrap_runtime_guards.py")
text = bootstrap_path.read_text(encoding="utf-8")
for label in [
    '    "mac scratch profile",\n)',
    '    "isolated output status",\n)',
]:
    pos = text.find(label)
    if pos < 0:
        raise SystemExit(f"bootstrap patch label not found: {label}")
    start = text.rfind("runtime = replace_once(", 0, pos)
    end = text.find("runtime = replace_once(", pos + len(label))
    if start < 0:
        raise SystemExit(f"bootstrap patch start not found: {label}")
    if end < 0:
        end = text.find("write(runtime_path, runtime)", pos)
    if end < 0:
        raise SystemExit(f"bootstrap patch end not found: {label}")
    text = text[:start] + text[end:]

local_section = text.find("# Make the configured strong-isolation mode meaningful")
if local_section < 0:
    raise SystemExit("coding-loop bootstrap section not found")
text = text[:local_section]
bootstrap_path.write_text(text, encoding="utf-8")
