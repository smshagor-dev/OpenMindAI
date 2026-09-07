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


def replace_span(value: str, start_marker: str, end_marker: str, replacement: str, label: str) -> str:
    start = value.find(start_marker)
    if start < 0:
        raise SystemExit(f"{label}: start marker missing")
    end = value.find(end_marker, start)
    if end < 0:
        raise SystemExit(f"{label}: end marker missing")
    return value[:start] + replacement + value[end:]


# Add the Unix rlimit dependency without affecting Windows builds.
cargo_path = "src-tauri/Cargo.toml"
cargo = read(cargo_path)
cargo = replace_once(
    cargo,
    "# Retained only so the existing lockfile remains reproducible while historical\n",
    "[target.'cfg(unix)'.dependencies]\nlibc = \"0.2\"\n\n# Retained only so the existing lockfile remains reproducible while historical\n",
    "unix libc dependency",
)
write(cargo_path, cargo)

# Register the shared process-control module.
lib_path = "src-tauri/src/lib.rs"
lib = read(lib_path)
lib = replace_once(
    lib,
    "mod isolated_runtime;\nmod local_agent;\n",
    "mod isolated_runtime;\nmod runtime_guards;\nmod local_agent;\n",
    "runtime module registration",
)
write(lib_path, lib)

# Make the process-control module portable across Linux/macOS resource constant types.
guards_path = "src-tauri/src/runtime_guards.rs"
guards = read(guards_path)
guards = replace_once(
    guards,
    '''#[cfg(unix)]
unsafe fn set_limit(resource: libc::__rlimit_resource_t, value: libc::rlim_t) -> io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: value,
        rlim_max: value,
    };
    // SAFETY: caller provides a valid resource constant and a live rlimit pointer.
    if unsafe { libc::setrlimit(resource, &limit) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
''',
    '''#[cfg(target_os = "linux")]
unsafe fn set_limit(resource: libc::__rlimit_resource_t, value: libc::rlim_t) -> io::Result<()> {
    let limit = libc::rlimit { rlim_cur: value, rlim_max: value };
    if unsafe { libc::setrlimit(resource, &limit) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
unsafe fn set_limit(resource: libc::c_int, value: libc::rlim_t) -> io::Result<()> {
    let limit = libc::rlimit { rlim_cur: value, rlim_max: value };
    if unsafe { libc::setrlimit(resource, &limit) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
''',
    "portable rlimit type",
)
write(guards_path, guards)

# Harden the isolation providers with bounded output, inherited resource ceilings,
# process-tree termination, and disposable scratch state.
runtime_path = "src-tauri/src/isolated_runtime.rs"
runtime = read(runtime_path)
runtime = replace_once(
    runtime,
    '#[cfg(target_os = "windows")]\nuse uuid::Uuid;\n\nuse crate::app_error::AppError;\n',
    '#[cfg(any(target_os = "windows", target_os = "macos"))]\nuse uuid::Uuid;\n\nuse crate::{app_error::AppError, runtime_guards};\n',
    "runtime imports",
)
runtime = replace_once(
    runtime,
    'const RUNTIME_CWD_MARKER: &str = "__OPENMIND_RUNTIME_CWD__";\n',
    'const RUNTIME_CWD_MARKER: &str = "__OPENMIND_RUNTIME_CWD__";\nconst WINDOWS_SANDBOX_MEMORY_MB: u32 = 4_096;\n',
    "windows memory constant",
)
runtime = replace_once(
    runtime,
    '''    pub strong_isolation: bool,
    pub message: String,
}''',
    '''    pub strong_isolation: bool,
    pub process_tree_control: bool,
    pub bounded_output: bool,
    pub disposable_scratch: bool,
    pub resource_limits: Vec<String>,
    pub message: String,
}''',
    "capability fields",
)
runtime = replace_once(
    runtime,
    '''        available: provider.is_some(),
        strong_isolation: provider.is_some(),
        message: provider''',
    '''        available: provider.is_some(),
        strong_isolation: provider.is_some(),
        process_tree_control: true,
        bounded_output: true,
        disposable_scratch: provider.is_some(),
        resource_limits: runtime_guards::resource_limit_labels(),
        message: provider''',
    "capability values",
)

