use serde_json::{json, Value};
use tauri::State;

use crate::{app_error::AppError, AppState};

const MAX_DELIVERY_RESULT_CHARS: usize = 16_000;
const MAX_LOG_DIAGNOSTIC_CHARS: usize = 8_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryRisk {
    ReadOnly,
    RemoteMutation,
    Merge,
}

pub fn operation_risk(operation: &str) -> Option<DeliveryRisk> {
    match operation {
        "branches" | "pull_request" | "checks" | "check_jobs" | "check_logs" => {
            Some(DeliveryRisk::ReadOnly)
        }
        "create_branch"
        | "commit_files"
        | "create_pull_request"
        | "update_pull_request"
        | "rerun_checks" => Some(DeliveryRisk::RemoteMutation),
        "merge_pull_request" => Some(DeliveryRisk::Merge),
        _ => None,
    }
}

pub fn policy_requires_approval(operation: &str) -> bool {
    !matches!(operation_risk(operation), Some(DeliveryRisk::ReadOnly))
}

pub async fn execute(
    state: &State<'_, AppState>,
    operation: &str,
    params: Value,
    approved: bool,
) -> Result<Value, AppError> {
    operation_risk(operation).ok_or_else(|| {
        AppError::internal(format!("unsupported delivery operation: {operation}"))
    })?;
    if policy_requires_approval(operation) && !approved {
        return Err(AppError::internal(
            "remote delivery mutation requires an exact approved action",
        ));
    }
    let action = action_name(operation)?;
    let value =
        crate::execute_github_workspace_action(action.to_string(), params, approved, state.clone())
            .await?;
    if operation == "check_logs" && value.to_string().chars().count() > MAX_DELIVERY_RESULT_CHARS {
        return Ok(json!({
            "truncated": true,
            "diagnostic": summarize_check_failure(&value),
        }));
    }
    Ok(bound_value(value, MAX_DELIVERY_RESULT_CHARS))
}

fn action_name(operation: &str) -> Result<&'static str, AppError> {
    match operation {
        "branches" => Ok("branches.list"),
        "pull_request" => Ok("pr.get"),
        "checks" => Ok("actions.runs"),
        "check_jobs" => Ok("actions.jobs"),
        "check_logs" => Ok("actions.job_logs"),
        "create_branch" => Ok("branch.create"),
        "commit_files" => Ok("commit.multi_file"),
        "create_pull_request" => Ok("pr.create"),
        "update_pull_request" => Ok("pr.update"),
        "rerun_checks" => Ok("actions.rerun"),
        "merge_pull_request" => Ok("pr.merge"),
        _ => Err(AppError::internal("unsupported delivery operation")),
    }
}

pub fn summarize_check_failure(value: &Value) -> String {
    let raw = value.to_string();
    let mut important = Vec::new();
    for line in raw.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("error")
            || lower.contains("failed")
            || lower.contains("failure")
            || lower.contains("panic")
            || lower.contains("warning")
            || lower.contains("exit code")
        {
            important.push(line.trim().to_string());
        }
        if important.len() >= 40 {
            break;
        }
    }
    if important.is_empty() {
        bounded(&raw, MAX_LOG_DIAGNOSTIC_CHARS)
    } else {
        bounded(&important.join("\n"), MAX_LOG_DIAGNOSTIC_CHARS)
    }
}

pub fn completion_gate(
    local_validation_passed: bool,
    check_states: &[String],
) -> Result<(), AppError> {
    if !local_validation_passed {
        return Err(AppError::internal(
            "delivery merge gate rejected: local validation has not passed",
        ));
    }
    if check_states.is_empty() {
        return Err(AppError::internal(
            "delivery merge gate rejected: no repository check result was supplied",
        ));
    }
    let failing = check_states.iter().filter(|state| {
        !matches!(
            state.trim().to_ascii_lowercase().as_str(),
            "success" | "passed" | "skipped" | "neutral"
        )
    });
    let failures = failing.cloned().collect::<Vec<_>>();
    if !failures.is_empty() {
        return Err(AppError::internal(format!(
            "delivery merge gate rejected: repository checks are not green ({})",
            failures.join(", ")
        )));
    }
    Ok(())
}

fn bound_value(value: Value, max_chars: usize) -> Value {
    let raw = value.to_string();
    if raw.chars().count() <= max_chars {
        return value;
    }
    json!({
        "truncated": true,
        "summary": bounded(&raw, max_chars)
    })
}

fn bounded(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out = value.chars().take(max_chars).collect::<String>();
    out.push_str("…[truncated]");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_mutations_require_approval() {
        assert!(!policy_requires_approval("checks"));
        assert!(policy_requires_approval("create_pull_request"));
        assert!(policy_requires_approval("merge_pull_request"));
    }

    #[test]
    fn merge_gate_requires_local_and_remote_success() {
        assert!(completion_gate(true, &["success".into(), "skipped".into()]).is_ok());
        assert!(completion_gate(false, &["success".into()]).is_err());
        assert!(completion_gate(true, &["failure".into()]).is_err());
        assert!(completion_gate(true, &[]).is_err());
    }

    #[test]
    fn failure_summary_prefers_diagnostic_lines() {
        let value = json!({"logs": "build started\nerror: missing symbol\nProcess completed with exit code 1"});
        let summary = summarize_check_failure(&value);
        assert!(summary.contains("error"));
        assert!(summary.contains("exit code"));
    }
}
