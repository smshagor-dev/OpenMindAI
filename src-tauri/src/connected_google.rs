use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD},
    Engine as _,
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use reqwest::{header, Client, StatusCode, Url};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::State;
use uuid::Uuid;

use crate::{
    app_error::AppError,
    github::secret_store,
    google::GoogleRepository,
    AppState,
};

const GOOGLE_CLIENT_SECRET_SLOT: &str = "google-client-secret";
const GOOGLE_ACCESS_TOKEN_SLOT: &str = "google-access-token";
const GOOGLE_REFRESH_TOKEN_SLOT: &str = "google-refresh-token";
const GOOGLE_OAUTH_SETTINGS_KEY: &str = "app.google.oauth";
const OAUTH_AUTHORIZE_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const OAUTH_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const USERINFO_ENDPOINT: &str = "https://openidconnect.googleapis.com/v1/userinfo";
const API_TIMEOUT_SECS: u64 = 30;
const OAUTH_CALLBACK_TIMEOUT_SECS: u64 = 300;
const MAX_GMAIL_RESULTS: usize = 50;
const MAX_DRIVE_RESULTS: usize = 100;
const MAX_DRIVE_CONTENT_BYTES: usize = 5 * 1024 * 1024;
const MAX_CONTACT_RESULTS: usize = 100;