host_start = "pub async fn run_host_shell("
host_end = "fn detected_provider() -> Option<IsolationProvider> {"
new_host = '''pub async fn run_host_shell(
    cwd: &Path,
    command: &str,
    timeout_secs: u64,
    max_output_chars: usize,
) -> Result<ShellExecutionResult, AppError> {
    let cwd = fs::canonicalize(cwd)?;
    if !cwd.is_dir() {
        return Err(AppError::internal("terminal working directory is not a directory"));
    }
    let command = command.trim();
    if command.is_empty() {
        return Err(AppError::internal("terminal command cannot be empty"));
    }

    let started = Instant::now();
    let process = host_shell_process(command, &cwd);
    let capture = runtime_guards::run_process(process, timeout_secs, max_output_chars).await?;
    let mut stdout = String::from_utf8_lossy(&capture.stdout).into_owned();
    let mut stderr = String::from_utf8_lossy(&capture.stderr).into_owned();
    if capture.timed_out {
        if !stderr.is_empty() && !stderr.ends_with('\\n') {
            stderr.push('\\n');
        }
        stderr.push_str(&format!("Command timed out after {timeout_secs} seconds."));
    }
    let resolved_cwd = take_cwd_marker(&mut stdout).unwrap_or_else(|| display_path(&cwd));
    let (stdout, stdout_truncated) = truncate_chars(&stdout, max_output_chars);
    let (stderr, stderr_truncated) = truncate_chars(&stderr, max_output_chars);
    Ok(ShellExecutionResult {
        cwd: resolved_cwd,
        exit_code: capture.exit_code,
        stdout,
        stderr,
        duration_ms: started.elapsed().as_millis(),
        timed_out: capture.timed_out,
        truncated: capture.truncated || stdout_truncated || stderr_truncated,
        backend: "host-explicit".to_string(),
        isolated: false,
        network_disabled: false,
    })
}

'''
runtime = replace_span(runtime, host_start, host_end, new_host, "host shell")

bubble_old = '''    let started = Instant::now();
    let output =
        match tokio::time::timeout(Duration::from_secs(timeout_secs), process.output()).await {
            Ok(result) => result?,
            Err(_) => {
                return Ok(timeout_result(
                    workspace_root,
                    cwd,
                    timeout_secs,
                    started,
                    "bubblewrap",
                ));
            }
        };
    finish_isolated_output(IsolatedOutput {
        workspace_root,
        cwd,
        exit_code: output.status.code().unwrap_or(-1),
        stdout: &output.stdout,
        stderr: &output.stderr,
        started,
        max_output_chars,
        backend: "bubblewrap",
        sandbox_workspace_prefix: Some("/workspace"),
    })
'''
bubble_new = '''    runtime_guards::apply_isolated_limits(&mut process, timeout_secs)?;
    let started = Instant::now();
    let output = runtime_guards::run_process(process, timeout_secs, max_output_chars).await?;
    finish_isolated_output(IsolatedOutput {
        workspace_root,
        cwd,
        exit_code: output.exit_code,
        stdout: &output.stdout,
        stderr: &output.stderr,
        started,
        max_output_chars,
        backend: "bubblewrap",
        sandbox_workspace_prefix: Some("/workspace"),
        timed_out: output.timed_out,
        pre_truncated: output.truncated,
        timeout_secs,
    })
'''
runtime = replace_once(runtime, bubble_old, bubble_new, "bubblewrap managed process")

