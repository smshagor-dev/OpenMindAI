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
    ShellExecution,
    HostExecution,
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
        RiskLevel::RemoteMutation => PolicyDecision::RequireApproval,
        RiskLevel::Destructive | RiskLevel::ShellExecution | RiskLevel::HostExecution => match mode
        {
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
        (PolicyDecision::Allow, RiskLevel::ShellExecution) => "trusted-workspace shell command",
        (PolicyDecision::RequireApproval, RiskLevel::WorkspaceWrite) => {
            "workspace edits require approval in Always Ask mode"
        }
        (PolicyDecision::RequireApproval, RiskLevel::Destructive) => {
            "destructive filesystem operation requires approval"
        }
        (PolicyDecision::RequireApproval, RiskLevel::HostExecution) => {
            "host command requires approval"
        }
        (PolicyDecision::RequireApproval, RiskLevel::ShellExecution) => {
            "shell command requires exact approval; use read-only tools for inspection"
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
    if host_execution {
        return RiskLevel::HostExecution;
    }
    // The executor passes the entire string to a shell (POSIX or PowerShell).
    // Prefixes and command names cannot prove read-only behavior: operators,
    // substitutions, command options and workspace executables can all write.
    // Keep the read-only exemption on structured tools, never shell strings.
    RiskLevel::ShellExecution
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
    fn isolated_shell_commands_require_exact_approval() {
        for command in [
            "git status --short",
            "git diff",
            "git log -1",
            "rg TODO src",
            "grep TODO file.txt",
            "ls",
            "dir",
            "cargo test",
            "npm install",
        ] {
            for mode in [ApprovalMode::RiskBased, ApprovalMode::AlwaysAsk] {
                let (decision, reason) = authorize_tool(
                    "terminal",
                    &json!({"command": command, "hostExecution": false}),
                    mode,
                );
                assert_eq!(
                    decision,
                    PolicyDecision::RequireApproval,
                    "{mode:?}: {command}"
                );
                assert!(reason.contains("exact approval"));
            }
        }
    }

    #[test]
    fn shell_syntax_and_write_options_cannot_use_a_read_only_exemption() {
        for command in [
            "git status; printf changed > fixture.txt",
            "git diff && touch fixture.txt",
            "git log || touch fixture.txt",
            "ls & touch fixture.txt",
            "ls\nprintf changed > fixture.txt",
            "dir\r\nSet-Content fixture.txt changed",
            "ls > fixture.txt",
            "ls >> fixture.txt",
            "ls 2> fixture.txt",
            "ls | tee fixture.txt",
            "ls $(touch fixture.txt)",
            "ls `touch fixture.txt`",
            "ls <(touch fixture.txt)",
            "ls $([IO.File]::WriteAllText('fixture.txt', 'changed'))",
            "dir; Set-Content fixture.txt changed",
            "dir | Out-File fixture.txt",
            "ls --% & echo changed > fixture.txt",
            "git diff --output=fixture.txt",
            "git log --output=fixture.txt",
            "rg --pre=./fixture-preprocessor TODO",
            "git status-extra",
            "lsof-command",
            "directory-command",
            "Git Status; Set-Content fixture.txt changed",
        ] {
            for mode in [ApprovalMode::RiskBased, ApprovalMode::AlwaysAsk] {
                assert_eq!(
                    authorize_tool("terminal", &json!({"command": command}), mode).0,
                    PolicyDecision::RequireApproval,
                    "{mode:?}: {command}"
                );
            }
        }
    }

    #[test]
    fn trusted_workspace_retains_explicit_shell_trust() {
        for host_execution in [false, true] {
            assert_eq!(
                authorize_tool(
                    "terminal",
                    &json!({"command": "cargo test", "hostExecution": host_execution}),
                    ApprovalMode::TrustedWorkspace
                )
                .0,
                PolicyDecision::Allow
            );
        }
    }

    #[test]
    fn structured_inspection_and_write_policies_stay_separate() {
        for tool in [
            "list_dir",
            "read_file",
            "search_text",
            "git_status",
            "git_diff",
        ] {
            assert_eq!(
                authorize_tool(tool, &json!({}), ApprovalMode::AlwaysAsk).0,
                PolicyDecision::Allow,
                "{tool}"
            );
        }
        assert_eq!(
            authorize_tool("write_file", &json!({}), ApprovalMode::RiskBased).0,
            PolicyDecision::Allow
        );
        assert_eq!(
            authorize_tool("write_file", &json!({}), ApprovalMode::AlwaysAsk).0,
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
