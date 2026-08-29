use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use futures_util::StreamExt;
use reqwest::{header, Client, Method, Response};
use serde_json::{json, Value};
use tauri::State;
use url::Url;

use crate::{app_error::AppError, github::GithubRepository, AppState};

const API_ROOT: &str = "https://api.github.com";
const API_VERSION: &str = "2022-11-28";
const USER_AGENT: &str = "OpenMindAI-Desktop";
const REQUEST_TIMEOUT_SECS: u64 = 30;
const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;
const MAX_LOG_BYTES: usize = 4 * 1024 * 1024;

fn connector_error(message: impl Into<String>) -> AppError {
    AppError::GithubApiError(message.into())
}

fn is_mutating(action: &str) -> bool {
    matches!(
        action,
        "file.put"
            | "file.delete"
            | "branch.create"
            | "commit.multi_file"
            | "issue.create"
            | "issue.comment"
            | "pr.create"
            | "pr.update"
            | "pr.merge"
            | "actions.dispatch"
            | "actions.rerun"
            | "actions.cancel"
            | "actions.workflow.enable"
            | "actions.workflow.disable"
            | "release.create"
            | "release.update"
            | "release.delete"
            | "tag.create"
    )
}

fn require_approval(action: &str, approved: bool) -> Result<(), AppError> {
    if approved {
        return Ok(());
    }
    Err(connector_error(format!(
        "action '{action}' changes GitHub data and requires explicit approval"
    )))
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

fn validate_repo(repo: &str) -> Result<(), AppError> {
    let parts: Vec<_> = repo.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|part| {
            part.is_empty()
                || part == &"."
                || part == &".."
                || !part
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_'))
        })
    {
        return Err(connector_error("repository must be in owner/name form"));
    }
    Ok(())
}

fn validate_repo_path(path: &str) -> Result<(), AppError> {
    if path.trim().is_empty()
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.split(['/', '\\']).any(|part| part == "..")
        || path.contains('\0')
    {
        return Err(connector_error("invalid repository path"));
    }
    Ok(())
}

fn encode_component(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn repo_endpoint(repo: &str, suffix: &str) -> Result<String, AppError> {
    validate_repo(repo)?;
    Ok(format!("{API_ROOT}/repos/{repo}{suffix}"))
}

fn auth_headers(token: &str) -> Result<header::HeaderMap, AppError> {
    let mut headers = header::HeaderMap::new();
    let mut auth = header::HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|error| connector_error(error.to_string()))?;
    auth.set_sensitive(true);
    headers.insert(header::AUTHORIZATION, auth);
    headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        header::USER_AGENT,
        header::HeaderValue::from_static(USER_AGENT),
    );
    headers.insert(
        "X-GitHub-Api-Version",
        header::HeaderValue::from_static(API_VERSION),
    );
    Ok(headers)
}

fn token(state: &State<'_, AppState>) -> Result<String, AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    GithubRepository::new(&db)
        .get_token()?
        .ok_or_else(|| connector_error("GitHub is not connected"))
}

async fn read_bounded(response: Response, max_bytes: usize) -> Result<(header::HeaderMap, Vec<u8>), AppError> {
    let status = response.status();
    let headers = response.headers().clone();
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| connector_error(error.to_string()))?;
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(connector_error(format!(
                "GitHub response exceeded the {} byte safety limit",
                max_bytes
            )));
        }
        body.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let detail = String::from_utf8_lossy(&body);
        let permissions = headers
            .get("x-accepted-github-permissions")
            .and_then(|value| value.to_str().ok())
            .map(|value| format!(" Required permissions: {value}."))
            .unwrap_or_default();
        return Err(connector_error(format!(
            "GitHub returned status {status}: {}{permissions}",
            detail.chars().take(1400).collect::<String>()
        )));
    }
    Ok((headers, body))
}

async fn json_response(response: Response) -> Result<Value, AppError> {
    let (_headers, body) = read_bounded(response, MAX_JSON_BYTES).await?;
    if body.is_empty() {
        return Ok(json!({"ok": true}));
    }
    serde_json::from_slice(&body)
        .map_err(|error| connector_error(format!("invalid GitHub JSON response: {error}")))
}

