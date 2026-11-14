use crate::app_error::AppError;
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use chrono::Utc;
use rusqlite::{params, Connection};
use std::{fs, io, path::Path};
use uuid::Uuid;

const MAX_NAME_ATTEMPTS: usize = 16;

/// Capture the selected root first; SQLite only opens paths in private OS temp
/// storage. Destination creation, publication and cleanup use a held capability.
pub fn create_snapshot(connection: &Connection, root: &Path) -> Result<(String, u64), AppError> {
    let root = Dir::open_ambient_dir(root, ambient_authority())?;
    let directory = open_backups(&root)?;
    create_at(
        connection,
        &directory,
        &Utc::now().format("%Y%m%d-%H%M%S").to_string(),
        Uuid::new_v4,
    )
}

fn open_backups(root: &Dir) -> io::Result<Dir> {
    root.create_dir_all("backups")?;
    root.open_dir("backups")
}

fn create_at(
    connection: &Connection,
    directory: &Dir,
    timestamp: &str,
    next_id: impl FnMut() -> Uuid,
) -> Result<(String, u64), AppError> {
    let staging = tempfile::Builder::new()
        .prefix("openmind-backup-")
        .tempdir()?;
    let staged = staging.path().join("snapshot.db");
    connection.execute(
        "VACUUM INTO ?1",
        params![staged
            .to_str()
            .ok_or_else(|| AppError::internal("Backup staging path is not valid UTF-8"))?],
    )?;
    let mut source = fs::File::open(&staged)?;
    let size = source.metadata()?.len();
    publish_at(&mut source, directory, timestamp, next_id, size)
}

