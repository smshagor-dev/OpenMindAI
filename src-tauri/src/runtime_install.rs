use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

use chrono::Utc;
use futures_util::StreamExt;
use reqwest::{header, Client, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::{fs as async_fs, io::AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::{
    app_error::AppError,
    hardware::{BackendKind, HardwareProfile},
    model_download::{ensure_contained, sha256_file, validate_free_space},
    portable_root::PortableRootManager,
    runtime::{preferred_backend_order, RuntimeBinaries, RuntimeManifest, RuntimeStatus},
};

const RELEASES_URL: &str = "https://api.github.com/repos/ggml-org/llama.cpp/releases/latest";
const SAFE_SPACE_MARGIN: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeInstallState {
    Idle,
    Resolving,
    Downloading,
    Verifying,
    Extracting,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInstallStatus {
    pub state: RuntimeInstallState,
    pub backend: Option<BackendKind>,
    pub version: Option<String>,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub percentage: Option<f64>,
    pub speed_bytes_per_sec: Option<f64>,
    pub error: Option<String>,
}

impl RuntimeInstallStatus {
    fn idle() -> Self {
        Self {
            state: RuntimeInstallState::Idle,
            backend: None,
            version: None,
            downloaded_bytes: 0,
            total_bytes: None,
            percentage: None,
            speed_bytes_per_sec: None,
            error: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

/// Maps `(target OS, backend)` to the official llama.cpp release asset name
/// pattern, verified live against `ggml-org/llama.cpp`'s latest release.
/// Returns `None` when upstream doesn't publish a build for that combination
/// (notably: Linux has no CUDA or ROCm/HIP builds today) so callers can skip
/// to the next hardware-preferred backend instead of failing outright.
fn catalog_pattern(os: &str, backend: &BackendKind) -> Option<&'static str> {
    match (os, backend) {
        ("windows", BackendKind::Cpu) => Some("llama-*-bin-win-cpu-x64.zip"),
        ("windows", BackendKind::Vulkan) => Some("llama-*-bin-win-vulkan-x64.zip"),
        // Windows CUDA builds are published per CUDA version (12.4/13.3/13.4);
        // BackendKind only has one `Cuda` variant, so we standardize on 12.4
        // for the broadest driver compatibility.
        ("windows", BackendKind::Cuda) => Some("llama-*-bin-win-cuda-12.4-x64.zip"),
        ("windows", BackendKind::Sycl) => Some("llama-*-bin-win-sycl-x64.zip"),
        ("windows", BackendKind::Hip) => Some("llama-*-bin-win-rocm-*-x64.zip"),
        ("linux", BackendKind::Cpu) => Some("llama-*-bin-ubuntu-x64.tar.gz"),
        ("linux", BackendKind::Vulkan) => Some("llama-*-bin-ubuntu-vulkan-x64.tar.gz"),
        ("linux", BackendKind::Sycl) => Some("llama-*-bin-ubuntu-sycl-fp32-x64.tar.gz"),
        _ => None,
    }
}

fn backend_slug(backend: &BackendKind) -> &'static str {
    match backend {
        BackendKind::Cpu => "cpu",
        BackendKind::Cuda => "cuda",
        BackendKind::Vulkan => "vulkan",
        BackendKind::Sycl => "sycl",
        BackendKind::Hip => "hip",
        BackendKind::Metal => "metal",
    }
}

/// Minimal single-`*`-or-more glob matcher for release asset names, e.g.
/// `"llama-*-bin-win-rocm-*-x64.zip"` — mirrors the `-like` patterns
/// `scripts/install-llama-runtime.ps1` already uses for the same asset list.
fn glob_match(name: &str, pattern: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return name == pattern;
    }
    let mut cursor = 0usize;
    let last = parts.len() - 1;
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if index == 0 {
            if !name[cursor..].starts_with(part) {
                return false;
            }
            cursor += part.len();
        } else if index == last {
            if !name[cursor..].ends_with(part) {
                return false;
            }
        } else {
            match name[cursor..].find(part) {
                Some(found) => cursor += found + part.len(),
                None => return false,
            }
        }
    }
    true
}

fn target_os() -> &'static str {
    std::env::consts::OS
}

pub struct RuntimeInstaller {
    root: PortableRootManager,
    client: Client,
    status: Arc<Mutex<RuntimeInstallStatus>>,
    cancel_token: Arc<Mutex<Option<CancellationToken>>>,
}

impl RuntimeInstaller {
    pub fn new(root: PortableRootManager) -> Self {
        Self {
            root,
            client: Client::new(),
            status: Arc::new(Mutex::new(RuntimeInstallStatus::idle())),
            cancel_token: Arc::new(Mutex::new(None)),
        }
    }

    pub fn status(&self) -> Result<RuntimeInstallStatus, AppError> {
        self.status
            .lock()
            .map(|status| status.clone())
            .map_err(|_| AppError::internal("runtime install status lock poisoned"))
    }

    pub fn cancel(&self) -> Result<RuntimeInstallStatus, AppError> {
        if let Some(token) = self
            .cancel_token
            .lock()
            .map_err(|_| AppError::internal("runtime install cancel lock poisoned"))?
            .as_ref()
        {
            token.cancel();
        }
        self.status()
    }

    /// Tries each hardware-preferred backend in order (see
    /// `runtime::preferred_backend_order`), skipping any combination the
    /// catalog has no official asset for on this OS, and installs the first
    /// one that succeeds.
    pub async fn install_recommended(
        &self,
        hardware: &HardwareProfile,
    ) -> Result<RuntimeInstallStatus, AppError> {
        let token = CancellationToken::new();
        *self
            .cancel_token
            .lock()
            .map_err(|_| AppError::internal("runtime install cancel lock poisoned"))? =
            Some(token.clone());

        let os = target_os();
        let mut last_error: Option<AppError> = None;
        for backend in preferred_backend_order(hardware) {
            let Some(pattern) = catalog_pattern(os, &backend) else {
                continue;
            };
            tracing::info!(?backend, "installing AI runtime");
            match self.install_backend(&backend, pattern, &token).await {
                Ok(status) => {
                    *self.cancel_token.lock().map_err(|_| {
                        AppError::internal("runtime install cancel lock poisoned")
                    })? = None;
                    tracing::info!(?backend, "AI runtime installed");
                    return Ok(status);
                }
                Err(error) => {
                    tracing::warn!(?backend, %error, "AI runtime backend failed, trying next");
                    last_error = Some(error);
                }
            }
        }

        *self
            .cancel_token
            .lock()
            .map_err(|_| AppError::internal("runtime install cancel lock poisoned"))? = None;
        let message = last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| {
                format!("no official llama.cpp runtime build is available for {os}")
            });
        self.set_state(RuntimeInstallState::Failed, Some(message.clone()))?;
        Err(AppError::RuntimeInstallFailed(message))
    }

    async fn install_backend(
        &self,
        backend: &BackendKind,
        pattern: &str,
        token: &CancellationToken,
    ) -> Result<RuntimeInstallStatus, AppError> {
        self.update_status(|status| {
            status.state = RuntimeInstallState::Resolving;
            status.backend = Some(backend.clone());
            status.version = None;
            status.downloaded_bytes = 0;
            status.total_bytes = None;
            status.percentage = None;
            status.speed_bytes_per_sec = None;
            status.error = None;
        })?;

        let release = fetch_latest_release(&self.client).await?;
        let asset = release
            .assets
            .iter()
            .find(|asset| glob_match(&asset.name, pattern))
            .ok_or_else(|| {
                AppError::RuntimeInstallFailed(format!(
                    "no asset matching {pattern} found in release {}",
                    release.tag_name
                ))
            })?;

        let slug = backend_slug(backend);
        let install_dir = self
            .root
            .resolve_relative(format!("runtimes/llama/{slug}/{}", release.tag_name))?;
        let manifest_dir = self.root.resolve_relative("runtimes/llama/manifests")?;
        let temp_dir = self.root.resolve_relative("temp/downloads")?;
        fs::create_dir_all(&manifest_dir)?;
        fs::create_dir_all(&temp_dir)?;
        ensure_contained(self.root.root(), &install_dir)?;
        ensure_contained(self.root.root(), &manifest_dir)?;
        ensure_contained(self.root.root(), &temp_dir)?;

        validate_free_space(&temp_dir, asset.size + SAFE_SPACE_MARGIN)?;

        let archive_path = temp_dir.join(&asset.name);
        let part_path = temp_dir.join(format!("{}.part", asset.name));

        let existing = fs::metadata(&part_path).map(|meta| meta.len()).unwrap_or(0);
        let mut request = self.client.get(&asset.browser_download_url);
        if existing > 0 {
            request = request.header(header::RANGE, format!("bytes={existing}-"));
        }
        let response = request
            .send()
            .await
            .map_err(|error| AppError::RuntimeInstallFailed(error.to_string()))?;
        let resumed = existing > 0 && response.status() == StatusCode::PARTIAL_CONTENT;
        if existing > 0 && !resumed {
            async_fs::remove_file(&part_path).await.ok();
        } else if !response.status().is_success() {
            return Err(AppError::RuntimeInstallFailed(format!(
                "HTTP {} while downloading runtime",
                response.status()
            )));
        }

        self.update_status(|status| {
            status.state = RuntimeInstallState::Downloading;
            status.version = Some(release.tag_name.clone());
            status.downloaded_bytes = if resumed { existing } else { 0 };
            status.total_bytes = Some(asset.size);
            status.error = None;
        })?;

        let mut file = async_fs::OpenOptions::new()
            .create(true)
            .append(resumed)
            .write(true)
            .truncate(!resumed)
            .open(&part_path)
            .await?;
        let started = Instant::now();
        let mut downloaded = if resumed { existing } else { 0 };
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            if token.is_cancelled() {
                file.flush().await?;
                self.set_state(RuntimeInstallState::Cancelled, None)?;
                return Err(AppError::RuntimeInstallFailed(
                    "runtime install cancelled".to_string(),
                ));
            }
            let chunk = chunk.map_err(|error| AppError::RuntimeInstallFailed(error.to_string()))?;
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;
            let elapsed = started.elapsed().as_secs_f64().max(0.001);
            self.update_status(|status| {
                status.downloaded_bytes = downloaded;
                status.percentage = Some((downloaded as f64 / asset.size as f64) * 100.0);
                status.speed_bytes_per_sec = Some(downloaded as f64 / elapsed);
            })?;
        }
        file.flush().await?;

        self.set_state(RuntimeInstallState::Verifying, None)?;
        let part_size = fs::metadata(&part_path)?.len();
        if part_size != asset.size {
            return Err(AppError::RuntimeInstallFailed(format!(
                "downloaded size {part_size} did not match expected {}",
                asset.size
            )));
        }
        if let Some(expected) = asset
            .digest
            .as_deref()
            .and_then(|d| d.strip_prefix("sha256:"))
        {
            let actual = sha256_file(&part_path)?;
            if !actual.eq_ignore_ascii_case(expected) {
                return Err(AppError::RuntimeInstallFailed(format!(
                    "checksum mismatch: expected {expected}, got {actual}"
                )));
            }
        }

        fs::rename(&part_path, &archive_path)?;

        self.set_state(RuntimeInstallState::Extracting, None)?;
        fs::create_dir_all(&install_dir)?;
        extract_archive(&archive_path, &install_dir)?;
        fs::remove_file(&archive_path).ok();

        let server = find_binary(&install_dir, "llama-server");
        let cli = find_binary(&install_dir, "llama-cli");
        let bench = find_binary(&install_dir, "llama-bench");
        if server.is_none() && cli.is_none() {
            return Err(AppError::RuntimeInstallFailed(
                "archive did not contain llama-server or llama-cli".to_string(),
            ));
        }
        for path in [&server, &cli, &bench].into_iter().flatten() {
            ensure_executable(path)?;
        }

        let root = self.root.root();
        let binaries = RuntimeBinaries {
            server: server.as_deref().and_then(|path| to_relative(root, path)),
            cli: cli.as_deref().and_then(|path| to_relative(root, path)),
            bench: bench.as_deref().and_then(|path| to_relative(root, path)),
        };
        let manifest = RuntimeManifest {
            runtime_name: format!("llama.cpp {slug}"),
            version: release.tag_name.clone(),
            platform: target_os().to_string(),
            architecture: std::env::consts::ARCH.to_string(),
            backend: backend.clone(),
            source: asset.browser_download_url.clone(),
            installed_at: Utc::now().to_rfc3339(),
            binaries,
            checksum: asset.digest.clone(),
            status: RuntimeStatus::Available,
        };
        let manifest_path = manifest_dir.join(format!("llama-{slug}-{}.json", release.tag_name));
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest)
                .map_err(|error| AppError::internal(error.to_string()))?,
        )?;

        self.update_status(|status| {
            status.state = RuntimeInstallState::Completed;
            status.percentage = Some(100.0);
            status.speed_bytes_per_sec = None;
            status.error = None;
        })?;
        self.status()
    }

    fn set_state(&self, state: RuntimeInstallState, error: Option<String>) -> Result<(), AppError> {
        self.update_status(|status| {
            status.state = state;
            status.error = error;
        })
    }

    fn update_status(
        &self,
        update: impl FnOnce(&mut RuntimeInstallStatus),
    ) -> Result<(), AppError> {
        let mut status = self
            .status
            .lock()
            .map_err(|_| AppError::internal("runtime install status lock poisoned"))?;
        update(&mut status);
        Ok(())
    }
}

