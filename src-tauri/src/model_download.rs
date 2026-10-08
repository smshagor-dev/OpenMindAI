use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use chrono::Utc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sysinfo::Disks;
use tokio_util::sync::CancellationToken;

use crate::{
    app_error::AppError,
    model_catalog::{entry_by_id, wildcard_match, ModelCatalogDownload, ModelCatalogEntry},
    portable_root::PortableRootManager,
};

#[path = "model_package.rs"]
mod model_package;
pub(crate) use model_package::validate_installed_dependencies;

const SAFE_SPACE_MARGIN: u64 = 768 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DownloadState {
    Queued,
    Resolving,
    Downloading,
    PausedInterrupted,
    Verifying,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadStatus {
    pub model_id: String,
    pub name: String,
    pub state: DownloadState,
    pub repo: String,
    pub quantization: String,
    pub filename: Option<String>,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub percentage: Option<f64>,
    pub speed_bytes_per_sec: Option<f64>,
    pub destination: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QwenModelManifest {
    pub repo: String,
    pub repo_sha: Option<String>,
    pub quantization: String,
    pub filename: String,
    pub size_bytes: u64,
    pub sha256: Option<String>,
    pub actual_sha256: Option<String>,
    pub verification: VerificationState,
    pub architecture: Option<String>,
    pub context_length: Option<u64>,
    pub chat_template_available: bool,
    pub source_url: String,
    pub installed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum VerificationState {
    Verified,
    Unverified,
    Failed,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HuggingFaceModel {
    sha: Option<String>,
    siblings: Vec<HuggingFaceSibling>,
    gguf: Option<HuggingFaceGguf>,
}

#[derive(Debug, Deserialize)]
struct HuggingFaceSibling {
    rfilename: String,
    size: Option<u64>,
    lfs: Option<HuggingFaceLfs>,
}

#[derive(Debug, Deserialize)]
struct HuggingFaceLfs {
    sha256: Option<String>,
    size: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HuggingFaceGguf {
    architecture: Option<String>,
    context_length: Option<u64>,
    chat_template: Option<String>,
    total_file_size: Option<u64>,
}

#[derive(Clone)]
pub struct ModelDownloadManager {
    root: PortableRootManager,
    status: Arc<Mutex<DownloadStatus>>,
    cancel_token: Arc<Mutex<Option<CancellationToken>>>,
    client: Client,
    catalog_entry: ModelCatalogEntry,
}

impl ModelDownloadManager {
    pub fn new(root: PortableRootManager) -> Self {
        let catalog_entry = crate::model_catalog::required_entry()
            .expect("bundled model catalog is malformed or has no required entry");
        Self {
            root,
            status: Arc::new(Mutex::new(DownloadStatus::queued(&catalog_entry))),
            cancel_token: Arc::new(Mutex::new(None)),
            client: crate::net::download_client(),
            catalog_entry,
        }
    }

    pub fn status(&self) -> Result<DownloadStatus, AppError> {
        self.status
            .lock()
            .map(|status| status.clone())
            .map_err(|_| AppError::internal("download status lock poisoned"))
    }

    pub fn cancel(&self) -> Result<DownloadStatus, AppError> {
        self.stop_download(DownloadState::Cancelled)
    }

    pub fn pause(&self) -> Result<DownloadStatus, AppError> {
        self.stop_download(DownloadState::PausedInterrupted)
    }

    fn stop_download(&self, state: DownloadState) -> Result<DownloadStatus, AppError> {
        if let Some(token) = self
            .cancel_token
            .lock()
            .map_err(|_| AppError::internal("download cancel lock poisoned"))?
            .as_ref()
        {
            token.cancel();
        }
        self.set_state(state, None)?;
        self.status()
    }

    pub async fn download_qwen_q4_k_m(&self) -> Result<DownloadStatus, AppError> {
        let model_id = self.catalog_entry.id.clone();
        self.download_catalog_model(&model_id).await
    }

    pub async fn download_catalog_model(&self, model_id: &str) -> Result<DownloadStatus, AppError> {
        let entry = entry_by_id(model_id)?;
        tracing::info!(repo = %entry.repo, model_id = %entry.id, "installing AI model package");
        self.update_status(|status| *status = DownloadStatus::queued(&entry))?;
        self.set_state(DownloadState::Resolving, None)?;

        let token = CancellationToken::new();
        *self
            .cancel_token
            .lock()
            .map_err(|_| AppError::internal("download cancel lock poisoned"))? =
            Some(token.clone());

        let result = self.download_entry_inner(entry, token).await;
        *self
            .cancel_token
            .lock()
            .map_err(|_| AppError::internal("download cancel lock poisoned"))? = None;

        match result {
            Ok(status) => {
                tracing::info!("AI model package installed");
                Ok(status)
            }
            Err(error) => {
                let message = error.to_string();
                tracing::warn!(%message, "AI model package install failed");
                if !matches!(error, AppError::InferenceCancelled(_)) {
                    let _ = self.set_state(DownloadState::Failed, Some(message.clone()));
                }
                Err(match error {
                    AppError::InferenceCancelled(_) => error,
                    _ => AppError::ModelDownloadFailed(message),
                })
            }
        }
    }

    async fn download_entry_inner(
        &self,
        entry: ModelCatalogEntry,
        token: CancellationToken,
    ) -> Result<DownloadStatus, AppError> {
        let metadata = self.resolve_model_metadata(&entry).await?;
        let download = entry.download.as_ref().ok_or_else(|| {
            AppError::ModelUnsupported(format!(
                "{} has no downloadable file configured",
                entry.name
            ))
        })?;
        let model_dir = self.root.resolve_relative(&download.destination_dir)?;
        fs::create_dir_all(&model_dir)?;
        ensure_contained(self.root.root(), &model_dir)?;

        // The catalog size covers the whole package (main file plus extras).
        let package_size = entry.size_bytes.max(metadata.size_bytes);
        let (final_path, verification) = self
            .download_primary_file(&metadata, &model_dir, package_size, &token)
            .await?;
        self.write_manifest(&model_dir, &metadata, &final_path, verification)?;

        if !download.dependencies.is_empty() {
            self.update_status(|status| {
                status.state = DownloadState::Resolving;
                status.filename = Some("model package dependencies".to_string());
                status.downloaded_bytes = 0;
                status.total_bytes = None;
                status.percentage = None;
                status.speed_bytes_per_sec = None;
                status.error = None;
            })?;
            model_package::ensure_dependencies(&self.root, &self.client, &entry, &token).await?;
        }

        if token.is_cancelled() {
            return Err(AppError::InferenceCancelled("download stopped".to_string()));
        }

        self.update_status(|status| {
            status.state = DownloadState::Completed;
            status.filename = Some(metadata.filename.clone());
            status.downloaded_bytes = metadata.size_bytes;
            status.total_bytes = Some(metadata.size_bytes);
            status.percentage = Some(100.0);
            status.speed_bytes_per_sec = None;
            status.destination = Some(final_path.display().to_string());
            status.error = None;
        })?;
        self.status()
    }

    async fn download_primary_file(
        &self,
        metadata: &QwenModelManifest,
        model_dir: &Path,
        package_size: u64,
        token: &CancellationToken,
    ) -> Result<(PathBuf, VerificationState), AppError> {
        let filename = metadata.filename.clone();
        let temp_dir = self.root.resolve_relative("temp/downloads")?;
        fs::create_dir_all(&temp_dir)?;
        ensure_contained(self.root.root(), &temp_dir)?;

        let final_path = model_dir.join(&filename);
        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent)?;
            ensure_contained(self.root.root(), parent)?;
        }
        let part_path = temp_dir.join(safe_part_filename(&filename));
        ensure_contained(self.root.root(), &final_path)?;
        ensure_contained(self.root.root(), &part_path)?;

        if final_path.exists() {
            let verification =
                verify_existing_file(&final_path, metadata.size_bytes, metadata.sha256.as_deref())?;
            if let Some(verification) = verification {
                self.update_status(|status| {
                    status.state = DownloadState::Verifying;
                    status.filename = Some(filename.clone());
                    status.downloaded_bytes = metadata.size_bytes;
                    status.total_bytes = Some(metadata.size_bytes);
                    status.percentage = Some(100.0);
                    status.speed_bytes_per_sec = None;
                    status.destination = Some(final_path.display().to_string());
                    status.error = None;
                })?;
                return Ok((final_path, verification));
            }

            tracing::warn!(
                path = %final_path.display(),
                "existing model file failed verification, re-downloading"
            );
            fs::remove_file(&final_path)?;
        }

        // Refuse before the first byte when the whole package cannot fit. Bytes
        // of an interrupted download are already on disk and are not needed twice.
        let partial_bytes = fs::metadata(&part_path)
            .map(|meta| meta.len())
            .unwrap_or(0)
            .min(metadata.size_bytes);
        validate_free_space(
            model_dir,
            package_space_required(package_size, partial_bytes),
        )?;

        match prepare_partial_download(&part_path, metadata.size_bytes, metadata.sha256.as_deref())?
        {
            PartialDownloadState::Complete { verification, .. } => {
                fs::rename(&part_path, &final_path)?;
                tracing::info!(
                    path = %final_path.display(),
                    "recovered complete verified model partial without re-downloading"
                );
                return Ok((final_path, verification));
            }
            PartialDownloadState::Resume(_) | PartialDownloadState::Fresh => {}
        }

        self.update_status(|status| {
            status.state = DownloadState::Downloading;
            status.filename = Some(filename.clone());
            status.total_bytes = Some(metadata.size_bytes);
            status.destination = Some(final_path.display().to_string());
            status.error = None;
        })?;
        crate::net::download_resumable(
            &self.client,
            &metadata.source_url,
            &part_path,
            Some(metadata.size_bytes),
            Some(token),
            |progress| {
                let _ = self.update_status(|status| {
                    status.downloaded_bytes = progress.downloaded;
                    status.percentage =
                        Some((progress.downloaded as f64 / metadata.size_bytes as f64) * 100.0);
                    status.speed_bytes_per_sec = Some(progress.bytes_per_sec);
                });
            },
        )
        .await
        .map_err(|error| {
            if error.is_cancelled() {
                AppError::InferenceCancelled("download stopped".to_string())
            } else {
                AppError::ModelDownloadFailed(format!("model download failed: {error}"))
            }
        })?;

        self.set_state(DownloadState::Verifying, None)?;
        let part_size = fs::metadata(&part_path)?.len();
        if part_size != metadata.size_bytes {
            let _ = fs::remove_file(&part_path);
            return Err(AppError::ModelDownloadFailed(format!(
                "downloaded size {part_size} did not match expected {}",
                metadata.size_bytes
            )));
        }

        let verification = if let Some(expected) = metadata.sha256.as_deref() {
            let actual = sha256_file(&part_path)?;
            if actual.eq_ignore_ascii_case(expected) {
                VerificationState::Verified
            } else {
                let _ = fs::remove_file(&part_path);
                return Err(AppError::ModelChecksumFailed(format!(
                    "expected {expected}, got {actual}"
                )));
            }
        } else {
            VerificationState::Unverified
        };

        fs::rename(&part_path, &final_path)?;
        Ok((final_path, verification))
    }

    async fn resolve_model_metadata(
        &self,
        entry: &ModelCatalogEntry,
    ) -> Result<QwenModelManifest, AppError> {
        let repo = &entry.repo;
        let quantization = &entry.quantization;
        let download = entry.download.as_ref().ok_or_else(|| {
            AppError::ModelUnsupported(format!(
                "{} has no downloadable file configured",
                entry.name
            ))
        })?;
        let api_url = format!("https://huggingface.co/api/models/{repo}?blobs=true");
        let model: HuggingFaceModel =
            crate::net::send_with_retry(|| self.client.get(&api_url), None)
                .await
                .map_err(|error| AppError::ModelDownloadFailed(error.to_string()))?
                .error_for_status()
                .map_err(|error| AppError::ModelDownloadFailed(error.to_string()))?
                .json()
                .await
                .map_err(|error| AppError::ModelDownloadFailed(error.to_string()))?;

        let sibling = select_sibling(&model.siblings, download).ok_or_else(|| {
            AppError::ModelDownloadFailed(format!(
                "no file matching {} found in {repo}",
                download.filename_pattern
            ))
        })?;
        let filename = sibling.rfilename.clone();
        let source_url = format!("https://huggingface.co/{repo}/resolve/main/{filename}");
        let size_bytes = sibling
            .lfs
            .as_ref()
            .and_then(|lfs| lfs.size)
            .or(sibling.size)
            .or_else(|| model.gguf.as_ref().and_then(|gguf| gguf.total_file_size))
            .unwrap_or(0);
        if size_bytes == 0 {
            return Err(AppError::ModelDownloadFailed(
                "official model size missing from Hugging Face metadata".to_string(),
            ));
        }

        Ok(QwenModelManifest {
            repo: repo.clone(),
            repo_sha: model.sha,
            quantization: quantization.clone(),
            filename,
            size_bytes,
            sha256: sibling.lfs.as_ref().and_then(|lfs| lfs.sha256.clone()),
            actual_sha256: None,
            verification: VerificationState::Unverified,
            architecture: model
                .gguf
                .as_ref()
                .and_then(|gguf| gguf.architecture.clone()),
            context_length: model.gguf.as_ref().and_then(|gguf| gguf.context_length),
            chat_template_available: model
                .gguf
                .as_ref()
                .and_then(|gguf| gguf.chat_template.as_ref())
                .is_some_and(|template| !template.trim().is_empty()),
            source_url,
            installed_at: Utc::now().to_rfc3339(),
        })
    }

    fn write_manifest(
        &self,
        model_dir: &Path,
        metadata: &QwenModelManifest,
        model_path: &Path,
        verification: VerificationState,
    ) -> Result<(), AppError> {
        let mut manifest = metadata.clone();
        manifest.actual_sha256 = Some(sha256_file(model_path)?);
        manifest.verification = verification;
        fs::write(
            model_dir.join("model-manifest.json"),
            serde_json::to_string_pretty(&manifest)
                .map_err(|error| AppError::internal(error.to_string()))?,
        )?;
        Ok(())
    }

    fn set_state(&self, state: DownloadState, error: Option<String>) -> Result<(), AppError> {
        self.update_status(|status| {
            status.state = state;
            status.error = error;
        })
    }

    fn update_status(&self, update: impl FnOnce(&mut DownloadStatus)) -> Result<(), AppError> {
        let mut status = self
            .status
            .lock()
            .map_err(|_| AppError::internal("download status lock poisoned"))?;
        update(&mut status);
        Ok(())
    }
}

impl DownloadStatus {
    fn queued(catalog_entry: &ModelCatalogEntry) -> Self {
        Self {
            model_id: catalog_entry.id.clone(),
            name: catalog_entry.name.clone(),
            state: DownloadState::Queued,
            repo: catalog_entry.repo.clone(),
            quantization: catalog_entry.quantization.clone(),
            filename: None,
            downloaded_bytes: 0,
            total_bytes: None,
            percentage: None,
            speed_bytes_per_sec: None,
            destination: None,
            error: None,
        }
    }
}

pub(crate) fn safe_part_filename(filename: &str) -> String {
    let flattened = filename
        .chars()
        .map(|character| {
            if matches!(character, '/' | '\\') {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    format!("{flattened}.part")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PartialDownloadState {
    Fresh,
    Resume(u64),
    Complete {
        verification: VerificationState,
        actual_sha256: String,
    },
}

pub(crate) fn prepare_partial_download(
    path: &Path,
    expected_size: u64,
    expected_sha256: Option<&str>,
) -> Result<PartialDownloadState, AppError> {
    let size = match fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PartialDownloadState::Fresh)
        }
        Err(error) => return Err(error.into()),
    };

    if size < expected_size {
        return Ok(PartialDownloadState::Resume(size));
    }
    if size > expected_size {
        tracing::warn!(
            path = %path.display(),
            size,
            expected_size,
            "discarding oversized stale partial download"
        );
        fs::remove_file(path)?;
        return Ok(PartialDownloadState::Fresh);
    }

    let actual_sha256 = sha256_file(path)?;
    let verification = match expected_sha256 {
        Some(expected) if actual_sha256.eq_ignore_ascii_case(expected) => {
            VerificationState::Verified
        }
        Some(expected) => {
            tracing::warn!(
                path = %path.display(),
                expected,
                actual = %actual_sha256,
                "discarding complete partial download with checksum mismatch"
            );
            fs::remove_file(path)?;
            return Ok(PartialDownloadState::Fresh);
        }
        None => VerificationState::Unverified,
    };

    Ok(PartialDownloadState::Complete {
        verification,
        actual_sha256,
    })
}

fn select_sibling<'a>(
    siblings: &'a [HuggingFaceSibling],
    download: &ModelCatalogDownload,
) -> Option<&'a HuggingFaceSibling> {
    siblings
        .iter()
        .filter(|sibling| wildcard_match(&download.filename_pattern, &sibling.rfilename))
        .max_by_key(|sibling| {
            sibling
                .lfs
                .as_ref()
                .and_then(|lfs| lfs.size)
                .or(sibling.size)
                .unwrap_or(0)
        })
}

pub fn validate_gguf_header(path: &Path, root: &PortableRootManager) -> Result<(), AppError> {
    let canonical = fs::canonicalize(path)?;
    let canonical_root = fs::canonicalize(root.root())?;
    if !canonical.starts_with(&canonical_root) {
        return Err(AppError::ModelInvalid(
            "model path escapes OpenMindAI Root".to_string(),
        ));
    }
    let metadata = fs::metadata(&canonical)?;
    if metadata.len() < 32 * 1024 * 1024 {
        return Err(AppError::ModelInvalid(
            "GGUF model is implausibly small".to_string(),
        ));
    }
    let mut file = fs::File::open(canonical)?;
    let mut magic = [0_u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != b"GGUF" {
        return Err(AppError::ModelInvalid(
            "GGUF magic/header invalid".to_string(),
        ));
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(())
}

fn verify_existing_file(
    path: &Path,
    expected_size: u64,
    expected_sha256: Option<&str>,
) -> Result<Option<VerificationState>, AppError> {
    let size_matches = fs::metadata(path)
        .map(|meta| meta.len() == expected_size)
        .unwrap_or(false);
    if !size_matches {
        return Ok(None);
    }
    let Some(expected) = expected_sha256 else {
        return Ok(Some(VerificationState::Unverified));
    };
    let actual = sha256_file(path)?;
    Ok(if actual.eq_ignore_ascii_case(expected) {
        Some(VerificationState::Verified)
    } else {
        None
    })
}

pub(crate) fn sha256_file(path: &Path) -> Result<String, AppError> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 128];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub(crate) fn ensure_contained(root: &Path, path: &Path) -> Result<(), AppError> {
    let canonical_root = fs::canonicalize(root)?;
    let candidate = if path.exists() {
        fs::canonicalize(path)?
    } else {
        let parent = path
            .parent()
            .ok_or_else(|| AppError::ModelInvalid("path has no parent".to_string()))?;
        fs::canonicalize(parent)?
    };
    if !candidate.starts_with(canonical_root) {
        return Err(AppError::ModelInvalid(
            "model destination escapes OpenMindAI Root".to_string(),
        ));
    }
    Ok(())
}

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

/// Free space a package install needs: what is still to be downloaded plus the
/// safety margin for temporary files.
fn package_space_required(package_size: u64, partial_bytes: u64) -> u64 {
    package_size
        .saturating_sub(partial_bytes)
        .saturating_add(SAFE_SPACE_MARGIN)
}

/// Human-readable size, e.g. "7.9 GiB" or "512 MiB".
pub(crate) fn format_bytes(bytes: u64) -> String {
    format_bytes_with(bytes, 1)
}

/// Formats two sizes that are being compared with just enough precision that
/// different values never print the same: "7.98 GiB" vs "8.00 GiB" rather than
/// "8.0 GiB" vs "8.0 GiB". Comparisons must still use the raw byte values.
pub(crate) fn format_byte_pair(first: u64, second: u64) -> (String, String) {
    for decimals in 1..=3 {
        let pair = (
            format_bytes_with(first, decimals),
            format_bytes_with(second, decimals),
        );
        if first == second || pair.0 != pair.1 {
            return pair;
        }
    }
    (format!("{first} bytes"), format!("{second} bytes"))
}

fn format_bytes_with(bytes: u64, decimals: usize) -> String {
    if bytes >= GIB {
        format!("{:.decimals$} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!(
            "{:.precision$} MiB",
            bytes as f64 / MIB as f64,
            precision = decimals.saturating_sub(1)
        )
    } else {
        format!("{bytes} bytes")
    }
}

pub(crate) fn validate_free_space(destination: &Path, required: u64) -> Result<(), AppError> {
    let disks = Disks::new_with_refreshed_list();
    let canonical_destination =
        crate::portable_root::strip_windows_verbatim_prefix(&fs::canonicalize(destination)?);
    let available = disks
        .iter()
        .filter(|disk| canonical_destination.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().as_os_str().len())
        .map(|disk| disk.available_space());
    if let Some(available) = available {
        if available < required {
            let (available, required) = format_byte_pair(available, required);
            return Err(AppError::InsufficientStorage(format!(
                "not enough free disk space: need {required} free (including a safety margin for temporary files), have {available}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{thread, time::Duration};

    #[test]
    fn byte_pairs_show_enough_precision_to_differ() {
        let at = |gib: f64| (gib * GIB as f64) as u64;
        let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(
            format_byte_pair(at(7.98), 8 * GIB),
            pair("7.98 GiB", "8.00 GiB")
        );
        assert_eq!(
            format_byte_pair(at(7.94), 8 * GIB),
            pair("7.9 GiB", "8.0 GiB")
        );
        assert_eq!(
            format_byte_pair(8 * GIB, 8 * GIB),
            pair("8.0 GiB", "8.0 GiB")
        );
        assert_eq!(
            format_byte_pair(at(8.01), 8 * GIB),
            pair("8.01 GiB", "8.00 GiB")
        );
        // Even a one-byte difference never prints as two equal sizes.
        let (low, high) = format_byte_pair(8 * GIB - 1, 8 * GIB);
        assert_ne!(low, high);
        assert_eq!(format_bytes(512 * MIB), "512 MiB");
    }

    #[test]
    fn canvas_install_is_refused_up_front_on_a_nearly_full_drive() {
        let canvas = entry_by_id("sdxl-base-1").unwrap();
        let available = 5_640 * MIB; // G: during the live test
        let required = package_space_required(canvas.size_bytes, 0);
        assert!(required > available, "{required} vs {available}");
        assert!(required >= canvas.size_bytes + SAFE_SPACE_MARGIN);

        // Resuming counts the bytes already on disk.
        let resumed = package_space_required(canvas.size_bytes, 6 * GIB);
        assert_eq!(resumed, canvas.size_bytes - 6 * GIB + SAFE_SPACE_MARGIN);
        assert!(resumed < available);
    }

    #[test]
    fn insufficient_disk_space_is_refused_with_readable_sizes() {
        let temp = tempfile::tempdir().unwrap();
        let error = validate_free_space(temp.path(), u64::MAX / 4)
            .unwrap_err()
            .to_string();
        assert!(error.contains("not enough free disk space"), "{error}");
        assert!(error.contains("GiB free"), "{error}");
        assert!(!error.contains(" bytes free"), "{error}");
    }

    #[test]
    fn rejects_gguf_outside_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("root"));
        root.ensure_directories().unwrap();
        let outside = temp.path().join("outside.gguf");
        fs::write(&outside, b"GGUF").unwrap();

        let err = validate_gguf_header(&outside, &root).unwrap_err();
        assert!(matches!(err, AppError::ModelInvalid(_)));
    }

    #[test]
    fn validates_part_destination_under_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("root"));
        root.ensure_directories().unwrap();
        let destination = root
            .resolve_relative("models/llm/qwen/qwen3-4b/model.gguf.part")
            .unwrap();
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        assert!(ensure_contained(root.root(), &destination).is_ok());
    }

    /// The SDXL repo also holds a larger diffusers-format UNet. Canvas must
    /// download the single-file checkpoint stable-diffusion.cpp loads.
    #[test]
    fn canvas_selects_single_file_sdxl_checkpoint_not_diffusers_unet() {
        let sibling = |name: &str, size: u64| HuggingFaceSibling {
            rfilename: name.to_string(),
            size: None,
            lfs: Some(HuggingFaceLfs {
                sha256: None,
                size: Some(size),
            }),
        };
        let siblings = vec![
            sibling("unet/diffusion_pytorch_model.safetensors", 10_270_077_736),
            sibling("sd_xl_base_1.0.safetensors", 6_938_078_334),
            sibling("sd_xl_base_1.0_0.9vae.safetensors", 6_938_078_334),
            sibling(
                "unet/diffusion_pytorch_model.fp16.safetensors",
                5_135_149_760,
            ),
            sibling("vae/diffusion_pytorch_model.safetensors", 334_643_268),
        ];
        let entry = entry_by_id("sdxl-base-1").unwrap();
        let selected = select_sibling(&siblings, entry.download.as_ref().unwrap()).unwrap();
        assert_eq!(selected.rfilename, "sd_xl_base_1.0_0.9vae.safetensors");
    }

    #[test]
    fn nested_hugging_face_paths_get_safe_partial_names() {
        assert_eq!(
            safe_part_filename("split_files/diffusion_models/model.safetensors"),
            "split_files_diffusion_models_model.safetensors.part"
        );
        assert_eq!(
            safe_part_filename("onnx\\model.onnx"),
            "onnx_model.onnx.part"
        );
    }

    #[test]
    fn verify_existing_file_accepts_matching_checksum() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("model.gguf");
        fs::write(&path, b"hello world").unwrap();
        let expected = sha256_file(&path).unwrap();

        let result = verify_existing_file(&path, 11, Some(&expected)).unwrap();
        assert_eq!(result, Some(VerificationState::Verified));
    }

    #[test]
    fn verify_existing_file_rejects_checksum_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("model.gguf");
        fs::write(&path, b"hello world").unwrap();

        let result = verify_existing_file(&path, 11, Some("not-the-real-hash")).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn verify_existing_file_rejects_size_mismatch_without_hashing() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("model.gguf");
        fs::write(&path, b"truncated").unwrap();

        let result = verify_existing_file(&path, 999_999, Some("irrelevant")).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn verify_existing_file_accepts_matching_size_without_checksum() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("model.gguf");
        fs::write(&path, b"hello world").unwrap();

        let result = verify_existing_file(&path, 11, None).unwrap();
        assert_eq!(result, Some(VerificationState::Unverified));
    }

    #[test]
    fn partial_download_resumes_when_smaller_than_expected() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("model.part");
        fs::write(&path, b"partial").unwrap();

        let state = prepare_partial_download(&path, 100, Some("unused")).unwrap();
        assert_eq!(state, PartialDownloadState::Resume(7));
        assert!(path.exists());
    }

    #[test]
    fn partial_download_recovers_complete_matching_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("model.part");
        fs::write(&path, b"complete payload").unwrap();
        let expected = sha256_file(&path).unwrap();

        let state = prepare_partial_download(&path, 16, Some(&expected)).unwrap();
        assert_eq!(
            state,
            PartialDownloadState::Complete {
                verification: VerificationState::Verified,
                actual_sha256: expected,
            }
        );
        assert!(path.exists());
    }

    #[test]
    fn partial_download_discards_complete_checksum_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("model.part");
        fs::write(&path, b"complete payload").unwrap();

        let state = prepare_partial_download(&path, 16, Some("wrong-checksum")).unwrap();
        assert_eq!(state, PartialDownloadState::Fresh);
        assert!(!path.exists());
    }

    #[test]
    fn partial_download_discards_oversized_stale_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("model.part");
        fs::write(&path, b"too-large").unwrap();

        let state = prepare_partial_download(&path, 4, None).unwrap();
        assert_eq!(state, PartialDownloadState::Fresh);
        assert!(!path.exists());
    }

    #[test]
    fn partial_download_accepts_complete_unhashed_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("model.part");
        fs::write(&path, b"payload").unwrap();
        let actual = sha256_file(&path).unwrap();

        let state = prepare_partial_download(&path, 7, None).unwrap();
        assert_eq!(
            state,
            PartialDownloadState::Complete {
                verification: VerificationState::Unverified,
                actual_sha256: actual,
            }
        );
        assert!(path.exists());
    }

    #[test]
    #[ignore = "downloads the real official Qwen3 4B GGUF"]
    fn real_qwen_download_cancel_resume_complete() {
        let root = PortableRootManager::resolve().unwrap();
        root.ensure_directories().unwrap();
        let manager = ModelDownloadManager::new(root.clone());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let task_manager = manager.clone();

        let handle = thread::spawn(move || {
            runtime.block_on(async { task_manager.download_qwen_q4_k_m().await })
        });

        let mut observed_progress = 0;
        for _ in 0..240 {
            let status = manager.status().unwrap();
            observed_progress = observed_progress.max(status.downloaded_bytes);
            if status.downloaded_bytes >= 64 * 1024 * 1024 {
                manager.cancel().unwrap();
                break;
            }
            thread::sleep(Duration::from_secs(1));
        }
        let _ = handle.join();

        let metadata = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { manager.resolve_model_metadata(&manager.catalog_entry).await })
            .unwrap();
        let part_path = root
            .resolve_relative(format!("temp/downloads/{}.part", metadata.filename))
            .unwrap();
        let final_path = root
            .resolve_relative(format!("models/llm/qwen/qwen3-4b/{}", metadata.filename))
            .unwrap();
        if observed_progress > 0 && !final_path.exists() {
            assert!(part_path.exists());
            assert!(fs::metadata(&part_path).unwrap().len() > 0);
        }

        let final_status = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { manager.download_qwen_q4_k_m().await })
            .unwrap();
        assert_eq!(final_status.state, DownloadState::Completed);
        assert_eq!(final_status.downloaded_bytes, metadata.size_bytes);
        assert!(!part_path.exists());

        validate_gguf_header(&final_path, &root).unwrap();
        assert_eq!(
            fs::metadata(&final_path).unwrap().len(),
            metadata.size_bytes
        );
        if let Some(expected) = metadata.sha256.as_deref() {
            assert_eq!(sha256_file(&final_path).unwrap(), expected);
        }
    }
}
