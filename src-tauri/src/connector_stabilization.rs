use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Utc;
use reqwest::{header, Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::State;
use url::Url;
use uuid::Uuid;

use crate::{
    app_error::AppError,
    github::{secret_store, GithubRepository},
    github_workspace,
    google::GoogleRepository,
    google_workspace, AppState,
};

const GOOGLE_TOKEN_SLOT: &str = "google-workspace-oauth";
const GOOGLE_CLIENT_SECRET_SLOT: &str = "google-client-secret";
const GOOGLE_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_REQUEST_TIMEOUT_SECS: u64 = 30;
const MAX_DRIVE_UPLOAD_BYTES: usize = 8 * 1024 * 1024;
const GITHUB_API_ROOT: &str = "https://api.github.com";
const GITHUB_API_VERSION: &str = "2022-11-28";
const GITHUB_USER_AGENT: &str = "OpenMindAI-Desktop";
const GITHUB_REQUEST_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleTokenBundle {
    access_token: String,
    refresh_token: String,
    expires_at: i64,
    token_type: String,
    scope: String,
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleRefreshResponse {
    access_token: String,
    expires_in: i64,
    token_type: Option<String>,
    scope: Option<String>,
}

fn connector_error(message: impl Into<String>) -> AppError {
    AppError::internal(format!("Connected actions: {}", message.into()))
}

fn google_error(message: impl Into<String>) -> AppError {
    AppError::internal(format!("Google Workspace: {}", message.into()))
}

fn github_error(message: impl Into<String>) -> AppError {
    AppError::GithubApiError(message.into())
}

fn required_str<'a>(params: &'a Value, key: &str) -> Result<&'a str, AppError> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| connector_error(format!("missing required parameter '{key}'")))
}

fn optional_str<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn encode_repo_path(path: &str) -> Result<String, AppError> {
    if path.trim().is_empty()
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.split(['/', '\\']).any(|part| part == "..")
        || path.contains('\0')
    {
        return Err(github_error("invalid repository path"));
    }
    Ok(path
        .split('/')
        .map(|part| url::form_urlencoded::byte_serialize(part.as_bytes()).collect::<String>())
        .collect::<Vec<_>>()
        .join("/"))
}

fn validate_repo(repo: &str) -> Result<(), AppError> {
    let parts: Vec<_> = repo.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|part| {
            part.is_empty()
                || *part == "."
                || *part == ".."
                || !part.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
                })
        })
    {
        return Err(github_error("repository must be in owner/name form"));
    }
    Ok(())
}

fn github_headers(token: &str) -> Result<header::HeaderMap, AppError> {
    let mut headers = header::HeaderMap::new();
    let mut auth = header::HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|error| github_error(error.to_string()))?;
    auth.set_sensitive(true);
    headers.insert(header::AUTHORIZATION, auth);
    headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        header::USER_AGENT,
        header::HeaderValue::from_static(GITHUB_USER_AGENT),
    );
    headers.insert(
        "X-GitHub-Api-Version",
        header::HeaderValue::from_static(GITHUB_API_VERSION),
    );
    Ok(headers)
}

fn github_token(state: &State<'_, AppState>) -> Result<String, AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    GithubRepository::new(&db)
        .get_token()?
        .ok_or_else(|| github_error("GitHub is not connected"))
}

