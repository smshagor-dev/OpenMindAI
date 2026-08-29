use std::{process::Command, time::Duration};

use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine as _,
};
use chrono::Utc;
use futures_util::StreamExt;
use reqwest::{multipart, Client, Method, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::State;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::timeout,
};
use url::Url;
use uuid::Uuid;

use crate::{app_error::AppError, github::secret_store, google::GoogleRepository, AppState};

const CLIENT_SECRET_SLOT: &str = "google-client-secret";
const TOKEN_SLOT: &str = "google-workspace-oauth";
const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const REVOKE_ENDPOINT: &str = "https://oauth2.googleapis.com/revoke";
const USERINFO_ENDPOINT: &str = "https://openidconnect.googleapis.com/v1/userinfo";
const REQUEST_TIMEOUT_SECS: u64 = 30;
const OAUTH_TIMEOUT_SECS: u64 = 300;
const MAX_JSON_BYTES: usize = 2 * 1024 * 1024;
const MAX_BINARY_BYTES: usize = 8 * 1024 * 1024;

const GOOGLE_SCOPES: &[&str] = &[
    "openid",
    "email",
    "profile",
    "https://www.googleapis.com/auth/gmail.modify",
    "https://www.googleapis.com/auth/gmail.send",
    "https://www.googleapis.com/auth/drive",
    "https://www.googleapis.com/auth/calendar.events",
    "https://www.googleapis.com/auth/calendar.calendarlist.readonly",
    "https://www.googleapis.com/auth/contacts.readonly",
];

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
struct GoogleTokenResponse {
    access_token: String,
    expires_in: i64,
    refresh_token: Option<String>,
    token_type: Option<String>,
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleUserInfo {
    email: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleWorkspaceStatus {
    pub configured: bool,
    pub connected: bool,
    pub client_id: Option<String>,
    pub email: Option<String>,
    pub expires_at: Option<i64>,
    pub scopes: Vec<String>,
}

fn connector_error(message: impl Into<String>) -> AppError {
    AppError::internal(format!("Google Workspace: {}", message.into()))
}

fn require_approval(action: &str, approved: bool) -> Result<(), AppError> {
    if approved {
        return Ok(());
    }
    Err(connector_error(format!(
        "action '{action}' changes remote Google data and requires explicit approval"
    )))
}

fn is_mutating(action: &str) -> bool {
    matches!(
        action,
        "gmail.send"
            | "gmail.reply"
            | "gmail.modify"
            | "gmail.archive"
            | "gmail.trash"
            | "gmail.untrash"
            | "drive.create"
            | "drive.update"
            | "drive.delete"
            | "calendar.create"
            | "calendar.update"
            | "calendar.delete"
    )
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

fn safe_header(value: &str, label: &str) -> Result<String, AppError> {
    if value.contains('\r') || value.contains('\n') {
        return Err(connector_error(format!(
            "{label} contains an invalid newline"
        )));
    }
    Ok(value.trim().to_string())
}

fn client_credentials(state: &State<'_, AppState>) -> Result<(String, String), AppError> {
    let status = {
        let db = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        GoogleRepository::new(&db).get_status()?
    }
    .ok_or_else(|| connector_error("Google OAuth client credentials are not configured"))?;
    let secret = secret_store::get_secret(CLIENT_SECRET_SLOT)?
        .ok_or_else(|| connector_error("Google OAuth client secret is missing"))?;
    Ok((status.client_id, secret))
}

fn load_tokens() -> Result<Option<GoogleTokenBundle>, AppError> {
    let Some(raw) = secret_store::get_secret(TOKEN_SLOT)? else {
        return Ok(None);
    };
    let token = serde_json::from_str(&raw)
        .map_err(|error| connector_error(format!("stored OAuth token is invalid: {error}")))?;
    Ok(Some(token))
}

fn save_tokens(tokens: &GoogleTokenBundle) -> Result<(), AppError> {
    let raw = serde_json::to_string(tokens)
        .map_err(|error| connector_error(format!("could not serialize OAuth token: {error}")))?;
    secret_store::set_secret(TOKEN_SLOT, &raw)
}

async fn read_bounded(response: Response, max_bytes: usize) -> Result<Vec<u8>, AppError> {
    let status = response.status();
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| connector_error(error.to_string()))?;
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(connector_error(format!(
                "response exceeded the {} byte safety limit",
                max_bytes
            )));
        }
        body.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let detail = String::from_utf8_lossy(&body);
        return Err(connector_error(format!(
            "Google API returned status {status}: {}",
            detail.chars().take(1200).collect::<String>()
        )));
    }
    Ok(body)
}

