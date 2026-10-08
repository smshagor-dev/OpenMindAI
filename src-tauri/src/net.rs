//! Network access for everything OpenMindAI fetches from the internet.
//!
//! Every download (AI models and their package dependencies, datasets, the
//! llama.cpp and stable-diffusion.cpp runtimes from GitHub) and every remote API
//! call goes through clients built here, so all of them follow one proxy policy:
//!
//! - `direct` (default): OpenMindAI connects on its own and ignores the PC's
//!   proxy configuration (environment variables, Windows proxy). Machines whose
//!   proxy blocks GitHub still get their llama.cpp runtime this way.
//! - `manual`: the proxy URL saved in Settings.
//! - `system`: the PC's proxy, `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY` when
//!   set, otherwise the Windows Internet Options proxy that browsers use.
//!
//! Loopback and private-network hosts always bypass the proxy, so the local
//! llama-server and other on-device endpoints are never routed through it.
//!
//! Downloads go through [`download_resumable`], which resumes with HTTP range
//! requests and retries transient network failures instead of failing the
//! whole install on one dropped connection.

use std::{
    fmt,
    net::IpAddr,
    path::Path,
    sync::{Mutex, OnceLock, RwLock},
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use reqwest::{header, Client, ClientBuilder, Proxy, RequestBuilder, Response, StatusCode};
use tokio::{fs as async_fs, io::AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::app_error::AppError;

const USER_AGENT: &str = "OpenMindAI-Desktop";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// A download stream that delivers nothing for this long is treated as a
/// dropped connection and resumed.
const DOWNLOAD_READ_TIMEOUT: Duration = Duration::from_secs(90);
/// Consecutive failed attempts without any new bytes before a download gives up.
const MAX_ATTEMPTS: u32 = 6;
const SYSTEM_PROXY_CACHE_TTL: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProxyMode {
    Direct,
    Manual,
    System,
}

impl ProxyMode {
    fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "manual" => Self::Manual,
            "system" => Self::System,
            _ => Self::Direct,
        }
    }
}

#[derive(Debug, Clone)]
struct ProxyPolicy {
    mode: ProxyMode,
    manual: Option<Url>,
}

static POLICY: RwLock<ProxyPolicy> = RwLock::new(ProxyPolicy {
    mode: ProxyMode::Direct,
    manual: None,
});

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SystemProxy {
    http: Option<Url>,
    https: Option<Url>,
    bypass: Vec<String>,
}

static SYSTEM_PROXY_CACHE: Mutex<Option<(Instant, SystemProxy)>> = Mutex::new(None);

/// Applies the saved proxy preferences (`mode` is "direct", "manual" or "system").
/// Called at startup and whenever preferences are saved; clients pick the
/// change up on their next request.
pub fn apply_proxy_settings(mode: &str, manual_url: &str) {
    let mode = ProxyMode::parse(mode);
    // The app updater runs its own HTTP client that reads the proxy environment
    // variables and cannot be told to ignore them, so direct mode removes them
    // from this process. The values captured at startup stay available for
    // `system` mode.
    for (name, value) in startup_proxy_env() {
        if mode == ProxyMode::Direct {
            std::env::remove_var(name);
        } else {
            std::env::set_var(name, value);
        }
    }
    let manual = match mode {
        ProxyMode::Manual => match normalize_proxy_url(manual_url) {
            Ok(url) => Some(url),
            Err(error) => {
                tracing::warn!(%error, "manual proxy is invalid; connecting directly");
                None
            }
        },
        _ => None,
    };
    if let Ok(mut policy) = POLICY.write() {
        *policy = ProxyPolicy { mode, manual };
    }
    if let Ok(mut cache) = SYSTEM_PROXY_CACHE.lock() {
        *cache = None;
    }
    tracing::info!(mode = ?mode, "network proxy policy applied");
}

/// Rejects a manual proxy setting that cannot be used, before it is saved.
pub fn validate_proxy_settings(mode: &str, manual_url: &str) -> Result<(), AppError> {
    if ProxyMode::parse(mode) == ProxyMode::Manual {
        normalize_proxy_url(manual_url).map_err(AppError::Internal)?;
    }
    Ok(())
}

