//! Native-only Cloud and loopback GPU-worker transports. Tokens never enter a webview request.
use crate::app_error::AppError;
use crate::github::secret_store;
use base64::Engine;
use reqwest::{Client, Method, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::Manager;

const CLOUD_BASE: &str = "https://cloud.openmindai.org/api/v1/";
const CREDENTIAL_SLOT: &str = "openmindai-cloud-user-session";
const LOCAL_GPU_BASE: &str = "http://127.0.0.1:8765/";

static GPU_WORKER: OnceLock<Mutex<Option<Child>>> = OnceLock::new();

fn worker_process() -> &'static Mutex<Option<Child>> {
    GPU_WORKER.get_or_init(|| Mutex::new(None))
}

pub(crate) fn stop_gpu_worker() {
    if let Ok(mut running) = worker_process().lock() {
        if let Some(mut worker) = running.take() {
            let _ = worker.kill();
            let _ = worker.wait();
        }
    }
}

fn worker_script(app: &tauri::AppHandle) -> Result<PathBuf, AppError> {
    if let Ok(resource) = app.path().resolve(
        "local-3d-worker/worker.py",
        tauri::path::BaseDirectory::Resource,
    ) {
        if resource.is_file() {
            return Ok(resource);
        }
    }
    // Development checkouts can run without a complete Tauri bundle.
    let current = std::env::current_dir().map_err(|_| failure("Cannot locate the local worker"))?;
    for candidate in [
        current.join("services/local-3d-worker/worker.py"),
        current.join("../services/local-3d-worker/worker.py"),
    ] {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(failure("Bundled local GPU worker is missing"))
}

fn spawn_python_worker(script: &Path) -> Result<Child, AppError> {
    #[cfg(windows)]
    let (python, prefix): (&str, &[&str]) = ("py", &["-3"]);
    #[cfg(not(windows))]
    let (python, prefix): (&str, &[&str]) = ("python3", &[]);
    let mut cmd = Command::new(python);
    cmd.args(prefix)
        .arg("-E")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn().map_err(|_| {
        failure(
            "Python 3 was not found. Install Python 3 plus numpy and pyopencl for local GPU meshes",
        )
    })
}

#[tauri::command]
pub(crate) async fn local_gpu_start(app: tauri::AppHandle) -> Result<Value, AppError> {
    // Avoid duplicate workers; never terminate an unrelated worker on this port.
    if let Ok(status) = local_gpu_health().await {
        return Ok(status);
    }
    let script = worker_script(&app)?;
    {
        let mut guard = worker_process()
            .lock()
            .map_err(|_| failure("GPU worker state locked"))?;
        let running = match guard.as_mut() {
            Some(child) => child
                .try_wait()
                .map_err(|_| failure("Could not inspect GPU worker"))?
                .is_none(),
            None => false,
        };
        if !running {
            guard.take();
            *guard = Some(spawn_python_worker(&script)?);
        }
    }
    for _ in 0..15 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if let Ok(status) = local_gpu_health().await {
            return Ok(status);
        }
    }
    Err(failure("GPU worker failed to start or did not bind 127.0.0.1:8765. Check OpenCL drivers and Python dependencies."))
}

#[tauri::command]
pub(crate) fn local_gpu_stop() -> Result<(), AppError> {
    stop_gpu_worker();
    Ok(())
}

async fn local_gpu_health() -> Result<Value, AppError> {
    let response = local_client()?
        .get(format!("{LOCAL_GPU_BASE}health"))
        .header("X-OpenMindAI-Desktop", "1")
        .send()
        .await
        .map_err(|_| failure("Local GPU worker is offline"))?;
    decode(response).await
}

fn failure(message: impl Into<String>) -> AppError {
    AppError::Internal(format!("Cloud/Studio: {}", message.into()))
}

fn client() -> Result<Client, AppError> {
    Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(35))
        // Cloud uses OS proxy settings, unlike the loopback GPU worker.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| failure("Could not initialize the API connection"))
}

fn local_client() -> Result<Client, AppError> {
    Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(35))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| failure("Could not initialize the local GPU connection"))
}

