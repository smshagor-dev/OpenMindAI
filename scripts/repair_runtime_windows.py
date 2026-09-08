from pathlib import Path

runtime = Path("src-tauri/src/isolated_runtime.rs")
text = runtime.read_text(encoding="utf-8")
needle = '''#[cfg(not(target_os = "windows"))]
async fn run_windows_sandbox(
'''
helper = '''#[cfg(target_os = "windows")]
fn timeout_result(
    _workspace_root: &Path,
    cwd: &Path,
    timeout_secs: u64,
    started: Instant,
    backend: &str,
) -> ShellExecutionResult {
    ShellExecutionResult {
        cwd: display_path(cwd),
        exit_code: -1,
        stdout: String::new(),
        stderr: format!("Command timed out after {timeout_secs} seconds."),
        duration_ms: started.elapsed().as_millis(),
        timed_out: true,
        truncated: false,
        backend: backend.to_string(),
        isolated: true,
        network_disabled: true,
    }
}

#[cfg(not(target_os = "windows"))]
async fn run_windows_sandbox(
'''
if needle not in text:
    raise SystemExit("Windows sandbox insertion anchor not found")
if "fn timeout_result(" not in text:
    text = text.replace(needle, helper, 1)
runtime.write_text(text, encoding="utf-8")

guards = Path("src-tauri/src/runtime_guards.rs")
text = guards.read_text(encoding="utf-8")
old = '''fn prepare_process_tree(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.as_std_mut().process_group(0);
    }
}
'''
new = '''#[cfg(unix)]
fn prepare_process_tree(command: &mut Command) {
    use std::os::unix::process::CommandExt as _;
    command.as_std_mut().process_group(0);
}

#[cfg(not(unix))]
fn prepare_process_tree(_command: &mut Command) {}
'''
if old not in text:
    raise SystemExit("process-tree normalization anchor not found")
text = text.replace(old, new, 1)
guards.write_text(text, encoding="utf-8")