/// The proxy OpenMindAI uses for `url`, or `None` for a direct connection.
/// Exposed to the frontend for the app updater, which runs its own HTTP client.
pub fn proxy_for_url(url: &str) -> Option<String> {
    let url = Url::parse(url).ok()?;
    proxy_for(&url).map(String::from)
}

fn proxy_for(url: &Url) -> Option<Url> {
    let host = url.host_str()?;
    if is_local_host(host) {
        return None;
    }
    let policy = POLICY.read().ok()?.clone();
    match policy.mode {
        ProxyMode::Direct => None,
        ProxyMode::Manual => policy.manual,
        ProxyMode::System => {
            let system = system_proxy();
            if system
                .bypass
                .iter()
                .any(|pattern| bypass_matches(host, pattern))
            {
                return None;
            }
            match url.scheme() {
                "https" => system.https,
                "http" => system.http,
                _ => None,
            }
        }
    }
}

/// Client for remote APIs and the shared app client (which also talks to the
/// local runtime, so it has no read timeout: the first token of a long prompt
/// can take minutes to arrive).
pub fn http_client() -> Client {
    with_proxy(Client::builder())
        .build()
        .expect("failed to build OpenMindAI HTTP client")
}

/// Shared client for file downloads.
pub fn download_client() -> Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            with_proxy(Client::builder())
                .read_timeout(DOWNLOAD_READ_TIMEOUT)
                .build()
                .expect("failed to build OpenMindAI download client")
        })
        .clone()
}

/// Adds the OpenMindAI proxy policy and defaults to a client builder.
pub fn with_proxy(builder: ClientBuilder) -> ClientBuilder {
    builder
        .proxy(Proxy::custom(proxy_for))
        .user_agent(USER_AGENT)
        .connect_timeout(CONNECT_TIMEOUT)
}

#[derive(Debug)]
pub enum NetError {
    Cancelled,
    Status(StatusCode),
    Transport(String),
    Io(std::io::Error),
    Size(String),
}

impl NetError {
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("download stopped"),
            Self::Status(status) => write!(f, "HTTP {status}"),
            Self::Transport(message) => write!(f, "network error: {message}"),
            Self::Io(error) => write!(f, "file error: {error}"),
            Self::Size(message) => f.write_str(message),
        }
    }
}

impl From<std::io::Error> for NetError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Sends a request, retrying connection failures and transient server errors
/// (408, 429, 5xx). Any other response is returned for the caller to inspect.
pub async fn send_with_retry(
    request: impl Fn() -> RequestBuilder,
    cancel: Option<&CancellationToken>,
) -> Result<Response, NetError> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        match request().send().await {
            Ok(response) if retryable_status(response.status()) && attempt < MAX_ATTEMPTS => {
                tracing::warn!(status = %response.status(), attempt, "transient HTTP status; retrying");
            }
            Ok(response) => return Ok(response),
            Err(error) if attempt < MAX_ATTEMPTS => {
                tracing::warn!(error = %error_chain(&error), attempt, "request failed; retrying");
            }
            Err(error) => return Err(NetError::Transport(error_chain(&error))),
        }
        backoff(attempt, cancel).await?;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub bytes_per_sec: f64,
}

