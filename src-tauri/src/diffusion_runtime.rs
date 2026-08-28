use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use chrono::Utc;
use futures_util::StreamExt;
use reqwest::{header, Client, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::{fs as async_fs, io::AsyncWriteExt, process::Command, time::timeout};
use uuid::Uuid;
use zip::ZipArchive;

use crate::{
    app_error::AppError,
    hardware::{BackendKind, GpuVendor, HardwareProfile},
    model_catalog::wildcard_match,
    model_download::{ensure_contained, sha256_file, validate_free_space},
    portable_root::PortableRootManager,
};

const PINNED_RUNTIME_TAG: &str = "master-829-0a565f2";
const RELEASE_URL: &str =
    "https://api.github.com/repos/leejet/stable-diffusion.cpp/releases/tags/master-829-0a565f2";
const RUNTIME_ROOT: &str = "runtimes/diffusion/stable-diffusion.cpp";
const MANIFEST_PATH: &str = "runtimes/diffusion/stable-diffusion.cpp/manifest.json";
const SAFE_SPACE_MARGIN: u64 = 256 * 1024 * 1024;
const MAX_PROMPT_CHARS: usize = 4_000;
const GENERATION_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const VIDEO_GENERATION_TIMEOUT: Duration = Duration::from_secs(90 * 60);
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const WEBM_SIGNATURE: &[u8; 4] = b"\x1a\x45\xdf\xa3";

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct DiffusionRuntimeManifest {
    version: String,
    backend: BackendKind,
    source_url: String,
    archive_sha256: String,
    cli_path: String,
    #[serde(default)]
    cli_sha256: String,
    #[serde(default)]
    cli_size: u64,
    installed_at: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RenderProfile {
    width: u32,
    height: u32,
    steps: u32,
    cfg_scale: u32,
    clip_on_cpu: bool,
    vae_on_cpu: bool,
    offload_to_cpu: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VideoRenderProfile {
    width: u32,
    height: u32,
    frames: u32,
    fps: u32,
    offload_to_cpu: bool,
    clip_on_cpu: bool,
    vae_on_cpu: bool,
}

pub async fn generate_image(
    root: &PortableRootManager,
    client: &Client,
    hardware: &HardwareProfile,
    model_path: &Path,
    prompt: &str,
    output_path: &Path,
) -> Result<(), AppError> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err(AppError::ArtifactGenerationFailed(
            "image prompt cannot be empty".to_string(),
        ));
    }
    if prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(AppError::ArtifactGenerationFailed(format!(
            "image prompt is too long; maximum is {MAX_PROMPT_CHARS} characters"
        )));
    }

    let model_path = canonical_file_under_root(root, model_path, "image model")?;
    let output_parent = output_path.parent().ok_or_else(|| {
        AppError::ArtifactGenerationFailed("image output path has no parent".to_string())
    })?;
    fs::create_dir_all(output_parent)?;
    ensure_contained(root.root(), output_path).map_err(|error| {
        AppError::ArtifactGenerationFailed(format!("image output path rejected: {error}"))
    })?;

    let runtime = ensure_runtime(root, client, hardware).await?;
    let cli_path = root.resolve_relative(&runtime.cli_path)?;
    let profile = render_profile(hardware, &runtime.backend);

    if output_path.exists() {
        fs::remove_file(output_path)?;
    }

    let mut command = Command::new(&cli_path);
    command
        .arg("-m")
        .arg(&model_path)
        .arg("-p")
        .arg(prompt)
        .arg("-o")
        .arg(output_path)
        .arg("-W")
        .arg(profile.width.to_string())
        .arg("-H")
        .arg(profile.height.to_string())
        .arg("--steps")
        .arg(profile.steps.to_string())
        .arg("--cfg-scale")
        .arg(profile.cfg_scale.to_string())
        .arg("--sampling-method")
        .arg("euler")
        .arg("--vae-tiling")
        .arg("--mmap")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    if profile.clip_on_cpu {
        command.arg("--clip-on-cpu");
    }
    if profile.vae_on_cpu {
        command.arg("--vae-on-cpu");
    }
    if profile.offload_to_cpu {
        command.arg("--offload-to-cpu");
    }
    if let Some(parent) = cli_path.parent() {
        command.current_dir(parent);
    }
    hide_console_window(&mut command);

    tracing::info!(
        backend = ?runtime.backend,
        width = profile.width,
        height = profile.height,
        steps = profile.steps,
        model = %model_path.display(),
        "starting local diffusion image generation"
    );

    let output = timeout(GENERATION_TIMEOUT, command.output())
        .await
        .map_err(|_| {
            AppError::ArtifactGenerationFailed(
                "local image generation timed out after 20 minutes".to_string(),
            )
        })?
        .map_err(|error| {
            AppError::ArtifactGenerationFailed(format!(
                "could not start stable-diffusion.cpp: {error}"
            ))
        })?;

    if !output.status.success() {
        let detail = process_error_detail(&output.stdout, &output.stderr);
        return Err(AppError::ArtifactGenerationFailed(format!(
            "stable-diffusion.cpp exited with {}: {detail}",
            output.status
        )));
    }

    validate_png(output_path)?;
    tracing::info!(path = %output_path.display(), "local diffusion image generated");
    Ok(())
}