fn allowed_cloud_path(method: &Method, path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    match (method.as_str(), parts.as_slice()) {
        ("GET", ["me"]) | ("GET", ["mobile", "models"]) => true,
        ("POST", ["web-search"]) | ("POST", ["mobile", "chat", "completions"]) => true,
        ("GET", ["research", "status" | "projects"]) => true,
        ("POST", ["research", "projects"]) => true,
        ("GET", ["research", "projects", id]) if valid_id(id) => true,
        ("POST", ["research", "projects", id, "runs" | "control"]) if valid_id(id) => true,
        ("GET", ["research", "projects", id, "events"]) if valid_id(id) => true,
        ("GET", ["3d-studio", "engines" | "jobs" | "assets"]) => true,
        ("POST", ["3d-studio", "jobs", "scene" | "image-model"]) => true,
        ("POST", ["3d-scenes", "uploads"]) => true,
        ("GET", ["3d-studio", "jobs", id]) if valid_uuid(id) => true,
        ("POST", ["3d-studio", "jobs", id, "cancel"]) if valid_uuid(id) => true,

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
    match (method.as_str(), parts.as_slice()) {
        ("GET", ["health"]) | ("POST", ["jobs"]) => true,
        ("GET", ["jobs", id]) if valid_uuid(id) => true,
        ("GET", ["jobs", id, "mesh"]) if valid_uuid(id) => true,
        ("POST", ["jobs", id, "cancel"]) if valid_uuid(id) => true,
        _ => false,
    }
}

async fn decode(response: reqwest::Response) -> Result<Value, AppError> {
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|_| failure("Response could not be read"))?;
    if bytes.len() > 2_000_000 {
        return Err(failure("Response exceeds the 2 MB safety limit"));
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| failure(format!("Server returned non-JSON response ({status})")))?;
    if !status.is_success() {
        let detail = value
            .get("message")
            .or_else(|| value.get("error").and_then(|error| error.get("message")))
            .and_then(Value::as_str)
            .unwrap_or("Request rejected");
        // Do not echo credentials or unbounded upstream payloads into the UI.
        return Err(failure(format!(
            "HTTP {status}: {}",
            detail.chars().take(240).collect::<String>()
        )));
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
    let response = client()?
        .get(format!("{CLOUD_BASE}me"))
        .bearer_auth(token)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|_| failure("Cloud API is unreachable"))?;
    decode(response).await
}

#[tauri::command]
pub(crate) fn cloud_begin_regeneration(
    conversation_id: String,
    assistant_message_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::chat::Message, AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| failure("Conversation database is unavailable"))?;
    let repository = crate::chat::ChatRepository::new(&db);
    // Reuse the same regeneration marker and immutable original reply as local chat.
    repository.begin_regeneration(&conversation_id, &assistant_message_id, None)
}

#[tauri::command]
pub(crate) async fn cloud_connect(token: String) -> Result<CloudConnection, AppError> {
    let token = token.trim();
    if token.is_empty() || token.len() > 4000 || token.contains('\r') || token.contains('\n') {
        return Err(failure("Invalid user-session token"));
    }
    let profile = validate_token(token).await?;
    secret_store::set_secret(CREDENTIAL_SLOT, token)?;
    Ok(CloudConnection {
        connected: true,
        user: Some(profile),
        message: None,
    })
}

#[tauri::command]
pub(crate) async fn cloud_login(
    email: String,
    password: String,
    code: Option<String>,
) -> Result<CloudConnection, AppError> {
    if email.trim().len() > 254 || !email.contains('@') || password.is_empty() {
        return Err(failure("Enter a valid email and password"));
    }
    let response = client()?
        .post(format!("{CLOUD_BASE}login"))
        .header("Accept", "application/json")
        .json(&json!({
            "email": email.trim(),
            "password": password,
            "code": code.unwrap_or_default(),
            "device_name": "OpenMindAI Desktop"
        }))
        .send()
        .await
        .map_err(|_| failure("Cloud login is unreachable"))?;
    if response.status() == StatusCode::ACCEPTED {
        // An unfamiliar-IP verification challenge must be completed via the account flow.
        return Err(failure("Sign-in needs new-device verification. Approve the security email, then sign in again."));
    }
    let value = decode(response).await?;
    let token = value
        .get("token")
        .and_then(Value::as_str)
        .ok_or_else(|| failure("Cloud did not return a user-session token"))?;
    let profile = validate_token(token).await?;
    secret_store::set_secret(CREDENTIAL_SLOT, token)?;
    Ok(CloudConnection {
        connected: true,
        user: Some(profile),
        message: None,
    })
}

#[tauri::command]
pub(crate) async fn cloud_status() -> Result<CloudConnection, AppError> {
    let Some(token) = secret_store::get_secret(CREDENTIAL_SLOT)? else {
        return Ok(CloudConnection {
            connected: false,
            user: None,
            message: None,
        });
    };
    match validate_token(&token).await {
        Ok(profile) => Ok(CloudConnection {
            connected: true,
            user: Some(profile),
            message: None,
        }),
        Err(_) => Ok(CloudConnection {
            connected: false,
            user: None,
            message: Some("Cloud is unavailable or your session expired. Sign in again.".into()),
        }),
    }
}

