//! Native-only Cloud and loopback 3D-worker transports. Tokens never enter a webview request.
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
    Err(failure("Bundled local 3D geometry worker is missing"))
}

fn dependency_manager_script(app: &tauri::AppHandle) -> Result<PathBuf, AppError> {
    if let Ok(resource) = app.path().resolve(
        "local-3d-worker/dependency_manager.py",
        tauri::path::BaseDirectory::Resource,
    ) {
        if resource.is_file() {
            return Ok(resource);
        }
    }
    let current =
        std::env::current_dir().map_err(|_| failure("Cannot locate the dependency manager"))?;
    for candidate in [
        current.join("services/local-3d-worker/dependency_manager.py"),
        current.join("../services/local-3d-worker/dependency_manager.py"),
    ] {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(failure("Bundled 3D dependency manager is missing"))
}

fn run_dependency_manager(app: &tauri::AppHandle, action: &str) -> Result<Value, AppError> {
    if !matches!(action, "inspect" | "install") {
        return Err(failure("Unsupported dependency manager operation"));
    }
    let script = dependency_manager_script(app)?;
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|_| failure("Cannot locate app-managed storage"))?;
    let override_python = std::env::var_os("OPENMINDAI_3D_PYTHON");
    #[cfg(windows)]
    let candidates: Vec<(std::ffi::OsString, Vec<&str>)> = match override_python {
        Some(path) => vec![(path, vec![])],
        // This manifest currently supports exactly CPython 3.13 x64. `py -3`
        // can select an unsupported free-threaded 3.14 interpreter, so prefer
        // the pinned ABI and only use the generic launcher for a clear report.
        None => vec![("py".into(), vec!["-3.13"]), ("py".into(), vec!["-3"])],
    };
    #[cfg(not(windows))]
    let candidates: Vec<(std::ffi::OsString, Vec<&str>)> =
        vec![(override_python.unwrap_or_else(|| "python3".into()), vec![])];
    if action == "install" {
        let managed = app
            .path()
            .app_data_dir()
            .map_err(|_| failure("Cannot locate app-managed storage"))?
            .join("3d-runtime");
        std::fs::create_dir_all(&managed)
            .map_err(|_| failure("Cannot create app-managed dependency storage"))?;
        let cancel = managed.join("install.cancel");
        let _ = std::fs::remove_file(&cancel);
        let _ = cancel;
    }
    let mut last_error = None;
    let mut completed = None;
    for (python, prefix) in candidates {
        let mut command = Command::new(python);
        command
            .args(prefix)
            .arg("-E")
            .arg(&script)
            .arg(action)
            .arg(&data_dir);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        match command.output() {
            Ok(output) => {
                // A started interpreter that returned a structured status is
                // authoritative, including the unsupported-ABI response.
                if String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .any(|line| serde_json::from_str::<Value>(line).is_ok())
                {
                    completed = Some(output);
                    break;
                }
                last_error = Some(output);
            }
            Err(_) => continue,
        }
    }
    let output = completed
        .or(last_error)
        .ok_or_else(|| failure("Python 3 could not launch the 3D dependency manager"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let result = stdout
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<Value>(line).ok())
        .ok_or_else(|| {
            let stderr = String::from_utf8_lossy(&output.stderr);
            failure(format!(
                "3D dependency manager returned no status ({})",
                stderr.trim()
            ))
        })?;
    if !output.status.success() {
        return Err(failure(
            result
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("3D dependency manager failed"),
        ));
    }
    Ok(result)
}

#[tauri::command]
pub(crate) fn studio3d_dependency_plan(app: tauri::AppHandle) -> Result<Value, AppError> {
    run_dependency_manager(&app, "inspect")
}

#[tauri::command]
pub(crate) async fn studio3d_install_dependencies(
    app: tauri::AppHandle,
    approved: bool,
) -> Result<Value, AppError> {
    if !approved {
        return Err(failure(
            "Optional dependency installation requires explicit user approval",
        ));
    }
    let result = tokio::task::spawn_blocking(move || run_dependency_manager(&app, "install"))
        .await
        .map_err(|_| failure("Dependency installation task stopped unexpectedly"))??;
    // Restart only the worker process owned by this app so it can pick up the
    // newly activated interpreter. Never stop a process that owns the port.
    stop_gpu_worker();
    Ok(result)
}

#[tauri::command]
pub(crate) fn studio3d_cancel_dependency_install(app: tauri::AppHandle) -> Result<(), AppError> {
    let managed = app
        .path()
        .app_data_dir()
        .map_err(|_| failure("Cannot locate app-managed storage"))?
        .join("3d-runtime");
    std::fs::create_dir_all(&managed)
        .map_err(|_| failure("Cannot access app-managed dependency storage"))?;
    std::fs::write(managed.join("install.cancel"), b"cancel")
        .map_err(|_| failure("Could not request dependency installation cancellation"))
}

fn managed_python(app: &tauri::AppHandle) -> Option<PathBuf> {
    let root = app.path().app_data_dir().ok()?.join("3d-runtime/active");
    #[cfg(windows)]
    let path = root.join("Scripts/python.exe");
    #[cfg(not(windows))]
    let path = root.join("bin/python");
    path.is_file().then_some(path)
}

fn spawn_python_worker(app: &tauri::AppHandle, script: &Path) -> Result<Child, AppError> {
    let override_python = std::env::var_os("OPENMINDAI_3D_PYTHON");
    // An explicitly selected interpreter remains authoritative. Otherwise use
    // the app-owned isolated environment after it has been atomically activated.
    if override_python.is_none() {
        if let Some(python) = managed_python(app) {
            let mut cmd = Command::new(python);
            cmd.arg(script)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000);
            }
            return cmd
                .spawn()
                .map_err(|_| failure("Could not start the managed 3D worker"));
        }
    }
    #[cfg(windows)]
    let candidates: Vec<(std::ffi::OsString, Vec<&str>)> = match override_python {
        Some(path) => vec![(path, vec![])],
        None => vec![("py".into(), vec!["-3.13"]), ("py".into(), vec!["-3"])],
    };
    #[cfg(not(windows))]
    let candidates: Vec<(std::ffi::OsString, Vec<&str>)> =
        vec![(override_python.unwrap_or_else(|| "python3".into()), vec![])];
    let mut last_error = None;
    for (python, prefix) in candidates {
        let mut cmd = Command::new(python);
        cmd.args(prefix)
            .arg("-E")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        match cmd.spawn() {
            Ok(child) => return Ok(child),
            Err(error) => last_error = Some(error),
        }
    }
    Err(failure(format!(
        "Python 3 was not found. Set OPENMINDAI_3D_PYTHON to a Python 3.10+ executable; CPU geometry does not require NumPy or PyOpenCL ({})",
        last_error.map(|error| error.to_string()).unwrap_or_default()
    )))
}

