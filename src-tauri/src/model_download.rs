use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

use chrono::Utc;
use futures_util::StreamExt;
use reqwest::{header, Client, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sysinfo::Disks;
use tokio::{fs as async_fs, io::AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::{
    app_error::AppError,
    model_catalog::{entry_by_id, wildcard_match, ModelCatalogDownload, ModelCatalogEntry},
    portable_root::PortableRootManager,
};

#[path = "model_package.rs"]
mod model_package;

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
            client: Client::new(),
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

        let (final_path, verification) = self
            .download_primary_file(&metadata, &model_dir, &token)
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
        token: &CancellationToken,
    ) -> Result<(PathBuf, VerificationState), AppError> {
        let filename = metadata.filename.clone();
        let temp_dir = self.root.resolve_relative("temp/downloads")?;
        fs::create_dir_all(&temp_dir)?;
        ensure_contained(self.root.root(), &temp_dir)?;

        let final_path = model_dir.join(&filename);
        let part_path = temp_dir.join(format!("{filename}.part"));
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

        validate_free_space(
            model_dir,
            metadata.size_bytes.saturating_add(SAFE_SPACE_MARGIN),
        )?;

        let mut existing = fs::metadata(&part_path).map(|meta| meta.len()).unwrap_or(0);
        let mut request = self.client.get(&metadata.source_url);
        if existing > 0 {
            request = request.header(header::RANGE, format!("bytes={existing}-"));
        }

        let response = request
            .send()
            .await
            .map_err(|error| AppError::ModelDownloadFailed(error.to_string()))?;
        let resumed = existing > 0 && response.status() == StatusCode::PARTIAL_CONTENT;
        if existing > 0 && !resumed {
            async_fs::remove_file(&part_path).await.ok();
            existing = 0;
        }
        if !response.status().is_success() {
            return Err(AppError::ModelDownloadFailed(format!(
                "HTTP {} while downloading model",
                response.status()
            )));
        }

        self.update_status(|status| {
            status.state = DownloadState::Downloading;
            status.filename = Some(filename.clone());
            status.downloaded_bytes = existing;
            status.total_bytes = Some(metadata.size_bytes);
            status.percentage = Some((existing as f64 / metadata.size_bytes as f64) * 100.0);
            status.destination = Some(final_path.display().to_string());
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
        let mut downloaded = existing;
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            if token.is_cancelled() {
                file.flush().await?;
                return Err(AppError::InferenceCancelled("download stopped".to_string()));
            }
            let chunk = chunk.map_err(|error| AppError::ModelDownloadFailed(error.to_string()))?;
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;
            let elapsed = started.elapsed().as_secs_f64().max(0.001);
            self.update_status(|status| {
                status.downloaded_bytes = downloaded;
                status.percentage = Some((downloaded as f64 / metadata.size_bytes as f64) * 100.0);
                status.speed_bytes_per_sec =
                    Some(downloaded.saturating_sub(existing) as f64 / elapsed);
            })?;
        }
        file.flush().await?;

        self.set_state(DownloadState::Verifying, None)?;
        let part_size = fs::metadata(&part_path)?.len();
        if part_size != metadata.size_bytes {
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
        let model: HuggingFaceModel = self
            .client
            .get(&api_url)
            .send()
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
            return Err(AppError::InsufficientStorage(format!(
                "need {} bytes free including safety margin, have {available}",
                required
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