#[tauri::command]
pub(crate) fn cloud_disconnect() -> Result<(), AppError> {
    secret_store::delete_secret(CREDENTIAL_SLOT)
}

#[tauri::command]
pub(crate) async fn cloud_request(
    path: String,
    method: String,
    payload: Option<Value>,
) -> Result<Value, AppError> {
    let method =
        Method::from_bytes(method.as_bytes()).map_err(|_| failure("Invalid HTTP method"))?;
    if !allowed_cloud_path(&method, &path) {
        return Err(failure("Cloud API operation is not approved"));
    }
    let token = secret_store::get_secret(CREDENTIAL_SLOT)?
        .ok_or_else(|| failure("Sign in to Cloud first"))?;
    // Align with Mobile: Research and 3D jobs can legitimately take minutes.
    // A short chat timeout would be mistaken for Cloud failure even when the
    // request is accepted and still running remotely.
    let timeout_seconds = if path.starts_with("research/")
        || path.starts_with("3d-studio/")
        || path.starts_with("3d-scenes/")
    {
        180
    } else if path == "mobile/chat/completions" {
        120
    } else {
        35
    };
    let mut request = client()?
        .request(method, format!("{CLOUD_BASE}{path}"))
        .timeout(Duration::from_secs(timeout_seconds))
        .bearer_auth(token)
        .header("Accept", "application/json");
    if let Some(payload) = payload {
        let json_bytes =
            serde_json::to_vec(&payload).map_err(|_| failure("Invalid JSON payload"))?;
        let cap = if path == "3d-scenes/uploads" {
            4_500_000
        } else {
            64_000
        };
        if json_bytes.len() > cap {
            return Err(failure("Payload exceeds the endpoint size limit"));
        }
        request = request
            .header("Content-Type", "application/json")
            .body(json_bytes);
    }
    decode(
        request
            .send()
            .await
            .map_err(|_| failure("Cloud API request failed"))?,
    )
    .await
}

/// Selects only chat-completion models advertised as enabled by the user-owned
/// Laravel Cloud account. The model cannot be overridden by an untrusted editor.
fn preferred_coding_model(catalog: &Value) -> Option<String> {
    let models = catalog.get("data")?.as_array()?;
    let available: Vec<&str> = models
        .iter()
        .filter(|item| {
            item.get("available").and_then(Value::as_bool) == Some(true)
                && item.get("operation").and_then(Value::as_str) == Some("chat.completions")
        })
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .filter(|id| !id.is_empty() && id.len() < 120)
        .collect();
    for id in ["openmindai-super", "openmindai-ultra", "openmindai-lite"] {
        if available.contains(&id) {
            return Some(id.to_string());
        }
    }
    available.first().map(|id| (*id).to_string())
}

/// A read-only, authenticated preflight for the Coding Agent. In Both mode,
/// failure here permits Local fallback BEFORE a Cloud completion POST is sent.
pub(crate) async fn coding_cloud_model() -> Result<String, AppError> {
    let token = secret_store::get_secret(CREDENTIAL_SLOT)?
        .ok_or_else(|| failure("Sign in to Cloud before using a hosted Coding Agent"))?;
    let catalog = decode(
        client()?
            .get(format!("{CLOUD_BASE}mobile/models"))
            .timeout(Duration::from_secs(12))
            .bearer_auth(token)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|_| failure("Cloud model catalog is unreachable"))?,
    )
    .await?;
    preferred_coding_model(&catalog)
        .ok_or_else(|| failure("No hosted chat model is enabled for your Cloud account"))
}

/// Exactly one hosted POST. Ambiguous remote timeouts must never trigger a
/// silent replay of a coding action on a different model.
pub(crate) async fn coding_cloud_completion(
    model: &str,
    messages: &[Value],
    max_tokens: i64,
) -> Result<(u16, Value), AppError> {
    if messages.is_empty() {
        return Err(failure("Coding Agent requires messages"));
    }
    let token = secret_store::get_secret(CREDENTIAL_SLOT)?
        .ok_or_else(|| failure("Cloud account disconnected before the Coding Agent request"))?;
    let body = json!({
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
        "stream": false,
    });
    let size = serde_json::to_vec(&body).map_err(|_| failure("Invalid Coding Agent payload"))?;
    if size.len() > 512_000 {
        return Err(failure(
            "Coding Agent context exceeds 512 KB; send less editor context",
        ));
    }
    let response = client()?
        .post(format!("{CLOUD_BASE}mobile/chat/completions"))
        .timeout(Duration::from_secs(120))
        .bearer_auth(token)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .body(size)
        .send()
        .await
        .map_err(|_| failure("Hosted Coding Agent request failed; remote acceptance is unknown, so it was not retried"))?;
    let data = decode(response).await?;
    if data.pointer("/choices/0/message").is_none() {
        return Err(failure("Cloud Coding Agent returned an invalid completion"));
    }
    Ok((200, data))
}

