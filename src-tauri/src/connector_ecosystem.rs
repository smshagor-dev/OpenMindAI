use std::{process::Command, time::Duration};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Utc;
use futures_util::StreamExt;
use reqwest::{header, Client, Method, Response};
use rusqlite::{params, OptionalExtension};
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

use crate::{app_error::AppError, github::secret_store, AppState};

const REQUEST_TIMEOUT_SECS: u64 = 30;
const OAUTH_TIMEOUT_SECS: u64 = 300;
const MAX_JSON_BYTES: usize = 2 * 1024 * 1024;
const MAX_BINARY_BYTES: usize = 8 * 1024 * 1024;
const NOTION_VERSION: &str = "2026-03-11";
const MCP_PROTOCOL_VERSION: &str = "2026-07-28";

const MICROSOFT_SCOPES: &str = "openid profile offline_access User.Read Mail.ReadWrite Mail.Send Files.ReadWrite Calendars.ReadWrite Contacts.Read";
const SLACK_BOT_SCOPES: &str = "channels:read,channels:history,groups:read,groups:history,im:read,im:history,mpim:read,mpim:history,chat:write,reactions:write,users:read";
const SLACK_USER_SCOPES: &str = "search:read";

const PROVIDERS: &[&str] = &["microsoft", "slack", "notion", "dropbox", "mcp"];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IntegrationRecord {
    #[serde(default)]
    config: Value,
    account_label: Option<String>,
    expires_at: Option<i64>,
    #[serde(default)]
    scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationStatus {
    provider: String,
    configured: bool,
    connected: bool,
    has_secret: bool,
    account_label: Option<String>,
    expires_at: Option<i64>,
    scopes: Vec<String>,
    config: Value,
}

fn connector_error(provider: &str, message: impl Into<String>) -> AppError {
    AppError::internal(format!(
        "{} connector: {}",
        provider_label(provider),
        message.into()
    ))
}

fn provider_label(provider: &str) -> &'static str {
    match provider {
        "microsoft" => "Microsoft 365",
        "slack" => "Slack",
        "notion" => "Notion",
        "dropbox" => "Dropbox",
        "mcp" => "MCP",
        _ => "Connected app",
    }
}

fn ensure_provider(provider: &str) -> Result<(), AppError> {
    if PROVIDERS.contains(&provider) {
        Ok(())
    } else {
        Err(AppError::internal(format!(
            "unsupported connected provider '{provider}'"
        )))
    }
}

fn settings_key(provider: &str) -> String {
    format!("app.integration.{provider}")
}

fn access_slot(provider: &str) -> String {
    format!("integration-{provider}-access")
}

fn refresh_slot(provider: &str) -> String {
    format!("integration-{provider}-refresh")
}

fn client_secret_slot(provider: &str) -> String {
    format!("integration-{provider}-client-secret")
}

fn slack_user_slot() -> &'static str {
    "integration-slack-user-access"
}

fn load_record(
    state: &State<'_, AppState>,
    provider: &str,
) -> Result<Option<IntegrationRecord>, AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    let raw: Option<String> = db
        .connection()
        .query_row(
            "SELECT value_json FROM app_settings WHERE key = ?1",
            params![settings_key(provider)],
            |row| row.get(0),
        )
        .optional()?;
    raw.map(|value| {
        serde_json::from_str(&value).map_err(|error| {
            connector_error(
                provider,
                format!("stored connector settings are invalid: {error}"),
            )
        })
    })
    .transpose()
}

fn save_record(
    state: &State<'_, AppState>,
    provider: &str,
    record: &IntegrationRecord,
) -> Result<(), AppError> {
    let raw = serde_json::to_string(record).map_err(|error| {
        connector_error(provider, format!("could not serialize settings: {error}"))
    })?;
    let db = state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    db.connection().execute(
        "INSERT INTO app_settings (key, value_json, updated_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
        params![settings_key(provider), raw, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

fn delete_record(state: &State<'_, AppState>, provider: &str) -> Result<(), AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    db.connection().execute(
        "DELETE FROM app_settings WHERE key = ?1",
        params![settings_key(provider)],
    )?;
    Ok(())
}

fn config_str<'a>(config: &'a Value, key: &str) -> Option<&'a str> {
    config
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn required_config<'a>(provider: &str, config: &'a Value, key: &str) -> Result<&'a str, AppError> {
    config_str(config, key).ok_or_else(|| {
        connector_error(provider, format!("configuration field '{key}' is required"))
    })
}

fn required_str<'a>(provider: &str, params: &'a Value, key: &str) -> Result<&'a str, AppError> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| connector_error(provider, format!("missing required parameter '{key}'")))
}

fn optional_str<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn safe_header(provider: &str, value: &str, label: &str) -> Result<header::HeaderValue, AppError> {
    if value.contains('\r') || value.contains('\n') {
        return Err(connector_error(
            provider,
            format!("{label} contains an invalid newline"),
        ));
    }
    header::HeaderValue::from_str(value.trim())
        .map_err(|error| connector_error(provider, format!("invalid {label}: {error}")))
}

fn configured(
    provider: &str,
    record: Option<&IntegrationRecord>,
) -> Result<(bool, bool), AppError> {
    let Some(record) = record else {
        return Ok((false, false));
    };
    let has_secret = match provider {
        "slack" | "notion" => secret_store::get_secret(&client_secret_slot(provider))?.is_some(),
        "mcp" => secret_store::get_secret(&access_slot(provider))?.is_some(),
        _ => false,
    };
    let ready = match provider {
        "microsoft" => {
            config_str(&record.config, "clientId").is_some()
                && config_str(&record.config, "redirectUri").is_some()
        }
        "slack" | "notion" => {
            config_str(&record.config, "clientId").is_some()
                && config_str(&record.config, "redirectUri").is_some()
                && has_secret
        }
        "dropbox" => {
            config_str(&record.config, "appKey").is_some()
                && config_str(&record.config, "redirectUri").is_some()
        }
        "mcp" => config_str(&record.config, "endpoint").is_some(),
        _ => false,
    };
    Ok((ready, has_secret))
}

fn validate_redirect(provider: &str, redirect_uri: &str) -> Result<Url, AppError> {
    let url = Url::parse(redirect_uri)
        .map_err(|error| connector_error(provider, format!("invalid redirect URI: {error}")))?;
    if url.scheme() != "http" {
        return Err(connector_error(
            provider,
            "desktop OAuth redirect URI must use http://localhost or http://127.0.0.1",
        ));
    }
    let host = url.host_str().unwrap_or_default();
    if host != "localhost" && host != "127.0.0.1" {
        return Err(connector_error(
            provider,
            "desktop OAuth redirect URI must use localhost or 127.0.0.1",
        ));
    }
    if url.port().is_none() {
        return Err(connector_error(
            provider,
            "desktop OAuth redirect URI must include a fixed local port",
        ));
    }
    Ok(url)
}

fn validate_mcp_endpoint(endpoint: &str) -> Result<Url, AppError> {
    let url = Url::parse(endpoint)
        .map_err(|error| connector_error("mcp", format!("invalid endpoint URL: {error}")))?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if url.scheme() != "https" && !(url.scheme() == "http" && local) {
        return Err(connector_error(
            "mcp",
            "remote MCP endpoints must use HTTPS; HTTP is allowed only for localhost",
        ));
    }
    Ok(url)
}