pub(crate) struct VideoGenerationRequest<'a> {
    pub diffusion_model_path: &'a Path,
    pub vae_path: &'a Path,
    pub text_encoder_path: &'a Path,
    pub prompt: &'a str,
    pub output_path: &'a Path,
}

pub async fn generate_video(
    root: &PortableRootManager,
    client: &Client,
    hardware: &HardwareProfile,
    request: VideoGenerationRequest<'_>,
) -> Result<(), AppError> {
    let VideoGenerationRequest {
        diffusion_model_path,
        vae_path,
        text_encoder_path,
        prompt,
        output_path,
    } = request;
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err(AppError::ArtifactGenerationFailed(
            "video prompt cannot be empty".to_string(),
        ));
    }
    if prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(AppError::ArtifactGenerationFailed(format!(
            "video prompt is too long; maximum is {MAX_PROMPT_CHARS} characters"
        )));
    }

    let diffusion_model_path =
        canonical_file_under_root(root, diffusion_model_path, "video diffusion model")?;
    let vae_path = canonical_file_under_root(root, vae_path, "video VAE")?;
    let text_encoder_path =
        canonical_file_under_root(root, text_encoder_path, "video text encoder")?;
    let output_parent = output_path.parent().ok_or_else(|| {
        AppError::ArtifactGenerationFailed("video output path has no parent".to_string())
    })?;
    fs::create_dir_all(output_parent)?;
    ensure_contained(root.root(), output_path).map_err(|error| {
        AppError::ArtifactGenerationFailed(format!("video output path rejected: {error}"))
    })?;

    let runtime = ensure_runtime(root, client, hardware).await?;
    let cli_path = root.resolve_relative(&runtime.cli_path)?;
    let profile = video_render_profile(hardware, &runtime.backend);
    if output_path.exists() {
        fs::remove_file(output_path)?;
    }

    let mut command = Command::new(&cli_path);
    command
        .arg("-M")
        .arg("vid_gen")
        .arg("--diffusion-model")
        .arg(&diffusion_model_path)
        .arg("--vae")
        .arg(&vae_path)
        .arg("--t5xxl")
        .arg(&text_encoder_path)
        .arg("-p")
        .arg(prompt)
        .arg("-n")
        .arg("worst quality, low quality, blurry, distorted, artifacts, text, watermark, flicker, jitter, temporal inconsistency")
        .arg("-o")
        .arg(output_path)
        .arg("--cfg-scale")
        .arg("6.0")
        .arg("--sampling-method")
        .arg("euler")
        .arg("-W")
        .arg(profile.width.to_string())
        .arg("-H")
        .arg(profile.height.to_string())
        .arg("--video-frames")
        .arg(profile.frames.to_string())
        .arg("--fps")
        .arg(profile.fps.to_string())
        .arg("--flow-shift")
        .arg("3.0")
        .arg("--diffusion-fa")
        .arg("--mmap")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    if profile.offload_to_cpu {
        command.arg("--offload-to-cpu");
    }
    if profile.clip_on_cpu {
        command.arg("--clip-on-cpu");
    }
    if profile.vae_on_cpu {
        command.arg("--vae-on-cpu");
    }
    if let Some(parent) = cli_path.parent() {
        command.current_dir(parent);
    }
    hide_console_window(&mut command);

    tracing::info!(
        backend = ?runtime.backend,
        width = profile.width,
        height = profile.height,
        frames = profile.frames,
        fps = profile.fps,
        model = %diffusion_model_path.display(),
        "starting local Wan video generation"
    );

    let output = timeout(VIDEO_GENERATION_TIMEOUT, command.output())
        .await
        .map_err(|_| {
            AppError::ArtifactGenerationFailed(
                "local video generation timed out after 90 minutes".to_string(),
            )
        })?
        .map_err(|error| {
            AppError::ArtifactGenerationFailed(format!(
                "could not start stable-diffusion.cpp video runtime: {error}"
            ))
        })?;

    if !output.status.success() {
        let detail = process_error_detail(&output.stdout, &output.stderr);
        return Err(AppError::ArtifactGenerationFailed(format!(
            "stable-diffusion.cpp video generation exited with {}: {detail}",
            output.status
        )));
    }

    validate_webm(output_path)?;
    tracing::info!(path = %output_path.display(), "local Wan WebM video generated");
    Ok(())
}