#[tauri::command]
pub(crate) async fn local_gpu_request(
    path: String,
    method: String,
    payload: Option<Value>,
) -> Result<Value, AppError> {
    let method =
        Method::from_bytes(method.as_bytes()).map_err(|_| failure("Invalid HTTP method"))?;
    if !allowed_gpu_path(&method, &path) {
        return Err(failure("Local GPU operation is not approved"));
    }
    let http = local_client()?;
    if path != "health" {
        let health = decode(
            http.get(format!("{LOCAL_GPU_BASE}health"))
                .header("X-OpenMindAI-Desktop", "1")
                .send()
                .await
                .map_err(|_| failure("Local 3D GPU worker is offline (127.0.0.1:8765)"))?,
        )
        .await?;
        if health.get("gpu_active").and_then(Value::as_bool) != Some(true) {
            return Err(failure(
                "Local 3D worker has no active GPU. No silent CPU/cloud fallback.",
            ));
        }
    }
    let mut request = http
        .request(method, format!("{LOCAL_GPU_BASE}{path}"))
        .header("X-OpenMindAI-Desktop", "1")
        .header("Accept", "application/json");
    if let Some(payload) = payload {
        let bytes = serde_json::to_vec(&payload).map_err(|_| failure("Invalid JSON payload"))?;
        if bytes.len() > 64_000 {
            return Err(failure("Payload exceeds 64 KB"));
        }
        request = request
            .header("Content-Type", "application/json")
            .body(bytes);
    }
    decode(
        request
            .send()
            .await
            .map_err(|_| failure("Local 3D GPU worker is unreachable"))?,
    )
    .await
}

#[tauri::command]
pub(crate) async fn cloud_download_mesh(asset_id: u64) -> Result<Value, AppError> {
    if asset_id == 0 {
        return Err(failure("A valid mesh asset id is required"));
    }
    let token = secret_store::get_secret(CREDENTIAL_SLOT)?
        .ok_or_else(|| failure("Sign in to Cloud first"))?;
    let response = client()?
        .get(format!("{CLOUD_BASE}3d-studio/assets/{asset_id}/download"))
        .bearer_auth(token)
        .header("Accept", "application/octet-stream")
        .send()
        .await
        .map_err(|_| failure("Mesh download failed"))?;
    if !response.status().is_success() {
        return Err(failure(format!(
            "Mesh download returned HTTP {}",
            response.status()
        )));
    }
    if response.content_length().unwrap_or(0) > 16 * 1024 * 1024 {
        return Err(failure("Mesh exceeds the 16 MB desktop download limit"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| failure("Could not read mesh"))?;
    if bytes.len() > 16 * 1024 * 1024 || bytes.len() < 12 || &bytes[0..4] != b"glTF" {
        return Err(failure("Expected a valid GLB asset (16 MB maximum)"));
    }
    Ok(json!({
        "filename": format!("openmindai-cloud-{asset_id}.glb"),
        "data_base64": base64::engine::general_purpose::STANDARD.encode(bytes),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hosted_coding_model_uses_only_enabled_completion_models() {
        let catalog = json!({"data": [
            {"id":"openmindai-ultra","available":false,"operation":"chat.completions"},
            {"id":"image-only","available":true,"operation":"images.generate"},
            {"id":"openmindai-lite","available":true,"operation":"chat.completions"},
            {"id":"openmindai-super","available":true,"operation":"chat.completions"}
        ]});
        assert_eq!(
            preferred_coding_model(&catalog).as_deref(),
            Some("openmindai-super")
        );
        assert!(preferred_coding_model(&json!({"data": []})).is_none());
    }

    #[test]
    fn cloud_route_gate_rejects_secrets_and_path_traversal() {
        assert!(allowed_cloud_path(&Method::POST, "web-search"));
        assert!(allowed_cloud_path(&Method::POST, "3d-studio/jobs/scene"));
        assert!(allowed_cloud_path(
            &Method::POST,
            "research/projects/42/runs"
        ));
        assert!(allowed_cloud_path(&Method::GET, "research/projects/42"));
        for path in [
            "../admin",
            "developer/keys",
            "research/projects/../runs",
            "3d-studio/jobs/bad",
        ] {
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
