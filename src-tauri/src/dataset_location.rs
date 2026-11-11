//! Dataset identity is case-sensitive even on a case-insensitive filesystem.
use crate::app_error::AppError;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

pub(super) fn key(id: &str) -> String {
    format!("by-id-v1/{:x}", Sha256::digest(id.as_bytes()))
}
fn legacy_key(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}
fn matching_manifest(path: &Path, id: &str) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.is_file() || meta.len() > 1024 * 1024 {
        return false;
    }
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    let mut bytes = Vec::new();
    if file.take(1024 * 1024 + 1).read_to_end(&mut bytes).is_err() || bytes.len() > 1024 * 1024 {
        return false;
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return false;
    };
    value["datasetId"] == id && value["source"] == "openmindai" && value["revision"] == "main"
}

/// Called under the downloader's per-root install lock. Never combine an old
/// colliding folder with a new installation, or guess its owner from its name.
pub(super) fn prepare(base: &Path, id: &str) -> Result<PathBuf, AppError> {
    let destination = base.join(key(id)).join("snapshot/main");
    if destination.try_exists()? {
        return Ok(destination);
    }
    let legacy = base.join(legacy_key(id)).join("snapshot/main");
    let reusable = fs::symlink_metadata(&legacy)
        .is_ok_and(|meta| meta.is_dir() && !meta.file_type().is_symlink())
        && matching_manifest(&legacy.join("openmindai-dataset.json"), id);
    if reusable {
        let canonical_base = fs::canonicalize(base)?;
        if !fs::canonicalize(&legacy)?.starts_with(&canonical_base) {
            return Err(AppError::internal("legacy dataset escapes dataset storage"));
        }
        let mut ancestor = destination.parent().unwrap();
        while let Err(error) = fs::symlink_metadata(ancestor) {
            if error.kind() != io::ErrorKind::NotFound {
                return Err(error.into());
            }
            ancestor = ancestor
                .parent()
                .ok_or_else(|| AppError::internal("dataset destination has no ancestor"))?;
        }
        if !fs::canonicalize(ancestor)?.starts_with(&canonical_base) {
            return Err(AppError::internal(
                "dataset destination escapes dataset storage",
            ));
        }
        fs::create_dir_all(destination.parent().unwrap())?;
        if !fs::canonicalize(destination.parent().unwrap())?.starts_with(&canonical_base) {
            return Err(AppError::internal(
                "dataset destination escapes dataset storage",
            ));
        }
        // Same-filesystem directory rename preserves partials, bytes and manifest.
        // No copy/delete fallback: a failed move must preserve the legacy install.
        match fs::rename(&legacy, &destination) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(destination),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn legacy(base: &Path, id: &str) -> PathBuf {
        let path = base.join(legacy_key(id)).join("snapshot/main");
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("openmindai-dataset.json"),
            serde_json::to_vec(
                &serde_json::json!({"datasetId":id,"source":"openmindai","revision":"main"}),
            )
            .unwrap(),
        )
        .unwrap();
        fs::write(path.join("data.txt"), id).unwrap();
        path
    }
    #[test]
    fn legal_collisions_case_and_unicode_have_distinct_case_folded_keys() {
        let ids = [
            "team/dataset",
            "team_dataset",
            "Team/dataset",
            "TEAM/DATASET",
            "team-dataset",
            "team.dataset",
            "বাংলা/ডেটা",
            "日本/データ",
            "é/data",
            "é/data",
        ];
        let mut keys = std::collections::HashSet::new();
        for id in ids {
            assert!(keys.insert(key(id).to_ascii_lowercase()));
            assert_eq!(key(id), key(id));
        }
        assert_eq!(legacy_key(ids[0]), legacy_key(ids[1]));
        assert_eq!(key(&"x".repeat(10000)).len(), 73);
    }
    #[test]
    fn migration_uses_manifest_identity_and_preserves_colliding_dataset_bytes() {
        let base = tempfile::tempdir().unwrap();
        let old = legacy(base.path(), "team/dataset");
        let other = prepare(base.path(), "team_dataset").unwrap();
        assert!(old.is_dir());
        assert!(!other.exists());
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("data.txt"), "other").unwrap();
        let moved = prepare(base.path(), "team/dataset").unwrap();
        assert_ne!(moved, other);
        assert!(!old.exists());
        assert_eq!(
            fs::read_to_string(moved.join("data.txt")).unwrap(),
            "team/dataset"
        );
        assert_eq!(fs::read_to_string(other.join("data.txt")).unwrap(), "other");
        assert_eq!(prepare(base.path(), "team/dataset").unwrap(), moved);
    }
    #[test]
    fn existing_new_install_never_gets_merged_or_overwritten_by_legacy() {
        let base = tempfile::tempdir().unwrap();
        let old = legacy(base.path(), "team/data");
        let new = base.path().join(key("team/data")).join("snapshot/main");
        fs::create_dir_all(&new).unwrap();
        fs::write(new.join("data.txt"), b"new").unwrap();
        assert_eq!(prepare(base.path(), "team/data").unwrap(), new);
        assert_eq!(fs::read(old.join("data.txt")).unwrap(), b"team/data");
        assert_eq!(fs::read(new.join("data.txt")).unwrap(), b"new");
    }
    #[test]
    fn malformed_missing_oversized_or_wrong_source_manifest_is_not_migrated() {
        for bad in [
            b"{".to_vec(),
            b"{}".to_vec(),
            vec![b' '; 1024 * 1024 + 1],
            br#"{"datasetId":"team/data","source":"other","revision":"main"}"#.to_vec(),
        ] {
            let base = tempfile::tempdir().unwrap();
            let old = legacy(base.path(), "team/data");
            fs::write(old.join("openmindai-dataset.json"), bad).unwrap();
            let new = prepare(base.path(), "team/data").unwrap();
            assert!(!new.exists());
            assert!(old.exists());
        }
    }
    #[test]
    fn failed_migration_preserves_legacy_install() {
        let base = tempfile::tempdir().unwrap();
        let old = legacy(base.path(), "team/data");
        fs::write(base.path().join("by-id-v1"), b"block").unwrap();
        assert!(prepare(base.path(), "team/data").is_err());
        assert_eq!(fs::read(old.join("data.txt")).unwrap(), b"team/data");
    }
    #[cfg(unix)]
    #[test]
    fn linked_migration_destination_is_rejected_before_creating_external_directories() {
        let base = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let old = legacy(base.path(), "team/data");
        std::os::unix::fs::symlink(outside.path(), base.path().join("by-id-v1")).unwrap();
        assert!(prepare(base.path(), "team/data").is_err());
        assert!(old.exists());
        assert!(fs::read_dir(outside.path()).unwrap().next().is_none());
    }
}