async fn ensure_runtime(
    root: &PortableRootManager,
    client: &Client,
    hardware: &HardwareProfile,
) -> Result<DiffusionRuntimeManifest, AppError> {
    let candidates =
        runtime_asset_candidates(std::env::consts::OS, std::env::consts::ARCH, hardware);
    let preferred_backend = candidates.first().map(|(backend, _)| backend);

    if let Some(manifest) = load_manifest(root)? {
        if runtime_manifest_is_reusable(root, &manifest, preferred_backend)? {
            return Ok(manifest);
        }
        tracing::warn!(
            version = %manifest.version,
            backend = ?manifest.backend,
            "installed diffusion runtime is stale or failed integrity validation; reinstalling pinned runtime"
        );
    }

    if candidates.is_empty() {
        return Err(AppError::RuntimeInstallFailed(format!(
            "stable-diffusion.cpp has no supported prebuilt runtime for {}/{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )));
    }

    let release = client
        .get(RELEASE_URL)
        .header(header::USER_AGENT, "OpenMindAI/2")
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|error| AppError::RuntimeInstallFailed(error.to_string()))?
        .error_for_status()
        .map_err(|error| AppError::RuntimeInstallFailed(error.to_string()))?
        .json::<GithubRelease>()
        .await
        .map_err(|error| AppError::RuntimeInstallFailed(error.to_string()))?;
    if release.tag_name != PINNED_RUNTIME_TAG {
        return Err(AppError::RuntimeInstallFailed(format!(
            "pinned stable-diffusion.cpp release endpoint returned unexpected tag {}; expected {PINNED_RUNTIME_TAG}",
            release.tag_name
        )));
    }

    let (backend, asset) = candidates
        .iter()
        .find_map(|(backend, pattern)| {
            release
                .assets
                .iter()
                .find(|asset| wildcard_match(pattern, &asset.name))
                .map(|asset| (backend.clone(), asset))
        })
        .ok_or_else(|| {
            AppError::RuntimeInstallFailed(format!(
                "pinned stable-diffusion.cpp release {} has no compatible runtime asset",
                release.tag_name
            ))
        })?;

    install_runtime_asset(root, client, &release.tag_name, backend, asset).await
}

async fn install_runtime_asset(
    root: &PortableRootManager,
    client: &Client,
    version: &str,
    backend: BackendKind,
    asset: &GithubAsset,
) -> Result<DiffusionRuntimeManifest, AppError> {
    let expected_sha256 = asset
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .filter(|digest| digest.len() == 64)
        .ok_or_else(|| {
            AppError::RuntimeInstallFailed(
                "stable-diffusion.cpp release asset is missing a SHA-256 digest".to_string(),
            )
        })?;

    let temp_dir = root.resolve_relative("temp/downloads")?;
    fs::create_dir_all(&temp_dir)?;
    ensure_contained(root.root(), &temp_dir)?;
    validate_free_space(&temp_dir, asset.size.saturating_add(SAFE_SPACE_MARGIN))?;

    let archive_path = temp_dir.join(format!("{}.part", asset.name));
    ensure_contained(root.root(), &archive_path)?;
    download_asset(client, asset, &archive_path).await?;

    let actual_sha256 = sha256_file(&archive_path)?;
    if !actual_sha256.eq_ignore_ascii_case(expected_sha256) {
        let _ = fs::remove_file(&archive_path);
        return Err(AppError::RuntimeInstallFailed(format!(
            "stable-diffusion.cpp runtime checksum mismatch: expected {expected_sha256}, got {actual_sha256}"
        )));
    }

    let install_dir = root.resolve_relative(format!(
        "{RUNTIME_ROOT}/{}/{}",
        backend_slug(&backend),
        safe_component(version)
    ))?;
    let cli_path = extract_runtime_archive(root, &archive_path, &install_dir)?;
    let _ = fs::remove_file(&archive_path);

    let relative_cli = cli_path
        .strip_prefix(root.root())
        .map_err(|_| {
            AppError::RuntimeInstallFailed("runtime CLI escaped OpenMindAI Root".to_string())
        })?
        .to_string_lossy()
        .replace('\\', "/");
    let cli_size = fs::metadata(&cli_path)?.len();
    if cli_size == 0 {
        return Err(AppError::RuntimeInstallFailed(
            "stable-diffusion.cpp runtime CLI is empty after extraction".to_string(),
        ));
    }
    let cli_sha256 = sha256_file(&cli_path)?;
    let manifest = DiffusionRuntimeManifest {
        version: version.to_string(),
        backend,
        source_url: asset.browser_download_url.clone(),
        archive_sha256: actual_sha256,
        cli_path: relative_cli,
        cli_sha256,
        cli_size,
        installed_at: Utc::now().to_rfc3339(),
    };
    write_manifest(root, &manifest)?;
    tracing::info!(
        version = %manifest.version,
        backend = ?manifest.backend,
        "stable-diffusion.cpp runtime installed"
    );
    Ok(manifest)
}

async fn download_asset(
    client: &Client,
    asset: &GithubAsset,
    destination: &Path,
) -> Result<(), AppError> {
    let mut existing = fs::metadata(destination)
        .map(|meta| meta.len())
        .unwrap_or(0);
    if existing > asset.size {
        fs::remove_file(destination)?;
        existing = 0;
    }

    let mut request = client
        .get(&asset.browser_download_url)
        .header(header::USER_AGENT, "OpenMindAI/2")
        .timeout(Duration::from_secs(10 * 60));
    if existing > 0 {
        request = request.header(header::RANGE, format!("bytes={existing}-"));
    }
    let response = request
        .send()
        .await
        .map_err(|error| AppError::RuntimeInstallFailed(error.to_string()))?;
    let resumed = existing > 0 && response.status() == StatusCode::PARTIAL_CONTENT;
    if existing > 0 && !resumed {
        async_fs::remove_file(destination).await.ok();
        existing = 0;
    }
    if !response.status().is_success() {
        return Err(AppError::RuntimeInstallFailed(format!(
            "HTTP {} while downloading stable-diffusion.cpp runtime",
            response.status()
        )));
    }

    let mut file = async_fs::OpenOptions::new()
        .create(true)
        .append(resumed)
        .write(true)
        .truncate(!resumed)
        .open(destination)
        .await?;
    let mut downloaded = existing;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| AppError::RuntimeInstallFailed(error.to_string()))?;
        file.write_all(&chunk).await?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if downloaded > asset.size {
            return Err(AppError::RuntimeInstallFailed(
                "stable-diffusion.cpp runtime download exceeded expected size".to_string(),
            ));
        }
    }
    file.flush().await?;
    let size = fs::metadata(destination)?.len();
    if size != asset.size {
        return Err(AppError::RuntimeInstallFailed(format!(
            "stable-diffusion.cpp runtime download size {size} did not match expected {}",
            asset.size
        )));
    }
    Ok(())
}