async fn github_default_branch(
    state: &State<'_, AppState>,
    token: &str,
    repo: &str,
) -> Result<String, AppError> {
    validate_repo(repo)?;
    let response = state
        .http
        .get(format!("{GITHUB_API_ROOT}/repos/{repo}"))
        .headers(github_headers(token)?)
        .timeout(Duration::from_secs(GITHUB_REQUEST_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    let status = response.status();
    let value: Value = response
        .json()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    if !status.is_success() {
        return Err(github_error(format!(
            "repository lookup failed ({status}): {value}"
        )));
    }
    value
        .get("default_branch")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| github_error("repository default branch is unavailable"))
}

async fn github_existing_file_sha(
    state: &State<'_, AppState>,
    token: &str,
    repo: &str,
    path: &str,
    branch: &str,
) -> Result<Option<String>, AppError> {
    validate_repo(repo)?;
    let encoded_path = encode_repo_path(path)?;
    let mut url = Url::parse(&format!(
        "{GITHUB_API_ROOT}/repos/{repo}/contents/{encoded_path}"
    ))
    .map_err(|error| github_error(error.to_string()))?;
    url.query_pairs_mut().append_pair("ref", branch);
    let response = state
        .http
        .get(url)
        .headers(github_headers(token)?)
        .timeout(Duration::from_secs(GITHUB_REQUEST_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let status = response.status();
    let value: Value = response
        .json()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    if !status.is_success() {
        return Err(github_error(format!(
            "file lookup failed ({status}): {value}"
        )));
    }
    if value.get("type").and_then(Value::as_str) != Some("file") {
        return Err(github_error("repository path is not a file"));
    }
    Ok(value
        .get("sha")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned))
}

async fn stabilize_github_file_params(
    state: &State<'_, AppState>,
    action: &str,
    mut params: Value,
) -> Result<Value, AppError> {
    if !matches!(action, "file.put" | "file.delete") {
        return Ok(params);
    }
    let repo = required_str(&params, "repo")?.to_string();
    let path = required_str(&params, "path")?.to_string();
    let token = github_token(state)?;
    let branch = if let Some(branch) = optional_str(&params, "branch") {
        branch.to_string()
    } else {
        github_default_branch(state, &token, &repo).await?
    };
    params["branch"] = Value::String(branch.clone());

    if optional_str(&params, "sha").is_none() {
        let existing = github_existing_file_sha(state, &token, &repo, &path, &branch).await?;
        match (action, existing) {
            ("file.put", Some(sha)) | ("file.delete", Some(sha)) => {
                params["sha"] = Value::String(sha);
            }
            ("file.delete", None) => {
                return Err(github_error("repository file does not exist"));
            }
            _ => {}
        }
    }
    Ok(params)
}

pub async fn execute_github_workspace_action(
    action: String,
    params: Value,
    approved: bool,
    state: State<'_, AppState>,
) -> Result<Value, AppError> {
    let params = stabilize_github_file_params(&state, &action, params).await?;
    github_workspace::execute_github_workspace_action(action, params, approved, state).await
}

fn load_google_tokens() -> Result<GoogleTokenBundle, AppError> {
    let raw = secret_store::get_secret(GOOGLE_TOKEN_SLOT)?
        .ok_or_else(|| google_error("Google account is not connected"))?;
    serde_json::from_str(&raw)
        .map_err(|error| google_error(format!("stored OAuth token is invalid: {error}")))
}

fn save_google_tokens(bundle: &GoogleTokenBundle) -> Result<(), AppError> {
    let raw = serde_json::to_string(bundle)
        .map_err(|error| google_error(format!("could not serialize OAuth token: {error}")))?;
    secret_store::set_secret(GOOGLE_TOKEN_SLOT, &raw)
}

async fn google_access_token(state: &State<'_, AppState>) -> Result<String, AppError> {
    let mut bundle = load_google_tokens()?;
    if bundle.expires_at > Utc::now().timestamp() + 60 {
        return Ok(bundle.access_token);
    }
    let (client_id, client_secret) = {
        let db = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        let status = GoogleRepository::new(&db)
            .get_status()?
            .ok_or_else(|| google_error("Google OAuth client credentials are not configured"))?;
        let secret = secret_store::get_secret(GOOGLE_CLIENT_SECRET_SLOT)?
            .ok_or_else(|| google_error("Google OAuth client secret is missing"))?;
        (status.client_id, secret)
    };
    let response = state
        .http
        .post(GOOGLE_TOKEN_ENDPOINT)
        .timeout(Duration::from_secs(GOOGLE_REQUEST_TIMEOUT_SECS))
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("refresh_token", bundle.refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|error| google_error(format!("token refresh failed: {error}")))?;
    let status = response.status();
    let value: Value = response
        .json()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    if !status.is_success() {
        return Err(google_error(format!(
            "token refresh failed ({status}): {value}"
        )));
    }
    let refreshed: GoogleRefreshResponse = serde_json::from_value(value)
        .map_err(|error| google_error(format!("invalid refresh response: {error}")))?;
    bundle.access_token = refreshed.access_token;
    bundle.expires_at = Utc::now().timestamp() + refreshed.expires_in;
    if let Some(token_type) = refreshed.token_type {
        bundle.token_type = token_type;
    }
    if let Some(scope) = refreshed.scope {
        bundle.scope = scope;
    }
    save_google_tokens(&bundle)?;
    Ok(bundle.access_token)
}

fn drive_content(params: &Value) -> Result<Option<Vec<u8>>, AppError> {
    if let Some(encoded) = optional_str(params, "contentBase64") {
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|error| google_error(format!("contentBase64 is invalid: {error}")))?;
        return Ok(Some(bytes));
    }
    Ok(optional_str(params, "content").map(|content| content.as_bytes().to_vec()))
}

fn multipart_related_body(metadata: &Value, content: &[u8], mime_type: &str) -> (String, Vec<u8>) {
    let boundary = format!("openmindai-{}", Uuid::new_v4().simple());
    let mut body = Vec::with_capacity(metadata.to_string().len() + content.len() + 256);
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{}\r\n",
            metadata
        )
        .as_bytes(),
    );
    body.extend_from_slice(format!("--{boundary}\r\nContent-Type: {mime_type}\r\n\r\n").as_bytes());
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (boundary, body)
}

