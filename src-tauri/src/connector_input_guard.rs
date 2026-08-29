use serde_json::Value;
use tauri::State;

use crate::{app_error::AppError, connector_stabilization, AppState};

const AUTO_SHA_PLACEHOLDERS: &[&str] = &["FILE_BLOB_SHA", "AUTO", "auto"];
const MAX_MIME_TYPE_CHARS: usize = 255;

fn connector_error(message: impl Into<String>) -> AppError {
    AppError::internal(format!("Connected actions: {}", message.into()))
}

fn normalize_github_params(action: &str, mut params: Value) -> Value {
    if matches!(action, "file.put" | "file.delete") {
        let remove_sha = params
            .get("sha")
            .and_then(Value::as_str)
            .is_some_and(|value| {
                let trimmed = value.trim();
                trimmed.is_empty() || AUTO_SHA_PLACEHOLDERS.contains(&trimmed)
            });
        if remove_sha {
            if let Some(object) = params.as_object_mut() {
                object.remove("sha");
            }
        }
    }
    params
}

fn validate_google_params(action: &str, params: &Value) -> Result<(), AppError> {
    if !matches!(action, "drive.create" | "drive.update") {
        return Ok(());
    }
    let has_content = params.get("content").and_then(Value::as_str).is_some()
        || params
            .get("contentBase64")
            .and_then(Value::as_str)
            .is_some();
    if !has_content {
        return Ok(());
    }
    if let Some(mime_type) = params.get("mimeType").and_then(Value::as_str) {
        let mime_type = mime_type.trim();
        if mime_type.is_empty()
            || mime_type.len() > MAX_MIME_TYPE_CHARS
            || mime_type.chars().any(char::is_control)
            || mime_type.contains(['\r', '\n'])
        {
            return Err(connector_error("invalid Drive mimeType"));
        }
    }
    Ok(())
}

pub async fn execute_github_workspace_action(
    action: String,
    params: Value,
    approved: bool,
    state: State<'_, AppState>,
) -> Result<Value, AppError> {
    connector_stabilization::execute_github_workspace_action(
        action.clone(),
        normalize_github_params(&action, params),
        approved,
        state,
    )
    .await
}

pub async fn execute_google_workspace_action(
    action: String,
    params: Value,
    approved: bool,
    state: State<'_, AppState>,
) -> Result<Value, AppError> {
    validate_google_params(&action, &params)?;
    connector_stabilization::execute_google_workspace_action(action, params, approved, state).await
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn github_placeholder_sha_is_removed_for_auto_resolution() {
        let params = normalize_github_params(
            "file.delete",
            json!({"repo": "owner/repo", "path": "old.txt", "sha": "FILE_BLOB_SHA"}),
        );
        assert!(params.get("sha").is_none());
    }

    #[test]
    fn real_github_sha_is_preserved() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let params = normalize_github_params(
            "file.put",
            json!({"repo": "owner/repo", "path": "README.md", "sha": sha}),
        );
        assert_eq!(params.get("sha").and_then(Value::as_str), Some(sha));
    }

    #[test]
    fn drive_mime_type_rejects_header_injection() {
        assert!(validate_google_params(
            "drive.create",
            &json!({"content": "hello", "mimeType": "text/plain\r\nX-Test: injected"}),
        )
        .is_err());
        assert!(validate_google_params(
            "drive.create",
            &json!({"content": "hello", "mimeType": "text/plain"}),
        )
        .is_ok());
    }
}
