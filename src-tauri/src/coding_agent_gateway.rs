//! Loopback endpoint for the VS Code extension.
//!
//! The desktop runtime keeps a single resident llama-server, so its loaded model is whatever
//! was used last (often OpenMindAI Core for chat). Talking to that server directly would make
//! VS Code silently use a different model than the coding agent configured in
//! Settings -> Agent Setup. This endpoint resolves the coding agent from the same saved
//! preferences OpenAgent uses, loads it with the same launch configuration, budgets
//! `max_tokens` against the real slot context, and forwards the request.
//!
//! The endpoint is advertised through a descriptor file next to the installation pointer so
//! the extension can find it without guessing ports. The descriptor carries a per-session
//! token that every request except `/health` must present, and an instance id the extension
//! checks so a stale descriptor never leads it to an unrelated process on a reused port.
//!
//! Model loading goes through `agent_runtime`, which waits for slow cold starts and shares
//! one startup between concurrent requests. Clients can start a load without blocking via
//! `POST /openmindai/coding-agent/start` and poll `GET /openmindai/coding-agent`.

use std::{env, fs, path::PathBuf, sync::OnceLock, time::Duration};

use chrono::Utc;
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Manager};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use uuid::Uuid;

use crate::{
    agent_runtime::{self, AgentModelInfo, AgentPhase},
    app_error::AppError,
    coding_control, AppState,
};

pub const SERVICE_NAME: &str = "openmindai-coding-agent";
const DESCRIPTOR_FILE: &str = "coding-agent-endpoint.json";
const CLIENT_HEADER: &str = "x-openmindai-client";
const TOKEN_HEADER: &str = "x-openmindai-token";
const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_OUTPUT_TOKENS: i64 = 2048;
pub const MIN_OUTPUT_TOKENS: i64 = 16;
pub const MAX_OUTPUT_TOKENS: i64 = 8192;
/// Tokens kept free for chat-template framing that tokenizing the prompt may miss.
pub const CONTEXT_SAFETY_MARGIN: i64 = 64;

/// Starts the endpoint on a background task. Failures are logged and never block startup.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Err(error) = serve(app).await {
            tracing::warn!(%error, "coding agent endpoint unavailable");
        }
    });
}

/// Per-process secrets shared with the extension through the descriptor file only.
struct Session {
    token: String,
    instance_id: String,
}

fn session() -> &'static Session {
    static SESSION: OnceLock<Session> = OnceLock::new();
    SESSION.get_or_init(|| Session {
        token: format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()),
        instance_id: Uuid::new_v4().to_string(),
    })
}

async fn serve(app: AppHandle) -> Result<(), AppError> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    write_descriptor(&app, &endpoint)?;
    tracing::info!(%endpoint, "coding agent endpoint listening");
    loop {
        let (stream, _) = listener.accept().await?;
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = handle_connection(app, stream).await {
                tracing::debug!(%error, "coding agent endpoint connection failed");
            }
        });
    }
}

/// Lives next to the installation pointer (`%LOCALAPPDATA%\OpenMindAI` on Windows).
pub fn descriptor_path() -> Option<PathBuf> {
    if let Ok(value) = env::var("OPENMINDAI_CODING_AGENT_DESCRIPTOR") {
        if !value.trim().is_empty() {
            return Some(PathBuf::from(value));
        }
    }
    dirs::config_local_dir().map(|dir| dir.join("OpenMindAI").join(DESCRIPTOR_FILE))
}

/// Writes the descriptor atomically (temp file + rename) so a reader never sees a partial
/// file. The file lives in the user's local app data, which only that user can read.
fn write_descriptor(app: &AppHandle, endpoint: &str) -> Result<(), AppError> {
    let path = descriptor_path()
        .ok_or_else(|| AppError::internal("could not determine local app data directory"))?;
    let state = app.state::<AppState>();
    let session = session();
    let descriptor = json!({
        "service": SERVICE_NAME,
        "endpoint": endpoint,
        "pid": std::process::id(),
        "instanceId": session.instance_id,
        "token": session.token,
        "root": state.root.root().display().to_string(),
        "startedAt": Utc::now().to_rfc3339(),
    });
    write_atomically(
        &path,
        &serde_json::to_vec_pretty(&descriptor).unwrap_or_default(),
    )
}