fn extract_runtime_archive(
    root: &PortableRootManager,
    archive_path: &Path,
    install_dir: &Path,
) -> Result<PathBuf, AppError> {
    let staging = root.resolve_relative(format!("temp/runtime-extract/{}", Uuid::new_v4()))?;
    fs::create_dir_all(&staging)?;
    ensure_contained(root.root(), &staging)?;

    let extraction_result = (|| -> Result<(), AppError> {
        let file = fs::File::open(archive_path)?;
        let mut archive = ZipArchive::new(file)
            .map_err(|error| AppError::RuntimeInstallFailed(error.to_string()))?;
        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .map_err(|error| AppError::RuntimeInstallFailed(error.to_string()))?;
            let enclosed = entry.enclosed_name().ok_or_else(|| {
                AppError::RuntimeInstallFailed(
                    "runtime archive contains an unsafe path".to_string(),
                )
            })?;
            let destination = staging.join(enclosed);
            if entry.is_dir() {
                fs::create_dir_all(&destination)?;
                continue;
            }
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output = fs::File::create(&destination)?;
            std::io::copy(&mut entry, &mut output)?;
            output.flush()?;
            #[cfg(unix)]
            if let Some(mode) = entry.unix_mode() {
                fs::set_permissions(&destination, fs::Permissions::from_mode(mode))?;
            }
        }
        Ok(())
    })();

    if let Err(error) = extraction_result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    let cli_in_staging = find_cli_binary(&staging).ok_or_else(|| {
        let _ = fs::remove_dir_all(&staging);
        AppError::RuntimeInstallFailed(
            "stable-diffusion.cpp archive did not contain sd-cli".to_string(),
        )
    })?;
    let relative_cli = cli_in_staging
        .strip_prefix(&staging)
        .map_err(|_| AppError::RuntimeInstallFailed("runtime CLI path invalid".to_string()))?
        .to_path_buf();

    if install_dir.exists() {
        fs::remove_dir_all(install_dir)?;
    }
    if let Some(parent) = install_dir.parent() {
        fs::create_dir_all(parent)?;
    }
    ensure_contained(root.root(), install_dir)?;
    fs::rename(&staging, install_dir)?;
    Ok(install_dir.join(relative_cli))
}