fn is_mutating(provider: &str, action: &str) -> bool {
    match provider {
        "microsoft" => matches!(
            action,
            "mail.send"
                | "mail.reply"
                | "mail.delete"
                | "drive.upload"
                | "drive.delete"
                | "calendar.create"
                | "calendar.update"
                | "calendar.delete"
        ),
        "slack" => matches!(
            action,
            "chat.send" | "chat.update" | "chat.delete" | "reactions.add" | "reactions.remove"
        ),
        "notion" => matches!(
            action,
            "page.create" | "page.update" | "block.append" | "comment.create"
        ),
        "dropbox" => matches!(action, "files.upload" | "files.move" | "files.delete"),
        "mcp" => action == "tools.call",
        _ => false,
    }
}

fn require_approval(provider: &str, action: &str, approved: bool) -> Result<(), AppError> {
    if approved {
        Ok(())
    } else {
        Err(connector_error(
            provider,
            format!("action '{action}' changes remote data and requires explicit approval"),
        ))
    }
}

async fn read_bounded(
    provider: &str,
    response: Response,
    max_bytes: usize,
) -> Result<Vec<u8>, AppError> {
    let status = response.status();
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| connector_error(provider, error.to_string()))?;
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(connector_error(
                provider,
                format!("response exceeded the {max_bytes} byte safety limit"),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let detail = String::from_utf8_lossy(&body);
        return Err(connector_error(
            provider,
            format!(
                "remote API returned status {status}: {}",
                detail.chars().take(1200).collect::<String>()
            ),
        ));
    }
    Ok(body)
}

async fn json_response(provider: &str, response: Response) -> Result<Value, AppError> {
    let body = read_bounded(provider, response, MAX_JSON_BYTES).await?;
    if body.is_empty() {
        return Ok(json!({"ok": true}));
    }
    serde_json::from_slice(&body)
        .map_err(|error| connector_error(provider, format!("invalid JSON response: {error}")))
}

fn open_browser(provider: &str, url: &str) -> Result<(), AppError> {
    #[cfg(target_os = "windows")]
    {
        Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
            .map_err(|error| {
                connector_error(provider, format!("could not open browser: {error}"))
            })?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(url).spawn().map_err(|error| {
            connector_error(provider, format!("could not open browser: {error}"))
        })?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(url).spawn().map_err(|error| {
            connector_error(provider, format!("could not open browser: {error}"))
        })?;
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
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

async fn oauth_listener(
    provider: &str,
    redirect_uri: &str,
) -> Result<(TcpListener, Url), AppError> {
    let redirect = validate_redirect(provider, redirect_uri)?;
    let port = redirect
        .port()
        .ok_or_else(|| connector_error(provider, "redirect URI has no port"))?;
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|error| {
            connector_error(
                provider,
                format!("could not bind OAuth callback port {port}: {error}"),
            )
        })?;
    Ok((listener, redirect))
}

async fn receive_oauth_callback(
    provider: &str,
    listener: TcpListener,
    redirect: &Url,
    expected_state: &str,
) -> Result<String, AppError> {
    let work = async {
        let (mut socket, _) = listener.accept().await?;
        let mut buffer = vec![0u8; 16 * 1024];
        let read = socket.read(&mut buffer).await?;
        let request = String::from_utf8_lossy(&buffer[..read]);
        let target = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .ok_or_else(|| connector_error(provider, "OAuth callback request was invalid"))?;
        let callback = Url::parse(&format!("http://127.0.0.1{target}")).map_err(|error| {
            connector_error(provider, format!("invalid OAuth callback: {error}"))
        })?;
        if callback.path() != redirect.path() {
            return Err(connector_error(
                provider,
                "OAuth callback path did not match configured redirect URI",
            ));
        }
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
            format!(
                "{} connected. You can return to OpenMindAI.",
                provider_label(provider)
            )
        } else {
            format!(
                "{} connection failed. Return to OpenMindAI for details.",
                provider_label(provider)
            )
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
            return Err(connector_error(
                provider,
                format!("authorization returned '{error}'"),
            ));
        }
        if state.as_deref() != Some(expected_state) {
            return Err(connector_error(provider, "OAuth state validation failed"));
        }
        code.ok_or_else(|| {
            connector_error(
                provider,
                "OAuth callback did not contain an authorization code",
            )
        })
    };
    timeout(Duration::from_secs(OAUTH_TIMEOUT_SECS), work)
        .await
        .map_err(|_| connector_error(provider, "authorization timed out"))?
}

fn token_field<'a>(provider: &str, value: &'a Value, key: &str) -> Result<&'a str, AppError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|item| !item.is_empty())
        .ok_or_else(|| connector_error(provider, format!("token response did not contain '{key}'")))
}

fn save_tokens(
    state: &State<'_, AppState>,
    provider: &str,
    access_token: &str,
    refresh_token: Option<&str>,
    expires_in: Option<i64>,
    scopes: Vec<String>,
    account_label: Option<String>,
) -> Result<(), AppError> {
    secret_store::set_secret(&access_slot(provider), access_token)?;
    if let Some(refresh) = refresh_token.filter(|value| !value.is_empty()) {
        secret_store::set_secret(&refresh_slot(provider), refresh)?;
    } else {
        secret_store::delete_secret(&refresh_slot(provider))?;
    }
    let mut record = load_record(state, provider)?.unwrap_or_default();
    record.expires_at = expires_in.map(|seconds| Utc::now().timestamp() + seconds);
    record.scopes = scopes;
    record.account_label = account_label;
    save_record(state, provider, &record)
}