fn write_atomically(path: &std::path::Path, bytes: &[u8]) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    fs::write(&temp, bytes)?;
    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(error.into());
    }
    Ok(())
}

/// Removes the descriptor on exit if it still belongs to this process.
pub fn remove_descriptor() {
    let Some(path) = descriptor_path() else {
        return;
    };
    let owned = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|value| value["instanceId"] == session().instance_id.as_str());
    if owned {
        let _ = fs::remove_file(path);
    }
}

struct HttpRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

struct HttpResponse {
    status: u16,
    body: Value,
}

impl HttpResponse {
    fn ok(body: Value) -> Self {
        Self { status: 200, body }
    }

    fn error(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            body: json!({"error": {"message": message.into()}}),
        }
    }
}

async fn handle_connection(app: AppHandle, mut stream: TcpStream) -> Result<(), AppError> {
    let response = match tokio::time::timeout(READ_TIMEOUT, read_request(&mut stream)).await {
        Ok(Ok(request)) => {
            tokio::select! {
                response = route(&app, request) => response,
                // The client gave up (for example its request timeout fired). Dropping the
                // route future closes the upstream llama-server request, which stops the
                // abandoned generation instead of letting it slow later requests.
                _ = client_disconnected(&stream) => return Ok(()),
            }
        }
        Ok(Err(response)) => response,
        Err(_) => HttpResponse::error(408, "request timed out"),
    };
    write_response(&mut stream, response).await
}

/// Resolves once the client closes its side of the connection.
async fn client_disconnected(stream: &TcpStream) {
    let mut probe = [0u8; 1];
    loop {
        match stream.peek(&mut probe).await {
            Ok(0) | Err(_) => return,
            // Unexpected extra bytes after the request; keep waiting without spinning.
            Ok(_) => tokio::time::sleep(Duration::from_millis(250)).await,
        }
    }
}

async fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, HttpResponse> {
    let mut buffer = Vec::with_capacity(4096);
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        if let Some(position) = find_head_end(&buffer) {
            break position;
        }
        if buffer.len() > MAX_HEADER_BYTES {
            return Err(HttpResponse::error(431, "request headers too large"));
        }
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|_| HttpResponse::error(400, "failed to read request"))?;
        if read == 0 {
            return Err(HttpResponse::error(400, "incomplete request"));
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let (method, path, headers) =
        parse_head(&buffer[..head_end]).map_err(|message| HttpResponse::error(400, message))?;
    if headers
        .iter()
        .any(|(key, value)| key == "transfer-encoding" && !value.eq_ignore_ascii_case("identity"))
    {
        return Err(HttpResponse::error(
            411,
            "chunked request bodies are not supported",
        ));
    }
    let length = headers
        .iter()
        .find(|(key, _)| key == "content-length")
        .map(|(_, value)| value.parse::<usize>())
        .transpose()
        .map_err(|_| HttpResponse::error(400, "invalid content-length"))?
        .unwrap_or(0);
    if length > MAX_BODY_BYTES {
        return Err(HttpResponse::error(413, "request body too large"));
    }
    let mut body = buffer[head_end + 4..].to_vec();
    while body.len() < length {
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|_| HttpResponse::error(400, "failed to read request body"))?;
        if read == 0 {
            return Err(HttpResponse::error(400, "incomplete request body"));
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(length);
    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn find_head_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

/// Method, path and lower-cased headers of a request.
type RequestHead = (String, String, Vec<(String, String)>);

/// Parses the request line and headers. Header names are lower-cased.
fn parse_head(head: &[u8]) -> Result<RequestHead, String> {
    let text = std::str::from_utf8(head).map_err(|_| "request head is not UTF-8".to_string())?;
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split(' ');
    let (Some(method), Some(target), Some(version)) = (parts.next(), parts.next(), parts.next())
    else {
        return Err("malformed request line".to_string());
    };
    if !version.starts_with("HTTP/1.") {
        return Err("unsupported HTTP version".to_string());
    }
    let path = target.split('?').next().unwrap_or_default().to_string();
    let mut headers = Vec::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (key, value) = line
            .split_once(':')
            .ok_or_else(|| "malformed header".to_string())?;
        headers.push((key.trim().to_ascii_lowercase(), value.trim().to_string()));
    }
    Ok((method.to_string(), path, headers))
}

async fn write_response(stream: &mut TcpStream, response: HttpResponse) -> Result<(), AppError> {
    let body = serde_json::to_vec(&response.body).unwrap_or_default();
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        response.status,
        reason_phrase(response.status),
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&body).await?;
    stream.shutdown().await?;
    Ok(())
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    }
}