async fn github_request(
    client: &Client,
    token: &str,
    method: Method,
    url: &str,
    body: Option<Value>,
) -> Result<Value, AppError> {
    let mut request = client
        .request(method, url)
        .headers(auth_headers(token)?)
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS));
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .map_err(|error| connector_error(error.to_string()))?;
    json_response(response).await
}

#[tauri::command]
pub async fn execute_github_workspace_action(
    action: String,
    params: Value,
    approved: bool,
    state: State<'_, AppState>,
) -> Result<Value, AppError> {
    if is_mutating(&action) {
        require_approval(&action, approved)?;
    }
    let auth_token = token(&state)?;
    match action.as_str() {
        "account.capabilities" => account_capabilities(&state.http, &auth_token).await,
        "repo.get" => {
            let repo = required_str(&params, "repo")?;
            github_request(&state.http, &auth_token, Method::GET, &repo_endpoint(repo, "")?, None).await
        }
        "branches.list" => {
            let repo = required_str(&params, "repo")?;
            let mut url = Url::parse(&repo_endpoint(repo, "/branches")?)
                .map_err(|error| connector_error(error.to_string()))?;
            url.query_pairs_mut().append_pair("per_page", "100");
            github_request(&state.http, &auth_token, Method::GET, url.as_str(), None).await
        }
        "file.get" => {
            let repo = required_str(&params, "repo")?;
            let path = required_str(&params, "path")?;
            validate_repo_path(path)?;
            let encoded = path.split('/').map(encode_component).collect::<Vec<_>>().join("/");
            let mut url = Url::parse(&repo_endpoint(repo, &format!("/contents/{encoded}"))?)
                .map_err(|error| connector_error(error.to_string()))?;
            if let Some(reference) = optional_str(&params, "ref") {
                url.query_pairs_mut().append_pair("ref", reference);
            }
            github_request(&state.http, &auth_token, Method::GET, url.as_str(), None).await
        }
        "commit.get" => {
            let repo = required_str(&params, "repo")?;
            let reference = required_str(&params, "ref")?;
            let url = repo_endpoint(repo, &format!("/commits/{}", encode_component(reference)))?;
            github_request(&state.http, &auth_token, Method::GET, &url, None).await
        }
        "file.put" => put_file(&state.http, &auth_token, &params).await,
        "file.delete" => delete_file(&state.http, &auth_token, &params).await,
        "branch.create" => create_branch(&state.http, &auth_token, &params).await,
        "commit.multi_file" => multi_file_commit(&state.http, &auth_token, &params).await,
        "issue.list" => {
            let repo = required_str(&params, "repo")?;
            let mut url = Url::parse(&repo_endpoint(repo, "/issues")?)
                .map_err(|error| connector_error(error.to_string()))?;
            url.query_pairs_mut()
                .append_pair("state", optional_str(&params, "state").unwrap_or("open"))
                .append_pair("per_page", "100");
            github_request(&state.http, &auth_token, Method::GET, url.as_str(), None).await
        }
        "issue.create" => {
            let repo = required_str(&params, "repo")?;
            let title = required_str(&params, "title")?;
            let body = json!({
                "title": title,
                "body": optional_str(&params, "body").unwrap_or(""),
                "labels": params.get("labels").cloned().unwrap_or_else(|| json!([])),
                "assignees": params.get("assignees").cloned().unwrap_or_else(|| json!([]))
            });
            github_request(&state.http, &auth_token, Method::POST, &repo_endpoint(repo, "/issues")?, Some(body)).await
        }
        "issue.comment" => {
            let repo = required_str(&params, "repo")?;
            let number = params.get("number").and_then(Value::as_u64).ok_or_else(|| connector_error("missing required numeric parameter 'number'"))?;
            let body_text = required_str(&params, "body")?;
            github_request(
                &state.http,
                &auth_token,
                Method::POST,
                &repo_endpoint(repo, &format!("/issues/{number}/comments"))?,
                Some(json!({"body": body_text})),
            )
            .await
        }
        "pr.list" => {
            let repo = required_str(&params, "repo")?;
            let mut url = Url::parse(&repo_endpoint(repo, "/pulls")?)
                .map_err(|error| connector_error(error.to_string()))?;
            url.query_pairs_mut()
                .append_pair("state", optional_str(&params, "state").unwrap_or("open"))
                .append_pair("per_page", "100");
            github_request(&state.http, &auth_token, Method::GET, url.as_str(), None).await
        }
        "pr.get" => {
            let repo = required_str(&params, "repo")?;
            let number = required_u64(&params, "number")?;
            github_request(&state.http, &auth_token, Method::GET, &repo_endpoint(repo, &format!("/pulls/{number}"))?, None).await
        }
        "pr.create" => {
            let repo = required_str(&params, "repo")?;
            let body = json!({
                "title": required_str(&params, "title")?,
                "head": required_str(&params, "head")?,
                "base": required_str(&params, "base")?,
                "body": optional_str(&params, "body").unwrap_or(""),
                "draft": params.get("draft").and_then(Value::as_bool).unwrap_or(false),
                "maintainer_can_modify": params.get("maintainerCanModify").and_then(Value::as_bool).unwrap_or(true)
            });
            github_request(&state.http, &auth_token, Method::POST, &repo_endpoint(repo, "/pulls")?, Some(body)).await
        }
        "pr.update" => {
            let repo = required_str(&params, "repo")?;
            let number = required_u64(&params, "number")?;
            let mut body = serde_json::Map::new();
            for key in ["title", "body", "state", "base"] {
                if let Some(value) = params.get(key) {
                    body.insert(key.to_string(), value.clone());
                }
            }
            github_request(
                &state.http,
                &auth_token,
                Method::PATCH,
                &repo_endpoint(repo, &format!("/pulls/{number}"))?,
                Some(Value::Object(body)),
            )
            .await
        }
        "pr.merge" => {
            let repo = required_str(&params, "repo")?;
            let number = required_u64(&params, "number")?;
            let body = json!({
                "commit_title": optional_str(&params, "commitTitle"),
                "commit_message": optional_str(&params, "commitMessage"),
                "merge_method": optional_str(&params, "mergeMethod").unwrap_or("squash")
            });
            github_request(
                &state.http,
                &auth_token,
                Method::PUT,
                &repo_endpoint(repo, &format!("/pulls/{number}/merge"))?,
                Some(body),
            )
            .await
        }
        "actions.workflows" => {
            let repo = required_str(&params, "repo")?;
            github_request(&state.http, &auth_token, Method::GET, &repo_endpoint(repo, "/actions/workflows?per_page=100")?, None).await
        }
        "actions.runs" => {
            let repo = required_str(&params, "repo")?;
            let suffix = if let Some(workflow) = optional_str(&params, "workflowId") {
                format!("/actions/workflows/{}/runs?per_page=100", encode_component(workflow))
            } else {
                "/actions/runs?per_page=100".to_string()
            };
            github_request(&state.http, &auth_token, Method::GET, &repo_endpoint(repo, &suffix)?, None).await
        }
        "actions.jobs" => {
            let repo = required_str(&params, "repo")?;
            let run_id = required_u64(&params, "runId")?;
            github_request(&state.http, &auth_token, Method::GET, &repo_endpoint(repo, &format!("/actions/runs/{run_id}/jobs?per_page=100"))?, None).await
        }
        "actions.job_logs" => job_logs(&state.http, &auth_token, &params).await,
        "actions.dispatch" => {
            let repo = required_str(&params, "repo")?;
            let workflow = required_str(&params, "workflowId")?;
            let body = json!({
                "ref": required_str(&params, "ref")?,
                "inputs": params.get("inputs").cloned().unwrap_or_else(|| json!({}))
            });
            github_request(
                &state.http,
                &auth_token,
                Method::POST,
                &repo_endpoint(repo, &format!("/actions/workflows/{}/dispatches", encode_component(workflow)))?,
                Some(body),
            )
            .await
        }
        "actions.rerun" | "actions.cancel" => {
            let repo = required_str(&params, "repo")?;
            let run_id = required_u64(&params, "runId")?;
            let op = action.trim_start_matches("actions.");
            github_request(
                &state.http,
                &auth_token,
                Method::POST,
                &repo_endpoint(repo, &format!("/actions/runs/{run_id}/{op}"))?,
                Some(json!({})),
            )
            .await
        }
        "actions.workflow.enable" | "actions.workflow.disable" => {
            let repo = required_str(&params, "repo")?;
            let workflow = required_str(&params, "workflowId")?;
            let op = if action.ends_with("enable") { "enable" } else { "disable" };
            github_request(
                &state.http,
                &auth_token,
                Method::PUT,
                &repo_endpoint(repo, &format!("/actions/workflows/{}/{op}", encode_component(workflow)))?,
                Some(json!({})),
            )
            .await
        }
        "release.list" => {
            let repo = required_str(&params, "repo")?;
            github_request(&state.http, &auth_token, Method::GET, &repo_endpoint(repo, "/releases?per_page=100")?, None).await
        }
        "release.get" => {
            let repo = required_str(&params, "repo")?;
            let release_id = required_u64(&params, "releaseId")?;
            github_request(&state.http, &auth_token, Method::GET, &repo_endpoint(repo, &format!("/releases/{release_id}"))?, None).await
        }
        "release.create" => {
            let repo = required_str(&params, "repo")?;
            let release = params.get("release").cloned().ok_or_else(|| connector_error("missing required parameter 'release'"))?;
            github_request(&state.http, &auth_token, Method::POST, &repo_endpoint(repo, "/releases")?, Some(release)).await
        }
        "release.update" => {
            let repo = required_str(&params, "repo")?;
            let release_id = required_u64(&params, "releaseId")?;
            let release = params.get("release").cloned().ok_or_else(|| connector_error("missing required parameter 'release'"))?;
            github_request(&state.http, &auth_token, Method::PATCH, &repo_endpoint(repo, &format!("/releases/{release_id}"))?, Some(release)).await
        }
        "release.delete" => {
            let repo = required_str(&params, "repo")?;
            let release_id = required_u64(&params, "releaseId")?;
            github_request(&state.http, &auth_token, Method::DELETE, &repo_endpoint(repo, &format!("/releases/{release_id}"))?, None).await
        }
        "tag.create" => create_tag(&state.http, &auth_token, &params).await,
        _ => Err(connector_error(format!("unsupported action '{action}'"))),
    }
}