async fn microsoft_connect(state: &State<'_, AppState>) -> Result<(), AppError> {
    let record = load_record(state, "microsoft")?
        .ok_or_else(|| connector_error("microsoft", "connector is not configured"))?;
    let client_id = required_config("microsoft", &record.config, "clientId")?.to_string();
    let tenant = config_str(&record.config, "tenant")
        .unwrap_or("common")
        .to_string();
    let redirect_uri = required_config("microsoft", &record.config, "redirectUri")?.to_string();
    let (listener, redirect) = oauth_listener("microsoft", &redirect_uri).await?;
    let verifier = pkce_verifier();
    let challenge = pkce_challenge(&verifier);
    let oauth_state = Uuid::new_v4().to_string();
    let mut auth = Url::parse(&format!(
        "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize"
    ))
    .map_err(|error| connector_error("microsoft", error.to_string()))?;
    auth.query_pairs_mut()
        .append_pair("client_id", &client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_mode", "query")
        .append_pair("scope", MICROSOFT_SCOPES)
        .append_pair("state", &oauth_state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    open_browser("microsoft", auth.as_str())?;
    let code = receive_oauth_callback("microsoft", listener, &redirect, &oauth_state).await?;
    let response = state
        .http
        .post(format!(
            "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token"
        ))
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .form(&[
            ("client_id", client_id.as_str()),
            ("scope", MICROSOFT_SCOPES),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .map_err(|error| connector_error("microsoft", format!("token exchange failed: {error}")))?;
    let token = json_response("microsoft", response).await?;
    let access = token_field("microsoft", &token, "access_token")?.to_string();
    let refresh = token
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let expires = token.get("expires_in").and_then(Value::as_i64);
    let scopes = token
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or(MICROSOFT_SCOPES)
        .split_whitespace()
        .map(ToOwned::to_owned)
        .collect();
    let label = microsoft_account_label(&state.http, &access).await?;
    save_tokens(
        state,
        "microsoft",
        &access,
        refresh.as_deref(),
        expires,
        scopes,
        Some(label),
    )
}

async fn slack_connect(state: &State<'_, AppState>) -> Result<(), AppError> {
    let record = load_record(state, "slack")?
        .ok_or_else(|| connector_error("slack", "connector is not configured"))?;
    let client_id = required_config("slack", &record.config, "clientId")?.to_string();
    let client_secret = secret_store::get_secret(&client_secret_slot("slack"))?
        .ok_or_else(|| connector_error("slack", "OAuth client secret is missing"))?;
    let redirect_uri = required_config("slack", &record.config, "redirectUri")?.to_string();
    let bot_scopes = config_str(&record.config, "botScopes").unwrap_or(SLACK_BOT_SCOPES);
    let user_scopes = config_str(&record.config, "userScopes").unwrap_or(SLACK_USER_SCOPES);
    let (listener, redirect) = oauth_listener("slack", &redirect_uri).await?;
    let oauth_state = Uuid::new_v4().to_string();
    let mut auth = Url::parse("https://slack.com/oauth/v2/authorize")
        .map_err(|error| connector_error("slack", error.to_string()))?;
    auth.query_pairs_mut()
        .append_pair("client_id", &client_id)
        .append_pair("scope", bot_scopes)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("state", &oauth_state);
    if !user_scopes.is_empty() {
        auth.query_pairs_mut()
            .append_pair("user_scope", user_scopes);
    }
    open_browser("slack", auth.as_str())?;
    let code = receive_oauth_callback("slack", listener, &redirect, &oauth_state).await?;
    let response = state
        .http
        .post("https://slack.com/api/oauth.v2.access")
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .basic_auth(&client_id, Some(&client_secret))
        .form(&[
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
        ])
        .send()
        .await
        .map_err(|error| connector_error("slack", format!("token exchange failed: {error}")))?;
    let token = json_response("slack", response).await?;
    if token.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(connector_error(
            "slack",
            format!(
                "OAuth failed: {}",
                token
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error")
            ),
        ));
    }
    let access = token_field("slack", &token, "access_token")?.to_string();
    let refresh = token
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let expires = token.get("expires_in").and_then(Value::as_i64);
    if let Some(user_token) = token
        .get("authed_user")
        .and_then(|item| item.get("access_token"))
        .and_then(Value::as_str)
    {
        secret_store::set_secret(slack_user_slot(), user_token)?;
    }
    let scopes = token
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .split(',')
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    let label = slack_account_label(&state.http, &access).await?;
    save_tokens(
        state,
        "slack",
        &access,
        refresh.as_deref(),
        expires,
        scopes,
        Some(label),
    )
}

async fn notion_connect(state: &State<'_, AppState>) -> Result<(), AppError> {
    let record = load_record(state, "notion")?
        .ok_or_else(|| connector_error("notion", "connector is not configured"))?;
    let client_id = required_config("notion", &record.config, "clientId")?.to_string();
    let client_secret = secret_store::get_secret(&client_secret_slot("notion"))?
        .ok_or_else(|| connector_error("notion", "OAuth client secret is missing"))?;
    let redirect_uri = required_config("notion", &record.config, "redirectUri")?.to_string();
    let (listener, redirect) = oauth_listener("notion", &redirect_uri).await?;
    let oauth_state = Uuid::new_v4().to_string();
    let mut auth = Url::parse("https://api.notion.com/v1/oauth/authorize")
        .map_err(|error| connector_error("notion", error.to_string()))?;
    auth.query_pairs_mut()
        .append_pair("owner", "user")
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("state", &oauth_state);
    open_browser("notion", auth.as_str())?;
    let code = receive_oauth_callback("notion", listener, &redirect, &oauth_state).await?;
    let response = state
        .http
        .post("https://api.notion.com/v1/oauth/token")
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .basic_auth(&client_id, Some(&client_secret))
        .header("Notion-Version", NOTION_VERSION)
        .json(&json!({
            "grant_type": "authorization_code",
            "code": code,
            "redirect_uri": redirect_uri
        }))
        .send()
        .await
        .map_err(|error| connector_error("notion", format!("token exchange failed: {error}")))?;
    let token = json_response("notion", response).await?;
    let access = token_field("notion", &token, "access_token")?.to_string();
    let refresh = token
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let expires = token.get("expires_in").and_then(Value::as_i64);
    let label = token
        .get("workspace_name")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "Notion workspace".to_string());
    save_tokens(
        state,
        "notion",
        &access,
        refresh.as_deref(),
        expires,
        Vec::new(),
        Some(label),
    )
}

async fn dropbox_connect(state: &State<'_, AppState>) -> Result<(), AppError> {
    let record = load_record(state, "dropbox")?
        .ok_or_else(|| connector_error("dropbox", "connector is not configured"))?;
    let app_key = required_config("dropbox", &record.config, "appKey")?.to_string();
    let redirect_uri = required_config("dropbox", &record.config, "redirectUri")?.to_string();
    let (listener, redirect) = oauth_listener("dropbox", &redirect_uri).await?;
    let verifier = pkce_verifier();
    let challenge = pkce_challenge(&verifier);
    let oauth_state = Uuid::new_v4().to_string();
    let mut auth = Url::parse("https://www.dropbox.com/oauth2/authorize")
        .map_err(|error| connector_error("dropbox", error.to_string()))?;
    auth.query_pairs_mut()
        .append_pair("client_id", &app_key)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("token_access_type", "offline")
        .append_pair("state", &oauth_state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    open_browser("dropbox", auth.as_str())?;
    let code = receive_oauth_callback("dropbox", listener, &redirect, &oauth_state).await?;
    let response = state
        .http
        .post("https://api.dropboxapi.com/oauth2/token")
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .form(&[
            ("code", code.as_str()),
            ("grant_type", "authorization_code"),
            ("client_id", app_key.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .map_err(|error| connector_error("dropbox", format!("token exchange failed: {error}")))?;
    let token = json_response("dropbox", response).await?;
    let access = token_field("dropbox", &token, "access_token")?.to_string();
    let refresh = token
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let expires = token.get("expires_in").and_then(Value::as_i64);
    let scopes = token
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .split_whitespace()
        .map(ToOwned::to_owned)
        .collect();
    let label = dropbox_account_label(&state.http, &access).await?;
    save_tokens(
        state,
        "dropbox",
        &access,
        refresh.as_deref(),
        expires,
        scopes,
        Some(label),
    )
}

async fn refresh_access_token(
    state: &State<'_, AppState>,
    provider: &str,
) -> Result<String, AppError> {
    let refresh = secret_store::get_secret(&refresh_slot(provider))?.ok_or_else(|| {
        connector_error(
            provider,
            "access token expired and no refresh token is available",
        )
    })?;
    let record = load_record(state, provider)?
        .ok_or_else(|| connector_error(provider, "connector settings are missing"))?;
    let response = match provider {
        "microsoft" => {
            let client_id = required_config(provider, &record.config, "clientId")?;
            let tenant = config_str(&record.config, "tenant").unwrap_or("common");
            state
                .http
                .post(format!(
                    "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token"
                ))
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
                .form(&[
                    ("client_id", client_id),
                    ("scope", MICROSOFT_SCOPES),
                    ("refresh_token", refresh.as_str()),
                    ("grant_type", "refresh_token"),
                ])
                .send()
                .await
        }
        "slack" => {
            let client_id = required_config(provider, &record.config, "clientId")?;
            let secret = secret_store::get_secret(&client_secret_slot(provider))?
                .ok_or_else(|| connector_error(provider, "OAuth client secret is missing"))?;
            state
                .http
                .post("https://slack.com/api/oauth.v2.access")
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
                .basic_auth(client_id, Some(secret))
                .form(&[
                    ("grant_type", "refresh_token"),
                    ("refresh_token", refresh.as_str()),
                ])
                .send()
                .await
        }
        "notion" => {
            let client_id = required_config(provider, &record.config, "clientId")?;
            let secret = secret_store::get_secret(&client_secret_slot(provider))?
                .ok_or_else(|| connector_error(provider, "OAuth client secret is missing"))?;
            state
                .http
                .post("https://api.notion.com/v1/oauth/token")
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
                .basic_auth(client_id, Some(secret))
                .header("Notion-Version", NOTION_VERSION)
                .json(&json!({"grant_type": "refresh_token", "refresh_token": refresh}))
                .send()
                .await
        }
        "dropbox" => {
            let app_key = required_config(provider, &record.config, "appKey")?;
            state
                .http
                .post("https://api.dropboxapi.com/oauth2/token")
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
                .form(&[
                    ("grant_type", "refresh_token"),
                    ("refresh_token", refresh.as_str()),
                    ("client_id", app_key),
                ])
                .send()
                .await
        }
        _ => return Err(connector_error(provider, "token refresh is not supported")),
    }
    .map_err(|error| connector_error(provider, format!("token refresh failed: {error}")))?;
    let value = json_response(provider, response).await?;
    if provider == "slack" && value.get("ok").and_then(Value::as_bool) == Some(false) {
        return Err(connector_error(
            provider,
            value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("token refresh failed"),
        ));
    }
    let access = token_field(provider, &value, "access_token")?.to_string();
    secret_store::set_secret(&access_slot(provider), &access)?;
    if let Some(new_refresh) = value.get("refresh_token").and_then(Value::as_str) {
        secret_store::set_secret(&refresh_slot(provider), new_refresh)?;
    }
    let mut updated = record;
    updated.expires_at = value
        .get("expires_in")
        .and_then(Value::as_i64)
        .map(|seconds| Utc::now().timestamp() + seconds);
    if let Some(scope) = value.get("scope").and_then(Value::as_str) {
        updated.scopes = if provider == "slack" {
            scope
                .split(',')
                .filter(|item| !item.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        } else {
            scope.split_whitespace().map(ToOwned::to_owned).collect()
        };
    }
    save_record(state, provider, &updated)?;
    Ok(access)
}

async fn access_token(state: &State<'_, AppState>, provider: &str) -> Result<String, AppError> {
    let record = load_record(state, provider)?
        .ok_or_else(|| connector_error(provider, "connector is not configured"))?;
    let token = secret_store::get_secret(&access_slot(provider))?
        .ok_or_else(|| connector_error(provider, "account is not connected"))?;
    if record
        .expires_at
        .is_some_and(|expires| expires <= Utc::now().timestamp() + 60)
    {
        refresh_access_token(state, provider).await
    } else {
        Ok(token)
    }
}

async fn microsoft_account_label(client: &Client, token: &str) -> Result<String, AppError> {
    let response = client
        .get("https://graph.microsoft.com/v1.0/me?$select=displayName,mail,userPrincipalName")
        .bearer_auth(token)
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|error| connector_error("microsoft", error.to_string()))?;
    let value = json_response("microsoft", response).await?;
    Ok(value
        .get("mail")
        .and_then(Value::as_str)
        .or_else(|| value.get("userPrincipalName").and_then(Value::as_str))
        .or_else(|| value.get("displayName").and_then(Value::as_str))
        .unwrap_or("Microsoft account")
        .to_string())
}

async fn slack_account_label(client: &Client, token: &str) -> Result<String, AppError> {
    let response = client
        .post("https://slack.com/api/auth.test")
        .bearer_auth(token)
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|error| connector_error("slack", error.to_string()))?;
    let value = json_response("slack", response).await?;
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(connector_error(
            "slack",
            value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("token validation failed"),
        ));
    }
    let team = value
        .get("team")
        .and_then(Value::as_str)
        .unwrap_or("Slack workspace");
    let user = value
        .get("user")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Ok(if user.is_empty() {
        team.to_string()
    } else {
        format!("{team} · {user}")
    })
}

async fn notion_account_label(client: &Client, token: &str) -> Result<String, AppError> {
    let response = client
        .get("https://api.notion.com/v1/users/me")
        .bearer_auth(token)
        .header("Notion-Version", NOTION_VERSION)
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|error| connector_error("notion", error.to_string()))?;
    let value = json_response("notion", response).await?;
    Ok(value
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Notion connection")
        .to_string())
}

async fn dropbox_account_label(client: &Client, token: &str) -> Result<String, AppError> {
    let response = client
        .post("https://api.dropboxapi.com/2/users/get_current_account")
        .bearer_auth(token)
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|error| connector_error("dropbox", error.to_string()))?;
    let value = json_response("dropbox", response).await?;
    Ok(value
        .get("email")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/name/display_name").and_then(Value::as_str))
        .unwrap_or("Dropbox account")
        .to_string())
}

async fn connect_with_token(
    state: &State<'_, AppState>,
    provider: &str,
    token: &str,
) -> Result<(), AppError> {
    if token.trim().is_empty() {
        return Err(connector_error(provider, "token cannot be empty"));
    }
    let label = match provider {
        "microsoft" => microsoft_account_label(&state.http, token).await?,
        "slack" => slack_account_label(&state.http, token).await?,
        "notion" => notion_account_label(&state.http, token).await?,
        "dropbox" => dropbox_account_label(&state.http, token).await?,
        "mcp" => {
            let slot = access_slot(provider);
            secret_store::set_secret(&slot, token)?;
            if let Err(error) = mcp_rpc(state, "tools/list", json!({}), None).await {
                let _ = secret_store::delete_secret(&slot);
                return Err(error);
            }
            load_record(state, provider)?
                .and_then(|record| config_str(&record.config, "name").map(ToOwned::to_owned))
                .unwrap_or_else(|| "MCP server".to_string())
        }
        _ => {
            return Err(connector_error(
                provider,
                "token connection is not supported",
            ))
        }
    };
    secret_store::set_secret(&access_slot(provider), token)?;
    secret_store::delete_secret(&refresh_slot(provider))?;
    let mut record = load_record(state, provider)?.unwrap_or_default();
    record.account_label = Some(label);
    record.expires_at = None;
    save_record(state, provider, &record)
}

async fn microsoft_request(
    state: &State<'_, AppState>,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, AppError> {
    let token = access_token(state, "microsoft").await?;
    let url = format!("https://graph.microsoft.com/v1.0{path}");
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
        .map_err(|error| connector_error("microsoft", error.to_string()))?;
    json_response("microsoft", response).await
}

async fn microsoft_binary(
    state: &State<'_, AppState>,
    method: Method,
    path: &str,
    body: Option<Vec<u8>>,
) -> Result<Value, AppError> {
    let token = access_token(state, "microsoft").await?;
    let url = format!("https://graph.microsoft.com/v1.0{path}");
    let mut request = state
        .http
        .request(method, url)
        .bearer_auth(token)
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS));
    if let Some(bytes) = body {
        request = request.body(bytes);
    }
    let response = request
        .send()
        .await
        .map_err(|error| connector_error("microsoft", error.to_string()))?;
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let bytes = read_bounded("microsoft", response, MAX_BINARY_BYTES).await?;
    Ok(json!({
        "mimeType": content_type,
        "size": bytes.len(),
        "contentBase64": STANDARD.encode(bytes)
    }))
}

