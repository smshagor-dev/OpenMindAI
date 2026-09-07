use std::{env, path::PathBuf, process::Command};

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalMode {
    RiskBased,
    AlwaysAsk,
    TrustedWorkspace,
}

impl ApprovalMode {
    pub fn parse(value: &str) -> Self {
        match value {
            "always_ask" => Self::AlwaysAsk,
            "trusted_workspace" => Self::TrustedWorkspace,
            _ => Self::RiskBased,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    RequireApproval,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RiskLevel {
    ReadOnly,
    WorkspaceWrite,
    Destructive,
    HostExecution,
    Prohibited,
}

pub fn authorize_tool(tool: &str, action: &Value, mode: ApprovalMode) -> (PolicyDecision, String) {
    let risk = classify_tool(tool, action);
    let decision = match risk {
        RiskLevel::Prohibited => PolicyDecision::Deny,
        RiskLevel::ReadOnly => PolicyDecision::Allow,
        RiskLevel::WorkspaceWrite => match mode {
            ApprovalMode::AlwaysAsk => PolicyDecision::RequireApproval,
            ApprovalMode::RiskBased | ApprovalMode::TrustedWorkspace => PolicyDecision::Allow,
        },
        RiskLevel::Destructive | RiskLevel::HostExecution => match mode {
            ApprovalMode::TrustedWorkspace => PolicyDecision::Allow,
            ApprovalMode::RiskBased | ApprovalMode::AlwaysAsk => PolicyDecision::RequireApproval,
        },
    };
    let reason = match (decision, risk) {
        (PolicyDecision::Allow, RiskLevel::ReadOnly) => "read-only workspace operation",
        (PolicyDecision::Allow, RiskLevel::WorkspaceWrite) => "bounded workspace edit",
        (PolicyDecision::Allow, RiskLevel::Destructive) => "trusted-workspace destructive operation",
        (PolicyDecision::Allow, RiskLevel::HostExecution) => "trusted-workspace host command",
        (PolicyDecision::RequireApproval, RiskLevel::WorkspaceWrite) => "workspace edits require approval in Always Ask mode",
        (PolicyDecision::RequireApproval, RiskLevel::Destructive) => "destructive filesystem operation requires approval",
        (PolicyDecision::RequireApproval, RiskLevel::HostExecution) => "host command requires approval",
        (PolicyDecision::Deny, RiskLevel::Prohibited) => "command violates the non-bypassable safety policy",
        _ => "operation is not authorized by the active policy",
    };
    (decision, reason.to_string())
}

fn classify_tool(tool: &str, action: &Value) -> RiskLevel {
    match tool {
        "list_dir" | "read_file" | "search_text" | "git_status" | "git_diff" => {
            RiskLevel::ReadOnly
        }
        "write_file" | "replace_text" | "create_dir" => RiskLevel::WorkspaceWrite,
        "move_path" | "delete_path" => RiskLevel::Destructive,
        "terminal" => classify_terminal(
            action
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ),
        _ => RiskLevel::Prohibited,
    }
}

fn classify_terminal(command: &str) -> RiskLevel {
    let normalized = command.trim().to_ascii_lowercase();
    if normalized.is_empty()
        || normalized.contains("rm -rf /")
        || normalized.contains("format c:")
        || normalized.contains("remove-item -recurse c:\\")
        || normalized.contains("shutdown ")
        || normalized.contains("reboot")
    {
        return RiskLevel::Prohibited;
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
    if normalized.starts_with("npm run lint")
        || normalized.starts_with("npm test")
        || normalized.starts_with("cargo test")
        || normalized.starts_with("cargo clippy")
        || normalized.starts_with("python -m pytest")
        || normalized.starts_with("go test")
        || normalized.starts_with("dotnet test")
    {
        return RiskLevel::WorkspaceWrite;
    }
    RiskLevel::HostExecution
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxCapability {
    pub platform: String,
    pub provider: Option<String>,
    pub available: bool,
    pub strong_isolation: bool,
    pub message: String,
}

#[tauri::command]
pub fn openagent_sandbox_capability() -> SandboxCapability {
    detect_sandbox_capability()
}

fn detect_sandbox_capability() -> SandboxCapability {
    let platform = env::consts::OS.to_string();
    let provider = match env::consts::OS {
        "linux" if executable_on_path("bwrap") => Some("bubblewrap".to_string()),
        "macos" if executable_on_path("sandbox-exec") => Some("sandbox-exec".to_string()),
        "windows" if windows_sandbox_available() => Some("Windows Sandbox".to_string()),
        _ => None,
    };
    let available = provider.is_some();
    SandboxCapability {
        platform,
        provider,
        available,
        strong_isolation: false,
        message: if available {
            "Isolation provider detected; execution adapter and qualification are still required"
                .to_string()
        } else {
            "No supported local isolation provider was detected".to_string()
        },
    }
}

fn executable_on_path(name: &str) -> bool {
    env::var_os("PATH").is_some_and(|paths| {
        env::split_paths(&paths).any(|directory| {
            let candidate = directory.join(name);
            candidate.is_file()
                || (cfg!(windows) && directory.join(format!("{name}.exe")).is_file())
        })
    })
}

fn windows_sandbox_available() -> bool {
    let system_root = env::var_os("SystemRoot").map(PathBuf::from);
    let executable = system_root.map(|root| root.join("System32/WindowsSandbox.exe"));
    if !executable.as_ref().is_some_and(|path| path.is_file()) {
        return false;
    }
    Command::new("dism")
        .args([
            "/Online",
            "/Get-FeatureInfo",
            "/FeatureName:Containers-DisposableClientVM",
        ])
        .output()
        .ok()
        .is_some_and(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains("State : Enabled")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn read_only_tools_never_prompt() {
        let (decision, _) = authorize_tool("read_file", &json!({}), ApprovalMode::AlwaysAsk);
        assert_eq!(decision, PolicyDecision::Allow);
    }

    #[test]
    fn risk_based_policy_prompts_for_delete_and_host_commands() {
        assert_eq!(
            authorize_tool("delete_path", &json!({}), ApprovalMode::RiskBased).0,
            PolicyDecision::RequireApproval
        );
        assert_eq!(
            authorize_tool(
                "terminal",
                &json!({"command": "npm install"}),
                ApprovalMode::RiskBased
            )
            .0,
            PolicyDecision::RequireApproval
        );
    }

    #[test]
    fn catastrophic_commands_are_denied_even_in_trusted_mode() {
        assert_eq!(
            authorize_tool(
                "terminal",
                &json!({"command": "rm -rf /"}),
                ApprovalMode::TrustedWorkspace
            )
            .0,
            PolicyDecision::Deny
        );
    }
}
