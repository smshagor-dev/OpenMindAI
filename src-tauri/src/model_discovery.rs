//! Bounded, non-recursive inventory. Directory links are not model locations.
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};

use crate::app_error::AppError;

#[derive(Clone, Copy)]
struct Limits {
    entries: usize,
    directories: usize,
    models: usize,
    depth: usize,
}
const DEFAULT_LIMITS: Limits = Limits {
    entries: 50_000,
    directories: 4_096,
    models: 4_096,
    depth: 32,
};

#[derive(Default)]
pub struct ScanReport {
    pub files: Vec<PathBuf>,
    pub skipped_entries: usize,
    pub limited: bool,
    entries_seen: usize,
}

pub fn scan_models(directory: &Path, portable_root: &Path) -> Result<ScanReport, AppError> {
    scan_with_limits(directory, portable_root, DEFAULT_LIMITS)
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Includes junctions and other reparse points, not only symbolic links.
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn scan_with_limits(
    directory: &Path,
    portable_root: &Path,
    limits: Limits,
) -> Result<ScanReport, AppError> {
    let canonical_root = fs::canonicalize(portable_root)?;
    let mut report = ScanReport::default();
    let mut pending = vec![(directory.to_path_buf(), 0)];
    let mut visited = HashSet::new();
    'scan: while let Some((directory, depth)) = pending.pop() {
        let metadata = match fs::symlink_metadata(&directory) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => {
                report.skipped_entries += 1;
                continue;
            }
        };
        if is_link(&metadata) || !metadata.is_dir() {
            report.skipped_entries += 1;
            continue;
        }
        let canonical = match fs::canonicalize(&directory) {
            Ok(path) if path.starts_with(&canonical_root) => path,
            _ => {
                report.skipped_entries += 1;
                continue;
            }
        };
        if !visited.insert(canonical.clone()) {
            continue;
        }
        let entries = match fs::read_dir(&canonical) {
            Ok(entries) => entries,
            Err(_) => {
                report.skipped_entries += 1;
                continue;
            }
        };
        for entry in entries {
            if report.entries_seen >= limits.entries {
                report.limited = true;
                break 'scan;
            }
            report.entries_seen += 1;
            let path = match entry {
                Ok(entry) => entry.path(),
                Err(_) => {
                    report.skipped_entries += 1;
                    continue;
                }
            };
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(_) => {
                    report.skipped_entries += 1;
                    continue;
                }
            };
            if is_link(&metadata) {
                report.skipped_entries += 1;
                continue;
            }
            if metadata.is_dir() {
                if depth >= limits.depth || visited.len() + pending.len() >= limits.directories {
                    report.limited = true;
                    continue;
                }
                pending.push((path, depth + 1));
            } else if metadata.is_file() && is_checkpoint(&path) {
                let canonical = match fs::canonicalize(&path) {
                    Ok(path) if path.starts_with(&canonical_root) => path,
                    _ => {
                        report.skipped_entries += 1;
                        continue;
                    }
                };
                if report.files.len() >= limits.models {
                    report.limited = true;
                    break 'scan;
                }
                report.files.push(canonical);
            }
        }
    }
    if report.skipped_entries > 0 || report.limited {
        tracing::warn!(path = %directory.display(), skipped_entries = report.skipped_entries,
            limited = report.limited, entries_seen = report.entries_seen,
            "model discovery skipped inaccessible/linked paths or reached its scan limits");
    }
    Ok(report)
}