#[tauri::command]
pub(crate) async fn local_gpu_start(app: tauri::AppHandle) -> Result<Value, AppError> {
    // Avoid duplicate workers; never terminate an unrelated worker on this port.
    if let Ok(status) = local_gpu_health().await {
        if status.get("worker_ready").and_then(Value::as_bool) == Some(true) {
            return Ok(status);
        }
        let owned_worker_running = worker_process()
            .lock()
            .ok()
            .and_then(|mut guard| {
                guard
                    .as_mut()
                    .map(|child| child.try_wait().map(|exit| exit.is_none()).unwrap_or(false))
            })
            .unwrap_or(false);
        if !owned_worker_running {
            // A different process owns this port. Report its health without
            // stopping it or attempting a competing worker launch.
            return Ok(status);
        }
        // The app-owned worker is alive but failed GPU initialization (for
        // example, dependencies or a driver were just repaired). Restart it.
        stop_gpu_worker();
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
            *guard = Some(spawn_python_worker(&app, &script)?);
        }
    }
    for _ in 0..15 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if let Ok(status) = local_gpu_health().await {
            if status.get("worker_ready").and_then(Value::as_bool) == Some(true) {
                return Ok(status);
            }
            let detail = status
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("local geometry worker initialization failed");
            stop_gpu_worker();
            return Err(failure(format!(
                "Local 3D worker started but is not ready: {detail}"
            )));
        }
    }
    Err(failure(
        "Local 3D worker failed to start or did not bind 127.0.0.1:8765",
    ))
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
        .map_err(|_| failure("Local geometry worker is offline"))?;
    decode(response).await
}

fn failure(message: impl Into<String>) -> AppError {
    AppError::Internal(format!("Cloud/Studio: {}", message.into()))
}

fn cloud_transport_failure(error: reqwest::Error) -> AppError {
    let detail = if error.is_timeout() {
        "request timed out"
    } else if error.is_connect() {
        "TLS, DNS, or network connection failed"
    } else {
        "request could not be sent"
    };
    failure(format!("Cloud API is unreachable ({detail})"))
}

fn client() -> Result<Client, AppError> {
    Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(35))
        // Cloud uses OS proxy settings, unlike the loopback geometry worker.
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
        // OpenMindAI developer model gateway (API-key auth).
        ("GET", ["openmindai", "models"]) => true,
        ("POST", ["openmindai", "chat", "completions"]) => true,
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
    decode_with_limit(response, 2_000_000).await
}