fn required_u64(params: &Value, key: &str) -> Result<u64, AppError> {
    params
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| connector_error(format!("missing required numeric parameter '{key}'")))
}

async fn account_capabilities(client: &Client, token: &str) -> Result<Value, AppError> {
    let response = client
        .get(format!("{API_ROOT}/user"))
        .headers(auth_headers(token)?)
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|error| connector_error(error.to_string()))?;
    let status = response.status();
    let headers = response.headers().clone();
    let (_headers, body) = read_bounded(response, MAX_JSON_BYTES).await?;
    let user: Value = serde_json::from_slice(&body)
        .map_err(|error| connector_error(format!("invalid GitHub account response: {error}")))?;
    Ok(json!({
        "status": status.as_u16(),
        "login": user.get("login"),
        "classicScopes": headers.get("x-oauth-scopes").and_then(|v| v.to_str().ok()).unwrap_or(""),
        "acceptedPermissions": headers.get("x-accepted-github-permissions").and_then(|v| v.to_str().ok()).unwrap_or("")
    }))
}

async fn put_file(client: &Client, token: &str, params: &Value) -> Result<Value, AppError> {
    let repo = required_str(params, "repo")?;
    let path = required_str(params, "path")?;
    validate_repo_path(path)?;
    let content = if let Some(encoded) = optional_str(params, "contentBase64") {
        STANDARD
            .decode(encoded)
            .map_err(|error| connector_error(format!("contentBase64 is invalid: {error}")))?
    } else {
        required_str(params, "content")?.as_bytes().to_vec()
    };
    if content.len() > 8 * 1024 * 1024 {
        return Err(connector_error("interactive GitHub file writes are limited to 8 MB"));
    }
    let encoded_path = path.split('/').map(encode_component).collect::<Vec<_>>().join("/");
    let mut body = json!({
        "message": required_str(params, "message")?,
        "content": STANDARD.encode(content),
        "branch": optional_str(params, "branch").unwrap_or("main")
    });
    if let Some(sha) = optional_str(params, "sha") {
        body["sha"] = Value::String(sha.to_string());
    }
    github_request(
        client,
        token,
        Method::PUT,
        &repo_endpoint(repo, &format!("/contents/{encoded_path}"))?,
        Some(body),
    )
    .await
}