/// Downloads `url` into `path`, appending to bytes already there with an HTTP
/// range request. Dropped connections, stalls and transient server errors are
/// resumed from the last byte written; it gives up after [`MAX_ATTEMPTS`]
/// consecutive attempts that make no progress. Returns the final file size.
/// Checksum verification and moving the file into place stay with the caller.
pub async fn download_resumable(
    client: &Client,
    url: &str,
    path: &Path,
    expected_size: Option<u64>,
    cancel: Option<&CancellationToken>,
    mut on_progress: impl FnMut(DownloadProgress),
) -> Result<u64, NetError> {
    let started = Instant::now();
    let mut session_bytes = 0_u64;
    let mut failures = 0_u32;
    let mut restarted_after_416 = false;

    loop {
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(NetError::Cancelled);
        }
        let mut existing = async_fs::metadata(path)
            .await
            .map(|meta| meta.len())
            .unwrap_or(0);
        if let Some(expected) = expected_size {
            if existing > expected {
                async_fs::remove_file(path).await.ok();
                existing = 0;
            } else if expected > 0 && existing == expected {
                return Ok(existing);
            }
        }

        let mut request = client.get(url);
        if existing > 0 {
            request = request.header(header::RANGE, format!("bytes={existing}-"));
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                failures += 1;
                if failures >= MAX_ATTEMPTS {
                    return Err(NetError::Transport(error_chain(&error)));
                }
                tracing::warn!(error = %error_chain(&error), attempt = failures, "download request failed; retrying");
                backoff(failures, cancel).await?;
                continue;
            }
        };

        let status = response.status();
        if existing > 0 && status == StatusCode::RANGE_NOT_SATISFIABLE && !restarted_after_416 {
            // The partial file no longer matches the remote file; start over once.
            restarted_after_416 = true;
            async_fs::remove_file(path).await.ok();
            continue;
        }
        if !status.is_success() {
            failures += 1;
            if retryable_status(status) && failures < MAX_ATTEMPTS {
                tracing::warn!(%status, attempt = failures, "transient download status; retrying");
                backoff(failures, cancel).await?;
                continue;
            }
            return Err(NetError::Status(status));
        }

        let resumed = existing > 0 && status == StatusCode::PARTIAL_CONTENT;
        if !resumed {
            existing = 0;
        }
        let mut file = async_fs::OpenOptions::new()
            .create(true)
            .append(resumed)
            .write(true)
            .truncate(!resumed)
            .open(path)
            .await?;
        let mut downloaded = existing;
        let report = |downloaded: u64, session_bytes: u64| DownloadProgress {
            downloaded,
            bytes_per_sec: session_bytes as f64 / started.elapsed().as_secs_f64().max(0.001),
        };
        on_progress(report(downloaded, session_bytes));

        let mut stream = response.bytes_stream();
        let mut stream_error = None;
        loop {
            let next = match cancel {
                Some(token) => tokio::select! {
                    _ = token.cancelled() => {
                        file.flush().await?;
                        return Err(NetError::Cancelled);
                    }
                    next = stream.next() => next,
                },
                None => stream.next().await,
            };
            let Some(chunk) = next else { break };
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    stream_error = Some(error_chain(&error));
                    break;
                }
            };
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;
            session_bytes += chunk.len() as u64;
            if let Some(expected) = expected_size {
                if downloaded > expected {
                    drop(file);
                    async_fs::remove_file(path).await.ok();
                    return Err(NetError::Size(format!(
                        "download exceeded the expected size of {expected} bytes"
                    )));
                }
            }
            on_progress(report(downloaded, session_bytes));
        }
        file.flush().await?;
        drop(file);

        let incomplete = expected_size.is_some_and(|expected| downloaded < expected);
        if stream_error.is_none() && !incomplete {
            return Ok(downloaded);
        }
        // A connection that delivered new bytes starts a fresh run of attempts.
        failures = if downloaded > existing {
            1
        } else {
            failures + 1
        };
        let reason = stream_error.unwrap_or_else(|| "connection closed early".to_string());
        if failures >= MAX_ATTEMPTS {
            return Err(NetError::Transport(reason));
        }
        tracing::warn!(
            error = %reason,
            downloaded,
            attempt = failures,
            "download interrupted; resuming"
        );
        backoff(failures, cancel).await?;
    }
}

fn retryable_status(status: StatusCode) -> bool {
    status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

async fn backoff(attempt: u32, cancel: Option<&CancellationToken>) -> Result<(), NetError> {
    let delay = Duration::from_secs((1_u64 << attempt.min(5)).min(20));
    match cancel {
        Some(token) => tokio::select! {
            _ = token.cancelled() => Err(NetError::Cancelled),
            _ = tokio::time::sleep(delay) => Ok(()),
        },
        None => {
            tokio::time::sleep(delay).await;
            Ok(())
        }
    }
}

/// reqwest's top-level message ("error sending request") hides the cause, such
/// as a refused proxy connection; include the whole source chain.
fn error_chain(error: &reqwest::Error) -> String {
    let mut message = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

fn normalize_proxy_url(value: &str) -> Result<Url, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("Manual proxy is selected but no proxy address is set".to_string());
    }
    let with_scheme = if value.contains("://") {
        value.to_string()
    } else {
        format!("http://{value}")
    };
    let url = Url::parse(&with_scheme)
        .map_err(|error| format!("Proxy address \"{value}\" is not valid: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!(
            "Proxy scheme \"{}\" is not supported; use an http:// or https:// proxy",
            url.scheme()
        ));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(format!("Proxy address \"{value}\" has no host"));
    }
    Ok(url)
}

