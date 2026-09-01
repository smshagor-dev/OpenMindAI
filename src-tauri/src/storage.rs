use std::{
    fs,
    path::Path,
    sync::{OnceLock, RwLock},
};

use serde::Serialize;

use crate::{
    app_error::AppError,
    portable_root::{available_bytes_for_path, PortableRootManager},
};

static STORAGE_SUMMARY_CACHE: OnceLock<RwLock<Option<(String, StorageSummary)>>> = OnceLock::new();
static STORAGE_SCAN_STARTED: OnceLock<()> = OnceLock::new();

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageSummary {
    pub root: String,
    pub models_bytes: u64,
    pub database_bytes: u64,
    pub cache_bytes: u64,
    pub generated_bytes: u64,
    pub available_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheClearResult {
    pub bytes_freed: u64,
}

pub struct StorageMonitor<'a> {
    root: &'a PortableRootManager,
}

impl<'a> StorageMonitor<'a> {
    pub fn new(root: &'a PortableRootManager) -> Self {
        Self { root }
    }

    /// Startup callers receive a cheap snapshot immediately. Recursive sizing
    /// of models/cache/generated folders is performed once on a background
    /// thread and subsequent refreshes receive the completed values.
    pub fn summary(&self) -> Result<StorageSummary, AppError> {
        let root_key = self.root.root().display().to_string();
        let cache = STORAGE_SUMMARY_CACHE.get_or_init(|| RwLock::new(None));
        if let Ok(current) = cache.read() {
            if let Some((cached_root, summary)) = current.as_ref() {
                if cached_root == &root_key {
                    return Ok(summary.clone());
                }
            }
        }

        if STORAGE_SCAN_STARTED.set(()).is_ok() {
            let root = self.root.clone();
            let key = root_key.clone();
            std::thread::spawn(move || {
                let monitor = StorageMonitor::new(&root);
                if let Ok(summary) = monitor.compute_summary() {
                    let cache = STORAGE_SUMMARY_CACHE.get_or_init(|| RwLock::new(None));
                    if let Ok(mut current) = cache.write() {
                        *current = Some((key, summary));
                    }
                }
            });
        }

        Ok(StorageSummary {
            root: root_key,
            models_bytes: 0,
            database_bytes: 0,
            cache_bytes: 0,
            generated_bytes: 0,
            available_bytes: None,
        })
    }

    fn compute_summary(&self) -> Result<StorageSummary, AppError> {
        Ok(StorageSummary {
            root: self.root.root().display().to_string(),
            models_bytes: directory_size(&self.root.resolve_relative("models")?)?,
            database_bytes: directory_size(&self.root.resolve_relative("data/database")?)?,
            cache_bytes: directory_size(&self.root.resolve_relative("cache")?)?,
            generated_bytes: directory_size(&self.root.resolve_relative("generated")?)?,
            available_bytes: available_bytes_for_path(self.root.root()),
        })
    }

    /// Empties `cache/` and `temp/` (contents only, directories themselves are
    /// kept). Both hold only regenerable scratch data -- never models, the
    /// database, conversation history, or generated artifacts -- so this is
    /// always safe to run without a repair/diagnostics pass first.
    pub fn clear_cache(&self) -> Result<CacheClearResult, AppError> {
        let mut bytes_freed = 0;
        for relative in ["cache", "temp"] {
            let path = self.root.resolve_relative(relative)?;
            bytes_freed += clear_directory_contents(&path)?;
        }

        if let Some(cache) = STORAGE_SUMMARY_CACHE.get() {
            if let Ok(mut current) = cache.write() {
                *current = None;
            }
        }

        Ok(CacheClearResult { bytes_freed })
    }
}

fn clear_directory_contents(path: &Path) -> Result<u64, AppError> {
    if !path.exists() {
        return Ok(0);
    }

    let mut freed = 0;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            freed += directory_size(&entry.path())?;
            fs::remove_dir_all(entry.path())?;
        } else {
            freed += metadata.len();
            fs::remove_file(entry.path())?;
        }
    }
    Ok(freed)
}

fn directory_size(path: &Path) -> Result<u64, AppError> {
    if !path.exists() {
        return Ok(0);
    }

    let mut total = 0;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            total += directory_size(&entry.path())?;
        } else {
            total += metadata.len();
        }
    }
    Ok(total)
}