async fn delete_file(client: &Client, token: &str, params: &Value) -> Result<Value, AppError> {
    let repo = required_str(params, "repo")?;
    let path = required_str(params, "path")?;
    validate_repo_path(path)?;
    let encoded_path = path.split('/').map(encode_component).collect::<Vec<_>>().join("/");
    let body = json!({
        "message": required_str(params, "message")?,
        "sha": required_str(params, "sha")?,
        "branch": optional_str(params, "branch").unwrap_or("main")
    });
    github_request(
        client,
        token,
        Method::DELETE,
        &repo_endpoint(repo, &format!("/contents/{encoded_path}"))?,
        Some(body),
    )
    .await
}

async fn resolve_branch_head(client: &Client, token: &str, repo: &str, branch: &str) -> Result<String, AppError> {
    let url = repo_endpoint(repo, &format!("/git/ref/heads/{}", encode_component(branch)))?;
    let value = github_request(client, token, Method::GET, &url, None).await?;
    value
        .pointer("/object/sha")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| connector_error("GitHub branch response did not include a head SHA"))
}

async fn default_branch(client: &Client, token: &str, repo: &str) -> Result<String, AppError> {
    let value = github_request(client, token, Method::GET, &repo_endpoint(repo, "")?, None).await?;
    value
        .get("default_branch")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| connector_error("repository response did not include a default branch"))
}