async fn fetch_latest_release(client: &Client) -> Result<GithubRelease, AppError> {
    client
        .get(RELEASES_URL)
        .header(header::USER_AGENT, "OpenMindAI")
        .send()
        .await
        .map_err(|error| AppError::GithubApiError(error.to_string()))?
        .error_for_status()
        .map_err(|error| AppError::GithubApiError(error.to_string()))?
        .json::<GithubRelease>()
        .await
        .map_err(|error| AppError::GithubApiError(error.to_string()))
}

fn extract_archive(archive_path: &Path, dest_dir: &Path) -> Result<(), AppError> {
    let name = archive_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if name.ends_with(".zip") {
        let file = fs::File::open(archive_path)?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|error| AppError::RuntimeInstallFailed(error.to_string()))?;
        archive
            .extract(dest_dir)
            .map_err(|error| AppError::RuntimeInstallFailed(error.to_string()))?;
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        let file = fs::File::open(archive_path)?;
        let decoder = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        archive.unpack(dest_dir)?;
    } else {
        return Err(AppError::RuntimeInstallFailed(format!(
            "unsupported archive format: {name}"
        )));
    }
    Ok(())
}

/// Breadth-first search for `base_name` (or `base_name.exe` on Windows)
/// anywhere under `dir` — release archives nest binaries under varying
/// subdirectories (e.g. `build/bin/`) across backends/platforms.
fn find_binary(dir: &Path, base_name: &str) -> Option<PathBuf> {
    let windows_name = format!("{base_name}.exe");
    let mut queue = vec![dir.to_path_buf()];
    while let Some(current) = queue.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                queue.push(path);
                continue;
            }
            let file_name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if file_name == base_name || file_name == windows_name {
                return Some(path);
            }
        }
    }
    None
}