const GOOGLE_SCOPES: &[&str] = &[
    "openid",
    "email",
    "https://www.googleapis.com/auth/gmail.modify",
    "https://www.googleapis.com/auth/drive",
    "https://www.googleapis.com/auth/calendar",
    "https://www.googleapis.com/auth/contacts.readonly",
];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GoogleOAuthMetadata {
    email: Option<String>,
    expires_at: Option<String>,
    scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleConnectionStatus {
    pub credentials_configured: bool,
    pub connected: bool,
    pub email: Option<String>,
    pub expires_at: Option<String>,
    pub scopes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserInfoResponse {
    email: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailMessage {
    pub id: String,
    pub thread_id: String,
    pub snippet: String,
    pub label_ids: Vec<String>,
    pub subject: String,
    pub from: String,
    pub to: String,
    pub date: String,
    pub body_text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GmailListResponse {
    #[serde(default)]
    messages: Vec<GmailMessageRef>,
}

#[derive(Debug, Deserialize)]
struct GmailMessageRef {
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GmailApiMessage {
    id: String,
    thread_id: String,
    #[serde(default)]
    label_ids: Vec<String>,
    #[serde(default)]
    snippet: String,
    payload: Option<GmailPayload>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GmailPayload {
    #[serde(default)]
    mime_type: String,
    #[serde(default)]
    headers: Vec<GmailHeader>,
    body: Option<GmailBody>,
    #[serde(default)]
    parts: Vec<GmailPayload>,
}

#[derive(Debug, Deserialize)]
struct GmailHeader {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct GmailBody {
    data: Option<String>,
}

#[derive(Debug, Serialize)]
struct GmailSendBody {
    raw: String,
    #[serde(rename = "threadId", skip_serializing_if = "Option::is_none")]
    thread_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub modified_time: Option<String>,
    pub size: Option<String>,
    pub web_view_link: Option<String>,
    #[serde(default)]
    pub parents: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct DriveListResponse {
    #[serde(default)]
    files: Vec<DriveFile>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveFileContent {
    pub file: DriveFile,
    pub content_base64: String,
    pub content_mime_type: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarSummary {
    pub id: String,
    pub summary: String,
    pub primary: Option<bool>,
    pub access_role: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CalendarListResponse {
    #[serde(default)]
    items: Vec<CalendarSummary>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    pub id: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub html_link: Option<String>,
    pub start: Value,
    pub end: Value,
    #[serde(default)]
    pub attendees: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct CalendarEventsResponse {
    #[serde(default)]
    items: Vec<CalendarEvent>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEventInput {
    pub summary: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start: String,
    pub end: String,
    pub time_zone: Option<String>,
    #[serde(default)]
    pub attendees: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleContact {
    pub resource_name: String,
    pub display_name: String,
    pub emails: Vec<String>,
    pub phones: Vec<String>,
    pub organizations: Vec<String>,
}

fn google_error(message: impl Into<String>) -> AppError {
    AppError::internal(format!("Google Workspace: {}", message.into()))
}

fn http_client() -> Result<Client, AppError> {
    Client::builder()
        .timeout(Duration::from_secs(API_TIMEOUT_SECS))
        .build()
        .map_err(|error| google_error(error.to_string()))
}

fn oauth_metadata(state: &State<'_, AppState>) -> Result<GoogleOAuthMetadata, AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    let value: Option<String> = db
        .connection()
        .query_row(
            "SELECT value_json FROM app_settings WHERE key = ?1",
            params![GOOGLE_OAUTH_SETTINGS_KEY],
            |row| row.get(0),
        )
        .optional()?;
    value
        .map(|raw| serde_json::from_str(&raw).map_err(|error| google_error(error.to_string())))
        .transpose()
        .map(|value| value.unwrap_or_default())
}

fn save_oauth_metadata(
    state: &State<'_, AppState>,
    metadata: &GoogleOAuthMetadata,
) -> Result<(), AppError> {
    let payload = serde_json::to_string(metadata).map_err(|error| google_error(error.to_string()))?;
    let now = Utc::now().to_rfc3339();
    let db = state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    db.connection().execute(
        "INSERT INTO app_settings (key, value_json, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
        params![GOOGLE_OAUTH_SETTINGS_KEY, payload, now],
    )?;
    Ok(())
}

fn clear_oauth_metadata(state: &State<'_, AppState>) -> Result<(), AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    db.connection().execute(
        "DELETE FROM app_settings WHERE key = ?1",
        params![GOOGLE_OAUTH_SETTINGS_KEY],
    )?;
    Ok(())
}

fn google_credentials(state: &State<'_, AppState>) -> Result<(String, String), AppError> {
    let client_id = {
        let db = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        GoogleRepository::new(&db)
            .get_status()?
            .ok_or_else(|| google_error("Google Client ID/Secret are not configured"))?
            .client_id
    };
    let client_secret = secret_store::get_secret(GOOGLE_CLIENT_SECRET_SLOT)?
        .ok_or_else(|| google_error("Google Client Secret is missing"))?;
    Ok((client_id, client_secret))
}

fn open_browser(url: &str) -> Result<(), AppError> {
    #[cfg(target_os = "windows")]
    {
        Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(url).spawn()?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(url).spawn()?;
    }
    Ok(())
}

fn receive_oauth_callback(listener: TcpListener, expected_state: String) -> Result<String, AppError> {
    listener.set_nonblocking(true)?;
    let started = Instant::now();
    loop {
        if started.elapsed() > Duration::from_secs(OAUTH_CALLBACK_TIMEOUT_SECS) {
            return Err(google_error("OAuth sign-in timed out"));
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_read_timeout(Some(Duration::from_secs(5)))?;
                let mut buffer = [0_u8; 8192];
                let count = stream.read(&mut buffer)?;
                let request = String::from_utf8_lossy(&buffer[..count]);
                let target = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .ok_or_else(|| google_error("invalid OAuth callback"))?;
                let callback = Url::parse(&format!("http://127.0.0.1{target}"))
                    .map_err(|error| google_error(error.to_string()))?;
                let mut code = None;
                let mut state = None;
                let mut oauth_error = None;
                for (key, value) in callback.query_pairs() {
                    match key.as_ref() {
                        "code" => code = Some(value.into_owned()),
                        "state" => state = Some(value.into_owned()),
                        "error" => oauth_error = Some(value.into_owned()),
                        _ => {}
                    }
                }
                let success = oauth_error.is_none() && state.as_deref() == Some(&expected_state);
                let body = if success {
                    "OpenMindAI connected to Google. You can close this tab and return to the app."
                } else {
                    "OpenMindAI could not complete Google sign-in. Return to the app for details."
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(), body
                );
                let _ = stream.write_all(response.as_bytes());
                if let Some(error) = oauth_error {
                    return Err(google_error(format!("OAuth authorization failed: {error}")));
                }
                if state.as_deref() != Some(&expected_state) {
                    return Err(google_error("OAuth state validation failed"));
                }
                return code.ok_or_else(|| google_error("OAuth callback did not contain a code"));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

async fn refresh_access_token(state: &State<'_, AppState>) -> Result<String, AppError> {
    let refresh_token = secret_store::get_secret(GOOGLE_REFRESH_TOKEN_SLOT)?
        .ok_or_else(|| google_error("Google is not connected. Sign in again."))?;
    let (client_id, client_secret) = google_credentials(state)?;
    let response = http_client()?
        .post(OAUTH_TOKEN_ENDPOINT)
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("refresh_token", refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        return Err(google_error(format!("token refresh failed ({status}): {detail}")));
    }
    let token: OAuthTokenResponse = response
        .json()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    secret_store::set_secret(GOOGLE_ACCESS_TOKEN_SLOT, &token.access_token)?;
    let mut metadata = oauth_metadata(state)?;
    metadata.expires_at = token
        .expires_in
        .map(|seconds| (Utc::now() + ChronoDuration::seconds(seconds)).to_rfc3339());
    if let Some(scope) = token.scope {
        metadata.scopes = scope.split_whitespace().map(ToOwned::to_owned).collect();
    }
    save_oauth_metadata(state, &metadata)?;
    Ok(token.access_token)
}

async fn access_token(state: &State<'_, AppState>) -> Result<String, AppError> {
    let metadata = oauth_metadata(state)?;
    let still_valid = metadata
        .expires_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .is_some_and(|expires| expires.with_timezone(&Utc) > Utc::now() + ChronoDuration::seconds(60));
    if still_valid {
        if let Some(token) = secret_store::get_secret(GOOGLE_ACCESS_TOKEN_SLOT)? {
            return Ok(token);
        }
    }
    refresh_access_token(state).await
}

async fn google_request(
    state: &State<'_, AppState>,
    method: reqwest::Method,
    url: &str,
) -> Result<reqwest::RequestBuilder, AppError> {
    let token = access_token(state).await?;
    Ok(http_client()?
        .request(method, url)
        .bearer_auth(token)
        .header(header::ACCEPT, "application/json"))
}

async fn ensure_success(response: reqwest::Response, operation: &str) -> Result<reqwest::Response, AppError> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let detail = response.text().await.unwrap_or_default();
    Err(google_error(format!("{operation} failed ({status}): {detail}")))
}

#[tauri::command]
pub fn google_connection_status(
    state: State<'_, AppState>,
) -> Result<GoogleConnectionStatus, AppError> {
    let credentials_configured = {
        let db = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        GoogleRepository::new(&db)
            .get_status()?
            .is_some_and(|status| status.has_secret)
    };
    let metadata = oauth_metadata(&state)?;
    let connected = secret_store::get_secret(GOOGLE_REFRESH_TOKEN_SLOT)?.is_some();
    Ok(GoogleConnectionStatus {
        credentials_configured,
        connected,
        email: metadata.email,
        expires_at: metadata.expires_at,
        scopes: metadata.scopes,
    })
}

#[tauri::command]
pub async fn google_connect(state: State<'_, AppState>) -> Result<GoogleConnectionStatus, AppError> {
    let (client_id, client_secret) = google_credentials(&state)?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/oauth2/callback");
    let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let expected_state = Uuid::new_v4().simple().to_string();

    let mut authorize = Url::parse(OAUTH_AUTHORIZE_ENDPOINT)
        .map_err(|error| google_error(error.to_string()))?;
    authorize
        .query_pairs_mut()
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", &GOOGLE_SCOPES.join(" "))
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("include_granted_scopes", "true")
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &expected_state);
    open_browser(authorize.as_str())?;

    let callback_state = expected_state.clone();
    let code = tauri::async_runtime::spawn_blocking(move || {
        receive_oauth_callback(listener, callback_state)
    })
    .await
    .map_err(|error| google_error(error.to_string()))??;

    let response = http_client()?
        .post(OAUTH_TOKEN_ENDPOINT)
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri.as_str()),
        ])
        .send()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    let response = ensure_success(response, "OAuth token exchange").await?;
    let token: OAuthTokenResponse = response
        .json()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    secret_store::set_secret(GOOGLE_ACCESS_TOKEN_SLOT, &token.access_token)?;
    if let Some(refresh) = token.refresh_token.as_deref() {
        secret_store::set_secret(GOOGLE_REFRESH_TOKEN_SLOT, refresh)?;
    }
    if secret_store::get_secret(GOOGLE_REFRESH_TOKEN_SLOT)?.is_none() {
        return Err(google_error("Google did not return a refresh token; disconnect and sign in again"));
    }

    let userinfo_response = http_client()?
        .get(USERINFO_ENDPOINT)
        .bearer_auth(&token.access_token)
        .send()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    let email = if userinfo_response.status().is_success() {
        userinfo_response
            .json::<UserInfoResponse>()
            .await
            .ok()
            .and_then(|value| value.email)
    } else {
        None
    };
    let metadata = GoogleOAuthMetadata {
        email: email.clone(),
        expires_at: token
            .expires_in
            .map(|seconds| (Utc::now() + ChronoDuration::seconds(seconds)).to_rfc3339()),
        scopes: token
            .scope
            .map(|scope| scope.split_whitespace().map(ToOwned::to_owned).collect())
            .unwrap_or_else(|| GOOGLE_SCOPES.iter().map(|scope| (*scope).to_string()).collect()),
    };
    save_oauth_metadata(&state, &metadata)?;
    Ok(GoogleConnectionStatus {
        credentials_configured: true,
        connected: true,
        email,
        expires_at: metadata.expires_at,
        scopes: metadata.scopes,
    })
}

#[tauri::command]
pub async fn google_disconnect(state: State<'_, AppState>) -> Result<(), AppError> {
    if let Some(token) = secret_store::get_secret(GOOGLE_REFRESH_TOKEN_SLOT)? {
        let _ = http_client()?
            .post("https://oauth2.googleapis.com/revoke")
            .form(&[("token", token.as_str())])
            .send()
            .await;
    }
    secret_store::delete_secret(GOOGLE_ACCESS_TOKEN_SLOT)?;
    secret_store::delete_secret(GOOGLE_REFRESH_TOKEN_SLOT)?;
    clear_oauth_metadata(&state)
}

fn gmail_header(payload: &GmailPayload, name: &str) -> String {
    payload
        .headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case(name))
        .map(|header| header.value.clone())
        .unwrap_or_default()
}

fn decode_gmail_body(data: &str) -> String {
    URL_SAFE_NO_PAD
        .decode(data)
        .or_else(|_| URL_SAFE.decode(data))
        .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
        .unwrap_or_default()
}

fn gmail_body_text(payload: &GmailPayload) -> String {
    if payload.mime_type.starts_with("text/plain") {
        if let Some(data) = payload.body.as_ref().and_then(|body| body.data.as_deref()) {
            return decode_gmail_body(data);
        }
    }
    for part in &payload.parts {
        let text = gmail_body_text(part);
        if !text.is_empty() {
            return text;
        }
    }
    if payload.mime_type.starts_with("text/html") {
        if let Some(data) = payload.body.as_ref().and_then(|body| body.data.as_deref()) {
            return decode_gmail_body(data);
        }
    }
    String::new()
}

fn map_gmail_message(message: GmailApiMessage) -> GmailMessage {
    let payload = message.payload.unwrap_or(GmailPayload {
        mime_type: String::new(),
        headers: Vec::new(),
        body: None,
        parts: Vec::new(),
    });
    GmailMessage {
        id: message.id,
        thread_id: message.thread_id,
        snippet: message.snippet,
        label_ids: message.label_ids,
        subject: gmail_header(&payload, "Subject"),
        from: gmail_header(&payload, "From"),
        to: gmail_header(&payload, "To"),
        date: gmail_header(&payload, "Date"),
        body_text: gmail_body_text(&payload),
    }
}

async fn gmail_get_api_message(state: &State<'_, AppState>, id: &str) -> Result<GmailApiMessage, AppError> {
    let url = format!("https://gmail.googleapis.com/gmail/v1/users/me/messages/{id}");
    let response = google_request(state, reqwest::Method::GET, &url)
        .await?
        .query(&[("format", "full")])
        .send()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    ensure_success(response, "Gmail read")
        .await?
        .json()
        .await
        .map_err(|error| google_error(error.to_string()))
}

#[tauri::command]
pub async fn gmail_search(
    query: String,
    max_results: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<GmailMessage>, AppError> {
    let limit = max_results.unwrap_or(20).clamp(1, MAX_GMAIL_RESULTS);
    let response = google_request(
        &state,
        reqwest::Method::GET,
        "https://gmail.googleapis.com/gmail/v1/users/me/messages",
    )
    .await?
    .query(&[("q", query.as_str()), ("maxResults", &limit.to_string())])
    .send()
    .await
    .map_err(|error| google_error(error.to_string()))?;
    let list: GmailListResponse = ensure_success(response, "Gmail search")
        .await?
        .json()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    let mut messages = Vec::with_capacity(list.messages.len());
    for item in list.messages.into_iter().take(limit) {
        messages.push(map_gmail_message(gmail_get_api_message(&state, &item.id).await?));
    }
    Ok(messages)
}

#[tauri::command]
pub async fn gmail_get_message(
    message_id: String,
    state: State<'_, AppState>,
) -> Result<GmailMessage, AppError> {
    Ok(map_gmail_message(gmail_get_api_message(&state, &message_id).await?))
}

fn reject_header_injection(value: &str, label: &str) -> Result<(), AppError> {
    if value.contains(['\r', '\n']) {
        return Err(google_error(format!("{label} contains an invalid newline")));
    }
    Ok(())
}

async fn gmail_send_raw(
    state: &State<'_, AppState>,
    raw: String,
    thread_id: Option<String>,
) -> Result<GmailMessage, AppError> {
    let encoded = URL_SAFE_NO_PAD.encode(raw.as_bytes());
    let response = google_request(
        state,
        reqwest::Method::POST,
        "https://gmail.googleapis.com/gmail/v1/users/me/messages/send",
    )
    .await?
    .json(&GmailSendBody { raw: encoded, thread_id })
    .send()
    .await
    .map_err(|error| google_error(error.to_string()))?;
    let sent: GmailApiMessage = ensure_success(response, "Gmail send")
        .await?
        .json()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    Ok(map_gmail_message(gmail_get_api_message(state, &sent.id).await?))
}

#[tauri::command]
pub async fn gmail_send(
    to: String,
    cc: Option<String>,
    bcc: Option<String>,
    subject: String,
    body: String,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<GmailMessage, AppError> {
    if !confirmed {
        return Err(google_error("sending email requires explicit confirmation"));
    }
    if to.trim().is_empty() {
        return Err(google_error("recipient is required"));
    }
    for (value, label) in [
        (to.as_str(), "To"),
        (subject.as_str(), "Subject"),
        (cc.as_deref().unwrap_or_default(), "Cc"),
        (bcc.as_deref().unwrap_or_default(), "Bcc"),
    ] {
        reject_header_injection(value, label)?;
    }
    let mut headers = vec![
        format!("To: {}", to.trim()),
        format!("Subject: {}", subject.trim()),
        "MIME-Version: 1.0".to_string(),
        "Content-Type: text/plain; charset=UTF-8".to_string(),
    ];
    if let Some(value) = cc.filter(|value| !value.trim().is_empty()) {
        headers.insert(1, format!("Cc: {}", value.trim()));
    }
    if let Some(value) = bcc.filter(|value| !value.trim().is_empty()) {
        headers.insert(1, format!("Bcc: {}", value.trim()));
    }
    gmail_send_raw(&state, format!("{}\r\n\r\n{}", headers.join("\r\n"), body), None).await
}

#[tauri::command]
pub async fn gmail_reply(
    message_id: String,
    body: String,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<GmailMessage, AppError> {
    if !confirmed {
        return Err(google_error("replying to email requires explicit confirmation"));
    }
    let original = gmail_get_api_message(&state, &message_id).await?;
    let payload = original
        .payload
        .as_ref()
        .ok_or_else(|| google_error("original message headers are unavailable"))?;
    let to = gmail_header(payload, "Reply-To");
    let to = if to.is_empty() { gmail_header(payload, "From") } else { to };
    let original_subject = gmail_header(payload, "Subject");
    let subject = if original_subject.to_ascii_lowercase().starts_with("re:") {
        original_subject
    } else {
        format!("Re: {original_subject}")
    };
    let message_id_header = gmail_header(payload, "Message-ID");
    let references = gmail_header(payload, "References");
    reject_header_injection(&to, "To")?;
    reject_header_injection(&subject, "Subject")?;
    let mut headers = vec![
        format!("To: {to}"),
        format!("Subject: {subject}"),
        "MIME-Version: 1.0".to_string(),
        "Content-Type: text/plain; charset=UTF-8".to_string(),
    ];
    if !message_id_header.is_empty() {
        headers.push(format!("In-Reply-To: {message_id_header}"));
        let refs = if references.is_empty() {
            message_id_header.clone()
        } else {
            format!("{references} {message_id_header}")
        };
        headers.push(format!("References: {refs}"));
    }
    gmail_send_raw(
        &state,
        format!("{}\r\n\r\n{}", headers.join("\r\n"), body),
        Some(original.thread_id),
    )
    .await
}

async fn gmail_post_no_body(
    state: &State<'_, AppState>,
    message_id: &str,
    action: &str,
    confirmed: bool,
) -> Result<GmailMessage, AppError> {
    if !confirmed {
        return Err(google_error(format!("Gmail {action} requires explicit confirmation")));
    }
    let url = format!("https://gmail.googleapis.com/gmail/v1/users/me/messages/{message_id}/{action}");
    let response = google_request(state, reqwest::Method::POST, &url)
        .await?
        .send()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    ensure_success(response, &format!("Gmail {action}")).await?;
    gmail_get_message(message_id.to_string(), State::from_inner(state.inner())).await
}

#[tauri::command]
pub async fn gmail_modify_labels(
    message_id: String,
    add_label_ids: Vec<String>,
    remove_label_ids: Vec<String>,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<GmailMessage, AppError> {
    if !confirmed {
        return Err(google_error("modifying Gmail labels requires explicit confirmation"));
    }
    let url = format!("https://gmail.googleapis.com/gmail/v1/users/me/messages/{message_id}/modify");
    let response = google_request(&state, reqwest::Method::POST, &url)
        .await?
        .json(&json!({"addLabelIds": add_label_ids, "removeLabelIds": remove_label_ids}))
        .send()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    ensure_success(response, "Gmail label update").await?;
    gmail_get_message(message_id, state).await
}

#[tauri::command]
pub async fn gmail_archive(
    message_id: String,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<GmailMessage, AppError> {
    gmail_modify_labels(message_id, Vec::new(), vec!["INBOX".to_string()], confirmed, state).await
}

#[tauri::command]
pub async fn gmail_trash(
    message_id: String,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<GmailMessage, AppError> {
    if !confirmed {
        return Err(google_error("moving email to Trash requires explicit confirmation"));
    }
    let url = format!("https://gmail.googleapis.com/gmail/v1/users/me/messages/{message_id}/trash");
    let response = google_request(&state, reqwest::Method::POST, &url)
        .await?
        .send()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    ensure_success(response, "Gmail trash").await?;
    gmail_get_message(message_id, state).await
}

#[tauri::command]
pub async fn gmail_untrash(
    message_id: String,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<GmailMessage, AppError> {
    if !confirmed {
        return Err(google_error("restoring email from Trash requires explicit confirmation"));
    }
    let url = format!("https://gmail.googleapis.com/gmail/v1/users/me/messages/{message_id}/untrash");
    let response = google_request(&state, reqwest::Method::POST, &url)
        .await?
        .send()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    ensure_success(response, "Gmail untrash").await?;
    gmail_get_message(message_id, state).await
}

#[tauri::command]
pub async fn drive_search(
    query: String,
    max_results: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<DriveFile>, AppError> {
    let limit = max_results.unwrap_or(30).clamp(1, MAX_DRIVE_RESULTS);
    let mut request = google_request(
        &state,
        reqwest::Method::GET,
        "https://www.googleapis.com/drive/v3/files",
    )
    .await?
    .query(&[
        ("pageSize", limit.to_string()),
        ("fields", "files(id,name,mimeType,modifiedTime,size,webViewLink,parents)".to_string()),
        ("orderBy", "modifiedTime desc".to_string()),
    ]);
    if !query.trim().is_empty() {
        request = request.query(&[("q", query.trim())]);
    }
    let response = request
        .send()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    let list: DriveListResponse = ensure_success(response, "Drive search")
        .await?
        .json()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    Ok(list.files)
}

async fn drive_metadata(state: &State<'_, AppState>, file_id: &str) -> Result<DriveFile, AppError> {
    let url = format!("https://www.googleapis.com/drive/v3/files/{file_id}");
    let response = google_request(state, reqwest::Method::GET, &url)
        .await?
        .query(&[("fields", "id,name,mimeType,modifiedTime,size,webViewLink,parents")])
        .send()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    ensure_success(response, "Drive metadata")
        .await?
        .json()
        .await
        .map_err(|error| google_error(error.to_string()))
}

#[tauri::command]
pub async fn drive_read_file(
    file_id: String,
    state: State<'_, AppState>,
) -> Result<DriveFileContent, AppError> {
    let file = drive_metadata(&state, &file_id).await?;
    let (url, content_mime_type) = match file.mime_type.as_str() {
        "application/vnd.google-apps.document" => (
            format!("https://www.googleapis.com/drive/v3/files/{file_id}/export"),
            "text/plain".to_string(),
        ),
        "application/vnd.google-apps.spreadsheet" => (
            format!("https://www.googleapis.com/drive/v3/files/{file_id}/export"),
            "text/csv".to_string(),
        ),
        "application/vnd.google-apps.presentation" => (
            format!("https://www.googleapis.com/drive/v3/files/{file_id}/export"),
            "application/pdf".to_string(),
        ),
        _ => (
            format!("https://www.googleapis.com/drive/v3/files/{file_id}"),
            file.mime_type.clone(),
        ),
    };
    let mut request = google_request(&state, reqwest::Method::GET, &url).await?;
    if file.mime_type.starts_with("application/vnd.google-apps.") {
        request = request.query(&[("mimeType", content_mime_type.as_str())]);
    } else {
        request = request.query(&[("alt", "media")]);
    }
    let response = ensure_success(
        request
            .send()
            .await
            .map_err(|error| google_error(error.to_string()))?,
        "Drive read",
    )
    .await?;
    let bytes = response
        .bytes()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    if bytes.len() > MAX_DRIVE_CONTENT_BYTES {
        return Err(google_error("Drive file exceeds the 5 MB in-app read limit"));
    }
    Ok(DriveFileContent {
        file,
        content_base64: STANDARD.encode(bytes),
        content_mime_type,
    })
}

fn multipart_drive_body(metadata: &Value, content: &[u8], content_type: &str) -> (String, Vec<u8>) {
    let boundary = format!("openmindai-{}", Uuid::new_v4().simple());
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{}\r\n", metadata).as_bytes());
    body.extend_from_slice(format!("--{boundary}\r\nContent-Type: {content_type}\r\n\r\n").as_bytes());
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (boundary, body)
}

#[tauri::command]
pub async fn drive_upload_file(
    name: String,
    mime_type: String,
    content_base64: String,
    parent_id: Option<String>,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<DriveFile, AppError> {
    if !confirmed {
        return Err(google_error("uploading to Drive requires explicit confirmation"));
    }
    let content = STANDARD
        .decode(content_base64)
        .map_err(|error| google_error(format!("invalid upload content: {error}")))?;
    if content.len() > MAX_DRIVE_CONTENT_BYTES {
        return Err(google_error("Drive upload exceeds the 5 MB in-app limit"));
    }
    let mut metadata = json!({"name": name});
    if let Some(parent) = parent_id.filter(|value| !value.trim().is_empty()) {
        metadata["parents"] = json!([parent]);
    }
    let (boundary, body) = multipart_drive_body(&metadata, &content, &mime_type);
    let response = google_request(
        &state,
        reqwest::Method::POST,
        "https://www.googleapis.com/upload/drive/v3/files",
    )
    .await?
    .query(&[("uploadType", "multipart"), ("fields", "id,name,mimeType,modifiedTime,size,webViewLink,parents")])
    .header(header::CONTENT_TYPE, format!("multipart/related; boundary={boundary}"))
    .body(body)
    .send()
    .await
    .map_err(|error| google_error(error.to_string()))?;
    ensure_success(response, "Drive upload")
        .await?
        .json()
        .await
        .map_err(|error| google_error(error.to_string()))
}

#[tauri::command]
pub async fn drive_create_folder(
    name: String,
    parent_id: Option<String>,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<DriveFile, AppError> {
    if !confirmed {
        return Err(google_error("creating a Drive folder requires explicit confirmation"));
    }
    let mut payload = json!({"name": name, "mimeType": "application/vnd.google-apps.folder"});
    if let Some(parent) = parent_id.filter(|value| !value.trim().is_empty()) {
        payload["parents"] = json!([parent]);
    }
    let response = google_request(
        &state,
        reqwest::Method::POST,
        "https://www.googleapis.com/drive/v3/files",
    )
    .await?
    .query(&[("fields", "id,name,mimeType,modifiedTime,size,webViewLink,parents")])
    .json(&payload)
    .send()
    .await
    .map_err(|error| google_error(error.to_string()))?;
    ensure_success(response, "Drive folder creation")
        .await?
        .json()
        .await
        .map_err(|error| google_error(error.to_string()))
}

#[tauri::command]
pub async fn drive_update_file(
    file_id: String,
    name: Option<String>,
    mime_type: Option<String>,
    content_base64: Option<String>,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<DriveFile, AppError> {
    if !confirmed {
        return Err(google_error("updating a Drive file requires explicit confirmation"));
    }
    let metadata = name.map(|value| json!({"name": value})).unwrap_or_else(|| json!({}));
    let response = if let Some(encoded) = content_base64 {
        let content = STANDARD
            .decode(encoded)
            .map_err(|error| google_error(format!("invalid update content: {error}")))?;
        if content.len() > MAX_DRIVE_CONTENT_BYTES {
            return Err(google_error("Drive update exceeds the 5 MB in-app limit"));
        }
        let content_type = mime_type.as_deref().unwrap_or("application/octet-stream");
        let (boundary, body) = multipart_drive_body(&metadata, &content, content_type);
        let url = format!("https://www.googleapis.com/upload/drive/v3/files/{file_id}");
        google_request(&state, reqwest::Method::PATCH, &url)
            .await?
            .query(&[("uploadType", "multipart"), ("fields", "id,name,mimeType,modifiedTime,size,webViewLink,parents")])
            .header(header::CONTENT_TYPE, format!("multipart/related; boundary={boundary}"))
            .body(body)
            .send()
            .await
            .map_err(|error| google_error(error.to_string()))?
    } else {
        let url = format!("https://www.googleapis.com/drive/v3/files/{file_id}");
        google_request(&state, reqwest::Method::PATCH, &url)
            .await?
            .query(&[("fields", "id,name,mimeType,modifiedTime,size,webViewLink,parents")])
            .json(&metadata)
            .send()
            .await
            .map_err(|error| google_error(error.to_string()))?
    };
    ensure_success(response, "Drive update")
        .await?
        .json()
        .await
        .map_err(|error| google_error(error.to_string()))
}

#[tauri::command]
pub async fn drive_delete_file(
    file_id: String,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    if !confirmed {
        return Err(google_error("deleting a Drive file requires explicit confirmation"));
    }
    let url = format!("https://www.googleapis.com/drive/v3/files/{file_id}");
    let response = google_request(&state, reqwest::Method::DELETE, &url)
        .await?
        .send()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    ensure_success(response, "Drive delete").await?;
    Ok(())
}

#[tauri::command]
pub async fn calendar_list(
    state: State<'_, AppState>,
) -> Result<Vec<CalendarSummary>, AppError> {
    let response = google_request(
        &state,
        reqwest::Method::GET,
        "https://www.googleapis.com/calendar/v3/users/me/calendarList",
    )
    .await?
    .send()
    .await
    .map_err(|error| google_error(error.to_string()))?;
    let list: CalendarListResponse = ensure_success(response, "Calendar list")
        .await?
        .json()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    Ok(list.items)
}

#[tauri::command]
pub async fn calendar_events(
    calendar_id: String,
    time_min: Option<String>,
    max_results: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<CalendarEvent>, AppError> {
    let calendar = if calendar_id.trim().is_empty() { "primary" } else { calendar_id.trim() };
    let url = format!("https://www.googleapis.com/calendar/v3/calendars/{}/events", encode_path_segment(calendar));
    let mut request = google_request(&state, reqwest::Method::GET, &url)
        .await?
        .query(&[
            ("singleEvents", "true".to_string()),
            ("orderBy", "startTime".to_string()),
            ("maxResults", max_results.unwrap_or(50).clamp(1, 100).to_string()),
        ]);
    if let Some(value) = time_min.filter(|value| !value.trim().is_empty()) {
        request = request.query(&[("timeMin", value)]);
    }
    let response = request
        .send()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    let list: CalendarEventsResponse = ensure_success(response, "Calendar events")
        .await?
        .json()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    Ok(list.items)
}

fn event_payload(input: CalendarEventInput) -> Value {
    let time_zone = input.time_zone.unwrap_or_else(|| "UTC".to_string());
    let attendees: Vec<Value> = input
        .attendees
        .into_iter()
        .filter(|email| !email.trim().is_empty())
        .map(|email| json!({"email": email.trim()}))
        .collect();
    json!({
        "summary": input.summary,
        "description": input.description,
        "location": input.location,
        "start": {"dateTime": input.start, "timeZone": time_zone},
        "end": {"dateTime": input.end, "timeZone": time_zone},
        "attendees": attendees
    })
}

fn encode_path_segment(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                vec![byte as char]
            } else {
                format!("%{byte:02X}").chars().collect()
            }
        })
        .collect()
}

#[tauri::command]
pub async fn calendar_create_event(
    calendar_id: String,
    event: CalendarEventInput,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<CalendarEvent, AppError> {
    if !confirmed {
        return Err(google_error("creating a calendar event requires explicit confirmation"));
    }
    let calendar = if calendar_id.trim().is_empty() { "primary" } else { calendar_id.trim() };
    let url = format!("https://www.googleapis.com/calendar/v3/calendars/{}/events", encode_path_segment(calendar));
    let response = google_request(&state, reqwest::Method::POST, &url)
        .await?
        .query(&[("sendUpdates", "all")])
        .json(&event_payload(event))
        .send()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    ensure_success(response, "Calendar event create")
        .await?
        .json()
        .await
        .map_err(|error| google_error(error.to_string()))
}

#[tauri::command]
pub async fn calendar_update_event(
    calendar_id: String,
    event_id: String,
    event: CalendarEventInput,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<CalendarEvent, AppError> {
    if !confirmed {
        return Err(google_error("updating a calendar event requires explicit confirmation"));
    }
    let calendar = if calendar_id.trim().is_empty() { "primary" } else { calendar_id.trim() };
    let url = format!(
        "https://www.googleapis.com/calendar/v3/calendars/{}/events/{}",
        encode_path_segment(calendar),
        encode_path_segment(&event_id)
    );
    let response = google_request(&state, reqwest::Method::PUT, &url)
        .await?
        .query(&[("sendUpdates", "all")])
        .json(&event_payload(event))
        .send()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    ensure_success(response, "Calendar event update")
        .await?
        .json()
        .await
        .map_err(|error| google_error(error.to_string()))
}

#[tauri::command]
pub async fn calendar_delete_event(
    calendar_id: String,
    event_id: String,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    if !confirmed {
        return Err(google_error("deleting a calendar event requires explicit confirmation"));
    }
    let calendar = if calendar_id.trim().is_empty() { "primary" } else { calendar_id.trim() };
    let url = format!(
        "https://www.googleapis.com/calendar/v3/calendars/{}/events/{}",
        encode_path_segment(calendar),
        encode_path_segment(&event_id)
    );
    let response = google_request(&state, reqwest::Method::DELETE, &url)
        .await?
        .query(&[("sendUpdates", "all")])
        .send()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    ensure_success(response, "Calendar event delete").await?;
    Ok(())
}

fn contact_from_person(person: &Value) -> GoogleContact {
    let display_name = person
        .get("names")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("displayName"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let emails = person
        .get("emailAddresses")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("value").and_then(Value::as_str).map(ToOwned::to_owned))
        .collect();
    let phones = person
        .get("phoneNumbers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("value").and_then(Value::as_str).map(ToOwned::to_owned))
        .collect();
    let organizations = person
        .get("organizations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("name").and_then(Value::as_str).map(ToOwned::to_owned))
        .collect();
    GoogleContact {
        resource_name: person
            .get("resourceName")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        display_name,
        emails,
        phones,
        organizations,
    }
}

#[tauri::command]
pub async fn contacts_list(
    query: String,
    max_results: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<GoogleContact>, AppError> {
    let limit = max_results.unwrap_or(50).clamp(1, MAX_CONTACT_RESULTS);
    let response = google_request(
        &state,
        reqwest::Method::GET,
        "https://people.googleapis.com/v1/people/me/connections",
    )
    .await?
    .query(&[
        ("personFields", "names,emailAddresses,phoneNumbers,organizations"),
        ("pageSize", &limit.to_string()),
        ("sortOrder", "LAST_NAME_ASCENDING"),
    ])
    .send()
    .await
    .map_err(|error| google_error(error.to_string()))?;
    let value: Value = ensure_success(response, "Contacts list")
        .await?
        .json()
        .await
        .map_err(|error| google_error(error.to_string()))?;
    let needle = query.trim().to_ascii_lowercase();
    let contacts = value
        .get("connections")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(contact_from_person)
        .filter(|contact| {
            needle.is_empty()
                || contact.display_name.to_ascii_lowercase().contains(&needle)
                || contact.emails.iter().any(|email| email.to_ascii_lowercase().contains(&needle))
                || contact.phones.iter().any(|phone| phone.to_ascii_lowercase().contains(&needle))
        })
        .take(limit)
        .collect();
    Ok(contacts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_verifier_and_challenge_are_valid_lengths() {
        let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        assert!((43..=128).contains(&verifier.len()));
        assert_eq!(challenge.len(), 43);
    }

    #[test]
    fn path_segment_encoding_blocks_slashes_and_spaces() {
        assert_eq!(encode_path_segment("a/b c"), "a%2Fb%20c");
    }

    #[test]
    fn email_header_injection_is_rejected() {
        assert!(reject_header_injection("safe@example.com", "To").is_ok());
        assert!(reject_header_injection("safe@example.com\r\nBcc: bad@example.com", "To").is_err());
    }
}
