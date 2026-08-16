use std::{fs, path::Path};

use serde::Serialize;

use crate::{
    app_error::AppError,
    portable_root::{available_bytes_for_path, PortableRootManager},
};

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

    pub fn summary(&self) -> Result<StorageSummary, AppError> {
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
