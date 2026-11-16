//! Portable path checks and capability-confined creation of new text outputs.
use crate::app_error::AppError;
#[cfg(test)]
use cap_std::{ambient_authority, fs::Dir};
#[cfg(test)]
use std::io::Write;
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

fn relative(path: &Path) -> Result<(), AppError> {
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return Err(AppError::internal("portable path traversal rejected"));
    }
    Ok(())
}

// Canonicalize the existing ancestor, retaining missing tail components. A
// dangling link is an error, not a nonexistent directory to append through.
fn canonical_destination(path: &Path) -> io::Result<PathBuf> {
    let mut ancestor = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut tail = Vec::new();
    loop {
        match fs::symlink_metadata(&ancestor) {
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                tail.push(ancestor.file_name().ok_or(error)?.to_os_string());
                ancestor = ancestor
                    .parent()
                    .ok_or_else(|| io::Error::other("path has no existing ancestor"))?
                    .to_path_buf();
            }
            Err(error) => return Err(error),
        }
    }
    let mut result = fs::canonicalize(ancestor)?;
    for part in tail.into_iter().rev() {
        result.push(part);
    }
    Ok(result)
}

pub(super) fn resolve(root: &Path, path: &Path) -> Result<PathBuf, AppError> {
    relative(path)?;
    let joined = root.join(path);
    let canonical_root = canonical_destination(root)?;
    if !canonical_destination(&joined)?.starts_with(canonical_root) {
        return Err(AppError::internal(
            "portable path escapes the selected storage root",
        ));
    }
    Ok(joined)
}

#[cfg(test)]
pub(super) fn write_new(root: &Path, path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    relative(path)?;
    let root = Dir::open_ambient_dir(root, ambient_authority())?;
    write_new_at(&root, path, bytes)
}

#[cfg(test)]
fn write_new_at(root: &Dir, path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let parent_path = path
        .parent()
        .ok_or_else(|| AppError::internal("output has no parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| AppError::internal("output has no filename"))?;
    // Keep the directory handle through creation: replacing its pathname later
    // cannot redirect the write to a new directory or an external symlink.
    let parent = root.open_dir(parent_path)?;
    let temporary = format!(".pending-{}", uuid::Uuid::new_v4());
    let result = (|| -> std::io::Result<()> {
        let mut options = cap_std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        let mut file = parent.open_with(&temporary, &options)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        match parent.symlink_metadata(Path::new(name)) {
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "artifact destination already exists",
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        parent.rename(&temporary, &parent, Path::new(name))?;
        #[cfg(unix)]
        parent.open(".")?.sync_all()?;
        Ok(())
    })();
    let _ = parent.remove_file(&temporary);
    result.map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allows_new_nested_paths_and_rejects_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("new-root");
        assert_eq!(
            resolve(&root, Path::new("generated/new/file.txt")).unwrap(),
            root.join("generated/new/file.txt")
        );
        for path in ["../outside", "/outside", "generated/../../outside"] {
            assert!(resolve(&root, Path::new(path)).is_err());
        }
        fs::create_dir_all(root.join("generated/new")).unwrap();
        write_new(&root, Path::new("generated/new/file.txt"), b"first").unwrap();
        assert!(write_new(&root, Path::new("generated/new/file.txt"), b"second").is_err());
        assert_eq!(
            fs::read(root.join("generated/new/file.txt")).unwrap(),
            b"first"
        );
    }
    #[cfg(unix)]
    #[test]
    fn external_links_and_dangling_links_are_rejected_but_contained_links_work() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.path().join("escape")).unwrap();
        symlink(outside.path().join("missing"), root.path().join("dangling")).unwrap();
        for path in ["escape/new/file", "dangling/file"] {
            assert!(resolve(root.path(), Path::new(path)).is_err());
        }
        fs::create_dir(root.path().join("real")).unwrap();
        symlink("real", root.path().join("alias")).unwrap();
        assert!(resolve(root.path(), Path::new("alias/new.txt")).is_ok());
        write_new(root.path(), Path::new("alias/new.txt"), b"inside").unwrap();
        assert!(fs::read_dir(outside.path()).unwrap().next().is_none());
    }
    #[cfg(unix)]
    #[test]
    fn actual_open_stays_confined_after_a_successful_path_check() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("generated")).unwrap();
        resolve(root.path(), Path::new("generated/new.txt")).unwrap();
        fs::rename(root.path().join("generated"), root.path().join("saved")).unwrap();
        symlink(outside.path(), root.path().join("generated")).unwrap();
        assert!(write_new(root.path(), Path::new("generated/new.txt"), b"no escape").is_err());
        assert!(!outside.path().join("new.txt").exists());
    }
    #[cfg(unix)]
    #[test]
    fn open_root_capability_survives_root_path_replacement() {
        use std::os::unix::fs::symlink;
        let base = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = base.path().join("root");
        fs::create_dir_all(root.join("generated")).unwrap();
        let handle = Dir::open_ambient_dir(&root, ambient_authority()).unwrap();
        fs::rename(&root, base.path().join("saved")).unwrap();
        symlink(outside.path(), &root).unwrap();
        write_new_at(&handle, Path::new("generated/file.txt"), b"retained handle").unwrap();
        assert_eq!(
            fs::read(base.path().join("saved/generated/file.txt")).unwrap(),
            b"retained handle"
        );
        assert!(!outside.path().join("generated/file.txt").exists());
    }
    #[cfg(windows)]
    #[test]
    fn junction_output_is_rejected() {
        use std::process::Command;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = root.path().join("junction");
        let result = Command::new("pwsh").args(["-NoProfile", "-NonInteractive", "-Command", "New-Item -ItemType Junction -Path $env:TEST_LINK -Target $env:TEST_TARGET -ErrorAction Stop | Out-Null"]).env("TEST_LINK", &link).env("TEST_TARGET", outside.path()).output().unwrap();
        assert!(
            result.status.success(),
            "junction fixture failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(resolve(root.path(), Path::new("junction/file.txt")).is_err());
        assert!(write_new(root.path(), Path::new("junction/file.txt"), b"no escape").is_err());
        assert!(!outside.path().join("file.txt").exists());
    }
}
