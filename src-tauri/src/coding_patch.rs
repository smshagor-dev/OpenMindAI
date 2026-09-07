use std::{
    collections::HashSet,
    fs::{self, File},
    io::Write,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::app_error::AppError;

const MAX_TRANSACTION_OPERATIONS: usize = 64;
const MAX_TRANSACTION_BYTES: usize = 16 * 1024 * 1024;
const TRANSACTION_DIR: &str = ".openmindai-patch-transactions";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", tag = "op")]
enum PatchOperation {
    #[serde(rename = "replace")]
    Replace {
        path: String,
        old: String,
        new: String,
    },
    #[serde(rename = "create")]
    Create { path: String, content: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchTransactionResult {
    pub transaction_id: String,
    pub changed_files: usize,
    pub created_files: usize,
    pub replaced_files: usize,
    pub paths: Vec<String>,
    pub recovered_interrupted_transactions: usize,
}

#[derive(Debug)]
struct PlannedChange {
    relative_path: String,
    target: PathBuf,
    original: Option<Vec<u8>>,
    replacement: Vec<u8>,
    created: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Journal {
    version: u32,
    transaction_id: String,
    entries: Vec<JournalEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JournalEntry {
    relative_path: String,
    original_existed: bool,
    backup_name: String,
    staged_name: String,
}

pub fn transaction_paths(action: &Value) -> Result<Vec<String>, AppError> {
    let operations = parse_operations(action)?;
    operations
        .iter()
        .map(|operation| match operation {
            PatchOperation::Replace { path, .. } | PatchOperation::Create { path, .. } => {
                validate_relative_path(path).map(|_| path.clone())
            }
        })
        .collect()
}

pub fn apply_patch_transaction(
    root: &Path,
    action: &Value,
) -> Result<PatchTransactionResult, AppError> {
    let root = fs::canonicalize(root)?;
    if !root.is_dir() {
        return Err(AppError::internal(
            "patch transaction root is not a directory",
        ));
    }
    let recovered = recover_interrupted_transactions(&root)?;
    let operations = parse_operations(action)?;
    let planned = preflight(&root, &operations)?;
    commit_transaction(&root, planned, recovered)
}

fn parse_operations(action: &Value) -> Result<Vec<PatchOperation>, AppError> {
    let operations = action
        .get("operations")
        .cloned()
        .ok_or_else(|| AppError::internal("patch_transaction requires operations"))?;
    let operations: Vec<PatchOperation> = serde_json::from_value(operations).map_err(|error| {
        AppError::internal(format!("invalid patch_transaction operations: {error}"))
    })?;
    if operations.is_empty() {
        return Err(AppError::internal(
            "patch_transaction requires at least one operation",
        ));
    }
    if operations.len() > MAX_TRANSACTION_OPERATIONS {
        return Err(AppError::internal(format!(
            "patch_transaction exceeds the {MAX_TRANSACTION_OPERATIONS}-operation safety limit"
        )));
    }
    Ok(operations)
}

fn preflight(root: &Path, operations: &[PatchOperation]) -> Result<Vec<PlannedChange>, AppError> {
    let mut seen = HashSet::new();
    let mut total_bytes = 0usize;
    let mut planned = Vec::with_capacity(operations.len());

    for operation in operations {
        let (relative_path, target) = match operation {
            PatchOperation::Replace { path, .. } | PatchOperation::Create { path, .. } => {
                let relative = validate_relative_path(path)?;
                let target = resolve_scoped_target(root, &relative)?;
                (normalize_relative(&relative), target)
            }
        };
        if !seen.insert(relative_path.clone()) {
            return Err(AppError::internal(format!(
                "patch_transaction contains duplicate target path: {relative_path}"
            )));
        }
        if target.exists() && fs::symlink_metadata(&target)?.file_type().is_symlink() {
            return Err(AppError::internal(format!(
                "patch_transaction refuses symlink target: {relative_path}"
            )));
        }

        let change = match operation {
            PatchOperation::Replace { old, new, .. } => {
                if old.is_empty() {
                    return Err(AppError::internal(format!(
                        "replace operation old text cannot be empty: {relative_path}"
                    )));
                }
                if !target.is_file() {
                    return Err(AppError::internal(format!(
                        "replace operation target is not a file: {relative_path}"
                    )));
                }
                let original = fs::read(&target)?;
                let text = String::from_utf8(original.clone()).map_err(|_| {
                    AppError::internal(format!(
                        "replace operation supports UTF-8 text only: {relative_path}"
                    ))
                })?;
                let matches = text.match_indices(old).count();
                if matches != 1 {
                    return Err(AppError::internal(format!(
                        "replace operation expected exactly one match in {relative_path}, found {matches}"
                    )));
                }
                let replacement = text.replacen(old, new, 1).into_bytes();
                total_bytes = total_bytes
                    .saturating_add(original.len())
                    .saturating_add(replacement.len());
                PlannedChange {
                    relative_path,
                    target,
                    original: Some(original),
                    replacement,
                    created: false,
                }
            }
            PatchOperation::Create { content, .. } => {
                if target.exists() {
                    return Err(AppError::internal(format!(
                        "create operation target already exists: {relative_path}"
                    )));
                }
                let replacement = content.as_bytes().to_vec();
                total_bytes = total_bytes.saturating_add(replacement.len());
                PlannedChange {
                    relative_path,
                    target,
                    original: None,
                    replacement,
                    created: true,
                }
            }
        };
        if total_bytes > MAX_TRANSACTION_BYTES {
            return Err(AppError::internal(format!(
                "patch_transaction exceeds the {} MiB safety limit",
                MAX_TRANSACTION_BYTES / (1024 * 1024)
            )));
        }
        planned.push(change);
    }
    Ok(planned)
}

fn commit_transaction(
    root: &Path,
    planned: Vec<PlannedChange>,
    recovered: usize,
) -> Result<PatchTransactionResult, AppError> {
    let transaction_id = Uuid::new_v4().to_string();
    let base = transaction_base(root)?;
    fs::create_dir_all(&base)?;
    reject_symlink(&base)?;
    let tx_dir = base.join(&transaction_id);
    fs::create_dir(&tx_dir)?;

    let entries = planned
        .iter()
        .enumerate()
        .map(|(index, change)| JournalEntry {
            relative_path: change.relative_path.clone(),
            original_existed: change.original.is_some(),
            backup_name: format!("{index}.backup"),
            staged_name: format!("{index}.stage"),
        })
        .collect::<Vec<_>>();
    let journal = Journal {
        version: 1,
        transaction_id: transaction_id.clone(),
        entries,
    };
    write_synced_json(&tx_dir.join("journal.json"), &journal)?;

    for (index, change) in planned.iter().enumerate() {
        let stage = tx_dir.join(format!("{index}.stage"));
        write_synced_bytes(&stage, &change.replacement)?;
        if let Some(original) = &change.original {
            let backup_copy = tx_dir.join(format!("{index}.original"));
            write_synced_bytes(&backup_copy, original)?;
        }
    }

    let apply_result = (|| -> Result<(), AppError> {
        for (index, change) in planned.iter().enumerate() {
            if let Some(parent) = change.target.parent() {
                fs::create_dir_all(parent)?;
            }
            if change.target.exists() {
                let backup = tx_dir.join(format!("{index}.backup"));
                fs::rename(&change.target, &backup)?;
            }
            let stage = tx_dir.join(format!("{index}.stage"));
            fs::rename(stage, &change.target)?;
        }
        Ok(())
    })();

    if let Err(error) = apply_result {
        let rollback_error = rollback_transaction(root, &tx_dir, &journal).err();
        let _ = cleanup_transaction_dir(&base, &tx_dir);
        return match rollback_error {
            Some(rollback_error) => Err(AppError::internal(format!(
                "patch_transaction failed: {error}; rollback also failed: {rollback_error}"
            ))),
            None => Err(AppError::internal(format!(
                "patch_transaction failed and was rolled back: {error}"
            ))),
        };
    }

    let paths = planned
        .iter()
        .map(|change| change.relative_path.clone())
        .collect::<Vec<_>>();
    let created_files = planned.iter().filter(|change| change.created).count();
    let replaced_files = planned.len().saturating_sub(created_files);
    cleanup_transaction_dir(&base, &tx_dir)?;

    Ok(PatchTransactionResult {
        transaction_id,
        changed_files: planned.len(),
        created_files,
        replaced_files,
        paths,
        recovered_interrupted_transactions: recovered,
    })
}

fn recover_interrupted_transactions(root: &Path) -> Result<usize, AppError> {
    let base = transaction_base(root)?;
    if !base.exists() {
        return Ok(0);
    }
    reject_symlink(&base)?;
    if !base.is_dir() {
        return Err(AppError::internal(
            "OpenAgent transaction path is not a directory",
        ));
    }
    let mut recovered = 0usize;
    let mut entries = fs::read_dir(&base)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let tx_dir = entry.path();
        if !tx_dir.is_dir() || fs::symlink_metadata(&tx_dir)?.file_type().is_symlink() {
            return Err(AppError::internal(
                "OpenAgent transaction store contains an unsafe entry",
            ));
        }
        let journal_path = tx_dir.join("journal.json");
        if !journal_path.is_file() {
            return Err(AppError::internal(
                "OpenAgent found an incomplete transaction without a journal",
            ));
        }
        let journal: Journal =
            serde_json::from_slice(&fs::read(&journal_path)?).map_err(|error| {
                AppError::internal(format!("invalid OpenAgent transaction journal: {error}"))
            })?;
        rollback_transaction(root, &tx_dir, &journal)?;
        fs::remove_dir_all(&tx_dir)?;
        recovered += 1;
    }
    let _ = fs::remove_dir(&base);
    Ok(recovered)
}

fn rollback_transaction(root: &Path, tx_dir: &Path, journal: &Journal) -> Result<(), AppError> {
    for (index, entry) in journal.entries.iter().enumerate().rev() {
        let relative = validate_relative_path(&entry.relative_path)?;
        let target = resolve_scoped_target(root, &relative)?;
        let backup = tx_dir.join(&entry.backup_name);
        let original_copy = tx_dir.join(format!("{index}.original"));
        if entry.original_existed {
            if backup.exists() {
                if target.exists() {
                    remove_regular_file(&target)?;
                }
                fs::rename(&backup, &target)?;
            } else if !target.exists() && original_copy.is_file() {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(&original_copy, &target)?;
            }
        } else if target.exists() {
            remove_regular_file(&target)?;
        }
    }
    Ok(())
}

fn cleanup_transaction_dir(base: &Path, tx_dir: &Path) -> Result<(), AppError> {
    if tx_dir.exists() {
        fs::remove_dir_all(tx_dir)?;
    }
    if base.exists() {
        let _ = fs::remove_dir(base);
    }
    Ok(())
}

fn transaction_base(root: &Path) -> Result<PathBuf, AppError> {
    let base = root.join(TRANSACTION_DIR);
    if base.exists() {
        let metadata = fs::symlink_metadata(&base)?;
        if metadata.file_type().is_symlink() {
            return Err(AppError::internal(
                "OpenAgent transaction directory cannot be a symlink",
            ));
        }
    }
    Ok(base)
}

fn validate_relative_path(raw: &str) -> Result<PathBuf, AppError> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains('\0') {
        return Err(AppError::internal("patch_transaction path is invalid"));
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(AppError::internal(
            "patch_transaction accepts workspace-relative paths only",
        ));
    }
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                if value == TRANSACTION_DIR {
                    return Err(AppError::internal(
                        "patch_transaction cannot edit its transaction store",
                    ));
                }
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(AppError::internal(
                    "patch_transaction path may not escape the workspace",
                ))
            }
        }
    }
    Ok(path.to_path_buf())
}

