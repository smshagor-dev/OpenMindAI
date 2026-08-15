use std::fs;

use chrono::Utc;
use rusqlite::params;
use serde::Serialize;

use crate::{
    app_error::AppError,
    database::Database,
    hardware::HardwareProfile,
    model_registry::{ModelLifecycleState, ModelRegistry},
    portable_root::{available_bytes_for_path, PortableRootManager, CURRENT_SCHEMA_VERSION},
    runtime::LlamaRuntimeManager,
};

const LOW_DISK_SPACE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CheckStatus {
    Ok,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticCheck {
    pub id: String,
    pub label: String,
    pub status: CheckStatus,
    pub detail: Option<String>,
}

impl DiagnosticCheck {
    fn new(id: &str, label: &str, status: CheckStatus, detail: Option<String>) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            status,
            detail,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticReport {
    pub checks: Vec<DiagnosticCheck>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairSummary {
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub name: String,
    pub size_bytes: u64,
    pub created_at: String,
}

/// Runs a read-only health sweep: storage writability, database integrity,
/// schema version, AI runtime availability, AI model readiness, and free
/// disk space. Intentionally has no side effects — repair is a separate,
/// explicit action.
pub fn run_diagnostics(
    root: &PortableRootManager,
    database: &Database,
    hardware: &HardwareProfile,
    runtime: &LlamaRuntimeManager,
) -> Result<DiagnosticReport, AppError> {
    let mut checks = Vec::new();

    checks.push(match root.validate_root() {
        Ok(()) => DiagnosticCheck::new("storage", "Storage", CheckStatus::Ok, None),
        Err(error) => DiagnosticCheck::new(
            "storage",
            "Storage",
            CheckStatus::Error,
            Some(error.to_string()),
        ),
    });

    checks.push(database_integrity_check(database));
    checks.push(schema_version_check(database));

    let inventory = runtime.inventory(hardware)?;
    checks.push(if inventory.selected.is_some() {
        DiagnosticCheck::new("runtime", "AI Runtime", CheckStatus::Ok, None)
    } else {
        DiagnosticCheck::new(
            "runtime",
            "AI Runtime",
            CheckStatus::Warning,
            Some("No usable llama.cpp runtime installed yet".to_string()),
        )
    });

    let models = ModelRegistry::new(database, root).discover_gguf_models()?;
    let model_ready = models
        .iter()
        .any(|model| model.state == ModelLifecycleState::Ready);
    checks.push(if model_ready {
        DiagnosticCheck::new("model", "AI Model", CheckStatus::Ok, None)
    } else {
        DiagnosticCheck::new(
            "model",
            "AI Model",
            CheckStatus::Warning,
            Some("No verified AI model installed yet".to_string()),
        )
    });

    checks.push(match available_bytes_for_path(root.root()) {
        Some(available) if available < LOW_DISK_SPACE_BYTES => DiagnosticCheck::new(
            "diskSpace",
            "Storage Space",
            CheckStatus::Warning,
            Some(format!("Only {available} bytes free")),
        ),
        Some(_) => DiagnosticCheck::new("diskSpace", "Storage Space", CheckStatus::Ok, None),
        None => DiagnosticCheck::new(
            "diskSpace",
            "Storage Space",
            CheckStatus::Warning,
            Some("Could not determine free disk space".to_string()),
        ),
    });

    checks.push(DiagnosticCheck::new(
        "hardware",
        "Hardware Detection",
        CheckStatus::Ok,
        Some(format!(
            "{} logical threads detected",
            hardware.cpu.logical_threads
        )),
    ));

    Ok(DiagnosticReport { checks })
}

fn database_integrity_check(database: &Database) -> DiagnosticCheck {
    let result: Result<String, _> =
        database
            .connection()
            .query_row("PRAGMA integrity_check", [], |row| row.get(0));
    match result {
        Ok(value) if value.eq_ignore_ascii_case("ok") => {
            DiagnosticCheck::new("database", "Database", CheckStatus::Ok, None)
        }
        Ok(value) => DiagnosticCheck::new("database", "Database", CheckStatus::Error, Some(value)),
        Err(error) => DiagnosticCheck::new(
            "database",
            "Database",
            CheckStatus::Error,
            Some(error.to_string()),
        ),
    }
}

fn schema_version_check(database: &Database) -> DiagnosticCheck {
    match database.schema_version() {
        Ok(version) if version == CURRENT_SCHEMA_VERSION => {
            DiagnosticCheck::new("schema", "Database Schema", CheckStatus::Ok, None)
        }
        Ok(version) => DiagnosticCheck::new(
            "schema",
            "Database Schema",
            CheckStatus::Warning,
            Some(format!(
                "schema version {version} does not match expected {CURRENT_SCHEMA_VERSION}"
            )),
        ),
        Err(error) => DiagnosticCheck::new(
            "schema",
            "Database Schema",
            CheckStatus::Error,
            Some(error.to_string()),
        ),
    }
}

/// Snapshots the live database into `backups/` using `VACUUM INTO`, the
/// correct way to back up a WAL-mode SQLite database while it's open — a
/// raw file copy can capture a torn, inconsistent state.
pub fn backup_database(
    root: &PortableRootManager,
    database: &Database,
) -> Result<BackupInfo, AppError> {
    let backups_dir = root.resolve_relative("backups")?;
    fs::create_dir_all(&backups_dir)?;
    let filename = format!("openmindai-{}.db", Utc::now().format("%Y%m%d-%H%M%S"));
    let dest_path = backups_dir.join(&filename);

    database
        .connection()
        .execute("VACUUM INTO ?1", params![dest_path.display().to_string()])?;

    let size_bytes = fs::metadata(&dest_path)?.len();
    Ok(BackupInfo {
        name: filename,
        size_bytes,
        created_at: Utc::now().to_rfc3339(),
    })
}

pub fn list_backups(root: &PortableRootManager) -> Result<Vec<BackupInfo>, AppError> {
    let backups_dir = root.resolve_relative("backups")?;
    if !backups_dir.exists() {
        return Ok(Vec::new());
    }
    let mut backups = Vec::new();
    for entry in fs::read_dir(&backups_dir)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if !metadata.is_file() {
            continue;
        }
        let modified: chrono::DateTime<Utc> = metadata.modified()?.into();
        backups.push(BackupInfo {
            name: entry.file_name().to_string_lossy().to_string(),
            size_bytes: metadata.len(),
            created_at: modified.to_rfc3339(),
        });
    }
    backups.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(backups)
}

/// Opens `relative` (resolved under the OpenMindAI root) in the OS file
/// explorer — mirrors `lib.rs`'s existing `reveal_artifact_in_folder`
/// pattern, generalized since Maintenance needs it for both `logs/` and
/// `backups/`.
pub fn open_folder_in_root(root: &PortableRootManager, relative: &str) -> Result<(), AppError> {
    let path = root.resolve_relative(relative)?;
    fs::create_dir_all(&path)?;
    std::process::Command::new("explorer")
        .arg(&path)
        .spawn()
        .map_err(|error| AppError::internal(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (PortableRootManager, Database) {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let database = Database::open(root.database_path()).unwrap();
        (root, database)
    }

    #[test]
    fn healthy_setup_reports_ok_storage_and_database() {
        let (root, database) = setup();
        let hardware = crate::hardware::HardwareProfiler::detect();
        let runtime = LlamaRuntimeManager::new(root.clone());

        let report = run_diagnostics(&root, &database, &hardware, &runtime).unwrap();
        let by_id = |id: &str| report.checks.iter().find(|check| check.id == id).unwrap();

        assert_eq!(by_id("storage").status, CheckStatus::Ok);
        assert_eq!(by_id("database").status, CheckStatus::Ok);
        assert_eq!(by_id("schema").status, CheckStatus::Ok);
        // No runtime/model installed in a fresh temp root — these are
        // expected warnings, not errors (repair, not a broken install).
        assert_eq!(by_id("runtime").status, CheckStatus::Warning);
        assert_eq!(by_id("model").status, CheckStatus::Warning);
    }

    #[test]
    fn backup_produces_a_valid_openable_database() {
        let (root, database) = setup();
        let info = backup_database(&root, &database).unwrap();

        let backup_path = root.resolve_relative("backups").unwrap().join(&info.name);
        assert!(backup_path.is_file());
        assert!(info.size_bytes > 0);

        // The one non-negotiable check: the backup must itself be a valid,
        // openable SQLite database — a backup nobody can restore is worse
        // than no backup.
        let reopened = rusqlite::Connection::open(&backup_path).unwrap();
        let table_count: i64 = reopened
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(table_count > 0);

        let backups = list_backups(&root).unwrap();
        assert_eq!(backups.len(), 1);
        assert_eq!(backups[0].name, info.name);
    }
}