fn is_local_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let lower = host.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") || lower.ends_with(".local") {
        return true;
    }
    match lower.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified()
        }
        Ok(IpAddr::V6(ip)) => {
            let first = ip.segments()[0];
            ip.is_loopback()
                || ip.is_unspecified()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || ip
                    .to_ipv4_mapped()
                    .is_some_and(|v4| v4.is_loopback() || v4.is_private() || v4.is_link_local())
        }
        Err(_) => false,
    }
}

/// Matches one `NO_PROXY` / Windows `ProxyOverride` entry: `*`, `<local>`
/// (hosts without a dot), `example.com` / `.example.com` / `*.example.com`
/// (the domain and its subdomains), or a `*` wildcard such as `10.*`.
fn bypass_matches(host: &str, pattern: &str) -> bool {
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    let pattern = pattern.trim().to_ascii_lowercase();
    if pattern.is_empty() {
        return false;
    }
    if pattern == "*" {
        return true;
    }
    if pattern == "<local>" {
        return !host.contains('.');
    }
    let domain = pattern
        .strip_prefix("*.")
        .or_else(|| pattern.strip_prefix('.'))
        .unwrap_or(&pattern);
    if !domain.contains('*') {
        return host == domain || host.ends_with(&format!(".{domain}"));
    }
    glob_match(domain, &host)
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let (first, rest) = parts.split_first().expect("split yields at least one part");
    let Some(mut remaining) = text.strip_prefix(first) else {
        return false;
    };
    let (last, middle) = rest.split_last().expect("pattern contains '*'");
    for part in middle {
        match remaining.find(part) {
            Some(index) => remaining = &remaining[index + part.len()..],
            None => return false,
        }
    }
    remaining.ends_with(last)
}

fn system_proxy() -> SystemProxy {
    if let Ok(cache) = SYSTEM_PROXY_CACHE.lock() {
        if let Some((read_at, proxy)) = cache.as_ref() {
            if read_at.elapsed() < SYSTEM_PROXY_CACHE_TTL {
                return proxy.clone();
            }
        }
    }
    let proxy = env_proxy().unwrap_or_else(os_proxy);
    if let Ok(mut cache) = SYSTEM_PROXY_CACHE.lock() {
        *cache = Some((Instant::now(), proxy.clone()));
    }
    proxy
}

const PROXY_ENV_VARS: [&str; 8] = [
    "ALL_PROXY",
    "all_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "NO_PROXY",
    "no_proxy",
];

/// Proxy environment variables as they were when OpenMindAI started.
fn startup_proxy_env() -> &'static [(&'static str, String)] {
    static SNAPSHOT: OnceLock<Vec<(&'static str, String)>> = OnceLock::new();
    SNAPSHOT.get_or_init(|| {
        PROXY_ENV_VARS
            .iter()
            .filter_map(|name| std::env::var(name).ok().map(|value| (*name, value)))
            .collect()
    })
}

fn env_proxy() -> Option<SystemProxy> {
    let var = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| {
                startup_proxy_env()
                    .iter()
                    .find(|(captured, _)| captured == name)
                    .map(|(_, value)| value.clone())
            })
            .filter(|value| !value.trim().is_empty())
    };
    let all = var(&["ALL_PROXY", "all_proxy"]);
    let http = var(&["HTTP_PROXY", "http_proxy"]).or_else(|| all.clone());
    let https = var(&["HTTPS_PROXY", "https_proxy"]).or(all);
    if http.is_none() && https.is_none() {
        return None;
    }
    let parse = |value: Option<String>| value.and_then(|value| normalize_proxy_url(&value).ok());
    Some(SystemProxy {
        http: parse(http),
        https: parse(https),
        bypass: var(&["NO_PROXY", "no_proxy"])
            .map(|value| split_list(&value, ','))
            .unwrap_or_default(),
    })
}