fn find_cli_binary(root: &Path) -> Option<PathBuf> {
    let expected = if cfg!(target_os = "windows") {
        "sd-cli.exe"
    } else {
        "sd-cli"
    };
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(dir).ok()? {
            let path = entry.ok()?.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(expected))
            {
                return Some(path);
            }
        }
    }
    None
}

fn runtime_manifest_is_reusable(
    root: &PortableRootManager,
    manifest: &DiffusionRuntimeManifest,
    preferred_backend: Option<&BackendKind>,
) -> Result<bool, AppError> {
    if manifest.version != PINNED_RUNTIME_TAG {
        return Ok(false);
    }
    if preferred_backend.is_some_and(|backend| *backend != manifest.backend) {
        return Ok(false);
    }
    if manifest.cli_size == 0 || manifest.cli_sha256.len() != 64 {
        return Ok(false);
    }

    let cli = match root.resolve_relative(&manifest.cli_path) {
        Ok(cli) => cli,
        Err(error) => {
            tracing::warn!(%error, cli_path = %manifest.cli_path, "invalid diffusion runtime CLI path in manifest");
            return Ok(false);
        }
    };
    if !cli.is_file() {
        return Ok(false);
    }
    let metadata = match fs::metadata(&cli) {
        Ok(metadata) => metadata,
        Err(error) => {
            tracing::warn!(%error, path = %cli.display(), "could not stat installed diffusion runtime CLI");
            return Ok(false);
        }
    };
    if metadata.len() != manifest.cli_size {
        return Ok(false);
    }
    let actual_sha256 = match sha256_file(&cli) {
        Ok(digest) => digest,
        Err(error) => {
            tracing::warn!(%error, path = %cli.display(), "could not verify installed diffusion runtime CLI");
            return Ok(false);
        }
    };
    Ok(actual_sha256.eq_ignore_ascii_case(&manifest.cli_sha256))
}

fn load_manifest(root: &PortableRootManager) -> Result<Option<DiffusionRuntimeManifest>, AppError> {
    let path = root.resolve_relative(MANIFEST_PATH)?;
    if !path.is_file() {
        return Ok(None);
    }
    let content = fs::read_to_string(path)?;
    match serde_json::from_str(&content) {
        Ok(manifest) => Ok(Some(manifest)),
        Err(error) => {
            tracing::warn!(%error, "ignoring invalid diffusion runtime manifest");
            Ok(None)
        }
    }
}

fn write_manifest(
    root: &PortableRootManager,
    manifest: &DiffusionRuntimeManifest,
) -> Result<(), AppError> {
    let path = root.resolve_relative(MANIFEST_PATH)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    ensure_contained(root.root(), &path)?;
    let temp = path.with_extension("json.tmp");
    fs::write(
        &temp,
        serde_json::to_string_pretty(manifest)
            .map_err(|error| AppError::internal(error.to_string()))?,
    )?;
    fs::rename(temp, path)?;
    Ok(())
}

fn runtime_asset_candidates(
    os: &str,
    arch: &str,
    hardware: &HardwareProfile,
) -> Vec<(BackendKind, &'static str)> {
    let has_nvidia = hardware
        .gpus
        .iter()
        .any(|gpu| gpu.vendor == GpuVendor::Nvidia && !gpu.is_software);
    let has_amd = hardware
        .gpus
        .iter()
        .any(|gpu| gpu.vendor == GpuVendor::Amd && !gpu.is_software);
    let has_intel = hardware
        .gpus
        .iter()
        .any(|gpu| gpu.vendor == GpuVendor::Intel && !gpu.is_software);

    match (os, arch) {
        ("windows", "x86_64") => {
            let mut result = Vec::new();
            if has_nvidia {
                result.push((BackendKind::Cuda, "sd-*-bin-win-cuda12-x64.zip"));
            }
            if has_amd || has_intel || (!has_nvidia && hardware.backends.vulkan) {
                result.push((BackendKind::Vulkan, "sd-*-bin-win-vulkan-x64.zip"));
            }
            result.push((BackendKind::Cpu, "sd-*-bin-win-cpu-x64.zip"));
            result
        }
        ("linux", "x86_64") => {
            let mut result = Vec::new();
            if has_amd || has_nvidia || has_intel || hardware.backends.vulkan {
                result.push((
                    BackendKind::Vulkan,
                    "sd-*-bin-Linux-Ubuntu-*-x86_64-vulkan.zip",
                ));
            }
            result.push((BackendKind::Cpu, "sd-*-bin-Linux-Ubuntu-*-x86_64.zip"));
            result
        }
        ("macos", "aarch64") => vec![(BackendKind::Metal, "sd-*-bin-Darwin-macOS-*-arm64.zip")],
        _ => Vec::new(),
    }
}