async fn stable_drive_write(
    state: &State<'_, AppState>,
    action: &str,
    params: &Value,
    approved: bool,
) -> Result<Option<Value>, AppError> {
    if !matches!(action, "drive.create" | "drive.update") {
        return Ok(None);
    }
    let Some(content) = drive_content(params)? else {
        return Ok(None);
    };
    if !approved {
        return Err(google_error(format!(
            "action '{action}' changes remote Google data and requires explicit approval"
        )));
    }
    if content.len() > MAX_DRIVE_UPLOAD_BYTES {
        return Err(google_error(
            "Drive upload exceeds the 8 MB interactive action limit",
        ));
    }
    let mut metadata = params.get("metadata").cloned().unwrap_or_else(|| json!({}));
    if action == "drive.create" && metadata.get("name").and_then(Value::as_str).is_none() {
        if let Some(name) = optional_str(params, "name") {
            metadata["name"] = Value::String(name.to_string());
        }
    }
    let mime_type = optional_str(params, "mimeType").unwrap_or("application/octet-stream");
    let (boundary, body) = multipart_related_body(&metadata, &content, mime_type);
    let url = if action == "drive.update" {
        let file_id = required_str(params, "fileId")?;
        format!(
            "https://www.googleapis.com/upload/drive/v3/files/{file_id}?uploadType=multipart&fields=id,name,mimeType,size,modifiedTime,webViewLink,parents"
        )
    } else {
        "https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart&fields=id,name,mimeType,size,modifiedTime,webViewLink,parents".to_string()
    };
    let token = google_access_token(state).await?;
    let response = state
        .http
        .request(
            if action == "drive.update" {
                Method::PATCH
            } else {
                Method::POST
            },
            url,
        )
        .bearer_auth(token)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/related; boundary={boundary}"),
        )
        .timeout(Duration::from_secs(GOOGLE_REQUEST_TIMEOUT_SECS))
        .body(body)
        .send()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    let status = response.status();
    let value: Value = response
        .json()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    if !status.is_success() {
        return Err(google_error(format!(
            "Drive write failed ({status}): {value}"
        )));
    }
    Ok(Some(value))
}

pub async fn execute_google_workspace_action(
    action: String,
    params: Value,
    approved: bool,
    state: State<'_, AppState>,
) -> Result<Value, AppError> {
    if let Some(value) = stable_drive_write(&state, &action, &params, approved).await? {
        return Ok(value);
    }
    google_workspace::execute_google_workspace_action(action, params, approved, state).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_paths_are_encoded_by_segment() {
        assert_eq!(
            encode_repo_path(".github/workflows/ci.yml").unwrap(),
            ".github/workflows/ci.yml"
        );
        assert!(encode_repo_path("../secret").is_err());
    }

    #[test]
    fn drive_multipart_body_uses_related_parts() {
        let metadata = json!({"name": "hello.txt"});
        let (boundary, body) = multipart_related_body(&metadata, b"hello", "text/plain");
        let text = String::from_utf8(body).unwrap();
        assert!(text.contains(&format!("--{boundary}")));
        assert!(text.contains("Content-Type: application/json; charset=UTF-8"));
        assert!(text.contains("Content-Type: text/plain"));
        assert!(text.ends_with(&format!("--{boundary}--\r\n")));
    }
}