/// Browsers always send Origin on cross-site requests and cannot add the client header
/// without a CORS preflight this endpoint never grants, so web pages cannot drive it.
/// Also requires the per-session token (except for `/health`), which only processes that can
/// read the current user's descriptor file know.
fn reject_untrusted(request: &HttpRequest, expected_token: &str) -> Option<HttpResponse> {
    if request.header("origin").is_some() {
        return Some(HttpResponse::error(
            403,
            "browser requests are not accepted",
        ));
    }
    if request.header(CLIENT_HEADER).is_none() {
        return Some(HttpResponse::error(
            403,
            "missing X-OpenMindAI-Client header",
        ));
    }
    let open = matches!(request.path.as_str(), "/health" | "/healthz");
    if !open && !token_matches(request.header(TOKEN_HEADER), expected_token) {
        return Some(HttpResponse::error(
            401,
            "missing or invalid OpenMindAI session token; reconnect to the running OpenMindAI desktop app",
        ));
    }
    None
}

fn token_matches(presented: Option<&str>, expected: &str) -> bool {
    let Some(presented) = presented else {
        return false;
    };
    presented.len() == expected.len()
        && presented
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

async fn route(app: &AppHandle, request: HttpRequest) -> HttpResponse {
    if let Some(response) = reject_untrusted(&request, &session().token) {
        return response;
    }
    let result = match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/health") | ("GET", "/healthz") => Ok(HttpResponse::ok(
            json!({"status": "ok", "service": SERVICE_NAME}),
        )),
        ("GET", "/openmindai/coding-agent") => agent_info(app).await.map(HttpResponse::ok),
        ("POST", "/openmindai/coding-agent/start") => start(app).await.map(HttpResponse::ok),
        ("GET", "/v1/models") | ("GET", "/readyz") => models(app),
        ("POST", "/v1/chat/completions") => chat(app, &request.body).await,
        (
            _,
            "/health"
            | "/healthz"
            | "/openmindai/coding-agent"
            | "/openmindai/coding-agent/start"
            | "/v1/models"
            | "/readyz"
            | "/v1/chat/completions",
        ) => Ok(HttpResponse::error(405, "method not allowed")),
        _ => Ok(HttpResponse::error(404, "not found")),
    };
    result.unwrap_or_else(|error| HttpResponse::error(500, error.to_string()))
}

/// Identity, Agent Setup configuration and live runtime state (no model load).
async fn agent_info(app: &AppHandle) -> Result<Value, AppError> {
    let status = agent_runtime::agent_status(app).await?;
    Ok(json!({
        "service": SERVICE_NAME,
        "instanceId": session().instance_id,
        "configured": status.model.is_some(),
        "source": "Settings -> Agent Setup",
        "codingEnabled": status.coding_enabled,
        "model": status.model,
        "contextSize": status.configured_context,
        "gpuLayers": status.gpu_layers,
        "parallelism": status.parallel_workers,
        "maxOutputTokens": MAX_OUTPUT_TOKENS,
        "message": status.message,
        "runtime": status,
    }))
}