fn video_render_profile(hardware: &HardwareProfile, backend: &BackendKind) -> VideoRenderProfile {
    if *backend == BackendKind::Cpu {
        return VideoRenderProfile {
            width: 384,
            height: 224,
            frames: 17,
            fps: 8,
            offload_to_cpu: false,
            clip_on_cpu: true,
            vae_on_cpu: true,
        };
    }

    let max_vram = hardware
        .gpus
        .iter()
        .filter(|gpu| !gpu.is_software)
        .filter_map(|gpu| gpu.dedicated_vram_bytes)
        .max()
        .unwrap_or(0);
    let gib = 1024_u64.pow(3);
    if max_vram <= 8 * gib {
        VideoRenderProfile {
            width: 512,
            height: 288,
            frames: 17,
            fps: 8,
            offload_to_cpu: true,
            clip_on_cpu: true,
            vae_on_cpu: true,
        }
    } else if max_vram <= 12 * gib {
        VideoRenderProfile {
            width: 640,
            height: 368,
            frames: 25,
            fps: 12,
            offload_to_cpu: true,
            clip_on_cpu: true,
            vae_on_cpu: true,
        }
    } else {
        VideoRenderProfile {
            width: 832,
            height: 480,
            frames: 33,
            fps: 16,
            offload_to_cpu: false,
            clip_on_cpu: false,
            vae_on_cpu: false,
        }
    }
}

fn render_profile(hardware: &HardwareProfile, backend: &BackendKind) -> RenderProfile {
    if *backend == BackendKind::Cpu {
        return RenderProfile {
            width: 512,
            height: 512,
            steps: 12,
            cfg_scale: 5,
            clip_on_cpu: true,
            vae_on_cpu: true,
            offload_to_cpu: false,
        };
    }

    let max_vram = hardware
        .gpus
        .iter()
        .filter(|gpu| !gpu.is_software)
        .filter_map(|gpu| gpu.dedicated_vram_bytes)
        .max();
    let gib = 1024_u64.pow(3);
    match max_vram {
        Some(bytes) if bytes <= 6 * gib => RenderProfile {
            width: 512,
            height: 512,
            steps: 16,
            cfg_scale: 5,
            clip_on_cpu: true,
            vae_on_cpu: true,
            offload_to_cpu: true,
        },
        Some(bytes) if bytes <= 8 * gib => RenderProfile {
            width: 768,
            height: 768,
            steps: 20,
            cfg_scale: 5,
            clip_on_cpu: true,
            vae_on_cpu: true,
            offload_to_cpu: false,
        },
        _ => RenderProfile {
            width: 1024,
            height: 1024,
            steps: 24,
            cfg_scale: 5,
            clip_on_cpu: false,
            vae_on_cpu: false,
            offload_to_cpu: false,
        },
    }
}

fn validate_webm(path: &Path) -> Result<(), AppError> {
    let metadata = fs::metadata(path).map_err(|error| {
        AppError::ArtifactGenerationFailed(format!("video output was not created: {error}"))
    })?;
    if metadata.len() < 4096 {
        return Err(AppError::ArtifactGenerationFailed(
            "video output is implausibly small".to_string(),
        ));
    }
    let mut file = fs::File::open(path)?;
    let mut signature = [0_u8; 4];
    file.read_exact(&mut signature)?;
    if &signature != WEBM_SIGNATURE {
        return Err(AppError::ArtifactGenerationFailed(
            "video runtime did not produce a valid WebM/EBML container".to_string(),
        ));
    }
    Ok(())
}

fn validate_png(path: &Path) -> Result<(), AppError> {
    let metadata = fs::metadata(path).map_err(|error| {
        AppError::ArtifactGenerationFailed(format!("image output was not created: {error}"))
    })?;
    if metadata.len() < 1024 {
        return Err(AppError::ArtifactGenerationFailed(
            "image output is implausibly small".to_string(),
        ));
    }
    let mut file = fs::File::open(path)?;
    let mut signature = [0_u8; 8];
    file.read_exact(&mut signature)?;
    if &signature != PNG_SIGNATURE {
        return Err(AppError::ArtifactGenerationFailed(
            "image runtime did not produce a valid PNG".to_string(),
        ));
    }
    Ok(())
}

