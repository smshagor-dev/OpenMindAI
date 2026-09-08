use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::app_error::AppError;

const MAX_TRANSACTION_OPERATIONS: usize = 64;
const MAX_TRANSACTION_BYTES: usize = 16 * 1024 * 1024;
const TRANSACTION_DIR: &str = ".openmindai-patch-transactions";
const JOURNAL_FILE: &str = "journal.json";
const COMMITTED_FILE: &str = "committed.json";
const JOURNAL_VERSION: u32 = 2;
const COMMIT_MARKER_VERSION: u32 = 1;

static PATCH_TRANSACTION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", tag = "op")]
enum PatchOperation {
    #[serde(rename = "replace")]
    Replace {
        path: String,
        old: String,
        new: String,
        #[serde(default)]
        expected_sha256: Option<String>,
    },
    #[serde(rename = "create")]
    Create { path: String, content: String },
    #[serde(rename = "delete")]
    Delete {
        path: String,
        #[serde(default)]
        expected_sha256: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlannedKind {
    Create,
    Replace,
    Delete,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchTransactionResult {
    pub transaction_id: String,
    pub changed_files: usize,
    pub created_files: usize,
    pub replaced_files: usize,
    pub deleted_files: usize,
    pub paths: Vec<String>,
    pub recovered_interrupted_transactions: usize,
    pub cleanup_deferred: bool,
}

#[derive(Debug)]
struct PlannedChange {
    relative_path: String,
    target: PathBuf,
    original: Option<Vec<u8>>,
    replacement: Option<Vec<u8>>,
    kind: PlannedKind,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommitMarker {
    version: u32,
    transaction_id: String,
}

pub fn transaction_paths(action: &Value) -> Result<Vec<String>, AppError> {
    let operations = parse_operations(action)?;
    operations
        .iter()
        .map(|operation| match operation {
            PatchOperation::Replace { path, .. }
            | PatchOperation::Create { path, .. }
            | PatchOperation::Delete { path, .. } => {
                validate_relative_path(path).map(|_| path.clone())
            }
        })
        .collect()
}

pub fn apply_patch_transaction(
    root: &Path,
    action: &Value,
) -> Result<PatchTransactionResult, AppError> {
    let lock = PATCH_TRANSACTION_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock
        .lock()
        .map_err(|_| AppError::internal("patch transaction lock is poisoned"))?;

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
        let path = match operation {
            PatchOperation::Replace { path, .. }
            | PatchOperation::Create { path, .. }
            | PatchOperation::Delete { path, .. } => path,
        };
        let relative = validate_relative_path(path)?;
        let relative_path = normalize_relative(&relative);
        if !seen.insert(relative_path.clone()) {
            return Err(AppError::internal(format!(
                "patch_transaction contains duplicate target path: {relative_path}"
            )));
        }
        let target = resolve_scoped_target(root, &relative)?;

        let change = match operation {
            PatchOperation::Replace {
                old,
                new,
                expected_sha256,
                ..
            } => {
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
                verify_expected_sha256(expected_sha256.as_deref(), &original, &relative_path)?;
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
                    replacement: Some(replacement),
                    kind: PlannedKind::Replace,
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
                    replacement: Some(replacement),
                    kind: PlannedKind::Create,
                }
            }
            PatchOperation::Delete {
                expected_sha256, ..
            } => {
                if !target.is_file() {
                    return Err(AppError::internal(format!(
                        "delete operation target is not a file: {relative_path}"
                    )));
                }
                let original = fs::read(&target)?;
                verify_expected_sha256(expected_sha256.as_deref(), &original, &relative_path)?;
                total_bytes = total_bytes.saturating_add(original.len());
                PlannedChange {
                    relative_path,
                    target,
                    original: Some(original),
                    replacement: None,
                    kind: PlannedKind::Delete,
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
        version: JOURNAL_VERSION,
        transaction_id: transaction_id.clone(),
        entries,
    };
    write_synced_json_atomic(&tx_dir.join(JOURNAL_FILE), &journal)?;

    for (index, change) in planned.iter().enumerate() {
        if let Some(replacement) = &change.replacement {
            write_synced_bytes_new(&tx_dir.join(format!("{index}.stage")), replacement)?;
        }
        if let Some(original) = &change.original {
            write_synced_bytes_new(&tx_dir.join(format!("{index}.original")), original)?;
        }
    }

    let apply_result = (|| -> Result<(), AppError> {
        for (index, change) in planned.iter().enumerate() {
            if let Some(parent) = change.target.parent() {
                create_scoped_parent_directories(root, parent)?;
            }
            if change.target.exists() {
                reject_symlink(&change.target)?;
                let backup = tx_dir.join(format!("{index}.backup"));
                fs::rename(&change.target, &backup)?;
            }
            if change.replacement.is_some() {
                let stage = tx_dir.join(format!("{index}.stage"));
                fs::rename(stage, &change.target)?;
            }
        }

        let marker = CommitMarker {
            version: COMMIT_MARKER_VERSION,
            transaction_id: transaction_id.clone(),
        };
        write_synced_json_atomic(&tx_dir.join(COMMITTED_FILE), &marker)?;
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
    let created_files = planned
        .iter()
        .filter(|change| change.kind == PlannedKind::Create)
        .count();
    let replaced_files = planned
        .iter()
        .filter(|change| change.kind == PlannedKind::Replace)
        .count();
    let deleted_files = planned
        .iter()
        .filter(|change| change.kind == PlannedKind::Delete)
        .count();
    let cleanup_deferred = cleanup_transaction_dir(&base, &tx_dir).is_err();

    Ok(PatchTransactionResult {
        transaction_id,
        changed_files: planned.len(),
        created_files,
        replaced_files,
        deleted_files,
        paths,
        recovered_interrupted_transactions: recovered,
        cleanup_deferred,
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
        let metadata = fs::symlink_metadata(&tx_dir)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(AppError::internal(
                "OpenAgent transaction store contains an unsafe entry",
            ));
        }

        let journal_path = tx_dir.join(JOURNAL_FILE);
        if !journal_path.is_file() {
            if transaction_dir_has_payload(&tx_dir)? {
                return Err(AppError::internal(
                    "OpenAgent found an incomplete transaction without a journal",
                ));
            }
            fs::remove_dir_all(&tx_dir)?;
            recovered += 1;
            continue;
        }

        let journal: Journal = match serde_json::from_slice(&fs::read(&journal_path)?) {
            Ok(journal) => journal,
            Err(error) => {
                if transaction_dir_has_payload(&tx_dir)? {
                    return Err(AppError::internal(format!(
                        "invalid OpenAgent transaction journal: {error}"
                    )));
                }
                fs::remove_dir_all(&tx_dir)?;
                recovered += 1;
                continue;
            }
        };
        validate_journal(&tx_dir, &journal)?;

        if committed_marker_matches(&tx_dir, &journal.transaction_id)? {
            fs::remove_dir_all(&tx_dir)?;
            recovered += 1;
            continue;
        }

        rollback_transaction(root, &tx_dir, &journal)?;
        fs::remove_dir_all(&tx_dir)?;
        recovered += 1;
    }

    let _ = fs::remove_dir(&base);
    Ok(recovered)
}

fn validate_journal(tx_dir: &Path, journal: &Journal) -> Result<(), AppError> {
    if !(1..=JOURNAL_VERSION).contains(&journal.version) {
        return Err(AppError::internal(format!(
            "unsupported OpenAgent transaction journal version: {}",
            journal.version
        )));
    }
    if journal.entries.is_empty() || journal.entries.len() > MAX_TRANSACTION_OPERATIONS {
        return Err(AppError::internal(
            "OpenAgent transaction journal contains an invalid entry count",
        ));
    }
    let directory_id = tx_dir
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| AppError::internal("OpenAgent transaction directory name is invalid"))?;
    if journal.transaction_id != directory_id {
        return Err(AppError::internal(
            "OpenAgent transaction journal id does not match its directory",
        ));
    }

    let mut paths = HashSet::new();
    for (index, entry) in journal.entries.iter().enumerate() {
        let relative = validate_relative_path(&entry.relative_path)?;
        let normalized = normalize_relative(&relative);
        if !paths.insert(normalized) {
            return Err(AppError::internal(
                "OpenAgent transaction journal contains duplicate paths",
            ));
        }
        if entry.backup_name != format!("{index}.backup")
            || entry.staged_name != format!("{index}.stage")
        {
            return Err(AppError::internal(
                "OpenAgent transaction journal contains unsafe artifact names",
            ));
        }
    }
    Ok(())
}

fn committed_marker_matches(tx_dir: &Path, transaction_id: &str) -> Result<bool, AppError> {
    let path = tx_dir.join(COMMITTED_FILE);
    if !path.exists() {
        return Ok(false);
    }
    if !path.is_file() || fs::symlink_metadata(&path)?.file_type().is_symlink() {
        return Ok(false);
    }
    let marker: CommitMarker = match serde_json::from_slice(&fs::read(path)?) {
        Ok(marker) => marker,
        Err(_) => return Ok(false),
    };
    Ok(marker.version == COMMIT_MARKER_VERSION && marker.transaction_id == transaction_id)
}

fn transaction_dir_has_payload(tx_dir: &Path) -> Result<bool, AppError> {
    for entry in fs::read_dir(tx_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".backup")
            || name.ends_with(".original")
            || name.ends_with(".stage")
            || name == COMMITTED_FILE
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn rollback_transaction(root: &Path, tx_dir: &Path, journal: &Journal) -> Result<(), AppError> {
    for (index, entry) in journal.entries.iter().enumerate().rev() {
        let relative = validate_relative_path(&entry.relative_path)?;
        let target = resolve_scoped_target(root, &relative)?;
        let backup = tx_dir.join(&entry.backup_name);
        let original_copy = tx_dir.join(format!("{index}.original"));

        if entry.original_existed {
            if backup.exists() {
                reject_symlink(&backup)?;
                if target.exists() {
                    remove_regular_file(&target)?;
                }
                if let Some(parent) = target.parent() {
                    create_scoped_parent_directories(root, parent)?;
                }
                fs::rename(&backup, &target)?;
            } else if !target.exists() && original_copy.is_file() {
                reject_symlink(&original_copy)?;
                if let Some(parent) = target.parent() {
                    create_scoped_parent_directories(root, parent)?;
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
    ensure_no_symlink_components(root, relative)?;
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

fn ensure_no_symlink_components(root: &Path, relative: &Path) -> Result<(), AppError> {
    let mut probe = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(value) = component else {
            continue;
        };
        probe.push(value);
        if !probe.exists() {
            continue;
        }
        if fs::symlink_metadata(&probe)?.file_type().is_symlink() {
            return Err(AppError::internal(
                "patch_transaction refuses paths containing symlinks",
            ));
        }
    }
    Ok(())
}

fn create_scoped_parent_directories(root: &Path, parent: &Path) -> Result<(), AppError> {
    if !parent.starts_with(root) {
        return Err(AppError::internal(
            "patch_transaction parent directory escaped the workspace",
        ));
    }
    fs::create_dir_all(parent)?;
    let relative = parent.strip_prefix(root).map_err(|_| {
        AppError::internal("patch_transaction parent directory escaped the workspace")
    })?;
    ensure_no_symlink_components(root, relative)
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

fn verify_expected_sha256(
    expected: Option<&str>,
    bytes: &[u8],
    relative_path: &str,
) -> Result<(), AppError> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let expected = expected.trim().to_ascii_lowercase();
    if expected.len() != 64
        || !expected
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(AppError::internal(format!(
            "invalid expectedSha256 precondition for {relative_path}"
        )));
    }
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected {
        return Err(AppError::internal(format!(
            "patch_transaction stale-file precondition failed for {relative_path}: expected {expected}, found {actual}"
        )));
    }
    Ok(())
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

fn write_synced_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), AppError> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        AppError::internal(format!("failed to serialize transaction metadata: {error}"))
    })?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| AppError::internal("transaction metadata path is invalid"))?;
    let temp = path.with_file_name(format!("{file_name}.tmp"));
    if temp.exists() {
        remove_regular_file(&temp)?;
    }
    write_synced_bytes_new(&temp, &bytes)?;
    fs::rename(&temp, path)?;
    Ok(())
}

fn write_synced_bytes_new(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
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
        assert_eq!(result.created_files, 1);
        assert_eq!(result.replaced_files, 2);
        assert_eq!(result.deleted_files, 0);
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
    fn transaction_deletes_file_atomically() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("remove.txt"), "remove me").unwrap();
        fs::write(temp.path().join("keep.txt"), "old").unwrap();
        let result = apply_patch_transaction(
            temp.path(),
            &json!({
                "operations": [
                    {"op":"delete","path":"remove.txt"},
                    {"op":"replace","path":"keep.txt","old":"old","new":"new"}
                ]
            }),
        )
        .unwrap();
        assert_eq!(result.deleted_files, 1);
        assert!(!temp.path().join("remove.txt").exists());
        assert_eq!(
            fs::read_to_string(temp.path().join("keep.txt")).unwrap(),
            "new"
        );
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
    fn stale_hash_precondition_blocks_transaction() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("a.txt"), "alpha").unwrap();
        let error = apply_patch_transaction(
            temp.path(),
            &json!({
                "operations": [{
                    "op":"replace",
                    "path":"a.txt",
                    "old":"alpha",
                    "new":"changed",
                    "expectedSha256":"0000000000000000000000000000000000000000000000000000000000000000"
                }]
            }),
        )
        .unwrap_err();
        assert!(error.to_string().contains("stale-file precondition"));
        assert_eq!(
            fs::read_to_string(temp.path().join("a.txt")).unwrap(),
            "alpha"
        );
    }

    #[test]
    fn committed_recovery_keeps_applied_content() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("a.txt"), "new").unwrap();
        let base = temp.path().join(TRANSACTION_DIR);
        fs::create_dir(&base).unwrap();
        let transaction_id = "tx-committed";
        let tx_dir = base.join(transaction_id);
        fs::create_dir(&tx_dir).unwrap();
        fs::write(tx_dir.join("0.backup"), "old").unwrap();
        let journal = Journal {
            version: JOURNAL_VERSION,
            transaction_id: transaction_id.to_string(),
            entries: vec![JournalEntry {
                relative_path: "a.txt".to_string(),
                original_existed: true,
                backup_name: "0.backup".to_string(),
                staged_name: "0.stage".to_string(),
            }],
        };
        fs::write(
            tx_dir.join(JOURNAL_FILE),
            serde_json::to_vec(&journal).unwrap(),
        )
        .unwrap();
        let marker = CommitMarker {
            version: COMMIT_MARKER_VERSION,
            transaction_id: transaction_id.to_string(),
        };
        fs::write(
            tx_dir.join(COMMITTED_FILE),
            serde_json::to_vec(&marker).unwrap(),
        )
        .unwrap();

        assert_eq!(recover_interrupted_transactions(temp.path()).unwrap(), 1);
        assert_eq!(
            fs::read_to_string(temp.path().join("a.txt")).unwrap(),
            "new"
        );
        assert!(!base.exists());
    }

    #[test]
    fn interrupted_recovery_restores_backup() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("a.txt"), "new").unwrap();
        let base = temp.path().join(TRANSACTION_DIR);
        fs::create_dir(&base).unwrap();
        let transaction_id = "tx-interrupted";
        let tx_dir = base.join(transaction_id);
        fs::create_dir(&tx_dir).unwrap();
        fs::write(tx_dir.join("0.backup"), "old").unwrap();
        let journal = Journal {
            version: JOURNAL_VERSION,
            transaction_id: transaction_id.to_string(),
            entries: vec![JournalEntry {
                relative_path: "a.txt".to_string(),
                original_existed: true,
                backup_name: "0.backup".to_string(),
                staged_name: "0.stage".to_string(),
            }],
        };
        fs::write(
            tx_dir.join(JOURNAL_FILE),
            serde_json::to_vec(&journal).unwrap(),
        )
        .unwrap();

        assert_eq!(recover_interrupted_transactions(temp.path()).unwrap(), 1);
        assert_eq!(
            fs::read_to_string(temp.path().join("a.txt")).unwrap(),
            "old"
        );
        assert!(!base.exists());
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