fn split_list(value: &str, separator: char) -> Vec<String> {
    value
        .split(separator)
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(target_os = "windows")]
fn os_proxy() -> SystemProxy {
    let Ok(settings) = windows_registry::CURRENT_USER
        .open(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
    else {
        return SystemProxy::default();
    };
    if settings.get_u32("ProxyEnable").unwrap_or(0) == 0 {
        return SystemProxy::default();
    }
    let server = settings.get_string("ProxyServer").unwrap_or_default();
    let bypass = settings.get_string("ProxyOverride").unwrap_or_default();
    parse_windows_proxy(&server, &bypass)
}

#[cfg(not(target_os = "windows"))]
fn os_proxy() -> SystemProxy {
    SystemProxy::default()
}

/// Parses Windows `ProxyServer` (`host:port`, or per protocol as
/// `http=host:port;https=host:port`) and `ProxyOverride` (`;`-separated).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn parse_windows_proxy(server: &str, bypass: &str) -> SystemProxy {
    let mut proxy = SystemProxy {
        bypass: split_list(bypass, ';'),
        ..SystemProxy::default()
    };
    let server = server.trim();
    if server.contains('=') {
        for entry in server.split(';') {
            let Some((protocol, address)) = entry.split_once('=') else {
                continue;
            };
            let url = normalize_proxy_url(address).ok();
            match protocol.trim().to_ascii_lowercase().as_str() {
                "http" => proxy.http = url,
                "https" => proxy.https = url,
                _ => {}
            }
        }
    } else if let Ok(url) = normalize_proxy_url(server) {
        proxy.http = Some(url.clone());
        proxy.https = Some(url);
    }
    proxy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_hosts_never_use_the_proxy() {
        for host in [
            "127.0.0.1",
            "localhost",
            "[::1]",
            "192.168.1.20",
            "10.0.0.5",
            "172.16.4.1",
            "169.254.1.1",
            "printer.local",
            "fd00::1",
        ] {
            assert!(is_local_host(host), "{host} should be local");
        }
        for host in ["github.com", "huggingface.co", "8.8.8.8", "172.32.0.1"] {
            assert!(!is_local_host(host), "{host} should be remote");
        }
    }

    #[test]
    fn parses_single_windows_proxy_server() {
        let proxy = parse_windows_proxy("127.0.0.1:10809", "localhost;127.*;<local>");
        assert_eq!(proxy.http.unwrap().as_str(), "http://127.0.0.1:10809/");
        assert_eq!(proxy.https.unwrap().as_str(), "http://127.0.0.1:10809/");
        assert_eq!(proxy.bypass, vec!["localhost", "127.*", "<local>"]);
    }

    #[test]
    fn parses_per_protocol_windows_proxy_server() {
        let proxy = parse_windows_proxy(
            "http=proxy.corp:8080;https=secure.corp:8443;socks=s:1080",
            "",
        );
        assert_eq!(proxy.http.unwrap().as_str(), "http://proxy.corp:8080/");
        assert_eq!(proxy.https.unwrap().as_str(), "http://secure.corp:8443/");
    }

    #[test]
    fn https_is_direct_when_windows_only_proxies_http() {
        let proxy = parse_windows_proxy("http=proxy.corp:8080", "");
        assert!(proxy.http.is_some());
        assert!(proxy.https.is_none());
    }

    #[test]
    fn bypass_patterns_match_like_windows_and_no_proxy() {
        assert!(bypass_matches("github.com", "github.com"));
        assert!(bypass_matches("api.github.com", "github.com"));
        assert!(bypass_matches("api.github.com", ".github.com"));
        assert!(bypass_matches("api.github.com", "*.github.com"));
        assert!(!bypass_matches("notgithub.com", "github.com"));
        assert!(bypass_matches("10.1.2.3", "10.*"));
        assert!(bypass_matches("intranet", "<local>"));
        assert!(!bypass_matches("huggingface.co", "<local>"));
        assert!(bypass_matches("anything.example", "*"));
        assert!(bypass_matches("build.corp.example", "build.*.example"));
    }

    #[test]
    fn manual_proxy_urls_are_normalized_and_validated() {
        assert_eq!(
            normalize_proxy_url(" proxy.corp:3128 ").unwrap().as_str(),
            "http://proxy.corp:3128/"
        );
        assert_eq!(
            normalize_proxy_url("http://user:pass@proxy.corp:3128")
                .unwrap()
                .username(),
            "user"
        );
        assert!(normalize_proxy_url("").is_err());
        assert!(normalize_proxy_url("socks5://127.0.0.1:1080").is_err());
    }

    /// The only test that changes the global policy; it restores "direct" at the end.
    #[tokio::test]
    async fn manual_proxy_carries_internet_requests_but_not_local_ones() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt as _},
            net::TcpListener,
        };

        let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_address = proxy.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = proxy.accept().await.unwrap();
            let mut request = vec![0_u8; 2048];
            let read = socket.read(&mut request).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
                .await
                .unwrap();
            String::from_utf8_lossy(&request[..read]).into_owned()
        });

        apply_proxy_settings("manual", &proxy_address.to_string());
        let expected_proxy = format!("http://{proxy_address}/");
        assert_eq!(
            proxy_for_url("https://github.com/ggml-org/llama.cpp/releases").as_deref(),
            Some(expected_proxy.as_str())
        );
        assert_eq!(proxy_for_url("http://127.0.0.1:8080/v1/models"), None);

        // A plain-http internet request must reach the proxy in absolute form.
        let body = download_client()
            .get("http://downloads.openmindai.invalid/file.bin")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let request = server.await.unwrap();
        assert_eq!(body, "ok");
        assert!(
            request.starts_with("GET http://downloads.openmindai.invalid/file.bin HTTP/1.1"),
            "{request}"
        );

        apply_proxy_settings("direct", "");
        assert_eq!(proxy_for_url("https://github.com/"), None);
    }

    #[tokio::test]
    async fn http_error_fails_the_download_without_retrying_forever() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt as _},
            net::TcpListener,
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 2048];
            let _ = socket.read(&mut request).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n")
                .await
                .unwrap();
        });

        let dir = std::env::temp_dir().join(format!("openmindai-net-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let error = download_resumable(
            &download_client(),
            &format!("http://{address}/missing.zip"),
            &dir.join("missing.part"),
            Some(10),
            None,
            |_| {},
        )
        .await
        .unwrap_err();
        server.await.unwrap();

        assert!(matches!(error, NetError::Status(status) if status == StatusCode::NOT_FOUND));
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn download_resumes_and_reports_progress() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt as _},
            net::TcpListener,
        };

        let body: Vec<u8> = (0..4096_u32).map(|value| (value % 251) as u8).collect();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let served = body.clone();
        let server = tokio::spawn(async move {
            // First connection: send half of the body, then drop it mid-stream.
            // Second connection: must arrive as a range request for the rest.
            for round in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0_u8; 2048];
                let read = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..read]).to_ascii_lowercase();
                if round == 0 {
                    let header = format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n",
                        served.len()
                    );
                    socket.write_all(header.as_bytes()).await.unwrap();
                    socket.write_all(&served[..2048]).await.unwrap();
                } else {
                    assert!(request.contains("range: bytes=2048-"), "{request}");
                    let rest = &served[2048..];
                    let header = format!(
                        "HTTP/1.1 206 Partial Content\r\ncontent-length: {}\r\ncontent-range: bytes 2048-4095/4096\r\n\r\n",
                        rest.len()
                    );
                    socket.write_all(header.as_bytes()).await.unwrap();
                    socket.write_all(rest).await.unwrap();
                }
            }
        });

        let dir = std::env::temp_dir().join(format!("openmindai-net-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("file.part");
        let mut last = None;
        let size = download_resumable(
            &download_client(),
            &format!("http://{address}/file"),
            &path,
            Some(body.len() as u64),
            None,
            |progress| last = Some(progress.downloaded),
        )
        .await
        .unwrap();
        server.await.unwrap();

        assert_eq!(size, body.len() as u64);
        assert_eq!(last, Some(body.len() as u64));
        assert_eq!(std::fs::read(&path).unwrap(), body);
        std::fs::remove_dir_all(dir).ok();
    }
}
