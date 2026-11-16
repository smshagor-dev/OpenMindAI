//! Crash-safe installation pointers. A broken pointer is never a fresh install.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use crate::{app_error::AppError, portable_root::InstallConfig};

const MAX_POINTER_BYTES: u64 = 64 * 1024;

fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".bak");
    PathBuf::from(name)
}

fn validate(config: &InstallConfig) -> Result<(), AppError> {
    if config.product != "OpenMindAI"
        || config.root.trim().is_empty()
        || config.database.is_empty()
        || config.database == "."
        || config.database == ".."
        || config.database.contains(['/', '\\', ':'])
    {
        return Err(AppError::SetupRequired(
            "Invalid installation pointer fields".into(),
        ));
    }
    Ok(())
}

fn read(path: &Path) -> Result<Option<InstallConfig>, AppError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_POINTER_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_POINTER_BYTES {
        return Err(AppError::SetupRequired(
            "Installation pointer is too large".into(),
        ));
    }
    let config = serde_json::from_slice(&bytes)
        .map_err(|error| AppError::SetupRequired(error.to_string()))?;
    validate(&config)?;
    Ok(Some(config))
}

fn encode(config: &InstallConfig) -> Result<Vec<u8>, AppError> {
    validate(config)?;
    let bytes =
        serde_json::to_vec_pretty(config).map_err(|error| AppError::internal(error.to_string()))?;
    if bytes.len() as u64 > MAX_POINTER_BYTES {
        return Err(AppError::SetupRequired(
            "Installation pointer is too large".into(),
        ));
    }
    Ok(bytes)
}

fn recovery_error(path: &Path) -> AppError {
    AppError::SetupRequired(format!(
        "Cannot read a valid installation pointer at {} or its backup {}. Restore a valid install.json from your backup or repair its permissions, then restart OpenMindAI. Your existing data has not been moved or replaced.",
        path.display(), backup_path(path).display()
    ))
}

pub fn load(path: &Path) -> Result<Option<InstallConfig>, AppError> {
    let primary = read(path);
    if let Ok(Some(config)) = primary {
        return Ok(Some(config));
    }
    match read(&backup_path(path)) {
        Ok(Some(config)) => {
            // Restore the primary before allowing startup to open any data store.
            atomic_write(path, &encode(&config)?)?;
            Ok(Some(config))
        }
        Ok(None) if matches!(primary, Ok(None)) => Ok(None),
        _ => Err(recovery_error(path)),
    }
}

pub fn save(path: &Path, config: &InstallConfig) -> Result<(), AppError> {
    save_using(path, config, atomic_write)
}

fn save_using(
    path: &Path,
    config: &InstallConfig,
    mut write: impl FnMut(&Path, &[u8]) -> Result<(), AppError>,
) -> Result<(), AppError> {
    let bytes = encode(config)?;
    let previous = match read(path) {
        Ok(Some(previous)) => Some(previous),
        Ok(None) => read(&backup_path(path)).map_err(|_| recovery_error(path))?,
        Err(_) => Some(
            read(&backup_path(path))
                .ok()
                .flatten()
                .ok_or_else(|| recovery_error(path))?,
        ),
    };
    if let Some(previous) = previous {
        // Do not replace a good backup with malformed or partially written JSON.
        write(&backup_path(path), &encode(&previous)?)?;
        write(path, &bytes)?;
    } else {
        write(path, &bytes)?;
        // A first successful install also has a recovery copy.
        write(&backup_path(path), &bytes)?;
    }
    Ok(())
}

struct TemporaryPath(PathBuf);
impl Drop for TemporaryPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    atomic_write_with(path, |file| {
        file.write_all(bytes)?;
        file.sync_all()
    })
}

