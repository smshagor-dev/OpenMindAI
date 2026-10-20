use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Component, Path, PathBuf},
};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::{
    app_error::AppError,
    model_catalog::{ModelCatalogDependency, ModelCatalogEntry},
    model_download::{
        ensure_contained, prepare_partial_download, sha256_file, validate_free_space,
        PartialDownloadState, VerificationState,
    },
    portable_root::PortableRootManager,
};

const PACKAGE_SPACE_MARGIN: u64 = 256 * 1024 * 1024;
const MAX_PACKAGE_MANIFEST_BYTES: u64 = 64 * 1024;

/**
 * Publish an individual model metadata file only after its full contents have
 * been written and synced. A failed write or rename leaves the previous
 * manifest untouched and removes this attempt's temporary file.
 *
 * The temporary file is deliberately on the same filesystem as its final
 * destination; neither a cross-device copy nor truncation of the live manifest
 * is acceptable for model packages on removable storage.
 */
pub(super) fn write_manifest_atomically(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::internal("model manifest has no parent directory"))?;
    let name = path
        .file_name()
        .ok_or_else(|| AppError::internal("model manifest has no file name"))?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        name.to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    let result = (|| -> Result<(), AppError> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}


#[derive(Debug, Deserialize)]
struct HuggingFaceModel {
    sha: Option<String>,
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
    primary_revision: Option<&str>,
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
        // Fetch dependencies at the *same snapshot* as their primary model.
        // Independent dependency repositories use their own metadata commit SHA.
        let pinned_revision = if repo == entry.repo.as_str() {
            Some(super::pinned_repo_revision(primary_revision)?)
        } else {
            None
        };
        let model = match fetch_model_metadata(client, repo, pinned_revision).await {
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
    write_manifest_atomically(
        &model_dir.join("package-manifest.json"),
        serde_json::to_string_pretty(&manifest)
            .map_err(|error| AppError::internal(error.to_string()))?
            .as_bytes(),
    )?;
    Ok(())
}

async fn fetch_model_metadata(
    client: &Client,
    repo: &str,
    pinned_revision: Option<&str>,
) -> Result<HuggingFaceModel, AppError> {
    let api_url = match pinned_revision {
        Some(revision) => {
            let revision = super::pinned_repo_revision(Some(revision))?;
            format!("https://huggingface.co/api/models/{repo}/revision/{revision}?blobs=true")
        }
        None => format!("https://huggingface.co/api/models/{repo}?blobs=true"),
    };
    let model: HuggingFaceModel = crate::net::send_with_retry(|| client.get(&api_url), None)
        .await
        .map_err(|error| AppError::ModelDownloadFailed(error.to_string()))?
        .error_for_status()
        .map_err(|error| AppError::ModelDownloadFailed(error.to_string()))?
        .json()
        .await
        .map_err(|error| AppError::ModelDownloadFailed(error.to_string()))?;
    let actual_revision = super::pinned_repo_revision(model.sha.as_deref())?;
    if pinned_revision.is_some_and(|pinned| !pinned.eq_ignore_ascii_case(actual_revision)) {
        return Err(AppError::ModelDownloadFailed(
            "Hugging Face dependency metadata changed from its pinned model revision".to_string(),
        ));
    }
    Ok(model)
}

fn resolve_dependency(
    repo: &str,
    model: &HuggingFaceModel,
    dependency: &ModelCatalogDependency,
) -> Result<ResolvedDependency, AppError> {
    let revision = super::pinned_repo_revision(model.sha.as_deref())?;
    let sibling = model
        .siblings
        .iter()
        .filter(|sibling| {
            valid_package_filename(&sibling.rfilename)
                && wildcard_match(&dependency.filename_pattern, &sibling.rfilename)
        })
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
    if sibling
        .lfs
        .as_ref()
        .and_then(|lfs| lfs.sha256.as_deref())
        .is_some_and(|digest| !valid_package_digest(digest))
    {
        return Err(AppError::ModelDownloadFailed(
            "model dependency metadata contains an invalid SHA-256".to_string(),
        ));
    }

    Ok(ResolvedDependency {
        role: dependency.role.clone(),
        filename: sibling.rfilename.clone(),
        format: dependency.format.clone(),
        size_bytes,
        sha256: sibling.lfs.as_ref().and_then(|lfs| lfs.sha256.clone()),
        source_url: format!(
            "https://huggingface.co/{repo}/resolve/{revision}/{}",
            sibling.rfilename
        ),
    })
}

/// Validate server-provided filenames before touching the filesystem.
/// Inspect each existing ancestor using symlink_metadata so a package-local
/// symlink cannot redirect the creation or replacement of dependency files.
pub(super) fn prepare_dependency_destination(
    root: &PortableRootManager,
    model_dir: &Path,
    filename: &str,
) -> Result<PathBuf, AppError> {
    if !valid_package_filename(filename) {
        return Err(AppError::ModelDownloadFailed(
            "model dependency filename is unsafe".to_string(),
        ));
    }
    let canonical_model_dir = fs::canonicalize(model_dir)?;
    ensure_contained(root.root(), &canonical_model_dir)?;
    let mut parent = model_dir.to_path_buf();
    let segments = filename.split('/').collect::<Vec<_>>();
    for segment in &segments[..segments.len() - 1] {
        parent.push(segment);
        match fs::symlink_metadata(&parent) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(AppError::ModelDownloadFailed(
                    "model dependency directory is a symlink or not a directory".to_string(),
                ));
            }
            Err(error) if error.kind() == ErrorKind::NotFound => fs::create_dir(&parent)?,
            Err(error) => return Err(error.into()),
        }
        if !fs::canonicalize(&parent)?.starts_with(&canonical_model_dir) {
            return Err(AppError::ModelDownloadFailed(
                "model dependency directory escapes its package".to_string(),
            ));
        }
    }
    let target = model_dir.join(filename);
    match fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(AppError::ModelDownloadFailed(
                "model dependency destination is not a regular file".to_string(),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    ensure_contained(&canonical_model_dir, &target)?;
    Ok(target)
}

/// Namespace resumable dependency downloads by installed package, remote
/// source and nested filename. The old flattened basename was shared by
/// unrelated model packages and could also collide on slash/underscore paths.
/// Use length-delimited data so distinct field boundaries cannot collide.
pub(super) fn namespaced_model_partial_path(
    root: &PortableRootManager,
    model_dir: &Path,
    source_url: &str,
    filename: &str,
) -> Result<PathBuf, AppError> {
    let relative_dir = model_dir.strip_prefix(root.root()).map_err(|_| {
        AppError::ModelDownloadFailed("model package is outside the portable root".to_string())
    })?;
    let relative_package = relative_dir.to_string_lossy();
    let mut digest = Sha256::new();
    for field in [
        relative_package.as_bytes(),
        source_url.as_bytes(),
        filename.as_bytes(),
    ] {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    let part_name = format!("dependency-{:x}.part", digest.finalize());
    let temp_dir = root.resolve_relative("temp/downloads")?;
    fs::create_dir_all(&temp_dir)?;
    ensure_contained(root.root(), &temp_dir)?;
    let part_path = temp_dir.join(part_name);
    match fs::symlink_metadata(&part_path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(AppError::ModelDownloadFailed(
                "model dependency partial path is not a regular file".to_string(),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    ensure_contained(&temp_dir, &part_path)?;
    Ok(part_path)
}

fn dependency_partial_path(
    root: &PortableRootManager,
    model_dir: &Path,
    dependency: &ResolvedDependency,
) -> Result<PathBuf, AppError> {
    namespaced_model_partial_path(
        root,
        model_dir,
        &dependency.source_url,
        &dependency.filename,
    )
}

async fn download_dependency(
    root: &PortableRootManager,
    client: &Client,
    model_dir: &Path,
    dependency: &ResolvedDependency,
    cancellation: &CancellationToken,
) -> Result<PackageFileManifest, AppError> {
    let final_path = prepare_dependency_destination(root, model_dir, &dependency.filename)?;
    let part_path = dependency_partial_path(root, model_dir, dependency)?;
    ensure_contained(model_dir, &final_path)?;

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

    match prepare_partial_download(
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
        PartialDownloadState::Resume(_) | PartialDownloadState::Fresh => {}
    }
    crate::net::download_resumable(
        client,
        &dependency.source_url,
        &part_path,
        Some(dependency.size_bytes),
        Some(cancellation),
        |_| {},
    )
    .await
    .map_err(|error| {
        if error.is_cancelled() {
            AppError::InferenceCancelled("download stopped".to_string())
        } else {
            AppError::ModelDownloadFailed(format!(
                "{} dependency download failed: {error}",
                dependency.role
            ))
        }
    })?;

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
    let canonical_model_dir = fs::canonicalize(&model_dir)?;
    let canonical_manifest = fs::canonicalize(&manifest_path)?;
    if !canonical_manifest.starts_with(&canonical_model_dir)
        || fs::metadata(&canonical_manifest)?.len() > MAX_PACKAGE_MANIFEST_BYTES
    {
        return Ok(false);
    }
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
        let mut matching = manifest.files.iter().filter(|file| {
            file.role == dependency.role
                && file.format == dependency.format
                && wildcard_match(&dependency.filename_pattern, &file.filename)
        });
        let Some(record) = matching.next() else {
            return Ok(false);
        };
        if matching.next().is_some() {
            return Ok(false);
        }
        if record.size_bytes == 0
            || !valid_package_digest(&record.actual_sha256)
            || !valid_package_filename(&record.filename)
            || record.verification == VerificationState::Failed
        {
            return Ok(false);
        }
        if record.sha256.as_deref().is_some_and(|expected| {
            !valid_package_digest(expected) || !expected.eq_ignore_ascii_case(&record.actual_sha256)
        }) {
            return Ok(false);
        }

        let file_path = model_dir.join(&record.filename);
        if !file_path.is_file() {
            return Ok(false);
        }
        ensure_contained(root.root(), &file_path)?;
        if !fs::canonicalize(&file_path)?.starts_with(&canonical_model_dir) {
            return Ok(false);
        }
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

pub(super) fn valid_package_digest(digest: &str) -> bool {
    digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn valid_package_filename(filename: &str) -> bool {
    !filename.is_empty()
        && !filename.chars().any(|c| {
            c.is_control()
                || matches!(
                    c,
                    '\\' | ':' | '?' | '*' | '"' | '<' | '>' | '|' | '#' | '%'
                )
        })
        && filename
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
        && Path::new(filename)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
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
    fn dependency_partials_are_distinct_across_packages_sources_and_nested_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let a = root.resolve_relative("models/vision/package-a").unwrap();
        let b = root.resolve_relative("models/vision/package-b").unwrap();
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        let template = ResolvedDependency {
            role: "voice".to_string(),
            filename: "voices/af_heart.pt".to_string(),
            format: "pt".to_string(),
            size_bytes: 11,
            sha256: None,
            source_url: "https://example.invalid/voices/af_heart.pt".to_string(),
        };
        let first = dependency_partial_path(&root, &a, &template).unwrap();
        assert_eq!(
            first,
            dependency_partial_path(&root, &a, &template).unwrap()
        );
        assert_ne!(
            first,
            dependency_partial_path(&root, &b, &template).unwrap()
        );

        let different_source = ResolvedDependency {
            source_url: "https://other.invalid/voices/af_heart.pt".to_string(),
            ..template.clone()
        };
        assert_ne!(
            first,
            dependency_partial_path(&root, &a, &different_source).unwrap()
        );
        // Flattening slashes to underscores made these two filenames collide.
        let path_one = ResolvedDependency {
            filename: "voices/a_b.pt".to_string(),
            ..template.clone()
        };
        let path_two = ResolvedDependency {
            filename: "voices_a/b.pt".to_string(),
            ..template
        };
        assert_ne!(
            dependency_partial_path(&root, &a, &path_one).unwrap(),
            dependency_partial_path(&root, &a, &path_two).unwrap()
        );
    }

    #[test]
    fn dependency_partial_name_survives_portable_root_relocation() {
        let first_temp = tempfile::tempdir().unwrap();
        let second_temp = tempfile::tempdir().unwrap();
        let a = PortableRootManager::from_root(first_temp.path().join("OpenMindAI"));
        let b = PortableRootManager::from_root(second_temp.path().join("OpenMindAI"));
        a.ensure_directories().unwrap();
        b.ensure_directories().unwrap();
        let dependency = ResolvedDependency {
            role: "voice".to_string(),
            filename: "voices/af_heart.pt".to_string(),
            format: "pt".to_string(),
            size_bytes: 11,
            sha256: None,
            source_url: "https://example.invalid/voices/af_heart.pt".to_string(),
        };
        let path_a = a.resolve_relative("models/audio/kokoro").unwrap();
        let path_b = b.resolve_relative("models/audio/kokoro").unwrap();
        fs::create_dir_all(&path_a).unwrap();
        fs::create_dir_all(&path_b).unwrap();
        let partial_a = dependency_partial_path(&a, &path_a, &dependency).unwrap();
        let partial_b = dependency_partial_path(&b, &path_b, &dependency).unwrap();
        assert_eq!(partial_a.file_name(), partial_b.file_name());
    }

    #[cfg(unix)]
    #[test]
    fn dependency_partial_rejects_preexisting_symlink() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let package = root.resolve_relative("models/vision/package").unwrap();
        fs::create_dir_all(&package).unwrap();
        let dependency = ResolvedDependency {
            role: "voice".to_string(),
            filename: "voices/af_heart.pt".to_string(),
            format: "pt".to_string(),
            size_bytes: 11,
            sha256: None,
            source_url: "https://example.invalid/voices/af_heart.pt".to_string(),
        };
        let part = dependency_partial_path(&root, &package, &dependency).unwrap();
        let unrelated = root.resolve_relative("temp/unrelated").unwrap();
        fs::write(&unrelated, b"untouched").unwrap();
        symlink(&unrelated, &part).unwrap();
        assert!(dependency_partial_path(&root, &package, &dependency).is_err());
        assert_eq!(fs::read(&unrelated).unwrap(), b"untouched");
    }

    #[test]
    fn atomic_model_manifest_replaces_existing_file_without_partial_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model-manifest.json");
        fs::write(&path, br#"{"version":"old"}"#).unwrap();
        write_manifest_atomically(&path, br#"{"version":"new","verified":true}"#).unwrap();
        let contents = fs::read(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&contents).unwrap();
        assert_eq!(parsed["version"], "new");
        assert_eq!(parsed["verified"], true);
        let files = fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(files, 1, "successful replacement must not leave a temp file");
    }

    #[test]
    fn atomic_model_manifest_failure_preserves_existing_target() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("package-manifest.json");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("existing.json"), b"important").unwrap();
        assert!(write_manifest_atomically(&destination, br#"{"should":"fail"}"#).is_err());
        assert_eq!(
            fs::read(destination.join("existing.json")).unwrap(),
            b"important"
        );
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            1,
            "failed rename should remove its new temporary file"
        );
    }

    #[test]
    fn dependency_resolver_skips_unsafe_remote_sibling_paths() {
        let dep = ModelCatalogDependency {
            role: "voice".to_string(),
            repo: None,
            filename_pattern: "*.pt".to_string(),
            format: "pt".to_string(),
            required: true,
        };
        let unsafe_name = "../af_heart.pt";
        let safe_name = "voices/af_heart.pt";
        let model = HuggingFaceModel {
            sha: Some("04393022a04dcb701fd2061b8d5dbe23ac7be751".to_string()),
            siblings: vec![
                HuggingFaceSibling {
                    rfilename: unsafe_name.to_string(),
                    size: Some(100),
                    lfs: None,
                },
                HuggingFaceSibling {
                    rfilename: safe_name.to_string(),
                    size: Some(9),
                    lfs: None,
                },
            ],
        };
        let selected = resolve_dependency("hexgrad/Kokoro-82M", &model, &dep).unwrap();
        assert_eq!(selected.filename, safe_name);
        assert_eq!(
            selected.source_url,
            "https://huggingface.co/hexgrad/Kokoro-82M/resolve/04393022a04dcb701fd2061b8d5dbe23ac7be751/voices/af_heart.pt"
        );

        let invalid_only = HuggingFaceModel {
            sha: Some("04393022a04dcb701fd2061b8d5dbe23ac7be751".to_string()),
            siblings: vec![HuggingFaceSibling {
                rfilename: unsafe_name.to_string(),
                size: Some(100),
                lfs: None,
            }],
        };
        assert!(resolve_dependency("hexgrad/Kokoro-82M", &invalid_only, &dep).is_err());
    }

    #[test]
    fn dependency_resolver_rejects_unpinned_repository_metadata() {
        let dependency = ModelCatalogDependency {
            role: "config".to_string(),
            repo: None,
            filename_pattern: "config.json".to_string(),
            format: "json".to_string(),
            required: true,
        };
        let mut model = HuggingFaceModel {
            sha: None,
            siblings: vec![HuggingFaceSibling {
                rfilename: "config.json".to_string(),
                size: Some(2351),
                lfs: None,
            }],
        };
        assert!(resolve_dependency("hexgrad/Kokoro-82M", &model, &dependency).is_err());
        model.sha = Some("main".to_string());
        assert!(resolve_dependency("hexgrad/Kokoro-82M", &model, &dependency).is_err());
    }

    #[test]
    fn dependency_destination_rejects_traversal_before_writing_directories() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let model_dir = root.resolve_relative("models/vision/test-package").unwrap();
        fs::create_dir_all(&model_dir).unwrap();
        for filename in [
            "../bad.pt",
            "voices/../../bad.pt",
            "/absolute.pt",
            "C:/bad.pt",
            "voices\\bad.pt",
        ] {
            assert!(prepare_dependency_destination(&root, &model_dir, filename).is_err());
        }
        assert!(!model_dir.join("voices").exists());
        let normal =
            prepare_dependency_destination(&root, &model_dir, "voices/af_heart.pt").unwrap();
        assert_eq!(normal, model_dir.join("voices/af_heart.pt"));
        assert!(model_dir.join("voices").is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn dependency_destination_rejects_package_local_symlink_escape() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let model_dir = root.resolve_relative("models/vision/test-package").unwrap();
        fs::create_dir_all(&model_dir).unwrap();
        let other = root.resolve_relative("models/other-package").unwrap();
        fs::create_dir_all(&other).unwrap();
        symlink(&other, model_dir.join("voices")).unwrap();
        assert!(prepare_dependency_destination(&root, &model_dir, "voices/af_heart.pt").is_err());
        assert!(!other.join("af_heart.pt").exists());
    }

    #[test]
    fn package_manifest_names_and_digests_must_be_canonical() {
        assert!(valid_package_filename("voices/af_heart.pt"));
        assert!(valid_package_filename("mmproj-file.gguf"));
        for invalid in [
            "",
            "../outside.gguf",
            "./file.gguf",
            "voices/../file.pt",
            "voices//file.pt",
            "/absolute.gguf",
            "C:/windows/file.pt",
            "voices\\af_heart.pt",
        ] {
            assert!(!valid_package_filename(invalid), "accepted {invalid}");
        }
        assert!(valid_package_digest(&"a".repeat(64)));
        assert!(!valid_package_digest(&"g".repeat(64)));
        assert!(!valid_package_digest("truncated"));
    }

    #[test]
    fn package_manifest_limits_reject_oversized_or_invalid_records() {
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
        let digest = sha256_file(&file_path).unwrap();
        let valid_record = PackageFileManifest {
            role: "mmproj".to_string(),
            filename: filename.to_string(),
            format: "gguf".to_string(),
            size_bytes: 9,
            sha256: Some(digest.clone()),
            actual_sha256: digest.clone(),
            verification: VerificationState::Verified,
            source_url: "https://example.invalid/file.gguf".to_string(),
        };
        let manifest_path = model_dir.join("package-manifest.json");
        let mut manifest = ModelPackageManifest {
            model_id: entry.id.clone(),
            repo: entry.repo.clone(),
            files: vec![valid_record.clone()],
        };
        let store = |record: &ModelPackageManifest| {
            fs::write(&manifest_path, serde_json::to_vec(record).unwrap()).unwrap();
        };
        store(&manifest);
        assert!(validate_installed_dependencies(&root, &entry, true).unwrap());

        manifest.files.push(valid_record.clone());
        store(&manifest);
        assert!(!validate_installed_dependencies(&root, &entry, true).unwrap());
        manifest.files.pop();

        manifest.files[0].actual_sha256 = "invalid".to_string();
        store(&manifest);
        assert!(!validate_installed_dependencies(&root, &entry, true).unwrap());
        manifest.files[0].actual_sha256 = digest;

        manifest.files[0].source_url = "x".repeat(MAX_PACKAGE_MANIFEST_BYTES as usize);
        store(&manifest);
        assert!(!validate_installed_dependencies(&root, &entry, false).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn package_manifest_cannot_reference_symlink_outside_model_directory() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let entry = crate::model_catalog::entry_by_id("qwen25-vl-3b-q4km").unwrap();
        let download = entry.download.as_ref().unwrap();
        let model_dir = root.resolve_relative(&download.destination_dir).unwrap();
        fs::create_dir_all(&model_dir).unwrap();
        let external = root.resolve_relative("models/other.gguf").unwrap();
        fs::write(&external, b"projector").unwrap();
        let filename = "mmproj-Qwen2.5-VL-3B-Instruct-Q8_0.gguf";
        symlink(&external, model_dir.join(filename)).unwrap();
        let digest = sha256_file(&external).unwrap();
        let manifest = ModelPackageManifest {
            model_id: entry.id.clone(),
            repo: entry.repo.clone(),
            files: vec![PackageFileManifest {
                role: "mmproj".to_string(),
                filename: filename.to_string(),
                format: "gguf".to_string(),
                size_bytes: 9,
                sha256: Some(digest.clone()),
                actual_sha256: digest,
                verification: VerificationState::Verified,
                source_url: "https://example.invalid/file.gguf".to_string(),
            }],
        };
        fs::write(
            model_dir.join("package-manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(!validate_installed_dependencies(&root, &entry, true).unwrap());
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
