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
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, RwLock},
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use reqwest::{header, Client, ClientBuilder, Proxy, RequestBuilder, Response, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{
    fs as async_fs,
    io::{AsyncReadExt, AsyncWriteExt},
};
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

// The sidecar is only a resume hint. Missing, interrupted or unrecognized
// metadata causes a full transfer, never an unvalidated append. Hash the source
// URL so signed URLs and credentials are not persisted in the sidecar.
#[derive(Debug, Serialize, Deserialize)]
struct ResumeMetadata {
    source: String,
    validator: ResumeValidator,
    total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum ResumeValidator {
    ETag(String),
    LastModified(String),
}

impl ResumeValidator {
    fn value(&self) -> &str {
        match self {
            Self::ETag(value) | Self::LastModified(value) => value,
        }
    }

    fn from_headers(headers: &header::HeaderMap) -> Option<Self> {
        if let Some(tag) = headers
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok())
        {
            if tag.len() <= 1024 && tag.starts_with('"') && tag.ends_with('"') {
                return Some(Self::ETag(tag.to_string()));
            }
        }
        // RFC 9110: a date validator must be strong enough for If-Range.
        // Require the origin Date to be at least 60 seconds newer than the
        // modification date; otherwise fall back to a complete transfer.
        let modified = headers.get(header::LAST_MODIFIED)?.to_str().ok()?;
        let date = headers.get(header::DATE)?.to_str().ok()?;
        let age = chrono::DateTime::parse_from_rfc2822(date).ok()?
            - chrono::DateTime::parse_from_rfc2822(modified).ok()?;
        (age.num_seconds() >= 60).then(|| Self::LastModified(modified.to_string()))
    }

    fn matches(&self, headers: &header::HeaderMap) -> bool {
        let name = match self {
            Self::ETag(_) => header::ETAG,
            Self::LastModified(_) => header::LAST_MODIFIED,
        };
        headers.get(name).and_then(|value| value.to_str().ok()) == Some(self.value())
    }
}

fn resume_metadata_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".resume.json");
    PathBuf::from(name)
}

async fn read_resume_metadata(path: &Path, url: &str) -> Option<ResumeMetadata> {
    let file = async_fs::File::open(resume_metadata_path(path))
        .await
        .ok()?;
    let mut bytes = Vec::new();
    file.take(8193).read_to_end(&mut bytes).await.ok()?;
    if bytes.len() > 8192 {
        return None;
    }
    let metadata: ResumeMetadata = serde_json::from_slice(&bytes).ok()?;
    // Revalidate the serialized header before passing it back to reqwest.
    if metadata.source != format!("{:x}", Sha256::digest(url.as_bytes()))
        || header::HeaderValue::from_str(metadata.validator.value()).is_err()
    {
        return None;
    }
    Some(metadata)
}