fn encode_graph_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(*byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn encode_graph_path(path: &str) -> String {
    path.trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(encode_graph_segment)
        .collect::<Vec<_>>()
        .join("/")
}

async fn slack_request(
    state: &State<'_, AppState>,
    method_name: &str,
    params: Value,
    prefer_user_token: bool,
) -> Result<Value, AppError> {
    let token = if prefer_user_token {
        secret_store::get_secret(slack_user_slot())?.unwrap_or(access_token(state, "slack").await?)
    } else {
        access_token(state, "slack").await?
    };
    let response = state
        .http
        .post(format!("https://slack.com/api/{method_name}"))
        .bearer_auth(token)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .json(&params)
        .send()
        .await
        .map_err(|error| connector_error("slack", error.to_string()))?;
    let value = json_response("slack", response).await?;
    if value.get("ok").and_then(Value::as_bool) == Some(false) {
        return Err(connector_error(
            "slack",
            value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Slack API request failed"),
        ));
    }
    Ok(value)
}

async fn notion_request(
    state: &State<'_, AppState>,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, AppError> {
    let token = access_token(state, "notion").await?;
    let mut request = state
        .http
        .request(method, format!("https://api.notion.com/v1{path}"))
        .bearer_auth(token)
        .header("Notion-Version", NOTION_VERSION)
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS));
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .map_err(|error| connector_error("notion", error.to_string()))?;
    json_response("notion", response).await
}