async fn json_response(response: Response) -> Result<Value, AppError> {
    let body = read_bounded(response, MAX_JSON_BYTES).await?;
    if body.is_empty() {
        return Ok(json!({"ok": true}));
    }
    serde_json::from_slice(&body)
        .map_err(|error| connector_error(format!("invalid Google JSON response: {error}")))
}

async fn refresh_access_token(
    client: &Client,
    client_id: &str,
    client_secret: &str,
    mut bundle: GoogleTokenBundle,
) -> Result<GoogleTokenBundle, AppError> {
    let response = client
        .post(TOKEN_ENDPOINT)
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("refresh_token", bundle.refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|error| connector_error(format!("token refresh failed: {error}")))?;
    let value = json_response(response).await?;
    let token: GoogleTokenResponse = serde_json::from_value(value)
        .map_err(|error| connector_error(format!("invalid refresh response: {error}")))?;
    bundle.access_token = token.access_token;
    bundle.expires_at = Utc::now().timestamp() + token.expires_in;
    bundle.token_type = token.token_type.unwrap_or_else(|| "Bearer".to_string());
    if let Some(scope) = token.scope {
        bundle.scope = scope;
    }
    save_tokens(&bundle)?;
    Ok(bundle)
}

async fn access_token(state: &State<'_, AppState>) -> Result<String, AppError> {
    let (client_id, client_secret) = client_credentials(state)?;
    let bundle =
        load_tokens()?.ok_or_else(|| connector_error("Google account is not connected"))?;
    let bundle = if bundle.expires_at <= Utc::now().timestamp() + 60 {
        refresh_access_token(&state.http, &client_id, &client_secret, bundle).await?
    } else {
        bundle
    };
    Ok(bundle.access_token)
}

async fn google_request(
    state: &State<'_, AppState>,
    method: Method,
    url: &str,
    body: Option<Value>,
) -> Result<Value, AppError> {
    let token = access_token(state).await?;
    let mut request = state
        .http
        .request(method, url)
        .bearer_auth(token)
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

fn open_browser(url: &str) -> Result<(), AppError> {
    #[cfg(target_os = "windows")]
    {
        Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
            .map_err(|error| connector_error(format!("could not open browser: {error}")))?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|error| connector_error(format!("could not open browser: {error}")))?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|error| connector_error(format!("could not open browser: {error}")))?;
    }
    Ok(())
}