// Return (last byte, total size); all arithmetic is checked before writing.
fn validated_content_range(
    headers: &header::HeaderMap,
    start: u64,
    expected_total: Option<u64>,
) -> Option<(u64, u64)> {
    let value = headers.get(header::CONTENT_RANGE)?.to_str().ok()?;
    let (span, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (first, last) = span.split_once('-')?;
    let number = |value: &str| {
        (!value.is_empty() && value.bytes().all(|ch| ch.is_ascii_digit()))
            .then(|| value.parse::<u64>().ok())
            .flatten()
    };
    let (first, last, total) = (number(first)?, number(last)?, number(total)?);
    if first != start
        || last < first
        || last >= total
        || expected_total.is_some_and(|expected| expected != total)
    {
        return None;
    }
    let length = last.checked_sub(first)?.checked_add(1)?;
    if headers
        .get(header::CONTENT_LENGTH)
        .is_some_and(|value| value.to_str().ok().and_then(number) != Some(length))
    {
        return None;
    }
    Some((last, total))
}

/// Downloads `url` into `path`, appending to bytes already there with an HTTP
/// range request only when a saved representation validator matches. Legacy or
/// unverifiable partials restart from zero. Dropped connections and transient errors are
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
            }
        }

        let metadata = if existing > 0 {
            read_resume_metadata(path, url).await
        } else {
            None
        };
        let can_resume = metadata.as_ref().is_some_and(|metadata| {
            !metadata.total.is_some_and(|total| existing >= total)
                && !expected_size
                    .zip(metadata.total)
                    .is_some_and(|(expected, total)| expected != total)
        });
        if existing > 0 && !can_resume {
            tracing::warn!("partial download has no usable representation validator; restarting");
            existing = 0;
        }
        let mut request = client.get(url).header(header::ACCEPT_ENCODING, "identity");
        if existing > 0 {
            request = request
                .header(header::RANGE, format!("bytes={existing}-"))
                .header(
                    header::IF_RANGE,
                    metadata.as_ref().unwrap().validator.value(),
                );
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

        if response
            .headers()
            .get(header::CONTENT_ENCODING)
            .is_some_and(|value| value != "identity")
        {
            return Err(NetError::Size(
                "encoded download cannot be checked by byte range".into(),
            ));
        }
        let resumed = existing > 0 && status == StatusCode::PARTIAL_CONTENT;
        let (response_end, total) = if status == StatusCode::PARTIAL_CONTENT {
            let prior = metadata.as_ref().filter(|_| resumed);
            let known_total = expected_size.or(prior.and_then(|metadata| metadata.total));
            let range = validated_content_range(response.headers(), existing, known_total);
            if !resumed || !prior.unwrap().validator.matches(response.headers()) || range.is_none()
            {
                // Do not alter the partial file for an invalid response. The
                // caller can retry later; no corrupt result can be installed.
                return Err(NetError::Size(
                    "invalid Content-Range or changed download validator".into(),
                ));
            }
            let (last, total) = range.unwrap();
            (Some(last + 1), Some(total))
        } else if status == StatusCode::OK {
            existing = 0; // A server ignoring Range/If-Range must replace, never append.
            let length = response.content_length();
            if expected_size
                .zip(length)
                .is_some_and(|(expected, actual)| expected != actual)
            {
                return Err(NetError::Size(
                    "download Content-Length differs from expected size".into(),
                ));
            }
            (length, expected_size.or(length))
        } else {
            return Err(NetError::Status(status));
        };
        let mut file = async_fs::OpenOptions::new()
            .create(true)
            .append(resumed)
            .write(true)
            .truncate(!resumed)
            .open(path)
            .await?;
        {
            let sidecar = resume_metadata_path(path);
            let validator = if resumed {
                metadata.as_ref().map(|metadata| metadata.validator.clone())
            } else {
                ResumeValidator::from_headers(response.headers())
            };
            if let Some(validator) = validator {
                let metadata = ResumeMetadata {
                    source: format!("{:x}", Sha256::digest(url.as_bytes())),
                    validator,
                    total,
                };
                // An interrupted JSON write is safe: parsing fails and the next
                // attempt restarts. Persist before writing any new body bytes.
                async_fs::write(
                    sidecar,
                    serde_json::to_vec(&metadata)
                        .map_err(|error| NetError::Size(error.to_string()))?,
                )
                .await?;
            } else {
                match async_fs::remove_file(sidecar).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
                tracing::warn!("download has no strong validator; interrupted transfers will restart; checksums remain the caller's responsibility");
            }
        }
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
            let next_size = downloaded
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| NetError::Size("download size overflow".into()))?;
            if response_end.is_some_and(|end| next_size > end)
                || total.is_some_and(|total| next_size > total)
            {
                return Err(NetError::Size(
                    "download body exceeded its declared byte range or size".into(),
                ));
            }
            file.write_all(&chunk).await?;
            downloaded = next_size;
            session_bytes += chunk.len() as u64;
            on_progress(report(downloaded, session_bytes));
        }
        file.flush().await?;
        drop(file);

        let incomplete = total.is_some_and(|total| downloaded < total)
            || response_end.is_some_and(|end| downloaded < end);
        if stream_error.is_none() && !incomplete {
            async_fs::remove_file(resume_metadata_path(path)).await.ok();
            return Ok(downloaded);
        }
        // A connection that delivered new bytes starts a fresh run of attempts.
        failures = if resumed && downloaded > existing {
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
    let url = Url::parse(&with_scheme).map_err(|_| "Invalid proxy address".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!(
            "Proxy scheme \"{}\" is not supported; use an http:// or https:// proxy",
            url.scheme()
        ));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err("Proxy address has no host".to_string());
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
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\netag: \"v1\"\r\n\r\n",
                        served.len()
                    );
                    socket.write_all(header.as_bytes()).await.unwrap();
                    socket.write_all(&served[..2048]).await.unwrap();
                } else {
                    assert!(request.contains("range: bytes=2048-"), "{request}");
                    assert!(request.contains("if-range: \"v1\""), "{request}");
                    let rest = &served[2048..];
                    let header = format!(
                        "HTTP/1.1 206 Partial Content\r\ncontent-length: {}\r\ncontent-range: bytes 2048-4095/4096\r\netag: \"v1\"\r\n\r\n",
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
    async fn resume_fixture(
        replies: Vec<String>,
        seed: &[u8],
        validator: Option<ResumeValidator>,
        expected: Option<u64>,
    ) -> (Result<u64, NetError>, Vec<u8>, Vec<String>) {
        use tokio::{io::AsyncReadExt, net::TcpListener};
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for reply in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut bytes = [0; 1024];
                    let n = socket.read(&mut bytes).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&bytes[..n]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                requests.push(String::from_utf8(request).unwrap().to_ascii_lowercase());
                socket.write_all(reply.as_bytes()).await.unwrap();
            }
            requests
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.part");
        std::fs::write(&path, seed).unwrap();
        if let Some(validator) = validator {
            std::fs::write(
                resume_metadata_path(&path),
                serde_json::to_vec(&ResumeMetadata {
                    source: format!("{:x}", Sha256::digest(url.as_bytes())),
                    validator,
                    total: Some(10),
                })
                .unwrap(),
            )
            .unwrap();
        }
        let result = download_resumable(
            &Client::builder().no_proxy().build().unwrap(),
            &url,
            &path,
            expected,
            None,
            |_| {},
        )
        .await;
        if result.is_ok() {
            assert!(!resume_metadata_path(&path).exists());
        }
        let requests = tokio::time::timeout(Duration::from_secs(10), server)
            .await
            .unwrap()
            .unwrap();
        (result, std::fs::read(path).unwrap_or_default(), requests)
    }

    fn v1() -> Option<ResumeValidator> {
        Some(ResumeValidator::ETag("\"v1\"".into()))
    }

    #[tokio::test]
    async fn wrong_range_or_validator_never_changes_the_partial_file() {
        for (range, length, tag) in [
            ("bytes 3-8/10", "6", "\"v1\""),
            ("bytes 4-10/10", "6", "\"v1\""),
            ("bytes 4-3/10", "6", "\"v1\""),
            ("bytes 4-9/11", "6", "\"v1\""),
            ("bytes 4-9/*", "6", "\"v1\""),
            ("bytes 4-9/10", "5", "\"v1\""),
            ("items 4-9/10", "6", "\"v1\""),
            ("bytes +4-9/10", "6", "\"v1\""),
            ("bytes 4-9/10", "6", "\"v2\""),
            ("bytes 4-9/10", "6", "W/\"v1\""),
            ("bytes 4-9/10", "6", ""),
            ("", "6", "\"v1\""),
        ] {
            let reply = format!("HTTP/1.1 206 Partial Content\r\nContent-Range: {range}\r\nContent-Length: {length}\r\nETag: {tag}\r\nConnection: close\r\n\r\nEFGHIJ");
            let (result, bytes, requests) =
                resume_fixture(vec![reply], b"abcd", v1(), Some(10)).await;
            assert!(
                matches!(result, Err(NetError::Size(_))),
                "{range}, {tag}: {result:?}"
            );
            assert_eq!(bytes, b"abcd");
            assert!(requests[0].contains("if-range: \"v1\""));
        }
    }

    #[tokio::test]
    async fn persisted_validator_resumes_unknown_size_and_tracks_short_ranges() {
        let (result, bytes, requests) = resume_fixture(vec![
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 4-6/10\r\nContent-Length: 3\r\nETag: \"v1\"\r\nConnection: close\r\n\r\nEFG".into(),
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 7-9/10\r\nContent-Length: 3\r\nETag: \"v1\"\r\nConnection: close\r\n\r\nHIJ".into(),
        ], b"abcd", v1(), None).await;
        assert_eq!(result.unwrap(), 10);
        assert_eq!(bytes, b"abcdEFGHIJ");
        assert!(requests[1].contains("range: bytes=7-"));
    }

    #[tokio::test]
    async fn changed_representation_or_ignored_range_replaces_instead_of_appending() {
        let reply = "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nETag: \"v2\"\r\nConnection: close\r\n\r\n0123456789";
        let (result, bytes, requests) =
            resume_fixture(vec![reply.into()], b"abcd", v1(), Some(10)).await;
        assert_eq!(result.unwrap(), 10);
        assert_eq!(bytes, b"0123456789");
        assert!(requests[0].contains("range: bytes=4-"));
    }

    #[tokio::test]
    async fn legacy_partial_and_size_only_complete_file_require_a_full_response() {
        for seed in [b"abcd".as_slice(), b"badbadbad!".as_slice()] {
            let reply =
                "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\n0123456789";
            let (result, bytes, requests) =
                resume_fixture(vec![reply.into()], seed, None, Some(10)).await;
            assert_eq!(result.unwrap(), 10);
            assert_eq!(bytes, b"0123456789");
            assert!(!requests[0].contains("range:"));
            assert!(requests[0].contains("accept-encoding: identity"));
        }
    }

    #[tokio::test]
    async fn weak_validator_restarts_an_interrupted_transfer_without_mixing_versions() {
        let (result, bytes, requests) = resume_fixture(vec![
            "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nETag: W/\"v1\"\r\nConnection: close\r\n\r\nabcd".into(),
            "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nETag: \"v2\"\r\nConnection: close\r\n\r\n0123456789".into(),
        ], b"", None, Some(10)).await;
        assert_eq!(result.unwrap(), 10);
        assert_eq!(bytes, b"0123456789");
        assert!(!requests[1].contains("range:"));
    }

    #[tokio::test]
    async fn overlong_chunked_range_is_rejected_before_it_can_be_appended() {
        let reply = "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 4-9/10\r\nTransfer-Encoding: chunked\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n7\r\nEFGHIJK\r\n0\r\n\r\n";
        let (result, bytes, _) = resume_fixture(vec![reply.into()], b"abcd", v1(), Some(10)).await;
        assert!(matches!(result, Err(NetError::Size(_))));
        assert_eq!(bytes, b"abcd");
    }

    #[test]
    fn last_modified_is_only_usable_with_a_strong_date_validator() {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::LAST_MODIFIED,
            "Sun, 06 Nov 1994 08:49:37 GMT".parse().unwrap(),
        );
        headers.insert(
            header::DATE,
            "Sun, 06 Nov 1994 08:49:38 GMT".parse().unwrap(),
        );
        assert!(ResumeValidator::from_headers(&headers).is_none());
        headers.insert(
            header::DATE,
            "Sun, 06 Nov 1994 08:50:37 GMT".parse().unwrap(),
        );
        assert!(matches!(
            ResumeValidator::from_headers(&headers),
            Some(ResumeValidator::LastModified(_))
        ));
    }
    #[tokio::test]
    async fn last_modified_resume_and_416_restart_are_safe() {
        let modified = "Sun, 06 Nov 1994 08:49:37 GMT";
        let (result, bytes, requests) = resume_fixture(vec![format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 4-9/10\r\nContent-Length: 6\r\nLast-Modified: {modified}\r\nConnection: close\r\n\r\nEFGHIJ"
        )], b"abcd", Some(ResumeValidator::LastModified(modified.into())), Some(10)).await;
        assert_eq!(result.unwrap(), 10);
        assert_eq!(bytes, b"abcdEFGHIJ");
        assert!(requests[0].contains("if-range: sun, 06 nov 1994 08:49:37 gmt"));
        let (result, bytes, requests) = resume_fixture(vec![
            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
            "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\n0123456789".into(),
        ], b"abcd", v1(), Some(10)).await;
        assert_eq!(result.unwrap(), 10);
        assert_eq!(bytes, b"0123456789");
        assert!(!requests[1].contains("range:"));
    }

    #[tokio::test]
    async fn bad_or_oversized_resume_metadata_is_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.part");
        for bytes in [
            b"{interrupted".to_vec(),
            vec![b' '; 8193],
            serde_json::to_vec(&ResumeMetadata {
                source: "wrong source".into(),
                validator: ResumeValidator::ETag("\"v1\"".into()),
                total: Some(10),
            })
            .unwrap(),
        ] {
            std::fs::write(resume_metadata_path(&path), bytes).unwrap();
            assert!(read_resume_metadata(&path, "http://localhost/file")
                .await
                .is_none());
        }
    }
}