fn atomic_write_with(
    path: &Path,
    write_and_sync: impl FnOnce(&mut File) -> io::Result<()>,
) -> Result<(), AppError> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let temporary = TemporaryPath(parent.join(format!(".install-{}.tmp", uuid::Uuid::new_v4())));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary.0)?;
    write_and_sync(&mut file)?;
    drop(file);
    // Same-directory rename replaces the destination atomically on supported
    // desktop filesystems, including Windows. Never remove the live file first.
    fs::rename(&temporary.0, path)?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(root: &str) -> InstallConfig {
        InstallConfig {
            product: "OpenMindAI".into(),
            app_version: "4.0.0".into(),
            root: root.into(),
            schema_version: 8,
            installation_complete: true,
            database: "chosen-profile.db".into(),
            runtime_ready: true,
            model_ready: true,
            setup_state: None,
        }
    }

    #[test]
    fn replacement_retains_last_good_pointer_and_restores_corrupt_primary() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config/install.json");
        assert!(load(&path).unwrap().is_none());
        save(&path, &config("old-root")).unwrap();
        assert_eq!(read(&backup_path(&path)).unwrap().unwrap().root, "old-root");
        save(&path, &config("new-root")).unwrap();
        assert_eq!(load(&path).unwrap().unwrap().root, "new-root");
        assert_eq!(read(&backup_path(&path)).unwrap().unwrap().root, "old-root");
        fs::write(&path, b"{interrupted").unwrap();
        let recovered = load(&path).unwrap().unwrap();
        assert_eq!(recovered.root, "old-root");
        assert_eq!(recovered.database, "chosen-profile.db");
        assert_eq!(read(&path).unwrap().unwrap().root, "old-root");
        assert_eq!(load(&path).unwrap().unwrap().root, "old-root");
    }

    #[test]
    fn missing_primary_recovers_backup_but_corruption_is_never_a_fresh_install() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("install.json");
        save(&path, &config("existing")).unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(load(&path).unwrap().unwrap().root, "existing");
        fs::write(&path, b"partial").unwrap();
        fs::write(backup_path(&path), b"also partial").unwrap();
        let error = load(&path).unwrap_err().to_string();
        assert!(error.contains("Restore a valid install.json"));
        fs::remove_file(&path).unwrap();
        assert!(load(&path).is_err());
        assert!(save(&path, &config("must-not-reset")).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn interrupted_and_disk_full_writes_leave_live_bytes_and_no_temporary_files() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("install.json");
        save(&path, &config("existing")).unwrap();
        let original = fs::read(&path).unwrap();
        for fail_after_sync in [false, true] {
            let result = atomic_write_with(&path, |file| {
                file.write_all(b"{partial replacement")?;
                if fail_after_sync {
                    file.sync_all()?;
                }
                Err(io::Error::new(
                    io::ErrorKind::StorageFull,
                    "injected disk full/interruption",
                ))
            });
            assert!(result.is_err());
            assert_eq!(fs::read(&path).unwrap(), original);
            assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
        }
    }

    #[test]
    fn failed_backup_or_primary_promotion_preserves_previous_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("install.json");
        save(&path, &config("previous")).unwrap();
        let original = fs::read(&path).unwrap();
        for failed_path in [backup_path(&path), path.clone()] {
            let result = save_using(&path, &config("replacement"), |target, bytes| {
                if target == failed_path {
                    return Err(AppError::internal("injected promotion failure"));
                }
                atomic_write(target, bytes)
            });
            assert!(result.is_err());
            assert_eq!(fs::read(&path).unwrap(), original);
            assert_eq!(read(&backup_path(&path)).unwrap().unwrap().root, "previous");
        }
    }

    #[test]
    fn recovery_cannot_report_success_when_primary_cannot_be_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("install.json");
        save(&path, &config("existing")).unwrap();
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        fs::write(path.join("blocker"), b"untouched").unwrap();
        assert!(load(&path).is_err());
        assert_eq!(read(&backup_path(&path)).unwrap().unwrap().root, "existing");
        assert_eq!(fs::read(path.join("blocker")).unwrap(), b"untouched");
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
    }

    #[test]
    fn invalid_or_oversized_pointers_are_rejected_before_any_write() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("install.json");
        save(&path, &config("existing")).unwrap();
        let original = fs::read(&path).unwrap();
        for database in ["", ".", "..", "../other.db", "..\\other.db", "C:other.db"] {
            let mut invalid = config("root");
            invalid.database = database.into();
            assert!(save(&path, &invalid).is_err());
        }
        assert!(save(&path, &config(&"x".repeat(MAX_POINTER_BYTES as usize))).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::write(&path, vec![b' '; MAX_POINTER_BYTES as usize + 1]).unwrap();
        assert_eq!(load(&path).unwrap().unwrap().root, "existing");
        fs::write(backup_path(&path), b"{}").unwrap();
        assert!(load(&path).unwrap().is_some()); // valid primary still wins
        fs::write(&path, b"{}").unwrap();
        assert!(load(&path).is_err());
    }

    #[test]
    fn saving_over_corruption_keeps_valid_backup_instead_of_poisoning_it() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("install.json");
        save(&path, &config("recoverable")).unwrap();
        fs::write(&path, b"broken").unwrap();
        save(&path, &config("replacement")).unwrap();
        assert_eq!(load(&path).unwrap().unwrap().root, "replacement");
        assert_eq!(
            read(&backup_path(&path)).unwrap().unwrap().root,
            "recoverable"
        );
    }

    #[test]
    fn rename_failure_preserves_destination_and_cleans_up_temporary_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("install.json");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("blocker"), b"important").unwrap();
        assert!(atomic_write(&path, b"replacement").is_err());
        assert_eq!(fs::read(path.join("blocker")).unwrap(), b"important");
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[test]
    fn readers_never_observe_partial_json_during_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("install.json");
        save(&path, &config("root-0")).unwrap();
        std::thread::scope(|scope| {
            let reader = scope.spawn(|| {
                for _ in 0..200 {
                    let pointer = load(&path).unwrap().unwrap();
                    assert!(pointer.root.starts_with("root-"));
                    assert_eq!(pointer.database, "chosen-profile.db");
                }
            });
            for version in 1..30 {
                save(&path, &config(&format!("root-{version}"))).unwrap();
            }
            reader.join().unwrap();
        });
    }
}
