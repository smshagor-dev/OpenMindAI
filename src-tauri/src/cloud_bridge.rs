//! Native-only Cloud and loopback GPU-worker transports. Tokens never enter a webview request.
use crate::app_error::AppError;
use crate::github::secret_store;
use reqwest::{Client, Method, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;

const CLOUD_BASE: &str = "https://api.openmindai.org/api/v1/";
const CREDENTIAL_SLOT: &str = "openmindai-cloud-user-session";
const LOCAL_GPU_BASE: &str = "http://127.0.0.1:8765/";

fn failure(message: impl Into<String>) -> AppError {
    AppError::Internal(format!("Cloud/Studio: {}", message.into()))
}

fn client() -> Result<Client, AppError> {
    Client::builder()
        .timeout(Duration::from_secs(35))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| failure("Could not initialize the API connection"))
}

fn allowed_cloud_path(method: &Method, path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    match (method, parts.as_slice()) {
        (&Method::GET, ["me" | "research/status" | "3d-studio/engines"]) => true,
        (&Method::POST, ["web-search" | "mobile/chat/completions"]) => true,
        (&Method::GET, ["research", "status" | "projects"]) => true,
        (&Method::POST, ["research", "projects"]) => true,
        (&Method::GET, ["research", "projects", id]) if valid_id(id) => true,
        (&Method::POST, ["research", "projects", id, "runs" | "control"]) if valid_id(id) => true,
        (&Method::GET, ["research", "projects", id, "events"]) if valid_id(id) => true,
        (&Method::GET, ["3d-studio", "engines" | "jobs" | "assets"]) => true,
        (&Method::POST, ["3d-studio", "jobs", "scene"]) => true,
        (&Method::GET, ["3d-studio", "jobs", id]) if valid_uuid(id) => true,
        (&Method::POST, ["3d-studio", "jobs", id, "cancel"]) if valid_uuid(id) => true,
        (&Method::GET, ["3d-studio", "assets", id, "data"]) if valid_id(id) => true,
        _ => false,
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 18 && id.bytes().all(|c| c.is_ascii_digit())
}

fn valid_uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok()
}

fn allowed_gpu_path(method: &Method, path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    match (method, parts.as_slice()) {
        (&Method::GET, ["health"]) | (&Method::POST, ["jobs"]) => true,
        (&Method::GET, ["jobs", id]) if valid_uuid(id) => true,
        (&Method::POST, ["jobs", id, "cancel"]) if valid_uuid(id) => true,
        _ => false,
    }
}