/// Starts loading the coding agent in the background and returns its status at once.
async fn start(app: &AppHandle) -> Result<Value, AppError> {
    let context = agent_runtime::plan_agent_runtime(app)?;
    if let Some(plan) = context.plan.filter(|_| context.coding_enabled) {
        if agent_runtime::agent_status(app).await?.state != AgentPhase::Ready {
            agent_runtime::start_in_background(app, plan);
        }
    }
    agent_info(app).await
}

fn models(app: &AppHandle) -> Result<HttpResponse, AppError> {
    let Some(plan) = agent_runtime::plan_agent_runtime(app)?.plan else {
        return Ok(HttpResponse::error(503, agent_runtime::NO_AGENT_MESSAGE));
    };
    Ok(HttpResponse::ok(json!({
        "object": "list",
        "data": [{
            "id": plan.model.id,
            "object": "model",
            "owned_by": "openmindai",
            "meta": {
                "name": plan.model.name,
                "repository": plan.model.source_repository,
                "n_ctx": plan.config.context_size,
            }
        }]
    })))
}

async fn chat(app: &AppHandle, raw_body: &[u8]) -> Result<HttpResponse, AppError> {
    let Ok(Value::Object(request)) = serde_json::from_slice::<Value>(raw_body) else {
        return Ok(HttpResponse::error(
            400,
            "request body must be a JSON object",
        ));
    };
    let Some(messages) = request.get("messages").and_then(Value::as_array).cloned() else {
        return Ok(HttpResponse::error(400, "messages must be an array"));
    };
    if messages.is_empty() {
        return Ok(HttpResponse::error(400, "messages must not be empty"));
    }

    let context = agent_runtime::plan_agent_runtime(app)?;
    let Some(plan) = context.plan else {
        return Ok(HttpResponse::error(503, agent_runtime::NO_AGENT_MESSAGE));
    };
    if !context.coding_enabled {
        return Ok(HttpResponse::error(403, agent_runtime::DISABLED_MESSAGE));
    }

    // Waits for a cold start (up to agent_runtime::AGENT_STARTUP_TIMEOUT) and joins any
    // startup already in progress instead of launching another one. Runs as its own task so
    // a client disconnect cannot abandon a model load halfway.
    let acquire = {
        let app = app.clone();
        let plan = plan.clone();
        tauri::async_runtime::spawn(async move {
            agent_runtime::acquire_agent_runtime(&app, &plan).await
        })
    };
    let acquired = acquire
        .await
        .map_err(|error| AppError::internal(error.to_string()))?;
    // The lease keeps the agent model resident until this request has been answered.
    let (endpoint, _model_lease) = match acquired {
        Ok(acquired) => acquired,
        Err(error) => {
            return Ok(HttpResponse::error(
                503,
                format!("{} could not be loaded: {error}", plan.model.name),
            ))
        }
    };
    let http = app.state::<AppState>().http.clone();
    let base = endpoint.trim_end_matches('/').to_string();

    let (slot_context, _) = agent_runtime::effective_context(
        agent_runtime::slot_context_size(&http, &base).await,
        plan.config.context_size,
        plan.config.parallelism,
    );
    let requested = requested_max_tokens(&request);
    let mut forwarded = forward_body(request, &plan.model.id, requested);
    let template_kwargs = forwarded["chat_template_kwargs"].clone();
    let prompt_tokens = match count_prompt_tokens(&http, &base, &messages, &template_kwargs).await {
        Some(tokens) => tokens,
        None => estimate_prompt_tokens(&messages),
    };
    let Some(max_tokens) = output_budget(requested, slot_context, prompt_tokens) else {
        return Ok(HttpResponse::error(
            400,
            format!(
                "The request (~{prompt_tokens} prompt tokens) does not fit the coding agent context window ({slot_context} tokens per request). Send less editor context, or raise Context size or lower Parallel read workers in OpenMindAI Settings -> Agent Setup."
            ),
        ));
    };

    forwarded["max_tokens"] = json!(max_tokens);
    let (status, mut payload) =
        post_with_retry(&http, &format!("{base}/v1/chat/completions"), &forwarded).await?;
    // Under severe memory pressure the GPU runtime was observed to emit one token forever
    // (`<tool_call><tool_call>…`). Never return that as an answer; reload instead.
    let content = payload
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if is_degenerate(content) {
        agent_runtime::unload_corrupted_agent(app, &plan).await;
        return Ok(HttpResponse::error(
            502,
            format!(
                "{} returned corrupted output (one fragment repeated until the token limit), so it was unloaded and will reload on the next request. This can happen when the computer is very low on memory. Try again.",
                plan.model.name
            ),
        ));
    }
    if let Value::Object(object) = &mut payload {
        object.insert(
            "openmindai".to_string(),
            json!({
                "route": "coding-agent",
                "model": AgentModelInfo::from_model(&plan.model),
                "runtimePlacement": plan.decision.placement,
                "contextSize": slot_context,
                "promptTokens": prompt_tokens,
                "requestedMaxTokens": requested,
                "maxTokens": max_tokens,
            }),
        );
    }
    Ok(HttpResponse {
        status,
        body: payload,
    })
}

