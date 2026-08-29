use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::{header, Client, Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::State;

use crate::{app_error::AppError, github::GithubRepository, AppState};

const GITHUB_API: &str = "https://api.github.com";
const GITHUB_API_VERSION: &str = "2022-11-28";
const USER_AGENT: &str = "OpenMindAI-Desktop";
const API_TIMEOUT_SECS: u64 = 30;
const MAX_TEXT_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_LOG_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubFileContent {
    pub path: String,
    pub sha: String,
    pub html_url: Option<String>,
    pub content: String,
    pub encoding: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubBranchInfo {
    pub name: String,
    pub sha: String,
    pub protected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubWriteResult {
    pub commit_sha: String,
    pub content_sha: Option<String>,
    pub html_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubPullRequestInfo {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub html_url: String,
    pub merged: Option<bool>,
    pub head_ref: String,
    pub base_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubWorkflowRun {
    pub id: u64,
    pub name: Option<String>,
    pub status: Option<String>,
    pub conclusion: Option<String>,
    pub event: Option<String>,
    pub head_branch: Option<String>,
    pub head_sha: String,
    pub html_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubWorkflowJob {
    pub id: u64,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub html_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubReleaseInfo {
    pub id: u64,
    pub tag_name: String,
    pub name: Option<String>,
    pub draft: bool,
    pub prerelease: bool,
    pub html_url: String,
}

fn github_error(message: impl Into<String>) -> AppError {
    AppError::GithubApiError(message.into())
}

fn require_confirmed(confirmed: bool, action: &str) -> Result<(), AppError> {
    if confirmed {
        Ok(())
    } else {
        Err(github_error(format!("{action} requires explicit confirmation")))
    }
}

fn validate_repo(repo: &str) -> Result<&str, AppError> {
    let repo = repo.trim();
    let mut parts = repo.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    if owner.is_empty() || name.is_empty() || parts.next().is_some() {
        return Err(github_error("repository must be in owner/name format"));
    }
    let safe = |value: &str| {
        value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    };
    if !safe(owner) || !safe(name) {
        return Err(github_error("repository contains unsupported characters"));
    }
    Ok(repo)
}

fn validate_path(path: &str) -> Result<&str, AppError> {
    let path = path.trim().trim_start_matches('/');
    if path.is_empty()
        || path.len() > 1024
        || path.split('/').any(|part| part.is_empty() || part == "." || part == "..")
        || path.chars().any(char::is_control)
    {
        return Err(github_error("invalid repository path"));
    }
    Ok(path)
}

fn validate_ref(value: &str) -> Result<&str, AppError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 255
        || value.contains("..")
        || value.ends_with('.')
        || value.ends_with('/')
        || value.starts_with('/')
        || value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace() || "~^:?*[\\".contains(ch))
    {
        return Err(github_error("invalid Git ref"));
    }
    Ok(value)
}

fn client() -> Result<Client, AppError> {
    Client::builder()
        .timeout(Duration::from_secs(API_TIMEOUT_SECS))
        .build()
        .map_err(|error| github_error(error.to_string()))
}

fn token(state: &State<'_, AppState>) -> Result<String, AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    GithubRepository::new(&db)
        .get_token()?
        .ok_or_else(|| github_error("GitHub is not connected"))
}

async fn request(
    state: &State<'_, AppState>,
    method: Method,
    url: String,
) -> Result<reqwest::RequestBuilder, AppError> {
    Ok(client()?
        .request(method, url)
        .bearer_auth(token(state)?)
        .header(header::ACCEPT, "application/vnd.github+json")
        .header(header::USER_AGENT, USER_AGENT)
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION))
}

async fn success(response: reqwest::Response, action: &str) -> Result<reqwest::Response, AppError> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let detail = response.text().await.unwrap_or_default();
    Err(github_error(format!("{action} failed ({status}): {detail}")))
}

fn encode_path(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            segment
                .bytes()
                .flat_map(|byte| {
                    if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                        vec![byte as char]
                    } else {
                        format!("%{byte:02X}").chars().collect()
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn pr_from_value(value: &Value) -> GithubPullRequestInfo {
    GithubPullRequestInfo {
        number: value.get("number").and_then(Value::as_u64).unwrap_or_default(),
        title: value.get("title").and_then(Value::as_str).unwrap_or_default().to_string(),
        state: value.get("state").and_then(Value::as_str).unwrap_or_default().to_string(),
        html_url: value.get("html_url").and_then(Value::as_str).unwrap_or_default().to_string(),
        merged: value.get("merged").and_then(Value::as_bool),
        head_ref: value
            .pointer("/head/ref")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        base_ref: value
            .pointer("/base/ref")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

#[tauri::command]
pub async fn github_list_branches(
    repo_full_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<GithubBranchInfo>, AppError> {
    let repo = validate_repo(&repo_full_name)?;
    let url = format!("{GITHUB_API}/repos/{repo}/branches?per_page=100");
    let response = request(&state, Method::GET, url)
        .await?
        .send()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    let values: Vec<Value> = success(response, "list branches")
        .await?
        .json()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    Ok(values
        .into_iter()
        .map(|value| GithubBranchInfo {
            name: value.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
            sha: value
                .pointer("/commit/sha")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            protected: value.get("protected").and_then(Value::as_bool).unwrap_or(false),
        })
        .collect())
}

#[tauri::command]
pub async fn github_get_file(
    repo_full_name: String,
    path: String,
    git_ref: Option<String>,
    state: State<'_, AppState>,
) -> Result<GithubFileContent, AppError> {
    let repo = validate_repo(&repo_full_name)?;
    let path = validate_path(&path)?;
    if let Some(reference) = git_ref.as_deref() {
        validate_ref(reference)?;
    }
    let url = format!("{GITHUB_API}/repos/{repo}/contents/{}", encode_path(path));
    let mut builder = request(&state, Method::GET, url).await?;
    if let Some(reference) = git_ref.as_deref() {
        builder = builder.query(&[("ref", reference)]);
    }
    let response = builder
        .send()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    let value: Value = success(response, "read repository file")
        .await?
        .json()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    if value.get("type").and_then(Value::as_str) != Some("file") {
        return Err(github_error("requested path is not a file"));
    }
    let encoded = value
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .replace('\n', "");
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|error| github_error(format!("could not decode repository file: {error}")))?;
    if bytes.len() > MAX_TEXT_FILE_BYTES {
        return Err(github_error("repository file exceeds the 2 MB in-app read limit"));
    }
    let content = String::from_utf8(bytes)
        .map_err(|_| github_error("repository file is not UTF-8 text"))?;
    Ok(GithubFileContent {
        path: value.get("path").and_then(Value::as_str).unwrap_or(path).to_string(),
        sha: value.get("sha").and_then(Value::as_str).unwrap_or_default().to_string(),
        html_url: value.get("html_url").and_then(Value::as_str).map(ToOwned::to_owned),
        content,
        encoding: "utf-8".to_string(),
    })
}

async fn branch_sha(state: &State<'_, AppState>, repo: &str, reference: &str) -> Result<String, AppError> {
    let reference = validate_ref(reference)?;
    let url = format!("{GITHUB_API}/repos/{repo}/git/ref/heads/{}", encode_path(reference));
    let response = request(state, Method::GET, url)
        .await?
        .send()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    let value: Value = success(response, "read branch ref")
        .await?
        .json()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    value
        .pointer("/object/sha")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| github_error("branch SHA missing from GitHub response"))
}

#[tauri::command]
pub async fn github_create_branch(
    repo_full_name: String,
    branch: String,
    from_ref: String,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<GithubBranchInfo, AppError> {
    require_confirmed(confirmed, "creating a GitHub branch")?;
    let repo = validate_repo(&repo_full_name)?;
    let branch = validate_ref(&branch)?;
    let from_ref = validate_ref(&from_ref)?;
    let sha = branch_sha(&state, repo, from_ref).await?;
    let url = format!("{GITHUB_API}/repos/{repo}/git/refs");
    let response = request(&state, Method::POST, url)
        .await?
        .json(&json!({"ref": format!("refs/heads/{branch}"), "sha": sha}))
        .send()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    let value: Value = success(response, "create branch")
        .await?
        .json()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    Ok(GithubBranchInfo {
        name: branch.to_string(),
        sha: value
            .pointer("/object/sha")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        protected: false,
    })
}

async fn existing_file_sha(
    state: &State<'_, AppState>,
    repo: &str,
    path: &str,
    branch: &str,
) -> Result<Option<String>, AppError> {
    let url = format!("{GITHUB_API}/repos/{repo}/contents/{}", encode_path(path));
    let response = request(state, Method::GET, url)
        .await?
        .query(&[("ref", branch)])
        .send()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let value: Value = success(response, "inspect repository file")
        .await?
        .json()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    Ok(value.get("sha").and_then(Value::as_str).map(ToOwned::to_owned))
}

#[tauri::command]
pub async fn github_write_file(
    repo_full_name: String,
    path: String,
    content: String,
    commit_message: String,
    branch: String,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<GithubWriteResult, AppError> {
    require_confirmed(confirmed, "writing a GitHub file")?;
    let repo = validate_repo(&repo_full_name)?;
    let path = validate_path(&path)?;
    let branch = validate_ref(&branch)?;
    if content.len() > MAX_TEXT_FILE_BYTES {
        return Err(github_error("file content exceeds the 2 MB in-app write limit"));
    }
    if commit_message.trim().is_empty() {
        return Err(github_error("commit message is required"));
    }
    let sha = existing_file_sha(&state, repo, path, branch).await?;
    let mut payload = json!({
        "message": commit_message.trim(),
        "content": STANDARD.encode(content.as_bytes()),
        "branch": branch,
    });
    if let Some(value) = sha {
        payload["sha"] = json!(value);
    }
    let url = format!("{GITHUB_API}/repos/{repo}/contents/{}", encode_path(path));
    let response = request(&state, Method::PUT, url)
        .await?
        .json(&payload)
        .send()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    let value: Value = success(response, "write repository file")
        .await?
        .json()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    Ok(GithubWriteResult {
        commit_sha: value
            .pointer("/commit/sha")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        content_sha: value.pointer("/content/sha").and_then(Value::as_str).map(ToOwned::to_owned),
        html_url: value.pointer("/content/html_url").and_then(Value::as_str).map(ToOwned::to_owned),
    })
}

#[tauri::command]
pub async fn github_delete_file(
    repo_full_name: String,
    path: String,
    commit_message: String,
    branch: String,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<GithubWriteResult, AppError> {
    require_confirmed(confirmed, "deleting a GitHub file")?;
    let repo = validate_repo(&repo_full_name)?;
    let path = validate_path(&path)?;
    let branch = validate_ref(&branch)?;
    let sha = existing_file_sha(&state, repo, path, branch)
        .await?
        .ok_or_else(|| github_error("repository file does not exist"))?;
    let url = format!("{GITHUB_API}/repos/{repo}/contents/{}", encode_path(path));
    let response = request(&state, Method::DELETE, url)
        .await?
        .json(&json!({"message": commit_message.trim(), "sha": sha, "branch": branch}))
        .send()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    let value: Value = success(response, "delete repository file")
        .await?
        .json()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    Ok(GithubWriteResult {
        commit_sha: value
            .pointer("/commit/sha")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        content_sha: None,
        html_url: value.pointer("/commit/html_url").and_then(Value::as_str).map(ToOwned::to_owned),
    })
}

#[tauri::command]
pub async fn github_create_issue(
    repo_full_name: String,
    title: String,
    body: Option<String>,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<Value, AppError> {
    require_confirmed(confirmed, "creating a GitHub issue")?;
    let repo = validate_repo(&repo_full_name)?;
    if title.trim().is_empty() {
        return Err(github_error("issue title is required"));
    }
    let url = format!("{GITHUB_API}/repos/{repo}/issues");
    let response = request(&state, Method::POST, url)
        .await?
        .json(&json!({"title": title.trim(), "body": body.unwrap_or_default()}))
        .send()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    success(response, "create issue")
        .await?
        .json()
        .await
        .map_err(|error| github_error(error.to_string()))
}

#[tauri::command]
pub async fn github_add_issue_comment(
    repo_full_name: String,
    issue_number: u64,
    body: String,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<Value, AppError> {
    require_confirmed(confirmed, "commenting on GitHub")?;
    let repo = validate_repo(&repo_full_name)?;
    if body.trim().is_empty() {
        return Err(github_error("comment body is required"));
    }
    let url = format!("{GITHUB_API}/repos/{repo}/issues/{issue_number}/comments");
    let response = request(&state, Method::POST, url)
        .await?
        .json(&json!({"body": body}))
        .send()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    success(response, "add issue comment")
        .await?
        .json()
        .await
        .map_err(|error| github_error(error.to_string()))
}

#[tauri::command]
pub async fn github_create_pull_request(
    repo_full_name: String,
    title: String,
    body: Option<String>,
    head: String,
    base: String,
    draft: bool,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<GithubPullRequestInfo, AppError> {
    require_confirmed(confirmed, "creating a pull request")?;
    let repo = validate_repo(&repo_full_name)?;
    validate_ref(&head)?;
    validate_ref(&base)?;
    let url = format!("{GITHUB_API}/repos/{repo}/pulls");
    let response = request(&state, Method::POST, url)
        .await?
        .json(&json!({"title": title, "body": body.unwrap_or_default(), "head": head, "base": base, "draft": draft}))
        .send()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    let value: Value = success(response, "create pull request")
        .await?
        .json()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    Ok(pr_from_value(&value))
}

#[tauri::command]
pub async fn github_get_pull_request(
    repo_full_name: String,
    pull_number: u64,
    state: State<'_, AppState>,
) -> Result<GithubPullRequestInfo, AppError> {
    let repo = validate_repo(&repo_full_name)?;
    let url = format!("{GITHUB_API}/repos/{repo}/pulls/{pull_number}");
    let response = request(&state, Method::GET, url)
        .await?
        .send()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    let value: Value = success(response, "read pull request")
        .await?
        .json()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    Ok(pr_from_value(&value))
}

#[tauri::command]
pub async fn github_update_pull_request(
    repo_full_name: String,
    pull_number: u64,
    title: Option<String>,
    body: Option<String>,
    state_value: Option<String>,
    base: Option<String>,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<GithubPullRequestInfo, AppError> {
    require_confirmed(confirmed, "updating a pull request")?;
    let repo = validate_repo(&repo_full_name)?;
    if let Some(base_ref) = base.as_deref() {
        validate_ref(base_ref)?;
    }
    if let Some(value) = state_value.as_deref() {
        if !matches!(value, "open" | "closed") {
            return Err(github_error("pull request state must be open or closed"));
        }
    }
    let url = format!("{GITHUB_API}/repos/{repo}/pulls/{pull_number}");
    let mut payload = serde_json::Map::new();
    if let Some(value) = title { payload.insert("title".to_string(), json!(value)); }
    if let Some(value) = body { payload.insert("body".to_string(), json!(value)); }
    if let Some(value) = state_value { payload.insert("state".to_string(), json!(value)); }
    if let Some(value) = base { payload.insert("base".to_string(), json!(value)); }
    let response = request(&state, Method::PATCH, url)
        .await?
        .json(&Value::Object(payload))
        .send()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    let value: Value = success(response, "update pull request")
        .await?
        .json()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    Ok(pr_from_value(&value))
}

#[tauri::command]
pub async fn github_merge_pull_request(
    repo_full_name: String,
    pull_number: u64,
    commit_title: Option<String>,
    merge_method: Option<String>,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<Value, AppError> {
    require_confirmed(confirmed, "merging a pull request")?;
    let repo = validate_repo(&repo_full_name)?;
    let method = merge_method.unwrap_or_else(|| "squash".to_string());
    if !matches!(method.as_str(), "merge" | "squash" | "rebase") {
        return Err(github_error("merge method must be merge, squash, or rebase"));
    }
    let url = format!("{GITHUB_API}/repos/{repo}/pulls/{pull_number}/merge");
    let response = request(&state, Method::PUT, url)
        .await?
        .json(&json!({"commit_title": commit_title, "merge_method": method}))
        .send()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    success(response, "merge pull request")
        .await?
        .json()
        .await
        .map_err(|error| github_error(error.to_string()))
}

#[tauri::command]
pub async fn github_list_workflow_runs(
    repo_full_name: String,
    branch: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<GithubWorkflowRun>, AppError> {
    let repo = validate_repo(&repo_full_name)?;
    if let Some(value) = branch.as_deref() { validate_ref(value)?; }
    let url = format!("{GITHUB_API}/repos/{repo}/actions/runs");
    let mut builder = request(&state, Method::GET, url).await?.query(&[("per_page", "50")]);
    if let Some(value) = branch.as_deref() { builder = builder.query(&[("branch", value)]); }
    let response = builder
        .send()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    let value: Value = success(response, "list workflow runs")
        .await?
        .json()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    Ok(value
        .get("workflow_runs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|run| GithubWorkflowRun {
            id: run.get("id").and_then(Value::as_u64).unwrap_or_default(),
            name: run.get("name").and_then(Value::as_str).map(ToOwned::to_owned),
            status: run.get("status").and_then(Value::as_str).map(ToOwned::to_owned),
            conclusion: run.get("conclusion").and_then(Value::as_str).map(ToOwned::to_owned),
            event: run.get("event").and_then(Value::as_str).map(ToOwned::to_owned),
            head_branch: run.get("head_branch").and_then(Value::as_str).map(ToOwned::to_owned),
            head_sha: run.get("head_sha").and_then(Value::as_str).unwrap_or_default().to_string(),
            html_url: run.get("html_url").and_then(Value::as_str).unwrap_or_default().to_string(),
        })
        .collect())
}

#[tauri::command]
pub async fn github_list_workflow_jobs(
    repo_full_name: String,
    run_id: u64,
    state: State<'_, AppState>,
) -> Result<Vec<GithubWorkflowJob>, AppError> {
    let repo = validate_repo(&repo_full_name)?;
    let url = format!("{GITHUB_API}/repos/{repo}/actions/runs/{run_id}/jobs?per_page=100");
    let response = request(&state, Method::GET, url)
        .await?
        .send()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    let value: Value = success(response, "list workflow jobs")
        .await?
        .json()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    Ok(value
        .get("jobs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|job| GithubWorkflowJob {
            id: job.get("id").and_then(Value::as_u64).unwrap_or_default(),
            name: job.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
            status: job.get("status").and_then(Value::as_str).unwrap_or_default().to_string(),
            conclusion: job.get("conclusion").and_then(Value::as_str).map(ToOwned::to_owned),
            html_url: job.get("html_url").and_then(Value::as_str).map(ToOwned::to_owned),
        })
        .collect())
}

#[tauri::command]
pub async fn github_get_job_logs(
    repo_full_name: String,
    job_id: u64,
    state: State<'_, AppState>,
) -> Result<String, AppError> {
    let repo = validate_repo(&repo_full_name)?;
    let url = format!("{GITHUB_API}/repos/{repo}/actions/jobs/{job_id}/logs");
    let response = success(
        request(&state, Method::GET, url)
            .await?
            .send()
            .await
            .map_err(|error| github_error(error.to_string()))?,
        "read workflow job logs",
    )
    .await?;
    let bytes = response
        .bytes()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    if bytes.len() > MAX_LOG_BYTES {
        return Err(github_error("workflow log exceeds the 4 MB in-app limit"));
    }
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

#[tauri::command]
pub async fn github_dispatch_workflow(
    repo_full_name: String,
    workflow_id: String,
    git_ref: String,
    inputs: Value,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    require_confirmed(confirmed, "dispatching a GitHub workflow")?;
    let repo = validate_repo(&repo_full_name)?;
    validate_ref(&git_ref)?;
    if workflow_id.trim().is_empty() || workflow_id.contains('/') || workflow_id.contains("..") {
        return Err(github_error("invalid workflow identifier"));
    }
    let url = format!("{GITHUB_API}/repos/{repo}/actions/workflows/{}/dispatches", encode_path(workflow_id.trim()));
    let response = request(&state, Method::POST, url)
        .await?
        .json(&json!({"ref": git_ref, "inputs": inputs}))
        .send()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    success(response, "dispatch workflow").await?;
    Ok(())
}

async fn workflow_run_action(
    state: &State<'_, AppState>,
    repo: &str,
    run_id: u64,
    action: &str,
    confirmed: bool,
) -> Result<(), AppError> {
    require_confirmed(confirmed, &format!("GitHub workflow {action}"))?;
    let url = format!("{GITHUB_API}/repos/{repo}/actions/runs/{run_id}/{action}");
    let response = request(state, Method::POST, url)
        .await?
        .send()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    success(response, &format!("workflow {action}")).await?;
    Ok(())
}

#[tauri::command]
pub async fn github_rerun_workflow(
    repo_full_name: String,
    run_id: u64,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let repo = validate_repo(&repo_full_name)?;
    workflow_run_action(&state, repo, run_id, "rerun", confirmed).await
}

#[tauri::command]
pub async fn github_cancel_workflow(
    repo_full_name: String,
    run_id: u64,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let repo = validate_repo(&repo_full_name)?;
    workflow_run_action(&state, repo, run_id, "cancel", confirmed).await
}

#[tauri::command]
pub async fn github_create_release(
    repo_full_name: String,
    tag_name: String,
    target_commitish: Option<String>,
    name: Option<String>,
    body: Option<String>,
    draft: bool,
    prerelease: bool,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<GithubReleaseInfo, AppError> {
    require_confirmed(confirmed, "creating a GitHub release")?;
    let repo = validate_repo(&repo_full_name)?;
    validate_ref(&tag_name)?;
    if let Some(target) = target_commitish.as_deref() { validate_ref(target)?; }
    let url = format!("{GITHUB_API}/repos/{repo}/releases");
    let response = request(&state, Method::POST, url)
        .await?
        .json(&json!({
            "tag_name": tag_name,
            "target_commitish": target_commitish,
            "name": name,
            "body": body,
            "draft": draft,
            "prerelease": prerelease
        }))
        .send()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    let value: Value = success(response, "create release")
        .await?
        .json()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    Ok(GithubReleaseInfo {
        id: value.get("id").and_then(Value::as_u64).unwrap_or_default(),
        tag_name: value.get("tag_name").and_then(Value::as_str).unwrap_or_default().to_string(),
        name: value.get("name").and_then(Value::as_str).map(ToOwned::to_owned),
        draft: value.get("draft").and_then(Value::as_bool).unwrap_or(false),
        prerelease: value.get("prerelease").and_then(Value::as_bool).unwrap_or(false),
        html_url: value.get("html_url").and_then(Value::as_str).unwrap_or_default().to_string(),
    })
}

#[tauri::command]
pub async fn github_delete_release(
    repo_full_name: String,
    release_id: u64,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    require_confirmed(confirmed, "deleting a GitHub release")?;
    let repo = validate_repo(&repo_full_name)?;
    let url = format!("{GITHUB_API}/repos/{repo}/releases/{release_id}");
    let response = request(&state, Method::DELETE, url)
        .await?
        .send()
        .await
        .map_err(|error| github_error(error.to_string()))?;
    success(response, "delete release").await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_repository_names() {
        assert!(validate_repo("owner/repo").is_ok());
        assert!(validate_repo("owner/repo/name").is_err());
        assert!(validate_repo("owner/../repo").is_err());
    }

    #[test]
    fn repository_paths_cannot_escape() {
        assert!(validate_path("src/main.rs").is_ok());
        assert!(validate_path("../secret").is_err());
        assert!(validate_path("src/../secret").is_err());
    }

    #[test]
    fn git_refs_reject_dangerous_syntax() {
        assert!(validate_ref("feature/connected-agent").is_ok());
        assert!(validate_ref("bad ref").is_err());
        assert!(validate_ref("heads/../main").is_err());
    }

    #[test]
    fn writes_require_confirmation() {
        assert!(require_confirmed(false, "write").is_err());
        assert!(require_confirmed(true, "write").is_ok());
    }
}
