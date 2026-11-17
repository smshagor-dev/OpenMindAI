use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
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
// Invalidations must also reject scans started before the filesystem changed.
static STORAGE_CACHE_EPOCH: AtomicU64 = AtomicU64::new(0);

fn scan_epoch_is_current(started: u64, current: u64) -> bool {
    started == current
}

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
            let scan_epoch = STORAGE_CACHE_EPOCH.load(Ordering::Acquire);
            std::thread::spawn(move || {
                let monitor = StorageMonitor::new(&root);
                if let Ok(summary) = monitor.compute_summary() {
                    let cache = STORAGE_SUMMARY_CACHE.get_or_init(|| RwLock::new(None));
                    if let Ok(mut current) = cache.write() {
                        // Hold the cache lock while comparing the epoch. A cache
                        // invalidation takes the same lock before advancing it.
                        if scan_epoch_is_current(
                            scan_epoch,
                            STORAGE_CACHE_EPOCH.load(Ordering::Acquire),
                        ) {
                            *current = Some((key, Instant::now(), summary));
                        }
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
                STORAGE_CACHE_EPOCH.fetch_add(1, Ordering::AcqRel);
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

    #[test]
    fn invalidation_epoch_rejects_old_inflight_scan_results() {
        let started = 41;
        assert!(scan_epoch_is_current(started, 41));
        assert!(!scan_epoch_is_current(started, 42));
        assert!(!scan_epoch_is_current(started, 43));
    }

    #[test]
    fn storage_cache_ttl_is_bounded() {
        assert!(STORAGE_CACHE_TTL <= Duration::from_secs(60));
        assert!(!STORAGE_CACHE_TTL.is_zero());
    }
}