/// Exact prompt size using the model's own chat template and tokenizer.
async fn count_prompt_tokens(
    http: &reqwest::Client,
    base: &str,
    messages: &[Value],
    template_kwargs: &Value,
) -> Option<i64> {
    let applied: Value = http
        .post(format!("{base}/apply-template"))
        .timeout(Duration::from_secs(10))
        .json(&json!({"messages": messages, "chat_template_kwargs": template_kwargs}))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let prompt = applied.get("prompt")?.as_str()?;
    let tokenized: Value = http
        .post(format!("{base}/tokenize"))
        .timeout(Duration::from_secs(10))
        .json(&json!({"content": prompt}))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    Some(tokenized.get("tokens")?.as_array()?.len() as i64)
}

pub fn estimate_prompt_tokens(messages: &[Value]) -> i64 {
    messages
        .iter()
        .map(|message| {
            let content = message
                .get("content")
                .map(|content| match content {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_default();
            coding_control::estimate_tokens(&content) + 8
        })
        .sum()
}

pub fn requested_max_tokens(request: &Map<String, Value>) -> i64 {
    request
        .get("max_tokens")
        .or_else(|| request.get("max_completion_tokens"))
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_OUTPUT_TOKENS)
        .clamp(MIN_OUTPUT_TOKENS, MAX_OUTPUT_TOKENS)
}

/// Output budget that fits the slot context, or `None` when the prompt leaves no usable room.
pub fn output_budget(requested: i64, slot_context: i64, prompt_tokens: i64) -> Option<i64> {
    let available = slot_context - prompt_tokens - CONTEXT_SAFETY_MARGIN;
    if available < MIN_OUTPUT_TOKENS {
        return None;
    }
    Some(
        requested
            .clamp(MIN_OUTPUT_TOKENS, MAX_OUTPUT_TOKENS)
            .min(available),
    )
}

/// Tool-call markup the agent models emit. This endpoint offers no tools, so an answer made
/// only of these tags carries no content.
const TOOL_CALL_TAGS: [&str; 4] = [
    "<tool_call>",
    "</tool_call>",
    "<tool_response>",
    "</tool_response>",
];

/// True for output a broken runtime produces instead of an answer:
/// - nothing but tool-call markup (ignoring whitespace and control characters), or
/// - one fragment of up to 32 characters repeated across at least 95% of a long answer
///   possibly after some leading junk.
///
/// Real answers, including ones that discuss `<tool_call>` or contain repetitive code or
/// tables, keep other text and are not periodic over their whole length.
pub fn is_degenerate(content: &str) -> bool {
    let compact: Vec<char> = content
        .chars()
        .filter(|character| !character.is_whitespace() && !character.is_control())
        .collect();
    if compact.is_empty() {
        // Empty answers are reported separately as "no answer content".
        return false;
    }
    let compact_text: String = compact.iter().collect();
    let mut remainder = compact_text.clone();
    let mut tags = 0;
    for tag in TOOL_CALL_TAGS {
        tags += remainder.matches(tag).count();
        remainder = remainder.replace(tag, "");
    }
    if tags > 0 && remainder.is_empty() {
        return true;
    }
    // A runaway fills the token budget; short answers are never judged by periodicity.
    if compact.len() < 200 {
        return false;
    }
    (1..=32usize).any(|period| {
        if compact.len() / period < 20 {
            return false;
        }
        // Compare each character with the one `period` earlier, so leading junk only costs
        // a few mismatches instead of breaking the alignment.
        let mismatches = compact
            .iter()
            .enumerate()
            .skip(period)
            .filter(|(index, character)| **character != compact[index - period])
            .count();
        mismatches * 20 <= compact.len()
    })
}

/// Pins the request to the Agent Setup model and its token budget.
pub fn forward_body(mut request: Map<String, Value>, model_id: &str, max_tokens: i64) -> Value {
    request.insert("model".to_string(), json!(model_id));
    request.insert("max_tokens".to_string(), json!(max_tokens));
    request.remove("max_completion_tokens");
    request.insert("stream".to_string(), json!(false));
    // NVIDIA Nemotron 3 chat templates default enable_thinking to true, and reasoning
    // tokens share max_tokens with the answer. Desktop OpenAgent requests disable it the
    // same way; a client that explicitly sets the flag keeps its choice.
    request
        .entry("chat_template_kwargs".to_string())
        .or_insert_with(|| json!({"enable_thinking": false}));
    // Same sampling as desktop OpenAgent requests. llama-server's general defaults
    // (temperature 0.8) let the small agent models ramble or repeat until max_tokens.
    crate::sampling::SamplingProfile::AGENT.apply_defaults(&mut request);
    Value::Object(request)
}

async fn post_with_retry(
    http: &reqwest::Client,
    url: &str,
    body: &Value,
) -> Result<(u16, Value), AppError> {
    let mut retry = 0u8;
    loop {
        let response = http.post(url).json(body).send().await.map_err(|error| {
            AppError::InferenceFailed(format!("coding agent request failed: {error}"))
        })?;
        let status = response.status();
        if status == reqwest::StatusCode::SERVICE_UNAVAILABLE && retry < 10 {
            retry += 1;
            tokio::time::sleep(Duration::from_millis(600)).await;
            continue;
        }
        let text = response.text().await.unwrap_or_default();
        let payload = serde_json::from_str::<Value>(&text).unwrap_or_else(|_| {
            json!({"error": {"message": if text.trim().is_empty() {
                format!("coding agent returned HTTP {status} with an empty body")
            } else {
                text.chars().take(500).collect::<String>()
            }}})
        });
        let status = if status.as_u16() >= 200 {
            status.as_u16()
        } else {
            502
        };
        return Ok((status, payload));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_request_head_with_lowercase_headers() {
        let (method, path, headers) = parse_head(
            b"POST /v1/chat/completions?x=1 HTTP/1.1\r\nHost: 127.0.0.1\r\nX-OpenMindAI-Client: vscode\r\nContent-Length: 2",
        )
        .unwrap();
        assert_eq!(method, "POST");
        assert_eq!(path, "/v1/chat/completions");
        assert!(headers.contains(&("x-openmindai-client".to_string(), "vscode".to_string())));
        assert!(headers.contains(&("content-length".to_string(), "2".to_string())));
    }

    #[test]
    fn rejects_malformed_request_lines() {
        assert!(parse_head(b"GARBAGE").is_err());
        assert!(parse_head(b"GET / SPDY/3").is_err());
    }

    fn request_with(headers: &[(&str, &str)]) -> HttpRequest {
        HttpRequest {
            method: "GET".to_string(),
            path: "/health".to_string(),
            headers: headers
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
            body: Vec::new(),
        }
    }

    #[test]
    fn rejects_browser_and_anonymous_requests() {
        let token = "secret-token";
        assert_eq!(
            reject_untrusted(&request_with(&[]), token).unwrap().status,
            403
        );
        assert_eq!(
            reject_untrusted(
                &request_with(&[
                    ("x-openmindai-client", "vscode"),
                    ("x-openmindai-token", token),
                    ("origin", "http://evil.example")
                ]),
                token
            )
            .unwrap()
            .status,
            403
        );
        // /health only needs the client header.
        assert!(
            reject_untrusted(&request_with(&[("x-openmindai-client", "vscode")]), token).is_none()
        );
    }

    #[test]
    fn requires_the_session_token_outside_health() {
        let token = "secret-token";
        let mut request = request_with(&[("x-openmindai-client", "vscode")]);
        request.path = "/v1/chat/completions".to_string();
        assert_eq!(reject_untrusted(&request, token).unwrap().status, 401);
        request
            .headers
            .push(("x-openmindai-token".to_string(), "secret-tokeX".to_string()));
        assert_eq!(reject_untrusted(&request, token).unwrap().status, 401);
        request.headers.pop();
        request
            .headers
            .push(("x-openmindai-token".to_string(), token.to_string()));
        assert!(reject_untrusted(&request, token).is_none());
        assert!(!token_matches(Some("short"), token));
        assert!(!token_matches(None, token));
    }

    #[test]
    fn descriptor_writes_are_atomic_replacements() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("coding-agent-endpoint.json");
        write_atomically(&path, b"{\"a\":1}").unwrap();
        write_atomically(&path, b"{\"a\":2}").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"{\"a\":2}");
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers.len(), 1, "{leftovers:?}");
    }

    #[test]
    fn requested_tokens_default_and_clamp() {
        let empty = Map::new();
        assert_eq!(requested_max_tokens(&empty), DEFAULT_OUTPUT_TOKENS);
        let mut huge = Map::new();
        huge.insert("max_tokens".to_string(), json!(1_000_000));
        assert_eq!(requested_max_tokens(&huge), MAX_OUTPUT_TOKENS);
        let mut tiny = Map::new();
        tiny.insert("max_completion_tokens".to_string(), json!(1));
        assert_eq!(requested_max_tokens(&tiny), MIN_OUTPUT_TOKENS);
    }

    #[test]
    fn output_budget_respects_context_window() {
        // Plenty of room: the configured request wins.
        assert_eq!(output_budget(2048, 8192, 1000), Some(2048));
        // Small slot: shrink to what fits after the prompt and safety margin.
        assert_eq!(
            output_budget(2048, 4096, 3000),
            Some(4096 - 3000 - CONTEXT_SAFETY_MARGIN)
        );
        // Prompt already fills the window.
        assert_eq!(output_budget(2048, 4096, 4090), None);
    }

    #[test]
    fn output_budget_follows_effective_slot_context() {
        use crate::agent_runtime::effective_context;
        // 1 worker: the whole configured context per request.
        let (one, _) = effective_context(None, 8192, 1);
        assert_eq!(output_budget(2048, one, 1000), Some(2048));
        // 2 workers: 4096 per request, so a 3000-token prompt leaves ~1000 tokens.
        let (two, _) = effective_context(None, 8192, 2);
        assert_eq!(
            output_budget(2048, two, 3000),
            Some(4096 - 3000 - CONTEXT_SAFETY_MARGIN)
        );
        // Raising the configured context raises the per-request budget.
        let (bigger, _) = effective_context(None, 16384, 2);
        assert_eq!(output_budget(2048, bigger, 3000), Some(2048));
        // Custom maxOutputTokens below capacity is respected.
        assert_eq!(output_budget(300, two, 1000), Some(300));
        // Prompt near the limit leaves only the minimum.
        assert_eq!(
            output_budget(2048, two, 4096 - CONTEXT_SAFETY_MARGIN - 16),
            Some(16)
        );
        // Prompt too large.
        assert_eq!(output_budget(2048, two, 4096), None);
        // A runtime that reports a unified 8192-token slot beats naive division.
        let (reported, source) = effective_context(Some(8192), 8192, 2);
        assert_eq!(source, "runtime");
        assert_eq!(output_budget(2048, reported, 5000), Some(2048));
    }

    #[test]
    fn forward_body_pins_agent_model_and_disables_nemotron_thinking() {
        let mut request = Map::new();
        request.insert("model".to_string(), json!("openmind-local"));
        request.insert(
            "messages".to_string(),
            json!([{"role": "user", "content": "hi"}]),
        );
        request.insert("max_completion_tokens".to_string(), json!(4000));
        request.insert("stream".to_string(), json!(true));
        let body = forward_body(request, "gguf-agent", 1500);
        assert_eq!(body["model"], "gguf-agent");
        assert_eq!(body["max_tokens"], 1500);
        assert_eq!(body["stream"], false);
        assert!(body.get("max_completion_tokens").is_none());
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(body["temperature"], 0.15);
        assert_eq!(body["top_p"], 0.85);
        assert_eq!(body["top_k"], 20);
    }

    #[test]
    fn detects_repeated_tool_call_garbage() {
        assert!(is_degenerate(&"<tool_call>".repeat(60)));
        // Truncated at the token limit mid-tag.
        assert!(is_degenerate(&format!(
            "{}<tool_",
            "<tool_call>".repeat(40)
        )));
        // Interleaved whitespace and newlines.
        assert!(is_degenerate(&"<tool_call>\n ".repeat(50)));
        assert!(is_degenerate(&"a".repeat(500)));
    }

    #[test]
    fn detects_tool_call_only_responses() {
        assert!(is_degenerate("<tool_call>"));
        assert!(is_degenerate("<tool_call><tool_call><tool_call>"));
        assert!(is_degenerate("  <tool_call>\n</tool_call>\n "));
        assert!(is_degenerate(
            "<tool_call></tool_call><tool_response></tool_response>"
        ));
    }

    #[test]
    fn detects_mixed_garbage_with_leading_junk_and_control_text() {
        let junk = format!("\u{0}\u{1}  ``\n{}", "<tool_call>".repeat(45));
        assert!(is_degenerate(&junk));
        let control = format!("{}\u{7}\r\n", "<tool_call>\u{1b}".repeat(30));
        assert!(is_degenerate(&control));
    }

    #[test]
    fn keeps_legitimate_answers_that_mention_or_repeat_things() {
        assert!(!is_degenerate("A Rust trait defines shared behavior."));
        assert!(!is_degenerate(
            "Hermes-style models wrap a function call in a `<tool_call>` tag, for example \
             `<tool_call>{\"name\": \"search\"}</tool_call>`, and the client runs it."
        ));
        assert!(!is_degenerate(
            "```xml\n<tool_call>\n  {\"name\": \"read_file\"}\n</tool_call>\n```\nThat is the format."
        ));
        let table = "| a | b |\n".repeat(30) + "Done.";
        assert!(!is_degenerate(&table));
        let code = "fn main() {\n    println!(\"hi\");\n}\n".repeat(10);
        assert!(!is_degenerate(&code));
        let list = (1..=40)
            .map(|n| format!("- item {n}\n"))
            .collect::<String>();
        assert!(!is_degenerate(&list));
        assert!(!is_degenerate(""));
        assert!(!is_degenerate("   \n"));
    }

    #[test]
    fn forward_body_keeps_client_sampling() {
        let mut request = Map::new();
        request.insert("temperature".to_string(), json!(0.7));
        let body = forward_body(request, "gguf-agent", 64);
        assert_eq!(body["temperature"], 0.7);
        assert_eq!(body["top_k"], 20);
    }

    #[test]
    fn forward_body_keeps_explicit_template_kwargs() {
        let mut request = Map::new();
        request.insert(
            "chat_template_kwargs".to_string(),
            json!({"enable_thinking": true}),
        );
        let body = forward_body(request, "gguf-agent", 64);
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], true);
    }

    #[test]
    fn estimates_prompt_tokens_with_message_overhead() {
        let messages = vec![json!({"role": "user", "content": "abcdefgh"})];
        assert_eq!(estimate_prompt_tokens(&messages), 2 + 8);
    }
}