async fn decode(response: reqwest::Response) -> Result<Value, AppError> {
    let status = response.status();
    let bytes = response.bytes().await.map_err(|_| failure("Response could not be read"))?;
    if bytes.len() > 2_000_000 {
        return Err(failure("Response exceeds the 2 MB safety limit"));
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| failure(format!("Server returned non-JSON response ({status})")))?;
    if !status.is_success() {
        let detail = value.get("message")
            .or_else(|| value.get("error").and_then(|error| error.get("message")))
            .and_then(Value::as_str)
            .unwrap_or("Request rejected");
        // Do not echo credentials or unbounded upstream payloads into the UI.
        return Err(failure(format!("HTTP {status}: {}", detail.chars().take(240).collect::<String>())));
    }
    Ok(value)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloudConnection {
    connected: bool,
    user: Option<Value>,
    message: Option<String>,
}

async fn validate_token(token: &str) -> Result<Value, AppError> {
    let response = client()?.get(format!("{CLOUD_BASE}me"))
        .bearer_auth(token).header("Accept", "application/json")
        .send().await.map_err(|_| failure("Cloud API is unreachable"))?;
    decode(response).await
}

#[tauri::command]
pub(crate) async fn cloud_connect(token: String) -> Result<CloudConnection, AppError> {
    let token = token.trim();
    if token.is_empty() || token.len() > 4000 || token.contains(['\r', '\n']) {
        return Err(failure("Invalid user-session token"));
    }
    let profile = validate_token(token).await?;
    secret_store::set_secret(CREDENTIAL_SLOT, token)?;
    Ok(CloudConnection { connected: true, user: Some(profile), message: None })
}

#[tauri::command]
pub(crate) async fn cloud_login(email: String, password: String, code: Option<String>) -> Result<CloudConnection, AppError> {
    if email.trim().len() > 254 || !email.contains('@') || password.is_empty() {
        return Err(failure("Enter a valid email and password"));
    }
    let response = client()?.post(format!("{CLOUD_BASE}login"))
        .header("Accept", "application/json")
        .json(&json!({
            "email": email.trim(),
            "password": password,
            "code": code.unwrap_or_default(),
            "device_name": "OpenMindAI Desktop"
        }))
        .send().await.map_err(|_| failure("Cloud login is unreachable"))?;
    if response.status() == StatusCode::ACCEPTED {
        // An unfamiliar-IP verification challenge must be completed via the account flow.
        return Err(failure("Sign-in needs new-device verification. Approve the security email, then sign in again."));
    }
    let value = decode(response).await?;
    let token = value.get("token").and_then(Value::as_str)
        .ok_or_else(|| failure("Cloud did not return a user-session token"))?;
    let profile = validate_token(token).await?;
    secret_store::set_secret(CREDENTIAL_SLOT, token)?;
    Ok(CloudConnection { connected: true, user: Some(profile), message: None })
}

#[tauri::command]
pub(crate) async fn cloud_status() -> Result<CloudConnection, AppError> {
    let Some(token) = secret_store::get_secret(CREDENTIAL_SLOT)? else {
        return Ok(CloudConnection { connected: false, user: None, message: None });
    };
    match validate_token(&token).await {
        Ok(profile) => Ok(CloudConnection { connected: true, user: Some(profile), message: None }),
        Err(_) => Ok(CloudConnection {
            connected: false, user: None,
            message: Some("Cloud is unavailable or your session expired. Sign in again.".into()),
        }),
    }
}

#[tauri::command]
pub(crate) fn cloud_disconnect() -> Result<(), AppError> {
    secret_store::delete_secret(CREDENTIAL_SLOT)
}

#[tauri::command]
pub(crate) async fn cloud_request(path: String, method: String, payload: Option<Value>) -> Result<Value, AppError> {
    let method = Method::from_bytes(method.as_bytes()).map_err(|_| failure("Invalid HTTP method"))?;
    if !allowed_cloud_path(&method, &path) {
        return Err(failure("Cloud API operation is not approved"));
    }
    let token = secret_store::get_secret(CREDENTIAL_SLOT)?.ok_or_else(|| failure("Sign in to Cloud first"))?;
    let mut request = client()?.request(method, format!("{CLOUD_BASE}{path}"))
        .bearer_auth(token).header("Accept", "application/json");
    if let Some(payload) = payload {
        let json_bytes = serde_json::to_vec(&payload).map_err(|_| failure("Invalid JSON payload"))?;
        if json_bytes.len() > 64_000 { return Err(failure("Payload exceeds 64 KB")); }
        request = request.header("Content-Type", "application/json").body(json_bytes);
    }
    decode(request.send().await.map_err(|_| failure("Cloud API request failed"))?).await
}

#[tauri::command]
pub(crate) async fn local_gpu_request(path: String, method: String, payload: Option<Value>) -> Result<Value, AppError> {
    let method = Method::from_bytes(method.as_bytes()).map_err(|_| failure("Invalid HTTP method"))?;
    if !allowed_gpu_path(&method, &path) {
        return Err(failure("Local GPU operation is not approved"));
    }
    let http = client()?;
    if path != "health" {
        let health = decode(http.get(format!("{LOCAL_GPU_BASE}health"))
            .send().await.map_err(|_| failure("Local 3D GPU worker is offline (127.0.0.1:8765)"))?).await?;
        if health.get("gpu_active").and_then(Value::as_bool) != Some(true) {
            return Err(failure("Local 3D worker has no active GPU. No silent CPU/cloud fallback."));
        }
    }
    let mut request = http.request(method, format!("{LOCAL_GPU_BASE}{path}"))
        .header("Accept", "application/json");
    if let Some(payload) = payload {
        let bytes = serde_json::to_vec(&payload).map_err(|_| failure("Invalid JSON payload"))?;
        if bytes.len() > 64_000 { return Err(failure("Payload exceeds 64 KB")); }
        request = request.header("Content-Type", "application/json").body(bytes);
    }
    decode(request.send().await.map_err(|_| failure("Local 3D GPU worker is unreachable"))?).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cloud_route_gate_rejects_secrets_and_path_traversal() {
        assert!(allowed_cloud_path(&Method::POST, "web-search"));
        assert!(allowed_cloud_path(&Method::POST, "3d-studio/jobs/scene"));
        assert!(allowed_cloud_path(&Method::POST, "research/projects/42/runs"));
        assert!(allowed_cloud_path(&Method::GET, "research/projects/42"));
        for path in ["../admin", "developer/keys", "research/projects/../runs", "3d-studio/jobs/bad"] {
            assert!(!allowed_cloud_path(&Method::GET, path));
            assert!(!allowed_cloud_path(&Method::POST, path));
        }
    }
    #[test]
    fn worker_is_loopback_and_strictly_scoped() {
        assert!(LOCAL_GPU_BASE.starts_with("http://127.0.0.1:"));
        assert!(allowed_gpu_path(&Method::GET, "health"));
        assert!(allowed_gpu_path(&Method::POST, "jobs"));
        assert!(!allowed_gpu_path(&Method::POST, "health"));
        assert!(!allowed_gpu_path(&Method::GET, "../health"));
    }
}