fn canonical_file_under_root(
    root: &PortableRootManager,
    path: &Path,
    label: &str,
) -> Result<PathBuf, AppError> {
    let canonical_root = fs::canonicalize(root.root())?;
    let canonical = fs::canonicalize(path).map_err(|error| {
        AppError::ArtifactGenerationFailed(format!("{label} is unavailable: {error}"))
    })?;
    if !canonical.starts_with(canonical_root) || !canonical.is_file() {
        return Err(AppError::ArtifactGenerationFailed(format!(
            "{label} must be a file inside OpenMindAI Root"
        )));
    }
    Ok(canonical)
}

fn process_error_detail(stdout: &[u8], stderr: &[u8]) -> String {
    let mut combined = String::from_utf8_lossy(stderr).trim().to_string();
    if combined.is_empty() {
        combined = String::from_utf8_lossy(stdout).trim().to_string();
    }
    if combined.is_empty() {
        return "no process output".to_string();
    }
    let chars = combined.chars().collect::<Vec<_>>();
    let start = chars.len().saturating_sub(4_000);
    chars[start..].iter().collect()
}

fn backend_slug(backend: &BackendKind) -> &'static str {
    match backend {
        BackendKind::Cpu => "cpu",
        BackendKind::Cuda => "cuda",
        BackendKind::Vulkan => "vulkan",
        BackendKind::Sycl => "sycl",
        BackendKind::Hip => "rocm",
        BackendKind::Metal => "metal",
    }
}