fn is_checkpoint(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("gguf"))
        && !path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.to_ascii_lowercase().starts_with("mmproj-"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(path: &Path) {
        fs::write(path, b"GGUF").unwrap();
    }

    #[test]
    fn nested_checkpoints_are_unique_and_projectors_are_excluded() {
        let temp = tempfile::tempdir().unwrap();
        let models = temp.path().join("models");
        fs::create_dir_all(models.join("llm/nested")).unwrap();
        fixture(&models.join("llm/nested/model.GGUF"));
        fixture(&models.join("llm/mmproj-test.gguf"));
        fixture(&models.join("llm/not-a-model.txt"));
        let report = scan_models(&models, temp.path()).unwrap();
        assert_eq!(
            report.files,
            vec![fs::canonicalize(models.join("llm/nested/model.GGUF")).unwrap()]
        );
        assert_eq!(report.skipped_entries, 0);
        assert!(!report.limited);
    }

    #[test]
    fn entry_and_model_budgets_bound_large_flat_trees() {
        let temp = tempfile::tempdir().unwrap();
        for index in 0..100 {
            fixture(&temp.path().join(format!("{index}.gguf")));
        }
        let limits = Limits {
            entries: 7,
            models: 100,
            ..DEFAULT_LIMITS
        };
        let report = scan_with_limits(temp.path(), temp.path(), limits).unwrap();
        assert_eq!(report.entries_seen, 7);
        assert_eq!(report.files.len(), 7);
        assert!(report.limited);
        let report = scan_with_limits(
            temp.path(),
            temp.path(),
            Limits {
                models: 3,
                ..limits
            },
        )
        .unwrap();
        assert_eq!(report.files.len(), 3);
        assert!(report.limited);
    }

    #[test]
    fn depth_and_pending_directory_budgets_are_bounded_without_recursion() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("a/b/c/d")).unwrap();
        fixture(&temp.path().join("a/model.gguf"));
        fixture(&temp.path().join("a/b/c/d/deep.gguf"));
        let report = scan_with_limits(
            temp.path(),
            temp.path(),
            Limits {
                depth: 1,
                ..DEFAULT_LIMITS
            },
        )
        .unwrap();
        assert_eq!(report.files.len(), 1);
        assert!(report.limited);
        for index in 0..100 {
            fs::create_dir(temp.path().join(format!("dir-{index}"))).unwrap();
        }
        let report = scan_with_limits(
            temp.path(),
            temp.path(),
            Limits {
                directories: 2,
                ..DEFAULT_LIMITS
            },
        )
        .unwrap();
        assert!(report.limited);
        assert!(report.entries_seen < 110);
    }

    #[test]
    fn missing_or_external_scan_directory_does_not_escape_the_portable_root() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fixture(&outside.path().join("outside.gguf"));
        assert!(scan_models(&root.path().join("missing"), root.path())
            .unwrap()
            .files
            .is_empty());
        let report = scan_models(outside.path(), root.path()).unwrap();
        assert!(report.files.is_empty());
        assert_eq!(report.skipped_entries, 1);
    }

    #[cfg(unix)]
    #[test]
    fn cycles_external_links_and_broken_links_do_not_hide_valid_models() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let models = root.path().join("models");
        fs::create_dir(&models).unwrap();
        fixture(&models.join("valid.gguf"));
        fixture(&outside.path().join("external.gguf"));
        symlink(&models, models.join("cycle")).unwrap();
        symlink(outside.path(), models.join("external-dir")).unwrap();
        symlink(
            outside.path().join("external.gguf"),
            models.join("external-file.gguf"),
        )
        .unwrap();
        symlink(models.join("missing"), models.join("dangling.gguf")).unwrap();
        let report = scan_models(&models, root.path()).unwrap();
        assert_eq!(
            report.files,
            vec![fs::canonicalize(models.join("valid.gguf")).unwrap()]
        );
        assert_eq!(report.skipped_entries, 4);
        assert!(!report.limited);
        let linked_root = root.path().join("linked-models");
        symlink(outside.path(), &linked_root).unwrap();
        assert!(scan_models(&linked_root, root.path())
            .unwrap()
            .files
            .is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn junction_cycles_and_external_junctions_are_not_followed() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let models = root.path().join("models");
        fs::create_dir(&models).unwrap();
        fixture(&models.join("valid.gguf"));
        fixture(&outside.path().join("external.gguf"));
        for (name, target) in [("cycle", models.as_path()), ("external", outside.path())] {
            let output = std::process::Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command", "New-Item -ItemType Junction -Path $env:OPENMIND_TEST_LINK -Target $env:OPENMIND_TEST_TARGET -ErrorAction Stop | Out-Null"])
                .env("OPENMIND_TEST_LINK", models.join(name)).env("OPENMIND_TEST_TARGET", target)
                .output().unwrap();
            assert!(
                output.status.success(),
                "junction fixture failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let report = scan_models(&models, root.path()).unwrap();
        assert_eq!(report.files.len(), 1);
        assert_eq!(report.skipped_entries, 2);
        assert!(!report.limited);
    }
}
