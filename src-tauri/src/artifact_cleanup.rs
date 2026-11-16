//! SQLite artifact cleanup through a retained root and parent capability.
use cap_std::{ambient_authority, fs::Dir};
use rusqlite::{functions::FunctionFlags, Connection, Error};
use std::{
    ffi::OsString,
    io,
    path::{Component, Path},
};

fn sqlite_error(error: io::Error) -> Error {
    Error::UserFunctionError(Box::new(error))
}

pub fn register(connection: &Connection, root: Option<&Path>) -> rusqlite::Result<()> {
    // Capture identity for the lifetime of the connection, not once per DELETE.
    let root = root
        .map(|path| Dir::open_ambient_dir(path, ambient_authority()))
        .transpose()
        .map_err(sqlite_error)?;
    connection.create_scalar_function(
        "openmind_delete_artifact",
        1,
        FunctionFlags::SQLITE_UTF8,
        move |context| {
            let path: String = context.get(0)?;
            let Some(root) = root.as_ref() else {
                return Ok(0_i64);
            };
            let Some((parent, name)) = parent_for(root, &path).map_err(sqlite_error)? else {
                return Ok(0_i64);
            };
            unlink(&parent, &name).map_err(sqlite_error)
        },
    )
}

fn parent_for(root: &Dir, relative: &str) -> io::Result<Option<(Dir, OsString)>> {
    let path = Path::new(relative);
    if !path.starts_with("generated")
        || path.components().count() < 2
        || relative.contains(['\\', ':'])
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "artifact cleanup must stay beneath generated",
        ));
    }
    let parent = match root.open_dir(path.parent().unwrap()) {
        Ok(parent) => parent,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(Some((parent, path.file_name().unwrap().to_owned())))
}
fn unlink(parent: &Dir, name: &std::ffi::OsStr) -> io::Result<i64> {
    // Unlink the final entry itself. A final symlink (including a dangling one)
    // is never followed to its target; directory errors remain SQLite errors.
    match parent.remove_file(Path::new(name)) {
        Ok(()) => Ok(1),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn fixture(root: &Path) -> Connection {
        fs::create_dir_all(root.join("generated/files")).unwrap();
        let db = Connection::open_in_memory().unwrap();
        register(&db, Some(root)).unwrap();
        db.execute_batch(include_str!("../migrations/001_initial.sql"))
            .unwrap();
        db.execute_batch(include_str!("../migrations/002_artifacts.sql"))
            .unwrap();
        db.execute_batch(include_str!(
            "../migrations/005_artifact_cleanup_and_media_kinds.sql"
        ))
        .unwrap();
        db.execute_batch("INSERT INTO conversations(id,title,created_at,updated_at) VALUES('owner','Owner','now','now');").unwrap();
        db
    }
    fn insert(db: &Connection, path: &str) {
        db.execute("INSERT INTO artifacts(id,conversation_id,name,path,mime_type,kind,status,created_at,updated_at) VALUES('artifact','owner','file',?1,'text/plain','text','ready','now','now')",[path]).unwrap();
    }
    fn link(parent: &Path, target: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, parent).unwrap();
        #[cfg(windows)]
        {
            let result=std::process::Command::new("pwsh").args(["-NoProfile","-NonInteractive","-Command","New-Item -ItemType Junction -Path $env:TEST_LINK -Target $env:TEST_TARGET -ErrorAction Stop | Out-Null"]).env("TEST_LINK",parent).env("TEST_TARGET",target).output().unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
    fn swap(parent: &Path, moved: &Path, outside: &Path) -> bool {
        match fs::rename(parent, moved) {
            Ok(()) => {
                link(parent, outside);
                true
            }
            Err(error) => {
                #[cfg(windows)]
                {
                    assert!(matches!(error.raw_os_error(), Some(5 | 32)), "{error}");
                    false
                }
                #[cfg(not(windows))]
                panic!("{error}");
            }
        }
    }
    #[test]
    fn real_delete_trigger_and_chat_cascade_remove_only_registered_output() {
        let root = tempfile::tempdir().unwrap();
        let db = fixture(root.path());
        fs::write(root.path().join("generated/files/file.txt"), b"output").unwrap();
        fs::write(root.path().join("generated/files/other.txt"), b"other").unwrap();
        insert(&db, "generated/files/file.txt");
        db.execute("DELETE FROM conversations WHERE id='owner'", [])
            .unwrap();
        assert!(!root.path().join("generated/files/file.txt").exists());
        assert_eq!(
            fs::read(root.path().join("generated/files/other.txt")).unwrap(),
            b"other"
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM artifacts", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn unsafe_paths_preserve_sql_rows_and_user_files() {
        for path in [
            "../keep.txt",
            "keep.txt",
            "generated/../keep.txt",
            "/generated/keep.txt",
            "generated\\keep.txt",
            "generated/C:keep.txt",
        ] {
            let root = tempfile::tempdir().unwrap();
            let db = fixture(root.path());
            fs::write(root.path().join("keep.txt"), b"keep").unwrap();
            insert(&db, path);
            assert!(db.execute("DELETE FROM artifacts", []).is_err());
            assert_eq!(
                db.query_row("SELECT count(*) FROM artifacts", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                1
            );
            assert_eq!(fs::read(root.path().join("keep.txt")).unwrap(), b"keep");
        }
    }
    #[test]
    fn missing_file_or_parent_is_idempotent() {
        for path in [
            "generated/files/missing.txt",
            "generated/missing-dir/missing.txt",
        ] {
            let root = tempfile::tempdir().unwrap();
            let db = fixture(root.path());
            insert(&db, path);
            assert_eq!(db.execute("DELETE FROM artifacts", []).unwrap(), 1);
            assert_eq!(
                db.query_row("SELECT openmind_delete_artifact(?1)", [path], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
    }
    #[test]
    fn external_parent_link_fails_closed_and_keeps_row() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let db = fixture(root.path());
        fs::write(outside.path().join("file.txt"), b"outside").unwrap();
        link(&root.path().join("generated/linked"), outside.path());
        insert(&db, "generated/linked/file.txt");
        assert!(db.execute("DELETE FROM artifacts", []).is_err());
        assert_eq!(
            fs::read(outside.path().join("file.txt")).unwrap(),
            b"outside"
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM artifacts", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        #[cfg(windows)]
        fs::remove_dir(root.path().join("generated/linked")).unwrap();
    }
    #[test]
    fn parent_replacement_after_open_cannot_redirect_unlink() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let _db = fixture(root.path());
        fs::write(root.path().join("generated/files/file.txt"), b"inside").unwrap();
        fs::write(outside.path().join("file.txt"), b"outside").unwrap();
        let directory = Dir::open_ambient_dir(root.path(), ambient_authority()).unwrap();
        let (parent, name) = parent_for(&directory, "generated/files/file.txt")
            .unwrap()
            .unwrap();
        let replaced = swap(
            &root.path().join("generated/files"),
            &root.path().join("held"),
            outside.path(),
        );
        assert_eq!(unlink(&parent, &name).unwrap(), 1);
        assert_eq!(
            fs::read(outside.path().join("file.txt")).unwrap(),
            b"outside"
        );
        let held = if replaced {
            root.path().join("held")
        } else {
            root.path().join("generated/files")
        };
        assert!(!held.join("file.txt").exists());
        #[cfg(windows)]
        if replaced {
            fs::remove_dir(root.path().join("generated/files")).unwrap();
        }
    }
    #[test]
    fn root_replacement_after_registration_does_not_change_delete_authority() {
        let container = tempfile::tempdir().unwrap();
        let root = container.path().join("root");
        let outside = tempfile::tempdir().unwrap();
        let db = fixture(&root);
        fs::write(root.join("generated/files/file.txt"), b"inside").unwrap();
        fs::create_dir_all(outside.path().join("generated/files")).unwrap();
        fs::write(outside.path().join("generated/files/file.txt"), b"outside").unwrap();
        insert(&db, "generated/files/file.txt");
        let replaced = swap(&root, &container.path().join("held"), outside.path());
        db.execute("DELETE FROM artifacts", []).unwrap();
        assert_eq!(
            fs::read(outside.path().join("generated/files/file.txt")).unwrap(),
            b"outside"
        );
        let held = if replaced {
            container.path().join("held")
        } else {
            root.clone()
        };
        assert!(!held.join("generated/files/file.txt").exists());
        #[cfg(windows)]
        if replaced {
            fs::remove_dir(&root).unwrap();
        }
    }
    #[cfg(unix)]
    #[test]
    fn final_and_dangling_links_are_unlinked_without_following_targets() {
        for dangling in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let db = fixture(root.path());
            let target = outside.path().join("target");
            if !dangling {
                fs::write(&target, b"outside").unwrap();
            }
            std::os::unix::fs::symlink(&target, root.path().join("generated/files/link")).unwrap();
            insert(&db, "generated/files/link");
            db.execute("DELETE FROM artifacts", []).unwrap();
            assert!(fs::symlink_metadata(root.path().join("generated/files/link")).is_err());
            if !dangling {
                assert_eq!(fs::read(target).unwrap(), b"outside");
            }
        }
    }
    #[test]
    fn directory_delete_error_keeps_row_and_contents() {
        let root = tempfile::tempdir().unwrap();
        let db = fixture(root.path());
        fs::create_dir(root.path().join("generated/files/folder")).unwrap();
        fs::write(root.path().join("generated/files/folder/file"), b"keep").unwrap();
        insert(&db, "generated/files/folder");
        assert!(db.execute("DELETE FROM artifacts", []).is_err());
        assert_eq!(
            fs::read(root.path().join("generated/files/folder/file")).unwrap(),
            b"keep"
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM artifacts", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