async fn dropbox_rpc(
    state: &State<'_, AppState>,
    path: &str,
    body: Value,
) -> Result<Value, AppError> {
    let token = access_token(state, "dropbox").await?;
    let response = state
        .http
        .post(format!("https://api.dropboxapi.com/2/{path}"))
        .bearer_auth(token)
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .json(&body)
        .send()
        .await
        .map_err(|error| connector_error("dropbox", error.to_string()))?;
    json_response("dropbox", response).await
}

async fn dropbox_current_account(state: &State<'_, AppState>) -> Result<Value, AppError> {
    let token = access_token(state, "dropbox").await?;
    let response = state
        .http
        .post("https://api.dropboxapi.com/2/users/get_current_account")
        .bearer_auth(token)
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|error| connector_error("dropbox", error.to_string()))?;
    json_response("dropbox", response).await
}

async fn dropbox_download(state: &State<'_, AppState>, path: &str) -> Result<Value, AppError> {
    let token = access_token(state, "dropbox").await?;
    let api_arg = serde_json::to_string(&json!({"path": path}))
        .map_err(|error| connector_error("dropbox", error.to_string()))?;
    let response = state
        .http
        .post("https://content.dropboxapi.com/2/files/download")
        .bearer_auth(token)
        .header(
            "Dropbox-API-Arg",
            safe_header("dropbox", &api_arg, "Dropbox-API-Arg")?,
        )
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|error| connector_error("dropbox", error.to_string()))?;
    let mime = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let bytes = read_bounded("dropbox", response, MAX_BINARY_BYTES).await?;
    Ok(json!({"mimeType": mime, "size": bytes.len(), "contentBase64": STANDARD.encode(bytes)}))
}

async fn dropbox_upload(state: &State<'_, AppState>, params: &Value) -> Result<Value, AppError> {
    let token = access_token(state, "dropbox").await?;
    let path = required_str("dropbox", params, "path")?;
    let bytes = if let Some(encoded) = optional_str(params, "contentBase64") {
        STANDARD.decode(encoded).map_err(|error| {
            connector_error("dropbox", format!("invalid contentBase64: {error}"))
        })?
    } else {
        optional_str(params, "content")
            .unwrap_or_default()
            .as_bytes()
            .to_vec()
    };
    if bytes.len() > MAX_BINARY_BYTES {
        return Err(connector_error(
            "dropbox",
            "interactive upload exceeds the 8 MB safety limit",
        ));
    }
    let api_arg = serde_json::to_string(&json!({
        "path": path,
        "mode": optional_str(params, "mode").unwrap_or("overwrite"),
        "autorename": false,
        "mute": false
    }))
    .map_err(|error| connector_error("dropbox", error.to_string()))?;
    let response = state
        .http
        .post("https://content.dropboxapi.com/2/files/upload")
        .bearer_auth(token)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            "Dropbox-API-Arg",
            safe_header("dropbox", &api_arg, "Dropbox-API-Arg")?,
        )
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .body(bytes)
        .send()
        .await
        .map_err(|error| connector_error("dropbox", error.to_string()))?;
    json_response("dropbox", response).await
}

async fn mcp_rpc(
    state: &State<'_, AppState>,
    method: &str,
    params: Value,
    name: Option<&str>,
) -> Result<Value, AppError> {
    let record = load_record(state, "mcp")?
        .ok_or_else(|| connector_error("mcp", "server is not configured"))?;
    let endpoint = required_config("mcp", &record.config, "endpoint")?;
    validate_mcp_endpoint(endpoint)?;
    let token = secret_store::get_secret(&access_slot("mcp"))?;
    let mut request = state
        .http
        .post(endpoint)
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header(header::CONTENT_TYPE, "application/json")
        .header("MCP-Protocol-Version", MCP_PROTOCOL_VERSION)
        .header("Mcp-Method", safe_header("mcp", method, "Mcp-Method")?)
        .header(
            "Mcp-Name",
            safe_header("mcp", name.unwrap_or(method), "Mcp-Name")?,
        )
        .json(&json!({
            "jsonrpc": "2.0",
            "id": Uuid::new_v4().to_string(),
            "method": method,
            "params": params
        }));
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .map_err(|error| connector_error("mcp", error.to_string()))?;
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let body = read_bounded("mcp", response, MAX_JSON_BYTES).await?;
    let value: Value = if content_type.contains("text/event-stream") {
        let text = String::from_utf8_lossy(&body);
        let data = text
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim)
            .rfind(|line| !line.is_empty())
            .ok_or_else(|| connector_error("mcp", "SSE response did not contain JSON data"))?;
        serde_json::from_str(data).map_err(|error| {
            connector_error("mcp", format!("invalid SSE JSON response: {error}"))
        })?
    } else {
        serde_json::from_slice(&body).map_err(|error| {
            connector_error("mcp", format!("invalid JSON-RPC response: {error}"))
        })?
    };
    if let Some(error) = value.get("error") {
        return Err(connector_error("mcp", format!("JSON-RPC error: {error}")));
    }
    Ok(value.get("result").cloned().unwrap_or(value))
}