async fn decode_with_limit(
    response: reqwest::Response,
    maximum_bytes: usize,
) -> Result<Value, AppError> {
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|length| length > maximum_bytes as u64)
    {
        return Err(failure(format!(
            "Response exceeds the {} MB safety limit",
            maximum_bytes / 1_000_000
        )));
    }
    let upstream_request_id = [
        "x-request-id",
        "request-id",
        "openai-request-id",
        "x-correlation-id",
    ]
    .iter()
    .find_map(|name| {
        response
            .headers()
            .get(*name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    });
    let bytes = response
        .bytes()
        .await
        .map_err(|_| failure("Response could not be read"))?;
    if bytes.len() > maximum_bytes {
        return Err(failure(format!(
            "Response exceeds the {} MB safety limit",
            maximum_bytes / 1_000_000
        )));
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
        failure(format!(
            "Server returned non-JSON response ({status}){}",
            upstream_request_id
                .as_ref()
                .map(|id| format!("; upstream request ID: {id}"))
                .unwrap_or_default()
        ))
    })?;
    if !status.is_success() {
        let detail = value
            .get("message")
            .or_else(|| value.get("error").and_then(|error| error.get("message")))
            .and_then(Value::as_str)
            .unwrap_or("Request rejected");
        // Do not echo credentials or unbounded upstream payloads into the UI.
        return Err(failure(format!(
            "HTTP {status}: {}{}",
            detail.chars().take(240).collect::<String>(),
            upstream_request_id
                .as_ref()
                .map(|id| format!("; upstream request ID: {id}"))
                .unwrap_or_default()
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

/// Validates a stored credential and resolves an optional account profile.
///
/// A full-account user-session token reads its profile from `/me`. A
/// model-gateway-scoped API key (issued at api.openmindai.org) is not allowed
/// to call `/me` and would be rejected there with HTTP 403; such a key is
/// instead confirmed against the model gateway (`mobile/models`) and accepted
/// with no account profile. This keeps hosted chat working with a gateway key
/// without requiring full-account access.
async fn validate_credential(token: &str) -> Result<Option<Value>, AppError> {
    let me = client()?
        .get(format!("{CLOUD_BASE}me"))
        .bearer_auth(token)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(cloud_transport_failure)?;
    if me.status().is_success() {
        return Ok(Some(decode(me).await?));
    }

    let models = client()?
        .get(format!("{CLOUD_BASE}openmindai/models"))
        .bearer_auth(token)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(cloud_transport_failure)?;
    if models.status().is_success() {
        return Ok(None);
    }

    Err(failure(
        "This key is not valid for OpenMindAI Cloud. Use an API key from api.openmindai.org.",
    ))
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
    let profile = validate_credential(token).await?;
    secret_store::set_secret(CREDENTIAL_SLOT, token)?;
    Ok(CloudConnection {
        connected: true,
        user: profile,
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
    let profile = validate_credential(token).await?;
    secret_store::set_secret(CREDENTIAL_SLOT, token)?;
    Ok(CloudConnection {
        connected: true,
        user: profile,
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
    match validate_credential(&token).await {
        Ok(profile) => Ok(CloudConnection {
            connected: true,
            user: profile,
            message: None,
        }),
        Err(AppError::Internal(error)) if error.contains("Cloud API is unreachable") => Ok(CloudConnection {
            connected: false,
            user: None,
            message: Some("Cloud API is unreachable. Check your internet connection and retry. Your saved API key has not been removed.".into()),
        }),
        Err(_) => Ok(CloudConnection {
            connected: false,
            user: None,
            message: Some("Cloud could not verify this session. It may have expired; reconnect if the problem continues.".into()),
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
    if let Some(nemotron) = available
        .iter()
        .find(|id| id.to_ascii_lowercase().contains("nemotron"))
    {
        return Some((*nemotron).to_string());
    }
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

pub(crate) struct ResearchCloudModel {
    pub model_id: String,
    pub developer_gateway: bool,
}

pub(crate) struct ResearchCloudCompletion {
    pub data: Value,
    pub upstream_request_id: Option<String>,
    pub http_status: u16,
}

/// Prefers the configured developer model gateway (the credential desktop
/// users commonly connect with), then accepts the full-account chat catalog.
/// Catalog reads are preflight only and do not incur inference usage.
pub(crate) async fn research_cloud_model() -> Result<ResearchCloudModel, AppError> {
    let token = secret_store::get_secret(CREDENTIAL_SLOT)?
        .ok_or_else(|| failure("Connect an OpenMindAI API key before using Cloud research"))?;
    let gateway = client()?
        .get(format!("{CLOUD_BASE}openmindai/models"))
        .timeout(Duration::from_secs(12))
        .bearer_auth(&token)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|_| failure("OpenMindAI model gateway is unreachable"))?;
    if gateway.status().is_success() {
        let catalog = decode(gateway).await?;
        if let Some(model_id) = preferred_gateway_model(&catalog) {
            return Ok(ResearchCloudModel {
                model_id,
                developer_gateway: true,
            });
        }
    }
    let account = client()?
        .get(format!("{CLOUD_BASE}mobile/models"))
        .timeout(Duration::from_secs(12))
        .bearer_auth(&token)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|_| failure("Cloud model catalog is unreachable"))?;
    let catalog = decode(account).await?;
    preferred_coding_model(&catalog)
        .map(|model_id| ResearchCloudModel {
            model_id,
            developer_gateway: false,
        })
        .ok_or_else(|| {
            failure("No enabled hosted chat-completion model is available for this credential")
        })
}

fn preferred_gateway_model(catalog: &Value) -> Option<String> {
    let models = catalog.get("data")?.as_array()?;
    let available: Vec<&str> = models
        .iter()
        .filter(|item| item.get("available").and_then(Value::as_bool) == Some(true))
        .filter(|item| item.get("operation").and_then(Value::as_str) == Some("chat.completions"))
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .filter(|id| !id.trim().is_empty() && id.len() < 120)
        .collect();
    available
        .iter()
        .find(|id| id.to_ascii_lowercase().contains("nemotron"))
        .or_else(|| available.first())
        .map(|id| (*id).to_string())
}

/// Exactly one model-gateway or account completion request. A transport error
/// after submission is ambiguous; Auto must not replay it on a different model.
pub(crate) async fn research_cloud_completion(
    model: &ResearchCloudModel,
    messages: &[Value],
    max_tokens: i64,
    client_request_id: &str,
) -> Result<ResearchCloudCompletion, AppError> {
    if messages.is_empty() {
        return Err(failure("Research completion requires messages"));
    }
    let token = secret_store::get_secret(CREDENTIAL_SLOT)?
        .ok_or_else(|| failure("Cloud credential was disconnected before research inference"))?;
    let body =
        json!({"model":model.model_id,"messages":messages,"max_tokens":max_tokens,"stream":false});
    let bytes =
        serde_json::to_vec(&body).map_err(|_| failure("Invalid research completion payload"))?;
    if bytes.len() > 512_000 {
        return Err(failure(
            "Research prompt exceeds the hosted model gateway limit",
        ));
    }
    let path = if model.developer_gateway {
        "openmindai/chat/completions"
    } else {
        "mobile/chat/completions"
    };
    let response = client()?.post(format!("{CLOUD_BASE}{path}"))
        .timeout(Duration::from_secs(120)).bearer_auth(token)
        .header("X-OpenMindAI-Request-ID", client_request_id)
        .header("Content-Type","application/json").header("Accept","application/json")
        .body(bytes).send().await
        .map_err(|_| failure("Hosted research completion failed; remote acceptance is unknown and it was not retried"))?;
    let http_status = response.status().as_u16();
    let response_id = [
        "x-request-id",
        "request-id",
        "openai-request-id",
        "x-correlation-id",
    ]
    .iter()
    .find_map(|name| {
        response
            .headers()
            .get(*name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    });
    let data = decode(response).await?;
    let upstream_request_id = response_id.or_else(|| {
        ["request_id", "requestId", "id"].iter().find_map(|key| {
            data.get(*key)
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .map(str::to_owned)
        })
    });
    Ok(ResearchCloudCompletion {
        data,
        upstream_request_id,
        http_status,
    })
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
                .map_err(|_| failure("Local 3D geometry worker is offline (127.0.0.1:8765)"))?,
        )
        .await?;
        if health.get("worker_ready").and_then(Value::as_bool) != Some(true) {
            return Err(failure("Local 3D geometry worker is not ready"));
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
    let response = request
        .send()
        .await
        .map_err(|_| failure("Local 3D geometry worker is unreachable"))?;
    if path.ends_with("/mesh") {
        decode_with_limit(response, 42 * 1_000_000).await
    } else {
        decode(response).await
    }
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
    fn research_gateway_prefers_available_nemotron_chat_model() {
        let catalog = json!({"data":[
            {"id":"openmindai-super","available":true},
            {"id":"nvidia/nemotron-super","available":true,"operation":"chat.completions"},
            {"id":"nvidia/nemotron-image","available":true,"operation":"images.generate"},
            {"id":"nvidia/nemotron-disabled","available":false}
        ]});
        assert_eq!(
            preferred_gateway_model(&catalog).as_deref(),
            Some("nvidia/nemotron-super")
        );
        assert_eq!(preferred_gateway_model(&json!({"data":[{"id":"openmindai-super","available":true,"operation":"chat.completions"}]})).as_deref(), Some("openmindai-super"));
        assert_eq!(preferred_gateway_model(&json!({"data":[]})), None);
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
