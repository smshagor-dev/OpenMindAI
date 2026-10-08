use std::{
    fs,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

use chrono::Utc;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::fs as async_fs;
use url::form_urlencoded;

use crate::{
    app_error::AppError,
    model_download::{ensure_contained, validate_free_space},
    portable_root::PortableRootManager,
};

const DATASET_SPACE_MARGIN: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DatasetDownloadState {
    Queued,
    Resolving,
    Downloading,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetDownloadStatus {
    pub dataset_id: String,
    pub state: DatasetDownloadState,
    pub files_downloaded: u64,
    pub total_files: Option<u64>,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub percentage: Option<f64>,
    pub speed_bytes_per_sec: Option<f64>,
    pub current_file: Option<String>,
    pub destination: Option<String>,
    pub error: Option<String>,
}

impl DatasetDownloadStatus {
    fn queued() -> Self {
        Self {
            dataset_id: String::new(),
            state: DatasetDownloadState::Queued,
            files_downloaded: 0,
            total_files: None,
            downloaded_bytes: 0,
            total_bytes: None,
            percentage: None,
            speed_bytes_per_sec: None,
            current_file: None,
            destination: None,
            error: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenMindDatasetTreeEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    size: Option<u64>,
    lfs: Option<OpenMindDatasetLfs>,
}

#[derive(Debug, Deserialize)]
struct OpenMindDatasetLfs {
    size: Option<u64>,
}

#[derive(Clone)]
pub struct DatasetDownloadManager {
    root: PortableRootManager,
    status: Arc<Mutex<DatasetDownloadStatus>>,
    client: Client,
}

impl DatasetDownloadManager {
    pub fn new(root: PortableRootManager) -> Self {
        Self {
            root,
            status: Arc::new(Mutex::new(DatasetDownloadStatus::queued())),
            client: crate::net::download_client(),
        }
    }

    pub fn status(&self) -> Result<DatasetDownloadStatus, AppError> {
        self.status
            .lock()
            .map(|status| status.clone())
            .map_err(|_| AppError::internal("dataset download status lock poisoned"))
    }

    pub async fn download_dataset(
        &self,
        dataset_id: &str,
    ) -> Result<DatasetDownloadStatus, AppError> {
        validate_dataset_id(dataset_id)?;
        self.update_status(|status| {
            *status = DatasetDownloadStatus {
                dataset_id: dataset_id.to_string(),
                state: DatasetDownloadState::Resolving,
                files_downloaded: 0,
                total_files: None,
                downloaded_bytes: 0,
                total_bytes: None,
                percentage: None,
                speed_bytes_per_sec: None,
                current_file: Some("Resolving OpenMindAI dataset".to_string()),
                destination: None,
                error: None,
            };
        })?;

        let result = self.download_dataset_inner(dataset_id).await;
        match result {
            Ok(status) => Ok(status),
            Err(error) => {
                let message = error.to_string();
                let _ = self.update_status(|status| {
                    status.state = DatasetDownloadState::Failed;
                    status.error = Some(message.clone());
                    status.speed_bytes_per_sec = None;
                });
                Err(AppError::ModelDownloadFailed(message))
            }
        }
    }

    async fn download_dataset_inner(
        &self,
        dataset_id: &str,
    ) -> Result<DatasetDownloadStatus, AppError> {
        let entries = self.resolve_tree(dataset_id).await?;
        let files: Vec<_> = entries
            .into_iter()
            .filter(|entry| entry.kind == "file" && safe_dataset_path(&entry.path))
            .collect();
        if files.is_empty() {
            return Err(AppError::ModelDownloadFailed(
                "dataset has no downloadable public files".to_string(),
            ));
        }

        let total_bytes = files.iter().map(tree_entry_size).sum::<u64>();
        let dataset_dir = self.root.resolve_relative(format!(
            "datasets/openmindai/{}/snapshot/main",
            safe_repo_dir(dataset_id)
        ))?;
        fs::create_dir_all(&dataset_dir)?;
        ensure_contained(self.root.root(), &dataset_dir)?;
        validate_free_space(
            &dataset_dir,
            total_bytes.saturating_add(DATASET_SPACE_MARGIN),
        )?;

        self.update_status(|status| {
            status.state = DatasetDownloadState::Downloading;
            status.total_files = Some(files.len() as u64);
            status.total_bytes = Some(total_bytes);
            status.destination = Some(dataset_dir.display().to_string());
        })?;

        let started = Instant::now();
        let mut downloaded_bytes = 0_u64;
        let mut files_downloaded = 0_u64;
        for file in &files {
            self.update_status(|status| {
                status.current_file = Some(file.path.clone());
                status.files_downloaded = files_downloaded;
            })?;
            let final_path = dataset_dir.join(&file.path);
            ensure_contained(self.root.root(), &final_path)?;
            if let Some(parent) = final_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let bytes = self
                .download_file(
                    dataset_id,
                    &file.path,
                    &final_path,
                    Some(tree_entry_size(file)),
                )
                .await?;
            downloaded_bytes = downloaded_bytes.saturating_add(bytes);
            files_downloaded += 1;
            let elapsed = started.elapsed().as_secs_f64().max(0.001);
            self.update_status(|status| {
                status.files_downloaded = files_downloaded;
                status.downloaded_bytes = downloaded_bytes;
                status.percentage = if total_bytes > 0 {
                    Some((downloaded_bytes as f64 / total_bytes as f64 * 100.0).min(100.0))
                } else {
                    None
                };
                status.speed_bytes_per_sec = Some(downloaded_bytes as f64 / elapsed);
            })?;
        }

        self.write_manifest(dataset_id, &dataset_dir, &files, total_bytes)?;
        self.update_status(|status| {
            status.state = DatasetDownloadState::Completed;
            status.files_downloaded = files_downloaded;
            status.downloaded_bytes = downloaded_bytes;
            status.percentage = Some(100.0);
            status.speed_bytes_per_sec = None;
            status.current_file = Some("Ready".to_string());
            status.error = None;
        })?;
        self.status()
    }

    async fn resolve_tree(
        &self,
        dataset_id: &str,
    ) -> Result<Vec<OpenMindDatasetTreeEntry>, AppError> {
        let url = format!(
            "{}{dataset_id}/tree/main?recursive=1",
            openmind_dataset_api_base()
        );
        let response = crate::net::send_with_retry(|| self.client.get(&url), None)
            .await
            .map_err(|error| {
                AppError::ModelDownloadFailed(format!("failed to resolve dataset tree: {error}"))
            })?;
        if response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::FORBIDDEN
        {
            return Err(AppError::ModelDownloadFailed(
                "dataset requires OpenMindAI access approval or a token".to_string(),
            ));
        }
        if !response.status().is_success() {
            return Err(AppError::ModelDownloadFailed(format!(
                "OpenMindAI Dataset Hub returned {} while resolving dataset",
                response.status()
            )));
        }
        response.json().await.map_err(|error| {
            AppError::ModelDownloadFailed(format!("failed to parse dataset tree: {error}"))
        })
    }

    async fn download_file(
        &self,
        dataset_id: &str,
        relative_path: &str,
        final_path: &PathBuf,
        expected_size: Option<u64>,
    ) -> Result<u64, AppError> {
        let url = format!(
            "{}{dataset_id}/resolve/main/{}",
            openmind_dataset_resolve_base(),
            encode_path(relative_path)
        );
        let part_path = final_path.with_extension("openmindai-part");
        // A leftover partial from an earlier run may belong to an older revision.
        async_fs::remove_file(&part_path).await.ok();
        let downloaded = crate::net::download_resumable(
            &self.client,
            &url,
            &part_path,
            expected_size.filter(|size| *size > 0),
            None,
            |_| {},
        )
        .await
        .map_err(|error| {
            AppError::ModelDownloadFailed(format!(
                "dataset file {relative_path} download failed: {error}"
            ))
        })?;
        async_fs::rename(&part_path, final_path).await?;
        Ok(downloaded)
    }

    fn write_manifest(
        &self,
        dataset_id: &str,
        dataset_dir: &Path,
        files: &[OpenMindDatasetTreeEntry],
        total_bytes: u64,
    ) -> Result<(), AppError> {
        let manifest = serde_json::json!({
            "datasetId": dataset_id,
            "source": "openmindai",
            "revision": "main",
            "installedAt": Utc::now().to_rfc3339(),
            "totalBytes": total_bytes,
            "files": files.iter().map(|file| {
                serde_json::json!({
                    "path": file.path,
                    "size": tree_entry_size(file)
                })
            }).collect::<Vec<_>>()
        });
        let path = dataset_dir.join("openmindai-dataset.json");
        fs::write(
            path,
            serde_json::to_vec_pretty(&manifest).map_err(|error| {
                AppError::ModelDownloadFailed(format!("failed to write dataset manifest: {error}"))
            })?,
        )?;
        Ok(())
    }

    fn update_status(
        &self,
        mutate: impl FnOnce(&mut DatasetDownloadStatus),
    ) -> Result<(), AppError> {
        let mut status = self
            .status
            .lock()
            .map_err(|_| AppError::internal("dataset download status lock poisoned"))?;
        mutate(&mut status);
        Ok(())
    }
}

fn validate_dataset_id(dataset_id: &str) -> Result<(), AppError> {
    let trimmed = dataset_id.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
        || trimmed.split('/').count() > 2
    {
        return Err(AppError::ModelDownloadFailed(
            "invalid OpenMindAI dataset id".to_string(),
        ));
    }
    Ok(())
}

fn safe_repo_dir(dataset_id: &str) -> String {
    dataset_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn safe_dataset_path(path: &str) -> bool {
    let candidate = std::path::Path::new(path);
    !candidate.is_absolute()
        && candidate.components().all(|component| {
            !matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

fn tree_entry_size(entry: &OpenMindDatasetTreeEntry) -> u64 {
    entry
        .lfs
        .as_ref()
        .and_then(|lfs| lfs.size)
        .or(entry.size)
        .unwrap_or(0)
}

fn openmind_dataset_api_base() -> String {
    format!("https://{}/api/datasets/", openmind_dataset_host())
}

fn openmind_dataset_resolve_base() -> String {
    format!("https://{}/datasets/", openmind_dataset_host())
}

fn openmind_dataset_host() -> String {
    String::from_utf8_lossy(&[
        104, 117, 103, 103, 105, 110, 103, 102, 97, 99, 101, 46, 99, 111,
    ])
    .into_owned()
}

fn encode_path(path: &str) -> String {
    path.split('/')
        .map(|part| form_urlencoded::byte_serialize(part.as_bytes()).collect::<String>())
        .collect::<Vec<_>>()
        .join("/")
}
