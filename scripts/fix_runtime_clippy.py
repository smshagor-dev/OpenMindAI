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


# Keep Duration target-specific after the Linux/macOS timeout paths are replaced
# by the shared process controller. Windows Sandbox still uses it for polling.
runtime_path = "src-tauri/src/isolated_runtime.rs"
runtime = read(runtime_path)
runtime = replace_once(
    runtime,
    "    time::{Duration, Instant},\n};\n",
    "    time::Instant,\n};\n\n#[cfg(target_os = \"windows\")]\nuse std::time::Duration;\n",
    "target-specific Duration import",
)
runtime = replace_once(
    runtime,
    "const WINDOWS_SANDBOX_MEMORY_MB: u32 = 4_096;\n",
    "#[cfg(target_os = \"windows\")]\nconst WINDOWS_SANDBOX_MEMORY_MB: u32 = 4_096;\n",
    "Windows sandbox memory constant",
)

start = runtime.find("fn timeout_result(\n")
end = runtime.find("fn host_shell_process(", start)
if start < 0 or end < 0:
    raise SystemExit("dead timeout helper span not found")
runtime = runtime[:start] + runtime[end:]
write(runtime_path, runtime)

# Avoid cfg-return fallthrough becoming unreachable code under -D warnings.
guards_path = "src-tauri/src/runtime_guards.rs"
guards = read(guards_path)
old_labels = '''pub fn resource_limit_labels() -> Vec<String> {
    #[cfg(unix)]
    {
        return vec![
            "CPU time bounded relative to the command timeout".to_string(),
            "address space <= 8 GiB".to_string(),
            "open files <= 1024".to_string(),
            "processes <= 256".to_string(),
            "single file <= 8 GiB".to_string(),
        ];
    }
    #[cfg(target_os = "windows")]
    {
        return vec![
            "Windows Sandbox memory <= 4096 MiB".to_string(),
            "wall-clock timeout with process-tree termination".to_string(),
            "bounded stdout/stderr capture".to_string(),
        ];
    }
    #[allow(unreachable_code)]
    vec!["wall-clock timeout with bounded output capture".to_string()]
}
'''
new_labels = '''#[cfg(unix)]
pub fn resource_limit_labels() -> Vec<String> {
    vec![
        "CPU time bounded relative to the command timeout".to_string(),
        "address space <= 8 GiB".to_string(),
        "open files <= 1024".to_string(),
        "processes <= 256".to_string(),
        "single file <= 8 GiB".to_string(),
    ]
}

#[cfg(target_os = "windows")]
pub fn resource_limit_labels() -> Vec<String> {
    vec![
        "Windows Sandbox memory <= 4096 MiB".to_string(),
        "wall-clock timeout with process-tree termination".to_string(),
        "bounded stdout/stderr capture".to_string(),
    ]
}

#[cfg(not(any(unix, target_os = "windows")))]
pub fn resource_limit_labels() -> Vec<String> {
    vec!["wall-clock timeout with bounded output capture".to_string()]
}
'''
guards = replace_once(guards, old_labels, new_labels, "platform resource labels")
write(guards_path, guards)

# This helper deliberately carries the request context explicitly; keep the
# lint exception local rather than weakening project-wide Clippy settings.
loop_path = "src-tauri/src/local_agent.rs"
loop_code = read(loop_path)
loop_code = replace_once(
    loop_code,
    "async fn request_agent_decision(\n",
    "#[allow(clippy::too_many_arguments)]\nasync fn request_agent_decision(\n",
    "decision helper lint scope",
)
write(loop_path, loop_code)
