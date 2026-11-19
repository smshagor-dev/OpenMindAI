use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock, RwLock,
    },
};

use serde::Serialize;

use crate::{
    app_error::AppError,
    portable_root::{available_bytes_for_path, PortableRootManager},
};

const STORAGE_CACHE_TTL: Duration = Duration::from_secs(30);
static STORAGE_SUMMARY_CACHE: OnceLock<RwLock<Option<(String, Instant, StorageSummary)>>> = OnceLock::new();
static STORAGE_SCAN_RUNNING: AtomicBool = AtomicBool::new(false);

fn snapshot_is_fresh(cached_root: &str, scanned_at: Instant, requested_root: &str) -> bool {
    cached_root == requested_root && scanned_at.elapsed() < STORAGE_CACHE_TTL
}

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

fn empty_storage_summary(root: String) -> StorageSummary {
    StorageSummary {
        root,
        models_bytes: 0,
        database_bytes: 0,
        cache_bytes: 0,
        generated_bytes: 0,
        available_bytes: None,
    }
}

fn snapshot_for_root(
    cached: &Option<(String, Instant, StorageSummary)>,
    requested_root: &str,
) -> Option<StorageSummary> {
    cached.as_ref().and_then(|(root, _, summary)| {
        (root == requested_root).then(|| summary.clone())
    })
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
    /// of models/cache/generated folders is performed in the background and
    /// subsequent refreshes receive the completed values.
    pub fn summary(&self) -> Result<StorageSummary, AppError> {
        let root_key = self.root.root().display().to_string();
        let cache = STORAGE_SUMMARY_CACHE.get_or_init(|| RwLock::new(None));
        if let Ok(current) = cache.read() {
            if let Some((cached_root, scanned_at, summary)) = current.as_ref() {
                if snapshot_is_fresh(cached_root, *scanned_at, &root_key) {
                    return Ok(summary.clone());
                }
            }
        }

        if STORAGE_SCAN_RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let root = self.root.clone();
            let key = root_key.clone();
            std::thread::spawn(move || {
                let monitor = StorageMonitor::new(&root);
                if let Ok(summary) = monitor.compute_summary() {
                    let cache = STORAGE_SUMMARY_CACHE.get_or_init(|| RwLock::new(None));
                    if let Ok(mut current) = cache.write() {
                        *current = Some((key, Instant::now(), summary));
                    }
                }
                STORAGE_SCAN_RUNNING.store(false, Ordering::Release);
            });
        }

        // A refresh can take seconds on large model directories. Preserve the
        // last completed snapshot for this root while the scan runs instead of
        // presenting misleading zero usage on every expired-cache request.
        if let Ok(current) = cache.read() {
            if let Some(summary) = snapshot_for_root(&current, &root_key) {
                return Ok(summary);
            }
        }

        Ok(empty_storage_summary(root_key))
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
        // Inspect the directory entry, not its target. Cache entries may be
        // symlinks to locations outside the portable root: never traverse or
        // recursively delete their targets when clearing regenerable cache.
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() {
            // A symlink occupies negligible space here; its target is not ours
            // to count or delete. Windows directory symlinks require remove_dir.
            #[cfg(windows)]
            {
                if fs::remove_file(entry.path()).is_err() {
                    fs::remove_dir(entry.path())?;
                }
            }
            #[cfg(not(windows))]
            fs::remove_file(entry.path())?;
        } else if metadata.is_dir() {
            freed = freed.saturating_add(directory_size(&entry.path())?);
            fs::remove_dir_all(entry.path())?;
        } else {
            freed = freed.saturating_add(metadata.len());
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

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    fn storage_cache_ttl_expires_stale_snapshots() {
        let recent = Instant::now();
        assert!(snapshot_is_fresh("root-a", recent, "root-a"));
        assert!(!snapshot_is_fresh("root-a", recent, "root-b"));
        let old = Instant::now() - STORAGE_CACHE_TTL - Duration::from_secs(1);
        assert!(!snapshot_is_fresh("root-a", old, "root-a"));
    }

    #[test]
    fn stale_snapshot_is_returned_only_for_its_own_root() {
        let mut summary = empty_storage_summary("root-a".into());
        summary.models_bytes = 123_456;
        summary.generated_bytes = 789;
        let cached = Some((
            "root-a".to_string(),
            Instant::now() - STORAGE_CACHE_TTL - Duration::from_secs(1),
            summary,
        ));
        let restored = snapshot_for_root(&cached, "root-a").unwrap();
        assert_eq!(restored.models_bytes, 123_456);
        assert_eq!(restored.generated_bytes, 789);
        assert!(snapshot_for_root(&cached, "root-b").is_none());
        assert!(snapshot_for_root(&None, "root-a").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn clear_cache_unlinks_directory_and_file_symlinks_without_touching_targets() {
        use std::os::unix::fs::symlink;

        let sandbox = tempfile::tempdir().unwrap();
        let external = sandbox.path().join("external");
        let cache = sandbox.path().join("cache");
        fs::create_dir_all(&external).unwrap();
        fs::create_dir_all(&cache).unwrap();
        let protected = external.join("keep.txt");
        fs::write(&protected, b"must survive").unwrap();
        symlink(&external, cache.join("linked-directory")).unwrap();
        symlink(&protected, cache.join("linked-file")).unwrap();
        fs::write(cache.join("scratch.txt"), b"scratch").unwrap();

        let freed = clear_directory_contents(&cache).unwrap();
        assert_eq!(freed, 7);
        assert!(fs::read_dir(&cache).unwrap().next().is_none());
        assert_eq!(fs::read(&protected).unwrap(), b"must survive");
    }

    #[test]
    fn clear_cache_removes_normal_nested_cache_entries() {
        let sandbox = tempfile::tempdir().unwrap();
        let cache = sandbox.path().join("cache");
        fs::create_dir_all(cache.join("nested")).unwrap();
        fs::write(cache.join("nested").join("data"), b"abc").unwrap();
        assert_eq!(clear_directory_contents(&cache).unwrap(), 3);
        assert!(fs::read_dir(&cache).unwrap().next().is_none());
    }

    #[test]
    fn storage_cache_ttl_is_bounded() {
        assert!(STORAGE_CACHE_TTL <= Duration::from_secs(60));
        assert!(!STORAGE_CACHE_TTL.is_zero());
    }
}