#[tauri::command]
pub fn integration_status(
    provider: String,
    state: State<'_, AppState>,
) -> Result<IntegrationStatus, AppError> {
    ensure_provider(&provider)?;
    let record = load_record(&state, &provider)?;
    let (is_configured, has_secret) = configured(&provider, record.as_ref())?;
    let connected = if provider == "mcp" {
        is_configured
            && record
                .as_ref()
                .and_then(|item| item.account_label.as_ref())
                .is_some()
    } else {
        secret_store::get_secret(&access_slot(&provider))?.is_some()
    };
    let record = record.unwrap_or_default();
    Ok(IntegrationStatus {
        provider,
        configured: is_configured,
        connected,
        has_secret,
        account_label: record.account_label,
        expires_at: record.expires_at,
        scopes: record.scopes,
        config: record.config,
    })
}

#[tauri::command]
pub fn save_integration_config(
    provider: String,
    config: Value,
    secret: Option<String>,
    state: State<'_, AppState>,
) -> Result<IntegrationStatus, AppError> {
    ensure_provider(&provider)?;
    if !config.is_object() {
        return Err(connector_error(
            &provider,
            "configuration must be a JSON object",
        ));
    }
    match provider.as_str() {
        "microsoft" => {
            required_config(&provider, &config, "clientId")?;
            validate_redirect(
                &provider,
                required_config(&provider, &config, "redirectUri")?,
            )?;
        }
        "slack" | "notion" => {
            required_config(&provider, &config, "clientId")?;
            validate_redirect(
                &provider,
                required_config(&provider, &config, "redirectUri")?,
            )?;
        }
        "dropbox" => {
            required_config(&provider, &config, "appKey")?;
            validate_redirect(
                &provider,
                required_config(&provider, &config, "redirectUri")?,
            )?;
        }
        "mcp" => {
            validate_mcp_endpoint(required_config(&provider, &config, "endpoint")?)?;
        }
        _ => unreachable!(),
    }
    let mut record = load_record(&state, &provider)?.unwrap_or_default();
    record.config = config;
    save_record(&state, &provider, &record)?;
    if let Some(secret) = secret {
        let slot = if provider == "mcp" {
            access_slot(&provider)
        } else {
            client_secret_slot(&provider)
        };
        if secret.trim().is_empty() {
            secret_store::delete_secret(&slot)?;
        } else {
            secret_store::set_secret(&slot, secret.trim())?;
        }
    }
    integration_status(provider, state)
}

#[tauri::command]
pub fn clear_integration_config(
    provider: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    ensure_provider(&provider)?;
    for slot in [
        access_slot(&provider),
        refresh_slot(&provider),
        client_secret_slot(&provider),
    ] {
        secret_store::delete_secret(&slot)?;
    }
    if provider == "slack" {
        secret_store::delete_secret(slack_user_slot())?;
    }
    delete_record(&state, &provider)
}

#[tauri::command]
pub async fn connect_integration(
    provider: String,
    token: Option<String>,
    state: State<'_, AppState>,
) -> Result<IntegrationStatus, AppError> {
    ensure_provider(&provider)?;
    if let Some(token) = token.filter(|value| !value.trim().is_empty()) {
        connect_with_token(&state, &provider, token.trim()).await?;
    } else {
        match provider.as_str() {
            "microsoft" => microsoft_connect(&state).await?,
            "slack" => slack_connect(&state).await?,
            "notion" => notion_connect(&state).await?,
            "dropbox" => dropbox_connect(&state).await?,
            "mcp" => {
                mcp_rpc(&state, "tools/list", json!({}), None).await?;
                let mut record = load_record(&state, "mcp")?
                    .ok_or_else(|| connector_error("mcp", "server is not configured"))?;
                record.account_label = Some(
                    config_str(&record.config, "name")
                        .unwrap_or("MCP server")
                        .to_string(),
                );
                save_record(&state, "mcp", &record)?;
            }
            _ => unreachable!(),
        }
    }
    integration_status(provider, state)
}