fn safe_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn hide_console_window(_command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        _command.as_std_mut().creation_flags(CREATE_NO_WINDOW);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::{BackendProfile, CpuProfile, GpuInfo, MemoryProfile};

    fn hardware(vendor: Option<GpuVendor>, vram: Option<u64>) -> HardwareProfile {
        let gpus = vendor
            .map(|vendor| {
                vec![GpuInfo {
                    id: "gpu0".to_string(),
                    name: "test gpu".to_string(),
                    vendor,
                    vendor_id: None,
                    device_id: None,
                    subsystem_id: None,
                    revision: None,
                    dedicated_vram_bytes: vram,
                    dedicated_system_memory_bytes: None,
                    shared_memory_bytes: None,
                    luid: None,
                    is_discrete: true,
                    is_integrated: false,
                    is_software: false,
                    available_backends: vec![BackendKind::Vulkan],
                    recommended_backend: BackendKind::Vulkan,
                }]
            })
            .unwrap_or_default();
        HardwareProfile {
            operating_system: "test".to_string(),
            architecture: "x86_64".to_string(),
            cpu: CpuProfile {
                name: "cpu".to_string(),
                physical_cores: Some(4),
                logical_threads: 8,
            },
            memory: MemoryProfile {
                total_bytes: 16 * 1024_u64.pow(3),
                available_bytes: 8 * 1024_u64.pow(3),
            },
            gpus,
            primary_gpu: Some("gpu0".to_string()),
            recommended_inference_gpu: Some("gpu0".to_string()),
            backends: BackendProfile {
                cpu: true,
                cuda: false,
                vulkan: true,
                sycl: false,
                hip: false,
                metal: false,
            },
        }
    }

    #[test]
    fn windows_amd_prefers_vulkan_before_cpu() {
        let candidates = runtime_asset_candidates(
            "windows",
            "x86_64",
            &hardware(Some(GpuVendor::Amd), Some(8 * 1024_u64.pow(3))),
        );
        assert_eq!(candidates[0].0, BackendKind::Vulkan);
        assert_eq!(candidates.last().unwrap().0, BackendKind::Cpu);
    }

    #[test]
    fn windows_nvidia_prefers_cuda() {
        let candidates = runtime_asset_candidates(
            "windows",
            "x86_64",
            &hardware(Some(GpuVendor::Nvidia), Some(12 * 1024_u64.pow(3))),
        );
        assert_eq!(candidates[0].0, BackendKind::Cuda);
    }

    #[test]
    fn rx580_class_profile_uses_memory_safe_sdxl_defaults() {
        let profile = render_profile(
            &hardware(Some(GpuVendor::Amd), Some(8 * 1024_u64.pow(3))),
            &BackendKind::Vulkan,
        );
        assert_eq!(
            (profile.width, profile.height, profile.steps),
            (768, 768, 20)
        );
        assert!(profile.clip_on_cpu);
        assert!(profile.vae_on_cpu);
        assert!(!profile.offload_to_cpu);
    }

    #[test]
    fn cpu_profile_uses_bounded_resolution() {
        let profile = render_profile(&hardware(None, None), &BackendKind::Cpu);
        assert_eq!(
            (profile.width, profile.height, profile.steps),
            (512, 512, 12)
        );
    }

    #[test]
    fn validates_real_png_signature_and_rejects_text() {
        let temp = tempfile::tempdir().unwrap();
        let good = temp.path().join("good.png");
        let mut png = PNG_SIGNATURE.to_vec();
        png.resize(2048, 0);
        fs::write(&good, png).unwrap();
        validate_png(&good).unwrap();

        let bad = temp.path().join("bad.png");
        fs::write(&bad, vec![b'x'; 2048]).unwrap();
        assert!(validate_png(&bad).is_err());
    }

    #[test]
    fn safe_component_removes_path_syntax() {
        assert_eq!(safe_component("master/829:abc"), "master-829-abc");
    }

    #[test]
    fn runtime_manifest_reuse_requires_pinned_version_backend_and_cli_integrity() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let cli_relative = if cfg!(target_os = "windows") {
            "runtimes/diffusion/stable-diffusion.cpp/cpu/master-829-0a565f2/sd-cli.exe"
        } else {
            "runtimes/diffusion/stable-diffusion.cpp/cpu/master-829-0a565f2/sd-cli"
        };
        let cli = root.resolve_relative(cli_relative).unwrap();
        fs::create_dir_all(cli.parent().unwrap()).unwrap();
        fs::write(&cli, b"known-good-runtime").unwrap();
        let cli_sha256 = sha256_file(&cli).unwrap();
        let cli_size = fs::metadata(&cli).unwrap().len();
        let mut manifest = DiffusionRuntimeManifest {
            version: PINNED_RUNTIME_TAG.to_string(),
            backend: BackendKind::Cpu,
            source_url: "https://example.invalid/runtime.zip".to_string(),
            archive_sha256: "a".repeat(64),
            cli_path: cli_relative.to_string(),
            cli_sha256,
            cli_size,
            installed_at: "2026-08-28T00:00:00Z".to_string(),
        };

        assert!(runtime_manifest_is_reusable(&root, &manifest, Some(&BackendKind::Cpu)).unwrap());
        assert!(
            !runtime_manifest_is_reusable(&root, &manifest, Some(&BackendKind::Vulkan)).unwrap()
        );

        manifest.version = "master-828-old".to_string();
        assert!(!runtime_manifest_is_reusable(&root, &manifest, Some(&BackendKind::Cpu)).unwrap());
        manifest.version = PINNED_RUNTIME_TAG.to_string();

        manifest.cli_path = "../outside/sd-cli".to_string();
        assert!(!runtime_manifest_is_reusable(&root, &manifest, Some(&BackendKind::Cpu)).unwrap());
        manifest.cli_path = cli_relative.to_string();

        fs::write(&cli, b"tampered-runtime!!").unwrap();
        assert_eq!(fs::metadata(&cli).unwrap().len(), cli_size);
        assert!(!runtime_manifest_is_reusable(&root, &manifest, Some(&BackendKind::Cpu)).unwrap());
    }

    #[test]
    fn legacy_manifest_without_cli_integrity_is_invalidated_for_reinstall() {
        let legacy = format!(
            r#"{{"version":"{PINNED_RUNTIME_TAG}","backend":"cpu","sourceUrl":"https://example.invalid/runtime.zip","archiveSha256":"{}","cliPath":"runtimes/diffusion/sd-cli","installedAt":"2026-08-28T00:00:00Z"}}"#,
            "b".repeat(64)
        );
        let manifest: DiffusionRuntimeManifest = serde_json::from_str(&legacy).unwrap();
        assert!(manifest.cli_sha256.is_empty());
        assert_eq!(manifest.cli_size, 0);
    }
}

#[cfg(test)]
mod video_runtime_tests {
    use super::*;

    #[test]
    fn webm_validator_accepts_ebml_container_signature() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("clip.webm");
        let mut bytes = WEBM_SIGNATURE.to_vec();
        bytes.resize(8192, 0);
        fs::write(&path, bytes).unwrap();
        validate_webm(&path).unwrap();
    }

    #[test]
    fn webm_validator_rejects_wrong_container() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("clip.webm");
        fs::write(&path, vec![0_u8; 8192]).unwrap();
        assert!(validate_webm(&path).is_err());
    }
}
