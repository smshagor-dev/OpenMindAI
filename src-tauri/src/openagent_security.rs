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
        (PolicyDecision::Allow, RiskLevel::Destructive) => {
            "trusted-workspace destructive operation"
        }
        (PolicyDecision::Allow, RiskLevel::HostExecution) => "trusted-workspace host command",
        (PolicyDecision::RequireApproval, RiskLevel::WorkspaceWrite) => {
            "workspace edits require approval in Always Ask mode"
        }
        (PolicyDecision::RequireApproval, RiskLevel::Destructive) => {
            "destructive filesystem operation requires approval"
        }
        (PolicyDecision::RequireApproval, RiskLevel::HostExecution) => {
            "host command requires approval"
        }
        (PolicyDecision::Deny, RiskLevel::Prohibited) => {
            "command violates the non-bypassable safety policy"
        }
        _ => "operation is not authorized by the active policy",
    };
    (decision, reason.to_string())
}

fn classify_tool(tool: &str, action: &Value) -> RiskLevel {
    match tool {
        "list_dir"
        | "read_file"
        | "search_text"
        | "symbol_search"
        | "symbol_definition"
        | "symbol_references"
        | "symbol_hover"
        | "symbol_outline"
        | "symbol_incoming_calls"
        | "symbol_outgoing_calls"
        | "symbol_diagnostics"
        | "git_status"
        | "git_diff" => RiskLevel::ReadOnly,
        "write_file" | "replace_text" | "patch_transaction" | "create_dir" => {
            RiskLevel::WorkspaceWrite
        }
        "move_path" | "delete_path" => RiskLevel::Destructive,
        "terminal" => classify_terminal(
            action
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            action
                .get("hostExecution")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ),
        _ => RiskLevel::Prohibited,
    }
}

fn classify_terminal(command: &str, host_execution: bool) -> RiskLevel {
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

pub use crate::isolated_runtime::SandboxCapability;

#[tauri::command]
pub fn openagent_sandbox_capability() -> SandboxCapability {
    crate::isolated_runtime::sandbox_capability()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn read_only_tools_never_prompt() {
        let (decision, _) = authorize_tool("read_file", &json!({}), ApprovalMode::AlwaysAsk);
        assert_eq!(decision, PolicyDecision::Allow);
        let (decision, _) =
            authorize_tool("symbol_diagnostics", &json!({}), ApprovalMode::AlwaysAsk);
        assert_eq!(decision, PolicyDecision::Allow);
        let (decision, _) = authorize_tool("symbol_outline", &json!({}), ApprovalMode::AlwaysAsk);
        assert_eq!(decision, PolicyDecision::Allow);
        let (decision, _) =
            authorize_tool("symbol_incoming_calls", &json!({}), ApprovalMode::AlwaysAsk);
        assert_eq!(decision, PolicyDecision::Allow);
        let (decision, _) =
            authorize_tool("symbol_outgoing_calls", &json!({}), ApprovalMode::AlwaysAsk);
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
                &json!({"command": "npm install", "hostExecution": true}),
                ApprovalMode::RiskBased
            )
            .0,
            PolicyDecision::RequireApproval
        );
    }

    #[test]
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