runtime = replace_once(
    runtime,
    '''    let workspace = sandbox_profile_escape(&display_path(workspace_root));
    let profile = format!(
''',
    '''    let workspace = sandbox_profile_escape(&display_path(workspace_root));
    let scratch_dir = env::temp_dir().join(format!("openmindai-runtime-{}", Uuid::new_v4()));
    fs::create_dir_all(&scratch_dir)?;
    let scratch = sandbox_profile_escape(&display_path(&scratch_dir));
    let profile = format!(
''',
    "mac scratch setup",
)
runtime = replace_once(
    runtime,
    '''           (subpath \"/dev\")\n\\
           (subpath \"{workspace}\"))\n\\
         (allow file-write*\n\\
           (subpath \"{workspace}\")\n\\
           (subpath \"/private/tmp\")\n\\
           (subpath \"/tmp\"))\n\\
         (deny network*)"
''',
    '''           (subpath \"/dev\")\n\\
           (subpath \"{workspace}\")\n\\
           (subpath \"{scratch}\"))\n\\
         (allow file-write*\n\\
           (subpath \"{workspace}\")\n\\
           (subpath \"{scratch}\"))\n\\
         (deny network*)"
''',
    "mac scratch profile",
)
runtime = replace_once(
    runtime,
    '''        .env("HOME", "/private/tmp")
        .env("TMPDIR", "/private/tmp")
''',
    '''        .env("HOME", &scratch_dir)
        .env("TMPDIR", &scratch_dir)
''',
    "mac scratch env",
)
mac_old = '''    let started = Instant::now();
    let output =
        match tokio::time::timeout(Duration::from_secs(timeout_secs), process.output()).await {
            Ok(result) => result?,
            Err(_) => {
                return Ok(timeout_result(
                    workspace_root,
                    cwd,
                    timeout_secs,
                    started,
                    "sandbox-exec",
                ));
            }
        };
    finish_isolated_output(IsolatedOutput {
        workspace_root,
        cwd,
        exit_code: output.status.code().unwrap_or(-1),
        stdout: &output.stdout,
        stderr: &output.stderr,
        started,
        max_output_chars,
        backend: "sandbox-exec",
        sandbox_workspace_prefix: None,
    })
'''
mac_new = '''    runtime_guards::apply_isolated_limits(&mut process, timeout_secs)?;
    let started = Instant::now();
    let output = runtime_guards::run_process(process, timeout_secs, max_output_chars).await;
    let _ = fs::remove_dir_all(&scratch_dir);
    let output = output?;
    finish_isolated_output(IsolatedOutput {
        workspace_root,
        cwd,
        exit_code: output.exit_code,
        stdout: &output.stdout,
        stderr: &output.stderr,
        started,
        max_output_chars,
        backend: "sandbox-exec",
        sandbox_workspace_prefix: None,
        timed_out: output.timed_out,
        pre_truncated: output.truncated,
        timeout_secs,
    })
'''
runtime = replace_once(runtime, mac_old, mac_new, "mac managed process")
runtime = replace_once(
    runtime,
    '           <MemoryInMB>2048</MemoryInMB>\\r\\n\\\n',
    '           <MemoryInMB>{WINDOWS_SANDBOX_MEMORY_MB}</MemoryInMB>\\r\\n\\\n',
    "windows memory config",
)
runtime = replace_once(
    runtime,
    '''            let _ = child.kill().await;
            let _ = fs::remove_dir_all(&control_dir);
''',
    '''            runtime_guards::terminate_process_tree(&mut child).await;
            let _ = fs::remove_dir_all(&control_dir);
''',
    "windows timeout tree kill",
)
runtime = replace_once(
    runtime,
    '''    {
        let _ = child.kill().await;
    }
    let _ = fs::remove_dir_all(&control_dir);
''',
    '''    {
        runtime_guards::terminate_process_tree(&mut child).await;
    }
    let _ = fs::remove_dir_all(&control_dir);
''',
    "windows completion tree kill",
)
runtime = replace_once(
    runtime,
    '''    backend: &'a str,
    sandbox_workspace_prefix: Option<&'a str>,
}''',
    '''    backend: &'a str,
    sandbox_workspace_prefix: Option<&'a str>,
    timed_out: bool,
    pre_truncated: bool,
    timeout_secs: u64,
}''',
    "isolated output fields",
)
runtime = replace_once(
    runtime,
    '''        backend,
        sandbox_workspace_prefix,
    } = output;
    let mut stdout = String::from_utf8_lossy(stdout).into_owned();
    let stderr = String::from_utf8_lossy(stderr).into_owned();
''',
    '''        backend,
        sandbox_workspace_prefix,
        timed_out,
        pre_truncated,
        timeout_secs,
    } = output;
    let mut stdout = String::from_utf8_lossy(stdout).into_owned();
    let mut stderr = String::from_utf8_lossy(stderr).into_owned();
    if timed_out {
        if !stderr.is_empty() && !stderr.ends_with('\\n') {
            stderr.push('\\n');
        }
        stderr.push_str(&format!("Command timed out after {timeout_secs} seconds."));
    }
''',
    "isolated output timeout",
)
runtime = replace_once(
    runtime,
    '''        timed_out: false,
        truncated: stdout_truncated || stderr_truncated,
''',
    '''        timed_out,
        truncated: pre_truncated || stdout_truncated || stderr_truncated,
''',
    "isolated output status",
)
write(runtime_path, runtime)

# Make the configured strong-isolation mode meaningful: it disallows explicit host escape,
# while the legacy workspace mode keeps the current isolated-by-default behavior.
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
    '''                    step,
                    &model.id,
                ) => result?,
''',
    '''                    step,
                    &model.id,
                    &sandbox_mode,
                ) => result?,
''',
    "decision sandbox mode",
)
loop_code = replace_once(
    loop_code,
    '''                result = execute_tool(tool, &decision, &agent_context.workspace) => result,
''',
    '''                result = execute_tool(tool, &decision, &agent_context.workspace, &sandbox_mode) => result,
''',
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
    '''    let terminal_rule = if sandbox.available {
''',
    '''    let strict_isolation = sandbox_mode == "isolated_sandbox";
    let terminal_rule = if strict_isolation && !sandbox.available {
        "terminal is NOT AVAILABLE because strict isolation is selected but no qualified isolation provider is available. Never fall back to the host.".to_string()
    } else if sandbox.available {
''',
    "strict terminal prompt",
)
loop_code = replace_once(
    loop_code,
    '''        "You are OpenAgent, OpenMindAI's local coding agent. You operate directly on a user's local project only to fulfill the latest user request.\\n\\
''',
    '''        "You are OpenAgent, OpenMindAI's local coding agent. You operate directly on a user's local project only to fulfill the latest user request.\\n\\
Active sandbox policy: {sandbox_mode}. When isolated_sandbox is selected, explicit hostExecution is forbidden.\\n\\
''',
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

# Extend the typed capability contract and surface the controls in Settings.
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
    "enable qualified isolation option",
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