fn publish_at(
    source: &mut (impl io::Read + io::Seek),
    directory: &Dir,
    timestamp: &str,
    mut next_id: impl FnMut() -> Uuid,
    size: u64,
) -> Result<(String, u64), AppError> {
    for _ in 0..MAX_NAME_ATTEMPTS {
        let name = format!("openmindai-{timestamp}-{}.db", next_id().simple());
        let temporary = format!(".pending-{name}");
        match directory.symlink_metadata(&name) {
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = match directory.open_with(&temporary, &options) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        };
        let result = (|| -> io::Result<()> {
            use std::io::SeekFrom;
            source.seek(SeekFrom::Start(0))?;
            io::copy(&mut *source, &mut output)?;
            output.sync_all()?;
            drop(output);
            // Exclusive pending names serialize equal UUIDs; never follow a
            // destination link. This mirrors confined text publication.
            match directory.symlink_metadata(&name) {
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "backup destination exists",
                    ))
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            directory.rename(&temporary, directory, &name)?;
            #[cfg(unix)]
            directory.open(".")?.sync_all()?;
            Ok(())
        })();
        let _ = directory.remove_file(&temporary);
        match result {
            Ok(()) => return Ok((name, size)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(AppError::internal(
        "Could not reserve a unique database backup filename; retry the backup.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashSet,
        sync::{Arc, Barrier},
    };

    fn create_test_snapshot(
        connection: &Connection,
        directory: &Path,
        timestamp: &str,
        next_id: impl FnMut() -> Uuid,
    ) -> Result<(String, u64), AppError> {
        fs::create_dir_all(directory)?;
        let directory = Dir::open_ambient_dir(directory, ambient_authority())?;
        create_at(connection, &directory, timestamp, next_id)
    }

    fn external_link(link: &Path, target: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        {
            let result = std::process::Command::new("pwsh").args(["-NoProfile", "-NonInteractive", "-Command", "New-Item -ItemType Junction -Path $env:TEST_LINK -Target $env:TEST_TARGET -ErrorAction Stop | Out-Null"])
                .env("TEST_LINK", link).env("TEST_TARGET", target).output().unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }

    #[test]
    fn external_backup_link_is_rejected_at_actual_open() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let connection = source(&root.path().join("source.db"));
        external_link(&root.path().join("backups"), outside.path());
        assert!(create_snapshot(&connection, root.path()).is_err());
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        #[cfg(windows)]
        fs::remove_dir(root.path().join("backups")).unwrap();
    }

    #[test]
    fn parent_path_replacement_cannot_redirect_publication_or_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let connection = source(&root.path().join("source.db"));
        let root_handle = Dir::open_ambient_dir(root.path(), ambient_authority()).unwrap();
        let directory = open_backups(&root_handle).unwrap();
        fs::rename(
            root.path().join("backups"),
            root.path().join("held-backups"),
        )
        .unwrap();
        external_link(&root.path().join("backups"), outside.path());
        let (name, _) = create_at(&connection, &directory, "held", Uuid::new_v4).unwrap();
        assert_snapshot(&root.path().join("held-backups").join(&name));
        connection.execute_batch("BEGIN").unwrap();
        assert!(create_at(&connection, &directory, "failure", Uuid::new_v4).is_err());
        connection.execute_batch("ROLLBACK").unwrap();
        assert_eq!(
            fs::read_dir(root.path().join("held-backups"))
                .unwrap()
                .count(),
            1
        );
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        #[cfg(windows)]
        fs::remove_dir(root.path().join("backups")).unwrap();
    }

    #[test]
    fn root_path_replacement_keeps_the_original_capability() {
        let container = tempfile::tempdir().unwrap();
        let root = container.path().join("root");
        let moved = container.path().join("moved-root");
        fs::create_dir(&root).unwrap();
        let outside = tempfile::tempdir().unwrap();
        let connection = source(&container.path().join("source.db"));
        let root_handle = Dir::open_ambient_dir(&root, ambient_authority()).unwrap();
        fs::rename(&root, &moved).unwrap();
        external_link(&root, outside.path());
        let directory = open_backups(&root_handle).unwrap();
        let (name, _) = create_at(&connection, &directory, "root-held", Uuid::new_v4).unwrap();
        assert_snapshot(&moved.join("backups").join(name));
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        #[cfg(windows)]
        fs::remove_dir(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn final_and_pending_symlinks_never_truncate_their_targets() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let connection = source(&root.path().join("source.db"));
        let root_handle = Dir::open_ambient_dir(root.path(), ambient_authority()).unwrap();
        let directory = open_backups(&root_handle).unwrap();
        let name = format!("openmindai-collision-{}.db", Uuid::nil().simple());
        let target = outside.path().join("sentinel");
        fs::write(&target, b"must survive").unwrap();
        std::os::unix::fs::symlink(&target, root.path().join("backups").join(&name)).unwrap();
        assert!(create_at(&connection, &directory, "collision", Uuid::nil).is_err());
        fs::remove_file(root.path().join("backups").join(&name)).unwrap();
        std::os::unix::fs::symlink(
            &target,
            root.path().join("backups").join(format!(".pending-{name}")),
        )
        .unwrap();
        assert!(create_at(&connection, &directory, "collision", Uuid::nil).is_err());
        assert_eq!(fs::read(target).unwrap(), b"must survive");
    }

    #[test]
    fn copy_error_cleans_only_its_pending_file_through_the_held_directory() {
        struct BrokenReader(bool);
        impl io::Read for BrokenReader {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                if self.0 {
                    return Err(io::Error::other("injected read failure"));
                }
                self.0 = true;
                let count = buffer.len().min(512);
                buffer[..count].fill(1);
                Ok(count)
            }
        }
        impl io::Seek for BrokenReader {
            fn seek(&mut self, _: io::SeekFrom) -> io::Result<u64> {
                Ok(0)
            }
        }
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root_handle = Dir::open_ambient_dir(root.path(), ambient_authority()).unwrap();
        let directory = open_backups(&root_handle).unwrap();
        fs::rename(root.path().join("backups"), root.path().join("held")).unwrap();
        external_link(&root.path().join("backups"), outside.path());
        fs::write(root.path().join("held/existing.db"), b"preserve").unwrap();
        assert!(publish_at(
            &mut BrokenReader(false),
            &directory,
            "copy-error",
            Uuid::new_v4,
            4096
        )
        .is_err());
        assert_eq!(fs::read_dir(root.path().join("held")).unwrap().count(), 1);
        assert_eq!(
            fs::read(root.path().join("held/existing.db")).unwrap(),
            b"preserve"
        );
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        #[cfg(windows)]
        fs::remove_dir(root.path().join("backups")).unwrap();
    }

    #[test]
    fn snapshot_includes_committed_wal_rows() {
        let root = tempfile::tempdir().unwrap();
        let connection = source(&root.path().join("source.db"));
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL; INSERT INTO evidence VALUES ('uncheckpointed commit');",
            )
            .unwrap();
        let (name, size) = create_snapshot(&connection, root.path()).unwrap();
        let path = root.path().join("backups").join(name);
        assert_eq!(fs::metadata(&path).unwrap().len(), size);
        let backup = Connection::open(path).unwrap();
        let count: i64 = backup
            .query_row("SELECT COUNT(*) FROM evidence", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
        let check: String = backup
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(check, "ok");
    }

    fn source(path: &Path) -> Connection {
        let connection = Connection::open(path).unwrap();
        connection.execute_batch("CREATE TABLE evidence (content TEXT); INSERT INTO evidence VALUES ('preserve this row');").unwrap();
        connection
    }
    fn assert_snapshot(path: &Path) {
        let connection = Connection::open(path).unwrap();
        let integrity: String = connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        let content: String = connection
            .query_row("SELECT content FROM evidence", [], |row| row.get(0))
            .unwrap();
        assert_eq!(content, "preserve this row");
    }

    #[test]
    fn sequential_backups_with_a_frozen_timestamp_remain_distinct_and_valid() {
        let temp = tempfile::tempdir().unwrap();
        let connection = source(&temp.path().join("source.db"));
        let backups = temp.path().join("backups");
        let mut names = HashSet::new();
        for _ in 0..5 {
            let (name, size) =
                create_test_snapshot(&connection, &backups, "20261110-000000", Uuid::new_v4)
                    .unwrap();
            assert!(names.insert(name.clone()));
            assert!(size > 0);
            assert_snapshot(&backups.join(name));
        }
        assert_eq!(fs::read_dir(backups).unwrap().count(), 5);
    }

    #[test]
    fn simultaneous_connections_reserve_independent_valid_backups() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("source.db");
        let connection = source(&source_path);
        drop(connection);
        let backups = temp.path().join("backups");
        let barrier = Arc::new(Barrier::new(6));
        let names = std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for _ in 0..6 {
                let barrier = Arc::clone(&barrier);
                let source_path = &source_path;
                let backups = &backups;
                handles.push(scope.spawn(move || {
                    let connection = Connection::open(source_path).unwrap();
                    connection
                        .busy_timeout(std::time::Duration::from_secs(5))
                        .unwrap();
                    barrier.wait();
                    create_test_snapshot(&connection, backups, "same-second", Uuid::new_v4)
                        .unwrap()
                        .0
                }));
            }
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<HashSet<_>>()
        });
        assert_eq!(names.len(), 6);
        for name in names {
            assert_snapshot(&backups.join(name));
        }
    }

    #[test]
    fn preexisting_collision_is_preserved_and_allocation_retries() {
        let temp = tempfile::tempdir().unwrap();
        let connection = source(&temp.path().join("source.db"));
        let backups = temp.path().join("backups");
        fs::create_dir(&backups).unwrap();
        let existing = backups.join(format!("openmindai-frozen-{}.db", Uuid::nil().simple()));
        fs::write(&existing, b"existing backup must survive").unwrap();
        let mut ids = [Uuid::nil(), Uuid::from_u128(1)].into_iter();
        let (name, _) =
            create_test_snapshot(&connection, &backups, "frozen", || ids.next().unwrap()).unwrap();
        assert_eq!(
            fs::read(&existing).unwrap(),
            b"existing backup must survive"
        );
        assert_snapshot(&backups.join(name));
        assert_eq!(fs::read_dir(&backups).unwrap().count(), 2);
        assert!(create_test_snapshot(&connection, &backups, "frozen", Uuid::nil).is_err());
        assert_eq!(
            fs::read(&existing).unwrap(),
            b"existing backup must survive"
        );
    }

    #[test]
    fn failed_vacuum_removes_only_its_own_reservation() {
        let temp = tempfile::tempdir().unwrap();
        let connection = source(&temp.path().join("source.db"));
        let backups = temp.path().join("backups");
        let (good, _) = create_snapshot(&connection, temp.path()).unwrap();
        connection.execute_batch("BEGIN").unwrap();
        assert!(create_snapshot(&connection, temp.path()).is_err());
        connection.execute_batch("ROLLBACK").unwrap();
        assert_eq!(fs::read_dir(&backups).unwrap().count(), 1);
        assert_snapshot(&backups.join(good));
    }
}
