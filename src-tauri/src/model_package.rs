use std::{fs, path::Path};

use futures_util::StreamExt;
use reqwest::{header, Client, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::{fs as async_fs, io::AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::{
    app_error::AppError,
    model_catalog::{ModelCatalogDependency, ModelCatalogEntry},
    model_download::{
        ensure_contained, prepare_partial_download, safe_part_filename, sha256_file,
        validate_free_space, PartialDownloadState, VerificationState,
    },
    portable_root::PortableRootManager,
};

const PACKAGE_SPACE_MARGIN: u64 = 256 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct HuggingFaceModel {
    siblings: Vec<HuggingFaceSibling>,
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

#[derive(Debug, Clone)]
struct ResolvedDependency {
    role: String,
    filename: String,
    format: String,
    size_bytes: u64,
    sha256: Option<String>,
    source_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageFileManifest {
    pub role: String,
    pub filename: String,
    pub format: String,
    pub size_bytes: u64,
    pub sha256: Option<String>,
    pub actual_sha256: String,
    pub verification: VerificationState,
    pub source_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPackageManifest {
    pub model_id: String,
    pub repo: String,
    pub files: Vec<PackageFileManifest>,
}

pub async fn ensure_dependencies(
    root: &PortableRootManager,
    client: &Client,
    entry: &ModelCatalogEntry,
    cancellation: &CancellationToken,
) -> Result<(), AppError> {
    let Some(download) = entry.download.as_ref() else {
        return Ok(());
    };
    if download.dependencies.is_empty() {
        return Ok(());
    }

    let model_dir = root.resolve_relative(&download.destination_dir)?;
    fs::create_dir_all(&model_dir)?;
    ensure_contained(root.root(), &model_dir)?;

    if cancellation.is_cancelled() {
        return Err(AppError::InferenceCancelled("download stopped".to_string()));
    }

    let mut files = Vec::new();
    for dependency in &download.dependencies {
        if cancellation.is_cancelled() {
            return Err(AppError::InferenceCancelled("download stopped".to_string()));
        }
        let repo = dependency.repo.as_deref().unwrap_or(&entry.repo);
        let model = match fetch_model_metadata(client, repo).await {
            Ok(model) => model,
            Err(error) if !dependency.required => {
                tracing::warn!(role = %dependency.role, %repo, %error, "optional model package repository unavailable");
                continue;
            }
            Err(error) => return Err(error),
        };
        match resolve_dependency(repo, &model, dependency) {
            Ok(resolved) => {
                let installed =
                    download_dependency(root, client, &model_dir, &resolved, cancellation).await?;
                files.push(installed);
            }
            Err(error) if !dependency.required => {
                tracing::warn!(role = %dependency.role, %error, "optional model package dependency unavailable");
            }
            Err(error) => return Err(error),
        }
    }

    let manifest = ModelPackageManifest {
        model_id: entry.id.clone(),
        repo: entry.repo.clone(),
        files,
    };
    fs::write(
        model_dir.join("package-manifest.json"),
        serde_json::to_string_pretty(&manifest)
            .map_err(|error| AppError::internal(error.to_string()))?,
    )?;
    Ok(())
}

async fn fetch_model_metadata(client: &Client, repo: &str) -> Result<HuggingFaceModel, AppError> {
    let api_url = format!("https://huggingface.co/api/models/{repo}?blobs=true");
    client
        .get(api_url)
        .send()
        .await
        .map_err(|error| AppError::ModelDownloadFailed(error.to_string()))?
        .error_for_status()
        .map_err(|error| AppError::ModelDownloadFailed(error.to_string()))?
        .json()
        .await
        .map_err(|error| AppError::ModelDownloadFailed(error.to_string()))
}

fn resolve_dependency(
    repo: &str,
    model: &HuggingFaceModel,
    dependency: &ModelCatalogDependency,
) -> Result<ResolvedDependency, AppError> {
    let sibling = model
        .siblings
        .iter()
        .filter(|sibling| wildcard_match(&dependency.filename_pattern, &sibling.rfilename))
        .max_by_key(|sibling| {
            sibling
                .lfs
                .as_ref()
                .and_then(|lfs| lfs.size)
                .or(sibling.size)
                .unwrap_or(0)
        })
        .ok_or_else(|| {
            AppError::ModelDownloadFailed(format!(
                "required {} file matching {} was not found in {}",
                dependency.role, dependency.filename_pattern, repo
            ))
        })?;
    let size_bytes = sibling
        .lfs
        .as_ref()
        .and_then(|lfs| lfs.size)
        .or(sibling.size)
        .unwrap_or(0);
    if size_bytes == 0 {
        return Err(AppError::ModelDownloadFailed(format!(
            "official size is missing for {} dependency {}",
            dependency.role, sibling.rfilename
        )));
    }

    Ok(ResolvedDependency {
        role: dependency.role.clone(),
        filename: sibling.rfilename.clone(),
        format: dependency.format.clone(),
        size_bytes,
        sha256: sibling.lfs.as_ref().and_then(|lfs| lfs.sha256.clone()),
        source_url: format!(
            "https://huggingface.co/{repo}/resolve/main/{}",
            sibling.rfilename
        ),
    })
}

async fn download_dependency(
    root: &PortableRootManager,
    client: &Client,
    model_dir: &Path,
    dependency: &ResolvedDependency,
    cancellation: &CancellationToken,
) -> Result<PackageFileManifest, AppError> {
    let final_path = model_dir.join(&dependency.filename);
    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent)?;
        ensure_contained(root.root(), parent)?;
    }
    let temp_dir = root.resolve_relative("temp/downloads")?;
    fs::create_dir_all(&temp_dir)?;
    let part_path = temp_dir.join(safe_part_filename(&dependency.filename));
    ensure_contained(root.root(), &final_path)?;
    ensure_contained(root.root(), &part_path)?;

    if final_path.exists() {
        let size_matches = fs::metadata(&final_path)
            .map(|metadata| metadata.len() == dependency.size_bytes)
            .unwrap_or(false);
        if size_matches {
            let actual = sha256_file(&final_path)?;
            let verification = match dependency.sha256.as_deref() {
                Some(expected) if actual.eq_ignore_ascii_case(expected) => {
                    VerificationState::Verified
                }
                Some(_) => VerificationState::Failed,
                None => VerificationState::Unverified,
            };
            if verification != VerificationState::Failed {
                return Ok(package_manifest_entry(dependency, actual, verification));
            }
        }
        tracing::warn!(path = %final_path.display(), role = %dependency.role, "model package dependency failed verification; downloading again");
        fs::remove_file(&final_path)?;
    }

    validate_free_space(
        model_dir,
        dependency.size_bytes.saturating_add(PACKAGE_SPACE_MARGIN),
    )?;

    let existing = match prepare_partial_download(
        &part_path,
        dependency.size_bytes,
        dependency.sha256.as_deref(),
    )? {
        PartialDownloadState::Complete {
            verification,
            actual_sha256,
        } => {
            fs::rename(&part_path, &final_path)?;
            tracing::info!(
                path = %final_path.display(),
                role = %dependency.role,
                "recovered complete verified dependency partial without re-downloading"
            );
            return Ok(package_manifest_entry(
                dependency,
                actual_sha256,
                verification,
            ));
        }
        PartialDownloadState::Resume(bytes) => bytes,
        PartialDownloadState::Fresh => 0,
    };
    let mut request = client.get(&dependency.source_url);
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
    }
    if !response.status().is_success() {
        return Err(AppError::ModelDownloadFailed(format!(
            "HTTP {} while downloading {} dependency",
            response.status(),
            dependency.role
        )));
    }

    let mut file = async_fs::OpenOptions::new()
        .create(true)
        .append(resumed)
        .write(true)
        .truncate(!resumed)
        .open(&part_path)
        .await?;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if cancellation.is_cancelled() {
            file.flush().await?;
            return Err(AppError::InferenceCancelled("download stopped".to_string()));
        }
        let chunk = chunk.map_err(|error| AppError::ModelDownloadFailed(error.to_string()))?;
        file.write_all(&chunk).await?;
    }
    file.flush().await?;

    let actual_size = fs::metadata(&part_path)?.len();
    if actual_size != dependency.size_bytes {
        let _ = fs::remove_file(&part_path);
        return Err(AppError::ModelDownloadFailed(format!(
            "downloaded {} dependency size {actual_size} did not match expected {}",
            dependency.role, dependency.size_bytes
        )));
    }

    let actual = sha256_file(&part_path)?;
    let verification = match dependency.sha256.as_deref() {
        Some(expected) if actual.eq_ignore_ascii_case(expected) => VerificationState::Verified,
        Some(expected) => {
            let _ = fs::remove_file(&part_path);
            return Err(AppError::ModelChecksumFailed(format!(
                "{} dependency expected {expected}, got {actual}",
                dependency.role
            )));
        }
        None => VerificationState::Unverified,
    };
    fs::rename(&part_path, &final_path)?;
    Ok(package_manifest_entry(dependency, actual, verification))
}

fn package_manifest_entry(
    dependency: &ResolvedDependency,
    actual_sha256: String,
    verification: VerificationState,
) -> PackageFileManifest {
    PackageFileManifest {
        role: dependency.role.clone(),
        filename: dependency.filename.clone(),
        format: dependency.format.clone(),
        size_bytes: dependency.size_bytes,
        sha256: dependency.sha256.clone(),
        actual_sha256,
        verification,
        source_url: dependency.source_url.clone(),
    }
}

pub(crate) fn validate_installed_dependencies(
    root: &PortableRootManager,
    entry: &ModelCatalogEntry,
    verify_hashes: bool,
) -> Result<bool, AppError> {
    let Some(download) = entry.download.as_ref() else {
        return Ok(true);
    };
    let required = download
        .dependencies
        .iter()
        .filter(|dependency| dependency.required)
        .collect::<Vec<_>>();
    if required.is_empty() {
        return Ok(true);
    }

    let model_dir = root.resolve_relative(&download.destination_dir)?;
    let manifest_path = model_dir.join("package-manifest.json");
    if !manifest_path.is_file() {
        return Ok(false);
    }
    ensure_contained(root.root(), &manifest_path)?;
    let manifest: ModelPackageManifest = match fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
    {
        Some(manifest) => manifest,
        None => return Ok(false),
    };
    if manifest.model_id != entry.id || manifest.repo != entry.repo {
        return Ok(false);
    }

    for dependency in required {
        let Some(record) = manifest.files.iter().find(|file| {
            file.role == dependency.role
                && file.format == dependency.format
                && wildcard_match(&dependency.filename_pattern, &file.filename)
        }) else {
            return Ok(false);
        };
        if record.size_bytes == 0
            || record.actual_sha256.trim().is_empty()
            || record.verification == VerificationState::Failed
        {
            return Ok(false);
        }
        if record
            .sha256
            .as_deref()
            .is_some_and(|expected| !expected.eq_ignore_ascii_case(&record.actual_sha256))
        {
            return Ok(false);
        }

        let file_path = model_dir.join(&record.filename);
        if !file_path.is_file() {
            return Ok(false);
        }
        ensure_contained(root.root(), &file_path)?;
        if fs::metadata(&file_path)?.len() != record.size_bytes {
            return Ok(false);
        }
        if verify_hashes {
            let actual = sha256_file(&file_path)?;
            if !actual.eq_ignore_ascii_case(&record.actual_sha256)
                || record
                    .sha256
                    .as_deref()
                    .is_some_and(|expected| !actual.eq_ignore_ascii_case(expected))
            {
                return Ok(false);
            }
        }
    }

    Ok(true)
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let pattern = pattern.to_ascii_lowercase();
    let value = value.to_ascii_lowercase();
    let parts = pattern.split('*').collect::<Vec<_>>();
    if parts.len() == 1 {
        return pattern == value;
    }

    let mut cursor = 0;
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        let Some(found) = value[cursor..].find(part) else {
            return false;
        };
        if index == 0 && found != 0 {
            return false;
        }
        cursor += found + part.len();
    }

    if pattern.ends_with('*') {
        true
    } else {
        parts.last().is_none_or(|last| value.ends_with(last))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_pattern_selects_qwen_mmproj() {
        assert!(wildcard_match(
            "mmproj-*Q8_0*.gguf",
            "mmproj-Qwen2.5-VL-3B-Instruct-Q8_0.gguf"
        ));
        assert!(!wildcard_match(
            "mmproj-*Q8_0*.gguf",
            "Qwen2.5-VL-3B-Instruct-Q8_0.gguf"
        ));
    }

    #[test]
    fn validates_package_manifest_and_detects_same_size_hash_tampering() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let entry = crate::model_catalog::entry_by_id("qwen25-vl-3b-q4km").unwrap();
        let download = entry.download.as_ref().unwrap();
        let model_dir = root.resolve_relative(&download.destination_dir).unwrap();
        fs::create_dir_all(&model_dir).unwrap();

        let filename = "mmproj-Qwen2.5-VL-3B-Instruct-Q8_0.gguf";
        let file_path = model_dir.join(filename);
        fs::write(&file_path, b"projector").unwrap();
        let actual = sha256_file(&file_path).unwrap();
        let manifest = ModelPackageManifest {
            model_id: entry.id.clone(),
            repo: entry.repo.clone(),
            files: vec![PackageFileManifest {
                role: "mmproj".to_string(),
                filename: filename.to_string(),
                format: "gguf".to_string(),
                size_bytes: 9,
                sha256: Some(actual.clone()),
                actual_sha256: actual,
                verification: VerificationState::Verified,
                source_url: "https://example.invalid/mmproj.gguf".to_string(),
            }],
        };
        fs::write(
            model_dir.join("package-manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        assert!(validate_installed_dependencies(&root, &entry, false).unwrap());
        assert!(validate_installed_dependencies(&root, &entry, true).unwrap());

        // Same-size corruption is intentionally cheap to tolerate during discovery,
        // but explicit validation must catch it by hashing the dependency.
        fs::write(&file_path, b"corrupted").unwrap();
        assert!(validate_installed_dependencies(&root, &entry, false).unwrap());
        assert!(!validate_installed_dependencies(&root, &entry, true).unwrap());
    }
}
