//! Model metadata is small; keep a synced recovery record before publication.
//! Recovery is permitted only after the caller revalidates the installed bytes.
use crate::app_error::AppError;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const MAX_BYTES: u64 = 64 * 1024;

pub(super) fn recovery_path(path: &Path) -> PathBuf {
    path.with_extension("json.recovery")
}

fn read<T: DeserializeOwned>(path: &Path) -> Result<T, AppError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_BYTES {
        return Err(AppError::ModelInvalid(
            "invalid model manifest path or size".into(),
        ));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(AppError::ModelInvalid(
            "model manifest exceeds size limit".into(),
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| AppError::ModelInvalid(error.to_string()))
}

fn publish<F>(path: &Path, bytes: &[u8], write: &mut F) -> Result<(), AppError>
where
    F: FnMut(&mut fs::File, &[u8]) -> io::Result<()>,
{
    let parent = path
        .parent()
        .ok_or_else(|| AppError::internal("model manifest has no parent"))?;
    if let Ok(meta) = fs::symlink_metadata(path) {
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(AppError::ModelInvalid(
                "model manifest is not a regular file".into(),
            ));
        }
    }
    let temporary = parent.join(format!(".manifest-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        write(&mut file, bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn write_with<F>(path: &Path, bytes: &[u8], mut write: F) -> Result<(), AppError>
where
    F: FnMut(&mut fs::File, &[u8]) -> io::Result<()>,
{
    // Reject malformed metadata before replacing either complete record.
    if bytes.len() as u64 > MAX_BYTES {
        return Err(AppError::ModelInvalid(
            "model manifest exceeds size limit".into(),
        ));
    }
    let _: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| AppError::ModelInvalid(error.to_string()))?;
    if let Ok(meta) = fs::symlink_metadata(path) {
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(AppError::ModelInvalid(
                "model manifest is not a regular file".into(),
            ));
        }
    }
    publish(&recovery_path(path), bytes, &mut write)?;
    publish(path, bytes, &mut write)
}

pub(super) fn write(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    write_with(path, bytes, |file, bytes| file.write_all(bytes))
}

pub(super) fn read_or_recover<T, F>(path: &Path, validate: F) -> Result<T, AppError>
where
    T: DeserializeOwned + Serialize,
    F: Fn(&T, bool) -> Result<bool, AppError>,
{
    let live: Option<T> = read(path)
        .ok()
        .filter(|value| validate(value, false).unwrap_or(false));
    let journal: Option<T> = read(&recovery_path(path)).ok();
    if let Some(value) = live.as_ref() {
        let matches = journal.as_ref().is_none_or(|record| {
            serde_json::to_value(record).ok() == serde_json::to_value(value).ok()
        });
        if matches {
            return Ok(live.unwrap());
        }
    }
    // A different journal can be the newer completed write from an interrupted
    // publication. It must match current installed bytes before replacing live.
    let recovered = match journal {
        Some(record) if validate(&record, true).unwrap_or(false) => record,
        _ => {
            return live.ok_or_else(|| {
                AppError::ModelInvalid(
                    "no recoverable model manifest; use model package repair".into(),
                )
            })
        }
    };
    let bytes = serde_json::to_vec_pretty(&recovered)
        .map_err(|error| AppError::internal(error.to_string()))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(AppError::ModelInvalid(
            "recovered model manifest exceeds size limit".into(),
        ));
    }
    publish(path, &bytes, &mut |file, bytes| file.write_all(bytes))?;
    Ok(recovered)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_or_full_disk_writes_preserve_complete_metadata_for_both_manifests() {
        for name in ["model-manifest.json", "package-manifest.json"] {
            for failed_write in [1, 2] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join(name);
                write(&path, br#"{"version":"old"}"#).unwrap();
                let mut calls = 0;
                assert!(write_with(&path, br#"{"version":"new"}"#, |file, bytes| {
                    calls += 1;
                    if calls == failed_write {
                        file.write_all(&bytes[..5])?;
                        return Err(io::Error::new(
                            io::ErrorKind::StorageFull,
                            "injected disk full",
                        ));
                    }
                    file.write_all(bytes)
                })
                .is_err());
                assert_eq!(fs::read(&path).unwrap(), br#"{"version":"old"}"#);
                let journal: serde_json::Value = read(&recovery_path(&path)).unwrap();
                assert_eq!(
                    journal["version"],
                    if failed_write == 1 { "old" } else { "new" }
                );
                assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
            }
        }
    }
    #[test]
    fn recovery_revalidates_before_restoring_missing_or_truncated_live_records() {
        for name in ["model-manifest.json", "package-manifest.json"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(name);
            write(&path, br#"{"version":"good"}"#).unwrap();
            fs::write(&path, b"{truncated").unwrap();
            assert!(read_or_recover::<serde_json::Value, _>(&path, |_, _| Ok(false)).is_err());
            assert_eq!(fs::read(&path).unwrap(), b"{truncated");
            for missing in [false, true] {
                if missing {
                    fs::remove_file(&path).unwrap();
                }
                let value: serde_json::Value = read_or_recover(&path, |_, _| Ok(true)).unwrap();
                assert_eq!(value["version"], "good");
                assert_eq!(read::<serde_json::Value>(&path).unwrap(), value);
            }
        }
    }
    #[test]
    fn malformed_or_oversized_new_metadata_preserves_both_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model-manifest.json");
        write(&path, b"{}").unwrap();
        for bad in [b"{".to_vec(), vec![b' '; MAX_BYTES as usize + 1]] {
            assert!(write(&path, &bad).is_err());
            assert_eq!(fs::read(&path).unwrap(), b"{}");
            assert_eq!(fs::read(recovery_path(&path)).unwrap(), b"{}");
        }
    }
    #[test]
    fn interrupted_publication_uses_the_newer_revalidated_journal() {
        for name in ["model-manifest.json", "package-manifest.json"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(name);
            write(&path, br#"{"version":"old"}"#).unwrap();
            let mut calls = 0;
            assert!(write_with(&path, br#"{"version":"new"}"#, |file, bytes| {
                calls += 1;
                if calls == 2 {
                    return Err(io::Error::new(
                        io::ErrorKind::StorageFull,
                        "injected disk full",
                    ));
                }
                file.write_all(bytes)
            })
            .is_err());
            assert_eq!(read::<serde_json::Value>(&path).unwrap()["version"], "old");
            let value: serde_json::Value =
                read_or_recover(&path, |record: &serde_json::Value, full| {
                    Ok(!full || record["version"] == "new")
                })
                .unwrap();
            assert_eq!(value["version"], "new");
            assert_eq!(read::<serde_json::Value>(&path).unwrap(), value);
        }
    }
}