fn pkce_verifier() -> String {
    format!(
        "{}{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

async fn receive_oauth_callback(
    listener: TcpListener,
    expected_state: &str,
) -> Result<String, AppError> {
    let work = async {
        let (mut socket, _) = listener.accept().await?;
        let mut buffer = vec![0u8; 16 * 1024];
        let read = socket.read(&mut buffer).await?;
        let request = String::from_utf8_lossy(&buffer[..read]);
        let first_line = request
            .lines()
            .next()
            .ok_or_else(|| connector_error("OAuth callback request was empty"))?;
        let target = first_line
            .split_whitespace()
            .nth(1)
            .ok_or_else(|| connector_error("OAuth callback path was invalid"))?;
        let callback = Url::parse(&format!("http://127.0.0.1{target}"))
            .map_err(|error| connector_error(format!("invalid OAuth callback URL: {error}")))?;
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
        let success =
            oauth_error.is_none() && state.as_deref() == Some(expected_state) && code.is_some();
        let message = if success {
            "Google account connected. You can return to OpenMindAI."
        } else {
            "Google connection failed. Return to OpenMindAI for details."
        };
        let html = format!(
            "<!doctype html><html><body style=\"font-family:sans-serif;padding:40px\"><h2>{message}</h2><p>You may close this tab.</p></body></html>"
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            html.len(), html
        );
        socket.write_all(response.as_bytes()).await?;
        if let Some(error) = oauth_error {
            return Err(connector_error(format!(
                "Google authorization returned '{error}'"
            )));
        }
        if state.as_deref() != Some(expected_state) {
            return Err(connector_error("OAuth state validation failed"));
        }
        code.ok_or_else(|| connector_error("OAuth callback did not contain an authorization code"))
    };
    timeout(Duration::from_secs(OAUTH_TIMEOUT_SECS), work)
        .await
        .map_err(|_| connector_error("Google authorization timed out"))?
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn connect_google_workspace(
    state: State<'_, AppState>,
) -> Result<GoogleWorkspaceStatus, AppError> {
    let (client_id, client_secret) = client_credentials(&state)?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let redirect_uri = format!("http://127.0.0.1:{}/oauth2/callback", address.port());
    let verifier = pkce_verifier();
    let challenge = pkce_challenge(&verifier);
    let oauth_state = Uuid::new_v4().to_string();
    let mut auth_url = Url::parse(AUTH_ENDPOINT)
        .map_err(|error| connector_error(format!("invalid Google auth endpoint: {error}")))?;
    auth_url
        .query_pairs_mut()
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", &GOOGLE_SCOPES.join(" "))
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("include_granted_scopes", "true")
        .append_pair("state", &oauth_state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    open_browser(auth_url.as_str())?;
    let code = receive_oauth_callback(listener, &oauth_state).await?;

    let response = state
        .http
        .post(TOKEN_ENDPOINT)
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .form(&[
            ("code", code.as_str()),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .map_err(|error| connector_error(format!("authorization-code exchange failed: {error}")))?;
    let value = json_response(response).await?;
    let token: GoogleTokenResponse = serde_json::from_value(value)
        .map_err(|error| connector_error(format!("invalid token response: {error}")))?;
    let refresh_token = token.refresh_token.ok_or_else(|| {
        connector_error(
            "Google did not return a refresh token; revoke prior consent and connect again",
        )
    })?;

    let userinfo_response = state
        .http
        .get(USERINFO_ENDPOINT)
        .bearer_auth(&token.access_token)
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|error| {
            connector_error(format!("could not read Google account profile: {error}"))
        })?;
    let userinfo_value = json_response(userinfo_response).await?;
    let userinfo: GoogleUserInfo = serde_json::from_value(userinfo_value)
        .map_err(|error| connector_error(format!("invalid Google profile response: {error}")))?;

    let bundle = GoogleTokenBundle {
        access_token: token.access_token,
        refresh_token,
        expires_at: Utc::now().timestamp() + token.expires_in,
        token_type: token.token_type.unwrap_or_else(|| "Bearer".to_string()),
        scope: token.scope.unwrap_or_else(|| GOOGLE_SCOPES.join(" ")),
        email: userinfo.email,
    };
    save_tokens(&bundle)?;
    google_workspace_status(state)
}

#[tauri::command]
pub fn google_workspace_status(
    state: State<'_, AppState>,
) -> Result<GoogleWorkspaceStatus, AppError> {
    let configured = {
        let db = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        GoogleRepository::new(&db).get_status()?
    };
    let tokens = load_tokens()?;
    Ok(GoogleWorkspaceStatus {
        configured: configured.is_some(),
        connected: tokens.is_some(),
        client_id: configured.map(|item| item.client_id),
        email: tokens.as_ref().and_then(|item| item.email.clone()),
        expires_at: tokens.as_ref().map(|item| item.expires_at),
        scopes: tokens
            .as_ref()
            .map(|item| {
                item.scope
                    .split_whitespace()
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

#[tauri::command]
pub async fn disconnect_google_workspace(state: State<'_, AppState>) -> Result<(), AppError> {
    if let Some(bundle) = load_tokens()? {
        let token = if bundle.refresh_token.is_empty() {
            bundle.access_token
        } else {
            bundle.refresh_token
        };
        let _ = state
            .http
            .post(REVOKE_ENDPOINT)
            .query(&[("token", token)])
            .timeout(Duration::from_secs(10))
            .send()
            .await;
    }
    secret_store::delete_secret(TOKEN_SLOT)
}

#[tauri::command]
pub async fn execute_google_workspace_action(
    action: String,
    params: Value,
    approved: bool,
    state: State<'_, AppState>,
) -> Result<Value, AppError> {
    if is_mutating(&action) {
        require_approval(&action, approved)?;
    }
    match action.as_str() {
        "gmail.search" => {
            let mut url = Url::parse("https://gmail.googleapis.com/gmail/v1/users/me/messages")
                .map_err(|error| connector_error(error.to_string()))?;
            let max_results = params
                .get("maxResults")
                .and_then(Value::as_u64)
                .unwrap_or(25)
                .clamp(1, 100);
            url.query_pairs_mut()
                .append_pair("maxResults", &max_results.to_string());
            if let Some(query) = optional_str(&params, "query") {
                url.query_pairs_mut().append_pair("q", query);
            }
            google_request(&state, Method::GET, url.as_str(), None).await
        }
        "gmail.get" => {
            let id = required_str(&params, "messageId")?;
            let url =
                format!("https://gmail.googleapis.com/gmail/v1/users/me/messages/{id}?format=full");
            google_request(&state, Method::GET, &url, None).await
        }
        "gmail.labels" => {
            google_request(
                &state,
                Method::GET,
                "https://gmail.googleapis.com/gmail/v1/users/me/labels",
                None,
            )
            .await
        }
        "gmail.send" => gmail_send(&state, &params, None).await,
        "gmail.reply" => gmail_reply(&state, &params).await,
        "gmail.modify" => {
            let id = required_str(&params, "messageId")?;
            let body = json!({
                "addLabelIds": params.get("addLabelIds").cloned().unwrap_or_else(|| json!([])),
                "removeLabelIds": params.get("removeLabelIds").cloned().unwrap_or_else(|| json!([]))
            });
            let url =
                format!("https://gmail.googleapis.com/gmail/v1/users/me/messages/{id}/modify");
            google_request(&state, Method::POST, &url, Some(body)).await
        }
        "gmail.archive" => {
            let id = required_str(&params, "messageId")?;
            let url =
                format!("https://gmail.googleapis.com/gmail/v1/users/me/messages/{id}/modify");
            google_request(
                &state,
                Method::POST,
                &url,
                Some(json!({"removeLabelIds": ["INBOX"]})),
            )
            .await
        }
        "gmail.trash" | "gmail.untrash" => {
            let id = required_str(&params, "messageId")?;
            let op = action.trim_start_matches("gmail.");
            let url = format!("https://gmail.googleapis.com/gmail/v1/users/me/messages/{id}/{op}");
            google_request(&state, Method::POST, &url, Some(json!({}))).await
        }
        "drive.list" => {
            let mut url = Url::parse("https://www.googleapis.com/drive/v3/files")
                .map_err(|error| connector_error(error.to_string()))?;
            url.query_pairs_mut()
                .append_pair("pageSize", &params.get("pageSize").and_then(Value::as_u64).unwrap_or(50).clamp(1, 1000).to_string())
                .append_pair("fields", "nextPageToken,files(id,name,mimeType,size,modifiedTime,webViewLink,parents,trashed)");
            if let Some(query) = optional_str(&params, "query") {
                url.query_pairs_mut().append_pair("q", query);
            }
            google_request(&state, Method::GET, url.as_str(), None).await
        }
        "drive.get" => {
            let id = required_str(&params, "fileId")?;
            let url = format!("https://www.googleapis.com/drive/v3/files/{id}?fields=id,name,mimeType,size,modifiedTime,webViewLink,parents,trashed,description");
            google_request(&state, Method::GET, &url, None).await
        }
        "drive.download" => drive_download(&state, &params, false).await,
        "drive.export" => drive_download(&state, &params, true).await,
        "drive.create" => drive_write(&state, &params, false).await,
        "drive.update" => drive_write(&state, &params, true).await,
        "drive.delete" => {
            let id = required_str(&params, "fileId")?;
            let url = format!("https://www.googleapis.com/drive/v3/files/{id}");
            google_request(&state, Method::DELETE, &url, None).await
        }
        "calendar.calendars" => {
            google_request(
                &state,
                Method::GET,
                "https://www.googleapis.com/calendar/v3/users/me/calendarList?maxResults=250",
                None,
            )
            .await
        }
        "calendar.events" => {
            let calendar_id = optional_str(&params, "calendarId").unwrap_or("primary");
            let encoded =
                url::form_urlencoded::byte_serialize(calendar_id.as_bytes()).collect::<String>();
            let mut url = Url::parse(&format!(
                "https://www.googleapis.com/calendar/v3/calendars/{encoded}/events"
            ))
            .map_err(|error| connector_error(error.to_string()))?;
            url.query_pairs_mut()
                .append_pair("singleEvents", "true")
                .append_pair("orderBy", "startTime");
            for key in ["timeMin", "timeMax", "q"] {
                if let Some(value) = optional_str(&params, key) {
                    url.query_pairs_mut().append_pair(key, value);
                }
            }
            google_request(&state, Method::GET, url.as_str(), None).await
        }
        "calendar.create" | "calendar.update" => {
            let calendar_id = optional_str(&params, "calendarId").unwrap_or("primary");
            let encoded_calendar =
                url::form_urlencoded::byte_serialize(calendar_id.as_bytes()).collect::<String>();
            let event = params
                .get("event")
                .cloned()
                .ok_or_else(|| connector_error("missing required parameter 'event'"))?;
            if action == "calendar.create" {
                let url = format!(
                    "https://www.googleapis.com/calendar/v3/calendars/{encoded_calendar}/events"
                );
                google_request(&state, Method::POST, &url, Some(event)).await
            } else {
                let event_id = required_str(&params, "eventId")?;
                let encoded_event =
                    url::form_urlencoded::byte_serialize(event_id.as_bytes()).collect::<String>();
                let url = format!("https://www.googleapis.com/calendar/v3/calendars/{encoded_calendar}/events/{encoded_event}");
                google_request(&state, Method::PATCH, &url, Some(event)).await
            }
        }
        "calendar.delete" => {
            let calendar_id = optional_str(&params, "calendarId").unwrap_or("primary");
            let event_id = required_str(&params, "eventId")?;
            let encoded_calendar =
                url::form_urlencoded::byte_serialize(calendar_id.as_bytes()).collect::<String>();
            let encoded_event =
                url::form_urlencoded::byte_serialize(event_id.as_bytes()).collect::<String>();
            let url = format!("https://www.googleapis.com/calendar/v3/calendars/{encoded_calendar}/events/{encoded_event}");
            google_request(&state, Method::DELETE, &url, None).await
        }
        "contacts.list" => {
            let mut url = Url::parse("https://people.googleapis.com/v1/people/me/connections")
                .map_err(|error| connector_error(error.to_string()))?;
            url.query_pairs_mut()
                .append_pair(
                    "personFields",
                    "names,emailAddresses,phoneNumbers,organizations,photos",
                )
                .append_pair(
                    "pageSize",
                    &params
                        .get("pageSize")
                        .and_then(Value::as_u64)
                        .unwrap_or(100)
                        .clamp(1, 1000)
                        .to_string(),
                );
            google_request(&state, Method::GET, url.as_str(), None).await
        }
        "contacts.search" => {
            let query = required_str(&params, "query")?;
            let mut url = Url::parse("https://people.googleapis.com/v1/people:searchContacts")
                .map_err(|error| connector_error(error.to_string()))?;
            url.query_pairs_mut()
                .append_pair("query", query)
                .append_pair(
                    "readMask",
                    "names,emailAddresses,phoneNumbers,organizations,photos",
                )
                .append_pair("pageSize", "30");
            google_request(&state, Method::GET, url.as_str(), None).await
        }
        "contacts.get" => {
            let resource = required_str(&params, "resourceName")?;
            if !resource.starts_with("people/") || resource.contains("..") {
                return Err(connector_error("invalid People API resource name"));
            }
            let url = format!("https://people.googleapis.com/v1/{resource}?personFields=names,emailAddresses,phoneNumbers,organizations,photos,addresses");
            google_request(&state, Method::GET, &url, None).await
        }
        _ => Err(connector_error(format!("unsupported action '{action}'"))),
    }
}

async fn gmail_send(
    state: &State<'_, AppState>,
    params: &Value,
    reply: Option<(&str, &str, &str)>,
) -> Result<Value, AppError> {
    let to = safe_header(required_str(params, "to")?, "recipient")?;
    let subject = safe_header(required_str(params, "subject")?, "subject")?;
    let body = required_str(params, "body")?;
    let cc = optional_str(params, "cc")
        .map(|value| safe_header(value, "cc"))
        .transpose()?;
    let bcc = optional_str(params, "bcc")
        .map(|value| safe_header(value, "bcc"))
        .transpose()?;
    let mut headers = vec![
        format!("To: {to}"),
        format!("Subject: {subject}"),
        "MIME-Version: 1.0".to_string(),
        "Content-Type: text/plain; charset=UTF-8".to_string(),
        "Content-Transfer-Encoding: 8bit".to_string(),
    ];
    if let Some(cc) = cc {
        headers.insert(1, format!("Cc: {cc}"));
    }
    if let Some(bcc) = bcc {
        headers.insert(1, format!("Bcc: {bcc}"));
    }
    if let Some((message_id, references, _thread_id)) = reply {
        headers.push(format!("In-Reply-To: {message_id}"));
        headers.push(format!("References: {references}"));
    }
    let raw = format!("{}\r\n\r\n{}", headers.join("\r\n"), body);
    let mut payload = json!({"raw": URL_SAFE_NO_PAD.encode(raw.as_bytes())});
    if let Some((_message_id, _references, thread_id)) = reply {
        payload["threadId"] = Value::String(thread_id.to_string());
    } else if let Some(thread_id) = optional_str(params, "threadId") {
        payload["threadId"] = Value::String(thread_id.to_string());
    }
    google_request(
        state,
        Method::POST,
        "https://gmail.googleapis.com/gmail/v1/users/me/messages/send",
        Some(payload),
    )
    .await
}

fn gmail_header<'a>(message: &'a Value, name: &str) -> Option<&'a str> {
    message
        .pointer("/payload/headers")
        .and_then(Value::as_array)
        .and_then(|headers| {
            headers.iter().find_map(|header| {
                let header_name = header.get("name")?.as_str()?;
                if header_name.eq_ignore_ascii_case(name) {
                    header.get("value")?.as_str()
                } else {
                    None
                }
            })
        })
}

async fn gmail_reply(state: &State<'_, AppState>, params: &Value) -> Result<Value, AppError> {
    let message_id = required_str(params, "messageId")?;
    let body = required_str(params, "body")?;
    let url =
        format!("https://gmail.googleapis.com/gmail/v1/users/me/messages/{message_id}?format=full");
    let original = google_request(state, Method::GET, &url, None).await?;
    let to = gmail_header(&original, "Reply-To")
        .or_else(|| gmail_header(&original, "From"))
        .ok_or_else(|| connector_error("original message has no reply address"))?;
    let original_subject = gmail_header(&original, "Subject").unwrap_or("Re:");
    let subject = if original_subject.to_ascii_lowercase().starts_with("re:") {
        original_subject.to_string()
    } else {
        format!("Re: {original_subject}")
    };
    let rfc_message_id = gmail_header(&original, "Message-ID")
        .or_else(|| gmail_header(&original, "Message-Id"))
        .unwrap_or(message_id);
    let existing_references = gmail_header(&original, "References").unwrap_or("");
    let references = format!("{} {}", existing_references, rfc_message_id)
        .trim()
        .to_string();
    let thread_id = original
        .get("threadId")
        .and_then(Value::as_str)
        .ok_or_else(|| connector_error("original message has no thread ID"))?;
    let send_params = json!({"to": to, "subject": subject, "body": body});
    gmail_send(
        state,
        &send_params,
        Some((rfc_message_id, &references, thread_id)),
    )
    .await
}

async fn drive_download(
    state: &State<'_, AppState>,
    params: &Value,
    export: bool,
) -> Result<Value, AppError> {
    let file_id = required_str(params, "fileId")?;
    let token = access_token(state).await?;
    let url = if export {
        let mime_type = optional_str(params, "mimeType").unwrap_or("application/pdf");
        let mut url = Url::parse(&format!(
            "https://www.googleapis.com/drive/v3/files/{file_id}/export"
        ))
        .map_err(|error| connector_error(error.to_string()))?;
        url.query_pairs_mut().append_pair("mimeType", mime_type);
        url.to_string()
    } else {
        format!("https://www.googleapis.com/drive/v3/files/{file_id}?alt=media")
    };
    let response = state
        .http
        .get(url)
        .bearer_auth(token)
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|error| connector_error(error.to_string()))?;
    let bytes = read_bounded(response, MAX_BINARY_BYTES).await?;
    Ok(json!({
        "fileId": file_id,
        "sizeBytes": bytes.len(),
        "dataBase64": STANDARD.encode(bytes)
    }))
}

async fn drive_write(
    state: &State<'_, AppState>,
    params: &Value,
    update: bool,
) -> Result<Value, AppError> {
    let token = access_token(state).await?;
    let file_id = if update {
        Some(required_str(params, "fileId")?)
    } else {
        None
    };
    let mut metadata = params.get("metadata").cloned().unwrap_or_else(|| json!({}));
    if !update && metadata.get("name").and_then(Value::as_str).is_none() {
        if let Some(name) = optional_str(params, "name") {
            metadata["name"] = Value::String(name.to_string());
        }
    }
    let content = if let Some(encoded) = optional_str(params, "contentBase64") {
        Some(
            STANDARD
                .decode(encoded)
                .map_err(|error| connector_error(format!("contentBase64 is invalid: {error}")))?,
        )
    } else {
        optional_str(params, "content").map(|value| value.as_bytes().to_vec())
    };
    if let Some(bytes) = content {
        if bytes.len() > MAX_BINARY_BYTES {
            return Err(connector_error(
                "Drive upload exceeds the 8 MB interactive action limit",
            ));
        }
        let mime_type = optional_str(params, "mimeType").unwrap_or("application/octet-stream");
        let metadata_part = multipart::Part::text(metadata.to_string())
            .mime_str("application/json; charset=UTF-8")
            .map_err(|error| connector_error(error.to_string()))?;
        let media_part = multipart::Part::bytes(bytes)
            .mime_str(mime_type)
            .map_err(|error| connector_error(error.to_string()))?;
        let form = multipart::Form::new()
            .part("metadata", metadata_part)
            .part("media", media_part);
        let url = if let Some(file_id) = file_id {
            format!("https://www.googleapis.com/upload/drive/v3/files/{file_id}?uploadType=multipart&fields=id,name,mimeType,size,modifiedTime,webViewLink")
        } else {
            "https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart&fields=id,name,mimeType,size,modifiedTime,webViewLink".to_string()
        };
        let method = if update { Method::PATCH } else { Method::POST };
        let response = state
            .http
            .request(method, url)
            .bearer_auth(token)
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .multipart(form)
            .send()
            .await
            .map_err(|error| connector_error(error.to_string()))?;
        return json_response(response).await;
    }
    let url = if let Some(file_id) = file_id {
        format!("https://www.googleapis.com/drive/v3/files/{file_id}?fields=id,name,mimeType,size,modifiedTime,webViewLink")
    } else {
        "https://www.googleapis.com/drive/v3/files?fields=id,name,mimeType,size,modifiedTime,webViewLink".to_string()
    };
    google_request(
        state,
        if update { Method::PATCH } else { Method::POST },
        &url,
        Some(metadata),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_url_safe() {
        let challenge =
            pkce_challenge("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~");
        assert!(!challenge.contains('='));
        assert!(!challenge.contains('+'));
        assert!(!challenge.contains('/'));
    }

    #[test]
    fn mutating_actions_are_approval_gated() {
        assert!(is_mutating("gmail.send"));
        assert!(is_mutating("drive.delete"));
        assert!(!is_mutating("gmail.search"));
        assert!(!is_mutating("contacts.list"));
    }

    #[test]
    fn header_injection_is_rejected() {
        assert!(safe_header("person@example.com\r\nBcc: evil@example.com", "recipient").is_err());
    }
}