#[tauri::command]
pub async fn disconnect_integration(
    provider: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    ensure_provider(&provider)?;
    if let Some(token) = secret_store::get_secret(&access_slot(&provider))? {
        match provider.as_str() {
            "slack" => {
                let _ = state
                    .http
                    .post("https://slack.com/api/auth.revoke")
                    .bearer_auth(&token)
                    .timeout(Duration::from_secs(10))
                    .send()
                    .await;
            }
            "dropbox" => {
                let _ = state
                    .http
                    .post("https://api.dropboxapi.com/2/auth/token/revoke")
                    .bearer_auth(&token)
                    .timeout(Duration::from_secs(10))
                    .send()
                    .await;
            }
            _ => {}
        }
    }
    secret_store::delete_secret(&access_slot(&provider))?;
    secret_store::delete_secret(&refresh_slot(&provider))?;
    if provider == "slack" {
        secret_store::delete_secret(slack_user_slot())?;
    }
    if let Some(mut record) = load_record(&state, &provider)? {
        record.account_label = None;
        record.expires_at = None;
        record.scopes.clear();
        save_record(&state, &provider, &record)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn execute_integration_action(
    provider: String,
    action: String,
    params: Value,
    approved: bool,
    state: State<'_, AppState>,
) -> Result<Value, AppError> {
    ensure_provider(&provider)?;
    if is_mutating(&provider, &action) {
        require_approval(&provider, &action, approved)?;
    }
    match provider.as_str() {
        "microsoft" => execute_microsoft(&state, &action, &params).await,
        "slack" => execute_slack(&state, &action, &params).await,
        "notion" => execute_notion(&state, &action, &params).await,
        "dropbox" => execute_dropbox(&state, &action, &params).await,
        "mcp" => execute_mcp(&state, &action, &params).await,
        _ => unreachable!(),
    }
}

async fn execute_microsoft(
    state: &State<'_, AppState>,
    action: &str,
    params: &Value,
) -> Result<Value, AppError> {
    match action {
        "account.get" => {
            microsoft_request(
                state,
                Method::GET,
                "/me?$select=id,displayName,mail,userPrincipalName",
                None,
            )
            .await
        }
        "mail.list" => {
            let top = params
                .get("top")
                .and_then(Value::as_u64)
                .unwrap_or(25)
                .clamp(1, 100);
            let path = format!("/me/messages?$top={top}&$select=id,subject,from,toRecipients,receivedDateTime,isRead,bodyPreview,conversationId&$orderby=receivedDateTime%20desc");
            microsoft_request(state, Method::GET, &path, None).await
        }
        "mail.get" => {
            let id = required_str("microsoft", params, "messageId")?;
            microsoft_request(state, Method::GET, &format!("/me/messages/{id}"), None).await
        }
        "mail.send" => {
            let to = required_str("microsoft", params, "to")?;
            let subject = required_str("microsoft", params, "subject")?;
            let body = required_str("microsoft", params, "body")?;
            microsoft_request(
                state,
                Method::POST,
                "/me/sendMail",
                Some(json!({
                    "message": {
                        "subject": subject,
                        "body": {"contentType": "Text", "content": body},
                        "toRecipients": [{"emailAddress": {"address": to}}]
                    },
                    "saveToSentItems": true
                })),
            )
            .await
        }
        "mail.reply" => {
            let id = required_str("microsoft", params, "messageId")?;
            let body = required_str("microsoft", params, "body")?;
            microsoft_request(
                state,
                Method::POST,
                &format!("/me/messages/{id}/reply"),
                Some(json!({"comment": body})),
            )
            .await
        }
        "mail.delete" => {
            let id = required_str("microsoft", params, "messageId")?;
            microsoft_request(state, Method::DELETE, &format!("/me/messages/{id}"), None).await
        }
        "drive.list" => {
            if let Some(query) = optional_str(params, "query") {
                let escaped = query.replace('\'', "''");
                microsoft_request(
                    state,
                    Method::GET,
                    &format!("/me/drive/root/search(q='{escaped}')?$top=100"),
                    None,
                )
                .await
            } else {
                microsoft_request(state, Method::GET, "/me/drive/root/children?$top=100", None)
                    .await
            }
        }
        "drive.get" => {
            let id = required_str("microsoft", params, "itemId")?;
            microsoft_request(state, Method::GET, &format!("/me/drive/items/{id}"), None).await
        }
        "drive.download" => {
            let id = required_str("microsoft", params, "itemId")?;
            microsoft_binary(
                state,
                Method::GET,
                &format!("/me/drive/items/{id}/content"),
                None,
            )
            .await
        }
        "drive.upload" => {
            let path = encode_graph_path(required_str("microsoft", params, "path")?);
            let bytes = if let Some(encoded) = optional_str(params, "contentBase64") {
                STANDARD.decode(encoded).map_err(|error| {
                    connector_error("microsoft", format!("invalid contentBase64: {error}"))
                })?
            } else {
                optional_str(params, "content")
                    .unwrap_or_default()
                    .as_bytes()
                    .to_vec()
            };
            if bytes.len() > MAX_BINARY_BYTES {
                return Err(connector_error(
                    "microsoft",
                    "interactive upload exceeds the 8 MB safety limit",
                ));
            }
            let token = access_token(state, "microsoft").await?;
            let response = state
                .http
                .put(format!(
                    "https://graph.microsoft.com/v1.0/me/drive/root:/{path}:/content"
                ))
                .bearer_auth(token)
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
                .body(bytes)
                .send()
                .await
                .map_err(|error| connector_error("microsoft", error.to_string()))?;
            json_response("microsoft", response).await
        }
        "drive.delete" => {
            let id = required_str("microsoft", params, "itemId")?;
            microsoft_request(
                state,
                Method::DELETE,
                &format!("/me/drive/items/{id}"),
                None,
            )
            .await
        }
        "calendar.events" => {
            let start = required_str("microsoft", params, "startDateTime")?;
            let end = required_str("microsoft", params, "endDateTime")?;
            let mut url = Url::parse("https://graph.microsoft.com/v1.0/me/calendarView")
                .map_err(|error| connector_error("microsoft", error.to_string()))?;
            url.query_pairs_mut()
                .append_pair("startDateTime", start)
                .append_pair("endDateTime", end)
                .append_pair("$top", "100");
            let token = access_token(state, "microsoft").await?;
            let response = state
                .http
                .get(url)
                .bearer_auth(token)
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
                .send()
                .await
                .map_err(|error| connector_error("microsoft", error.to_string()))?;
            json_response("microsoft", response).await
        }
        "calendar.create" => {
            let event = params.get("event").cloned().ok_or_else(|| {
                connector_error("microsoft", "missing required parameter 'event'")
            })?;
            microsoft_request(state, Method::POST, "/me/events", Some(event)).await
        }
        "calendar.update" => {
            let id = required_str("microsoft", params, "eventId")?;
            let event = params.get("event").cloned().ok_or_else(|| {
                connector_error("microsoft", "missing required parameter 'event'")
            })?;
            microsoft_request(
                state,
                Method::PATCH,
                &format!("/me/events/{id}"),
                Some(event),
            )
            .await
        }
        "calendar.delete" => {
            let id = required_str("microsoft", params, "eventId")?;
            microsoft_request(state, Method::DELETE, &format!("/me/events/{id}"), None).await
        }
        "contacts.list" | "contacts.search" => {
            let top = params
                .get("top")
                .and_then(Value::as_u64)
                .unwrap_or(100)
                .clamp(1, 200);
            let mut value = microsoft_request(
                state,
                Method::GET,
                &format!("/me/contacts?$top={top}&$select=id,displayName,emailAddresses,mobilePhone,businessPhones,companyName"),
                None,
            )
            .await?;
            if action == "contacts.search" {
                let query = required_str("microsoft", params, "query")?.to_ascii_lowercase();
                if let Some(items) = value.get_mut("value").and_then(Value::as_array_mut) {
                    items.retain(|item| item.to_string().to_ascii_lowercase().contains(&query));
                }
            }
            Ok(value)
        }
        _ => Err(connector_error(
            "microsoft",
            format!("unsupported action '{action}'"),
        )),
    }
}

async fn execute_slack(
    state: &State<'_, AppState>,
    action: &str,
    params: &Value,
) -> Result<Value, AppError> {
    match action {
        "account.get" => slack_request(state, "auth.test", json!({}), false).await,
        "channels.list" => slack_request(
            state,
            "conversations.list",
            json!({
                "limit": params.get("limit").and_then(Value::as_u64).unwrap_or(100).clamp(1, 200),
                "types": optional_str(params, "types").unwrap_or("public_channel,private_channel,im,mpim")
            }),
            false,
        )
        .await,
        "channels.history" => slack_request(
            state,
            "conversations.history",
            json!({
                "channel": required_str("slack", params, "channel")?,
                "limit": params.get("limit").and_then(Value::as_u64).unwrap_or(50).clamp(1, 100)
            }),
            false,
        )
        .await,
        "channels.replies" => slack_request(
            state,
            "conversations.replies",
            json!({
                "channel": required_str("slack", params, "channel")?,
                "ts": required_str("slack", params, "ts")?,
                "limit": params.get("limit").and_then(Value::as_u64).unwrap_or(50).clamp(1, 100)
            }),
            false,
        )
        .await,
        "search.messages" => slack_request(
            state,
            "search.messages",
            json!({
                "query": required_str("slack", params, "query")?,
                "count": params.get("count").and_then(Value::as_u64).unwrap_or(50).clamp(1, 100)
            }),
            true,
        )
        .await,
        "users.list" => slack_request(state, "users.list", json!({"limit": 200}), false).await,
        "chat.send" => {
    let mut payload = json!({
        "channel": required_str("slack", params, "channel")?,
        "text": required_str("slack", params, "text")?
    });
    if let Some(thread_ts) = optional_str(params, "threadTs") {
        payload["thread_ts"] = Value::String(thread_ts.to_string());
    }
    slack_request(state, "chat.postMessage", payload, false).await
}
        "chat.update" => slack_request(
            state,
            "chat.update",
            json!({
                "channel": required_str("slack", params, "channel")?,
                "ts": required_str("slack", params, "ts")?,
                "text": required_str("slack", params, "text")?
            }),
            false,
        )
        .await,
        "chat.delete" => slack_request(
            state,
            "chat.delete",
            json!({
                "channel": required_str("slack", params, "channel")?,
                "ts": required_str("slack", params, "ts")?
            }),
            false,
        )
        .await,
        "reactions.add" | "reactions.remove" => slack_request(
            state,
            if action == "reactions.add" { "reactions.add" } else { "reactions.remove" },
            json!({
                "channel": required_str("slack", params, "channel")?,
                "timestamp": required_str("slack", params, "timestamp")?,
                "name": required_str("slack", params, "name")?
            }),
            false,
        )
        .await,
        _ => Err(connector_error("slack", format!("unsupported action '{action}'"))),
    }
}

async fn execute_notion(
    state: &State<'_, AppState>,
    action: &str,
    params: &Value,
) -> Result<Value, AppError> {
    match action {
        "account.get" => notion_request(state, Method::GET, "/users/me", None).await,
        "search" => notion_request(
            state,
            Method::POST,
            "/search",
            Some(json!({
                "query": optional_str(params, "query").unwrap_or(""),
                "page_size": params.get("pageSize").and_then(Value::as_u64).unwrap_or(50).clamp(1, 100)
            })),
        )
        .await,
        "page.get" => {
            let id = required_str("notion", params, "pageId")?;
            notion_request(state, Method::GET, &format!("/pages/{id}"), None).await
        }
        "block.children" => {
            let id = required_str("notion", params, "blockId")?;
            let page_size = params.get("pageSize").and_then(Value::as_u64).unwrap_or(100).clamp(1, 100);
            notion_request(state, Method::GET, &format!("/blocks/{id}/children?page_size={page_size}"), None).await
        }
        "data_source.query" => {
            let id = required_str("notion", params, "dataSourceId")?;
            notion_request(
                state,
                Method::POST,
                &format!("/data_sources/{id}/query"),
                Some(params.get("query").cloned().unwrap_or_else(|| json!({}))),
            )
            .await
        }
        "page.create" => notion_request(
            state,
            Method::POST,
            "/pages",
            Some(params.get("page").cloned().ok_or_else(|| connector_error("notion", "missing required parameter 'page'"))?),
        )
        .await,
        "page.update" => {
            let id = required_str("notion", params, "pageId")?;
            let patch = params.get("patch").cloned().ok_or_else(|| connector_error("notion", "missing required parameter 'patch'"))?;
            notion_request(state, Method::PATCH, &format!("/pages/{id}"), Some(patch)).await
        }
        "block.append" => {
            let id = required_str("notion", params, "blockId")?;
            let children = params.get("children").cloned().ok_or_else(|| connector_error("notion", "missing required parameter 'children'"))?;
            notion_request(
                state,
                Method::PATCH,
                &format!("/blocks/{id}/children"),
                Some(json!({"children": children})),
            )
            .await
        }
        "comment.list" => {
            let block = required_str("notion", params, "blockId")?;
            let mut url = Url::parse("https://api.notion.com/v1/comments")
                .map_err(|error| connector_error("notion", error.to_string()))?;
            url.query_pairs_mut().append_pair("block_id", block);
            let token = access_token(state, "notion").await?;
            let response = state
                .http
                .get(url)
                .bearer_auth(token)
                .header("Notion-Version", NOTION_VERSION)
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
                .send()
                .await
                .map_err(|error| connector_error("notion", error.to_string()))?;
            json_response("notion", response).await
        }
        "comment.create" => notion_request(
            state,
            Method::POST,
            "/comments",
            Some(params.get("comment").cloned().ok_or_else(|| connector_error("notion", "missing required parameter 'comment'"))?),
        )
        .await,
        _ => Err(connector_error("notion", format!("unsupported action '{action}'"))),
    }
}

async fn execute_dropbox(
    state: &State<'_, AppState>,
    action: &str,
    params: &Value,
) -> Result<Value, AppError> {
    match action {
        "account.get" => dropbox_current_account(state).await,
        "files.list" => dropbox_rpc(
            state,
            "files/list_folder",
            json!({
                "path": optional_str(params, "path").unwrap_or(""),
                "recursive": params.get("recursive").and_then(Value::as_bool).unwrap_or(false),
                "limit": params.get("limit").and_then(Value::as_u64).unwrap_or(100).clamp(1, 2000)
            }),
        )
        .await,
        "files.search" => dropbox_rpc(
            state,
            "files/search_v2",
            json!({
                "query": required_str("dropbox", params, "query")?,
                "options": {"path": optional_str(params, "path").unwrap_or(""), "max_results": 100}
            }),
        )
        .await,
        "files.download" => dropbox_download(state, required_str("dropbox", params, "path")?).await,
        "files.upload" => dropbox_upload(state, params).await,
        "files.move" => {
            dropbox_rpc(
                state,
                "files/move_v2",
                json!({
                    "from_path": required_str("dropbox", params, "fromPath")?,
                    "to_path": required_str("dropbox", params, "toPath")?,
                    "autorename": false
                }),
            )
            .await
        }
        "files.delete" => {
            dropbox_rpc(
                state,
                "files/delete_v2",
                json!({"path": required_str("dropbox", params, "path")?}),
            )
            .await
        }
        _ => Err(connector_error(
            "dropbox",
            format!("unsupported action '{action}'"),
        )),
    }
}

async fn execute_mcp(
    state: &State<'_, AppState>,
    action: &str,
    params: &Value,
) -> Result<Value, AppError> {
    match action {
        "tools.list" => mcp_rpc(state, "tools/list", json!({}), None).await,
        "tools.call" => {
            let name = required_str("mcp", params, "name")?;
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            mcp_rpc(
                state,
                "tools/call",
                json!({"name": name, "arguments": arguments}),
                Some(name),
            )
            .await
        }
        "resources.list" => mcp_rpc(state, "resources/list", json!({}), None).await,
        "resources.read" => {
            let uri = required_str("mcp", params, "uri")?;
            mcp_rpc(state, "resources/read", json!({"uri": uri}), Some(uri)).await
        }
        "prompts.list" => mcp_rpc(state, "prompts/list", json!({}), None).await,
        "prompts.get" => {
            let name = required_str("mcp", params, "name")?;
            mcp_rpc(
                state,
                "prompts/get",
                json!({"name": name, "arguments": params.get("arguments").cloned().unwrap_or_else(|| json!({}))}),
                Some(name),
            )
            .await
        }
        _ => Err(connector_error(
            "mcp",
            format!("unsupported action '{action}'"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutating_actions_are_guarded() {
        assert!(is_mutating("microsoft", "mail.send"));
        assert!(is_mutating("slack", "chat.delete"));
        assert!(is_mutating("notion", "page.update"));
        assert!(is_mutating("dropbox", "files.delete"));
        assert!(is_mutating("mcp", "tools.call"));
        assert!(!is_mutating("microsoft", "mail.list"));
        assert!(!is_mutating("mcp", "tools.list"));
    }

    #[test]
    fn oauth_redirects_are_loopback_only() {
        assert!(validate_redirect("slack", "http://localhost:17895/oauth/slack").is_ok());
        assert!(validate_redirect("notion", "http://127.0.0.1:17896/oauth/notion").is_ok());
        assert!(validate_redirect("dropbox", "https://example.com/callback").is_err());
        assert!(validate_redirect("microsoft", "http://localhost/oauth").is_err());
    }

    #[test]
    fn mcp_remote_http_is_rejected() {
        assert!(validate_mcp_endpoint("https://mcp.example.com/api").is_ok());
        assert!(validate_mcp_endpoint("http://localhost:3000/mcp").is_ok());
        assert!(validate_mcp_endpoint("http://example.com/mcp").is_err());
    }

    #[test]
    fn graph_paths_encode_segments() {
        assert_eq!(encode_graph_path("Docs/My File.txt"), "Docs/My%20File.txt");
    }
}