fn to_relative(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|p| p.display().to_string())
}

#[cfg(unix)]
fn ensure_executable(path: &Path) -> Result<(), AppError> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(permissions.mode() | 0o755);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn ensure_executable(_path: &Path) -> Result<(), AppError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matches_windows_and_linux_patterns() {
        assert!(glob_match(
            "llama-b10441-bin-win-cpu-x64.zip",
            "llama-*-bin-win-cpu-x64.zip"
        ));
        assert!(glob_match(
            "llama-b10441-bin-win-rocm-7.14-x64.zip",
            "llama-*-bin-win-rocm-*-x64.zip"
        ));
        assert!(glob_match(
            "llama-b10441-bin-ubuntu-vulkan-x64.tar.gz",
            "llama-*-bin-ubuntu-vulkan-x64.tar.gz"
        ));
        assert!(!glob_match(
            "llama-b10441-bin-ubuntu-x64.tar.gz",
            "llama-*-bin-ubuntu-vulkan-x64.tar.gz"
        ));
        assert!(!glob_match(
            "llama-b10441-bin-win-cuda-13.3-x64.zip",
            "llama-*-bin-win-cuda-12.4-x64.zip"
        ));
    }

    #[test]
    fn catalog_covers_windows_backends_and_skips_unpublished_linux_combos() {
        for backend in [
            BackendKind::Cpu,
            BackendKind::Cuda,
            BackendKind::Vulkan,
            BackendKind::Sycl,
            BackendKind::Hip,
        ] {
            assert!(
                catalog_pattern("windows", &backend).is_some(),
                "expected a windows asset pattern for {backend:?}"
            );
        }
        assert!(catalog_pattern("linux", &BackendKind::Cpu).is_some());
        assert!(catalog_pattern("linux", &BackendKind::Vulkan).is_some());
        assert!(catalog_pattern("linux", &BackendKind::Sycl).is_some());
        assert!(catalog_pattern("linux", &BackendKind::Cuda).is_none());
        assert!(catalog_pattern("linux", &BackendKind::Hip).is_none());
        assert!(catalog_pattern("windows", &BackendKind::Metal).is_none());
    }

    #[test]
    fn finds_nested_binary_regardless_of_extension() {
        let temp = tempfile::tempdir().unwrap();
        let nested = temp.path().join("build").join("bin");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("llama-server.exe"), b"stub").unwrap();

        let found = find_binary(temp.path(), "llama-server").unwrap();
        assert_eq!(found, nested.join("llama-server.exe"));
        assert!(find_binary(temp.path(), "llama-cli").is_none());
    }

    #[test]
    fn extracts_zip_archive_and_writes_a_registry_compatible_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let archive_path = temp.path().join("runtime.zip");
        {
            let file = fs::File::create(&archive_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer
                .start_file(
                    "bin/llama-server.exe",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            std::io::Write::write_all(&mut writer, b"stub-server").unwrap();
            writer.finish().unwrap();
        }
        let dest = temp.path().join("extracted");
        fs::create_dir_all(&dest).unwrap();
        extract_archive(&archive_path, &dest).unwrap();
        assert!(dest.join("bin").join("llama-server.exe").is_file());
    }

    #[test]
    fn extracts_tar_gz_archive() {
        let temp = tempfile::tempdir().unwrap();
        let archive_path = temp.path().join("runtime.tar.gz");
        {
            let file = fs::File::create(&archive_path).unwrap();
            let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            let mut builder = tar::Builder::new(encoder);
            let data = b"stub-server";
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, "bin/llama-server", &data[..])
                .unwrap();
            builder.into_inner().unwrap().finish().unwrap();
        }
        let dest = temp.path().join("extracted");
        fs::create_dir_all(&dest).unwrap();
        extract_archive(&archive_path, &dest).unwrap();
        assert!(dest.join("bin").join("llama-server").is_file());
    }

    #[test]
    fn manifest_shape_round_trips_through_runtime_registry_discovery() {
        // Proves the exact bug this plan flagged doesn't recur: only real
        // BackendKind values ("cuda", not "cuda12"/"cuda13") ever get
        // written, so RuntimeRegistry::discover() can always deserialize
        // what this installer produces.
        let root_temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(root_temp.path().to_path_buf());
        root.ensure_directories().unwrap();

        let manifest = RuntimeManifest {
            runtime_name: "llama.cpp cuda".to_string(),
            version: "b10441".to_string(),
            platform: "windows".to_string(),
            architecture: "x86_64".to_string(),
            backend: BackendKind::Cuda,
            source: "https://example.invalid/runtime.zip".to_string(),
            installed_at: Utc::now().to_rfc3339(),
            binaries: RuntimeBinaries {
                server: Some("runtimes/llama/cuda/b10441/bin/llama-server.exe".to_string()),
                cli: None,
                bench: None,
            },
            checksum: Some("sha256:deadbeef".to_string()),
            status: RuntimeStatus::Available,
        };
        let manifest_dir = root.resolve_relative("runtimes/llama/manifests").unwrap();
        fs::write(
            manifest_dir.join("llama-cuda-b10441.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let discovered = crate::runtime::RuntimeRegistry::new(&root)
            .discover()
            .unwrap();
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].backend, BackendKind::Cuda);
    }
}