fn resolve_scoped_target(root: &Path, relative: &Path) -> Result<PathBuf, AppError> {
    let target = root.join(relative);
    let security_path = if target.exists() {
        fs::canonicalize(&target)?
    } else {
        canonical_existing_parent(&target)?
    };
    if !security_path.starts_with(root) {
        return Err(AppError::internal(
            "patch_transaction path escaped the attached workspace",
        ));
    }
    Ok(target)
}

fn canonical_existing_parent(path: &Path) -> Result<PathBuf, AppError> {
    let mut probe = path.to_path_buf();
    while !probe.exists() {
        if !probe.pop() {
            return Err(AppError::internal("no existing parent directory found"));
        }
    }
    fs::canonicalize(probe).map_err(AppError::from)
}

fn normalize_relative(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn reject_symlink(path: &Path) -> Result<(), AppError> {
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(AppError::internal("unsafe symlink in transaction path"));
    }
    Ok(())
}

fn remove_regular_file(path: &Path) -> Result<(), AppError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AppError::internal(
            "patch transaction rollback encountered a non-regular file",
        ));
    }
    fs::remove_file(path)?;
    Ok(())
}

fn write_synced_json(path: &Path, value: &impl Serialize) -> Result<(), AppError> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        AppError::internal(format!("failed to serialize transaction journal: {error}"))
    })?;
    write_synced_bytes(path, &bytes)
}