async fn create_branch(client: &Client, token: &str, params: &Value) -> Result<Value, AppError> {
    let repo = required_str(params, "repo")?;
    let branch = required_str(params, "branch")?;
    if branch.contains("..") || branch.starts_with('/') || branch.ends_with('/') {
        return Err(connector_error("invalid branch name"));
    }
    let source = match optional_str(params, "sourceRef") {
        Some(value) => value.to_string(),
        None => default_branch(client, token, repo).await?,
    };
    let sha = resolve_branch_head(client, token, repo, &source).await?;
    github_request(
        client,
        token,
        Method::POST,
        &repo_endpoint(repo, "/git/refs")?,
        Some(json!({"ref": format!("refs/heads/{branch}"), "sha": sha})),
    )
    .await
}

async fn multi_file_commit(client: &Client, token: &str, params: &Value) -> Result<Value, AppError> {
    let repo = required_str(params, "repo")?;
    let branch = required_str(params, "branch")?;
    let message = required_str(params, "message")?;
    let files = params
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| connector_error("missing required array parameter 'files'"))?;
    if files.is_empty() || files.len() > 100 {
        return Err(connector_error("multi-file commit requires between 1 and 100 file changes"));
    }
    let parent_sha = resolve_branch_head(client, token, repo, branch).await?;
    if let Some(expected) = optional_str(params, "expectedHeadSha") {
        if expected != parent_sha {
            return Err(connector_error(format!(
                "branch head changed: expected {expected}, found {parent_sha}"
            )));
        }
    }
    let parent = github_request(
        client,
        token,
        Method::GET,
        &repo_endpoint(repo, &format!("/git/commits/{parent_sha}"))?,
        None,
    )
    .await?;
    let base_tree = parent
        .pointer("/tree/sha")
        .and_then(Value::as_str)
        .ok_or_else(|| connector_error("parent commit did not include a tree SHA"))?;
    let mut tree_entries = Vec::with_capacity(files.len());
    for file in files {
        let path = required_str(file, "path")?;
        validate_repo_path(path)?;
        if file.get("delete").and_then(Value::as_bool).unwrap_or(false) {
            tree_entries.push(json!({"path": path, "mode": "100644", "type": "blob", "sha": Value::Null}));
            continue;
        }
        let (content, encoding) = if let Some(encoded) = optional_str(file, "contentBase64") {
            (encoded.to_string(), "base64")
        } else {
            (required_str(file, "content")?.to_string(), "utf-8")
        };
        let blob = github_request(
            client,
            token,
            Method::POST,
            &repo_endpoint(repo, "/git/blobs")?,
            Some(json!({"content": content, "encoding": encoding})),
        )
        .await?;
        let sha = blob
            .get("sha")
            .and_then(Value::as_str)
            .ok_or_else(|| connector_error("created blob did not include a SHA"))?;
        tree_entries.push(json!({"path": path, "mode": "100644", "type": "blob", "sha": sha}));
    }
    let tree = github_request(
        client,
        token,
        Method::POST,
        &repo_endpoint(repo, "/git/trees")?,
        Some(json!({"base_tree": base_tree, "tree": tree_entries})),
    )
    .await?;
    let tree_sha = tree
        .get("sha")
        .and_then(Value::as_str)
        .ok_or_else(|| connector_error("created tree did not include a SHA"))?;
    let commit = github_request(
        client,
        token,
        Method::POST,
        &repo_endpoint(repo, "/git/commits")?,
        Some(json!({"message": message, "tree": tree_sha, "parents": [parent_sha]})),
    )
    .await?;
    let commit_sha = commit
        .get("sha")
        .and_then(Value::as_str)
        .ok_or_else(|| connector_error("created commit did not include a SHA"))?;
    let updated_ref = github_request(
        client,
        token,
        Method::PATCH,
        &repo_endpoint(repo, &format!("/git/refs/heads/{}", encode_component(branch)))?,
        Some(json!({"sha": commit_sha, "force": false})),
    )
    .await?;
    Ok(json!({"commit": commit, "ref": updated_ref}))
}

