use std::{
    fs, io,
    path::{Path, PathBuf},
};

use chrono::Utc;
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::app_error::AppError;

const MAX_NAME_ATTEMPTS: usize = 16;

/// VACUUM INTO accepts an existing empty file. Reserve that file exclusively
/// before SQLite opens it, so another request can never claim the same name.
pub fn create_snapshot(
    connection: &Connection,
    directory: &Path,
) -> Result<(String, u64), AppError> {
    create_with_ids(
        connection,
        directory,
        &Utc::now().format("%Y%m%d-%H%M%S").to_string(),
        Uuid::new_v4,
    )
}

struct Reservation {
    path: PathBuf,
    completed: bool,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.completed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn create_with_ids(
    connection: &Connection,
    directory: &Path,
    timestamp: &str,
    mut next_id: impl FnMut() -> Uuid,
) -> Result<(String, u64), AppError> {
    fs::create_dir_all(directory)?;
    for _ in 0..MAX_NAME_ATTEMPTS {
        let name = format!("openmindai-{timestamp}-{}.db", next_id().simple());
        let path = directory.join(&name);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let reservation = match options.open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        };
        // Release the handle before SQLite opens the path (also on Windows).
        drop(reservation);
        let mut reservation = Reservation {
            path,
            completed: false,
        };
        connection.execute(
            "VACUUM INTO ?1",
            params![reservation.path.to_string_lossy()],
        )?;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&reservation.path)?;
        file.sync_all()?;
        let size = file.metadata()?.len();
        drop(file);
        #[cfg(unix)]
        fs::File::open(directory)?.sync_all()?;
        reservation.completed = true;
        return Ok((name, size));
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
                create_with_ids(&connection, &backups, "20261110-000000", Uuid::new_v4).unwrap();
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
                    create_with_ids(&connection, backups, "same-second", Uuid::new_v4)
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
            create_with_ids(&connection, &backups, "frozen", || ids.next().unwrap()).unwrap();
        assert_eq!(
            fs::read(&existing).unwrap(),
            b"existing backup must survive"
        );
        assert_snapshot(&backups.join(name));
        assert_eq!(fs::read_dir(&backups).unwrap().count(), 2);
        assert!(create_with_ids(&connection, &backups, "frozen", Uuid::nil).is_err());
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
        let (good, _) = create_snapshot(&connection, &backups).unwrap();
        connection.execute_batch("BEGIN").unwrap();
        assert!(create_snapshot(&connection, &backups).is_err());
        connection.execute_batch("ROLLBACK").unwrap();
        assert_eq!(fs::read_dir(&backups).unwrap().count(), 1);
        assert_snapshot(&backups.join(good));
    }
}