fn write_synced_bytes(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn transaction_replaces_multiple_files_and_creates_one() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("a.rs"), "fn a() { 1 }\n").unwrap();
        fs::write(temp.path().join("b.rs"), "fn b() { 2 }\n").unwrap();
        let result = apply_patch_transaction(
            temp.path(),
            &json!({
                "operations": [
                    {"op":"replace","path":"a.rs","old":"1","new":"10"},
                    {"op":"replace","path":"b.rs","old":"2","new":"20"},
                    {"op":"create","path":"c.rs","content":"fn c() {}\n"}
                ]
            }),
        )
        .unwrap();
        assert_eq!(result.changed_files, 3);
        assert_eq!(
            fs::read_to_string(temp.path().join("a.rs")).unwrap(),
            "fn a() { 10 }\n"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("b.rs")).unwrap(),
            "fn b() { 20 }\n"
        );
        assert!(temp.path().join("c.rs").is_file());
        assert!(!temp.path().join(TRANSACTION_DIR).exists());
    }

    #[test]
    fn preflight_failure_leaves_every_file_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("a.txt"), "alpha").unwrap();
        fs::write(temp.path().join("b.txt"), "beta").unwrap();
        let error = apply_patch_transaction(
            temp.path(),
            &json!({
                "operations": [
                    {"op":"replace","path":"a.txt","old":"alpha","new":"changed"},
                    {"op":"replace","path":"b.txt","old":"missing","new":"changed"}
                ]
            }),
        )
        .unwrap_err();
        assert!(error.to_string().contains("exactly one match"));
        assert_eq!(
            fs::read_to_string(temp.path().join("a.txt")).unwrap(),
            "alpha"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("b.txt")).unwrap(),
            "beta"
        );
    }

    #[test]
    fn transaction_paths_reject_escape_and_duplicates_are_caught_in_preflight() {
        assert!(transaction_paths(&json!({
            "operations": [{"op":"create","path":"../escape","content":"x"}]
        }))
        .is_err());
        let temp = tempfile::tempdir().unwrap();
        assert!(apply_patch_transaction(
            temp.path(),
            &json!({"operations":[
                {"op":"create","path":"same.txt","content":"a"},
                {"op":"create","path":"same.txt","content":"b"}
            ]})
        )
        .is_err());
    }
}