async fn job_logs(client: &Client, token: &str, params: &Value) -> Result<Value, AppError> {
    let repo = required_str(params, "repo")?;
    let job_id = required_u64(params, "jobId")?;
    let response = client
        .get(repo_endpoint(repo, &format!("/actions/jobs/{job_id}/logs"))?)
        .headers(auth_headers(token)?)
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|error| connector_error(error.to_string()))?;
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let (_headers, bytes) = read_bounded(response, MAX_LOG_BYTES).await?;
    if content_type.starts_with("text/") || std::str::from_utf8(&bytes).is_ok() {
        return Ok(json!({
            "jobId": job_id,
            "contentType": content_type,
            "text": String::from_utf8_lossy(&bytes)
        }));
    }
    Ok(json!({
        "jobId": job_id,
        "contentType": content_type,
        "dataBase64": STANDARD.encode(bytes)
    }))
}

async fn create_tag(client: &Client, token: &str, params: &Value) -> Result<Value, AppError> {
    let repo = required_str(params, "repo")?;
    let tag = required_str(params, "tag")?;
    if tag.contains("..") || tag.starts_with('/') || tag.ends_with('/') {
        return Err(connector_error("invalid tag name"));
    }
    let target = if let Some(sha) = optional_str(params, "sha") {
        sha.to_string()
    } else {
        let reference = optional_str(params, "ref").unwrap_or("main");
        resolve_branch_head(client, token, repo, reference).await?
    };
    github_request(
        client,
        token,
        Method::POST,
        &repo_endpoint(repo, "/git/refs")?,
        Some(json!({"ref": format!("refs/tags/{tag}"), "sha": target})),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_validation_blocks_traversal() {
        assert!(validate_repo("owner/repo").is_ok());
        assert!(validate_repo("owner/repo/extra").is_err());
        assert!(validate_repo("../repo").is_err());
    }

    #[test]
    fn path_validation_blocks_parent_segments() {
        assert!(validate_repo_path("src/main.rs").is_ok());
        assert!(validate_repo_path(".github/workflows/ci.yml").is_ok());
        assert!(validate_repo_path("src/../secret").is_err());
    }

    #[test]
    fn write_actions_require_approval() {
        assert!(is_mutating("commit.multi_file"));
        assert!(is_mutating("pr.merge"));
        assert!(is_mutating("actions.dispatch"));
        assert!(!is_mutating("actions.runs"));
        assert!(!is_mutating("file.get"));
    }
}
