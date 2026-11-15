//! Committed message deletion -> retryable, capability-confined index unlink.
//! Shared content-addressed image blobs are deliberately not collected here.
use cap_std::fs::Dir;
use rusqlite::Connection;
use std::{error::Error, io};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

pub fn drain(db: &Connection, root: &Dir) -> Result<()> {
    // Never consume uncommitted tombstones, including in callers' transactions.
    if !db.is_autocommit() {
        return Ok(());
    }
    let tx = db.unchecked_transaction()?;
    // Serialize with publishers and delete transactions on other connections.
    tx.execute("DELETE FROM deleted_media_indexes WHERE 0", [])?;
    let ids = {
        let mut query = tx.prepare("SELECT message_id FROM deleted_media_indexes WHERE NOT EXISTS (SELECT 1 FROM messages WHERE id = message_id)")?;
        let rows = query
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let directory = match root.open_dir("data/media-index") {
        Ok(dir) => dir,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for id in ids {
        // Production message IDs are canonical UUIDs. Never turn database text
        // into arbitrary deletion authority, even for damaged/legacy databases.
        if !canonical_uuid(&id) {
            return Err(
                io::Error::new(io::ErrorKind::InvalidData, "unsafe deleted message ID").into(),
            );
        }
        match directory.remove_file(format!("{id}.json")) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    // Keep tombstones: a crash, older process or failed publication can recreate
    // an index. Replaying deletion on next startup remains safe and idempotent.
    tx.commit()?;
    Ok(())
}
fn canonical_uuid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::ambient_authority;
    use std::fs;
    const A: &str = "00000000-0000-4000-8000-000000000001";
    const B: &str = "00000000-0000-4000-8000-000000000002";
    fn fixture() -> (tempfile::TempDir, Connection, Dir) {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("data/media-index")).unwrap();
        fs::create_dir_all(temp.path().join("data/media")).unwrap();
        let db = Connection::open(temp.path().join("test.db")).unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        db.execute_batch(include_str!("../migrations/001_initial.sql"))
            .unwrap();
        db.execute_batch(include_str!("../migrations/014_deleted_media_indexes.sql"))
            .unwrap();
        db.execute_batch("INSERT INTO conversations(id,title,created_at,updated_at) VALUES('chat','Chat','now','now')").unwrap();
        for id in [A, B] {
            db.execute("INSERT INTO messages(id,conversation_id,role,content,status,created_at,updated_at) VALUES(?1,'chat','user','x','completed','now','now')",[id]).unwrap();
            fs::write(
                temp.path().join(format!("data/media-index/{id}.json")),
                b"private index",
            )
            .unwrap();
        }
        fs::write(temp.path().join("data/media/shared.png"), b"shared bytes").unwrap();
        let root = Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap();
        (temp, db, root)
    }
    fn index(t: &tempfile::TempDir, id: &str) -> bool {
        t.path()
            .join(format!("data/media-index/{id}.json"))
            .exists()
    }
    #[test]
    fn committed_delete_removes_only_its_index_and_preserves_shared_blob() {
        let (temp, db, root) = fixture();
        db.execute("DELETE FROM messages WHERE id=?1", [A]).unwrap();
        drain(&db, &root).unwrap();
        assert!(!index(&temp, A));
        assert!(index(&temp, B));
        assert_eq!(
            fs::read(temp.path().join("data/media/shared.png")).unwrap(),
            b"shared bytes"
        );
    }
    #[test]
    fn real_conversation_cascade_queues_every_message() {
        let (temp, db, root) = fixture();
        db.execute("DELETE FROM conversations", []).unwrap();
        drain(&db, &root).unwrap();
        assert!(!index(&temp, A));
        assert!(!index(&temp, B));
    }
    #[test]
    fn rollback_never_removes_live_index() {
        let (temp, db, root) = fixture();
        db.execute_batch("BEGIN IMMEDIATE; DELETE FROM messages;")
            .unwrap();
        drain(&db, &root).unwrap();
        assert!(index(&temp, A));
        db.execute_batch("ROLLBACK;").unwrap();
        drain(&db, &root).unwrap();
        assert!(index(&temp, A));
        assert!(index(&temp, B));
    }
    #[test]
    fn restart_replays_committed_tombstone_and_recreated_index() {
        let (temp, db, root) = fixture();
        db.execute("DELETE FROM messages WHERE id=?1", [A]).unwrap();
        drop(db);
        let reopened = Connection::open(temp.path().join("test.db")).unwrap();
        drain(&reopened, &root).unwrap();
        assert!(!index(&temp, A));
        fs::write(
            temp.path().join(format!("data/media-index/{A}.json")),
            b"late writer",
        )
        .unwrap();
        drain(&reopened, &root).unwrap();
        drain(&reopened, &root).unwrap();
        assert!(!index(&temp, A));
    }
    #[test]
    fn reused_live_id_is_preserved() {
        let (temp, db, root) = fixture();
        db.execute("INSERT INTO deleted_media_indexes VALUES(?1)", [A])
            .unwrap();
        drain(&db, &root).unwrap();
        assert!(index(&temp, A));
    }
    #[test]
    fn unsafe_id_cannot_delete_outside_index_directory() {
        let (temp, db, root) = fixture();
        fs::write(temp.path().join("sentinel.json"), b"safe").unwrap();
        db.execute(
            "INSERT INTO deleted_media_indexes VALUES('../../sentinel')",
            [],
        )
        .unwrap();
        assert!(drain(&db, &root).is_err());
        assert_eq!(
            fs::read(temp.path().join("sentinel.json")).unwrap(),
            b"safe"
        );
    }
    #[test]
    fn directory_collision_is_retryable_not_recursively_deleted() {
        let (temp, db, root) = fixture();
        db.execute("DELETE FROM messages WHERE id=?1", [A]).unwrap();
        let path = temp.path().join(format!("data/media-index/{A}.json"));
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(drain(&db, &root).is_err());
        assert!(path.is_dir());
        fs::remove_dir(&path).unwrap();
        drain(&db, &root).unwrap();
    }
    #[test]
    fn another_writer_prevents_cleanup_until_its_transaction_finishes() {
        let (temp, db, root) = fixture();
        db.execute("DELETE FROM messages WHERE id=?1", [A]).unwrap();
        let publisher = Connection::open(temp.path().join("test.db")).unwrap();
        publisher.execute_batch("BEGIN IMMEDIATE;").unwrap();
        db.busy_timeout(std::time::Duration::ZERO).unwrap();
        assert!(drain(&db, &root).is_err());
        assert!(index(&temp, A));
        publisher.execute_batch("ROLLBACK;").unwrap();
        drain(&db, &root).unwrap();
        assert!(!index(&temp, A));
    }
    #[cfg(unix)]
    #[test]
    fn outside_parent_symlink_is_rejected_and_final_symlink_is_unlinked() {
        let (temp, db, root) = fixture();
        let outside = tempfile::tempdir().unwrap();
        let sentinel = outside.path().join(format!("{A}.json"));
        fs::write(&sentinel, b"safe").unwrap();
        db.execute("DELETE FROM messages WHERE id=?1", [A]).unwrap();
        let parent = temp.path().join("data/media-index");
        fs::rename(&parent, temp.path().join("held")).unwrap();
        std::os::unix::fs::symlink(outside.path(), &parent).unwrap();
        assert!(drain(&db, &root).is_err());
        assert_eq!(fs::read(&sentinel).unwrap(), b"safe");
        fs::remove_file(&parent).unwrap();
        fs::create_dir(&parent).unwrap();
        std::os::unix::fs::symlink(&sentinel, parent.join(format!("{A}.json"))).unwrap();
        drain(&db, &root).unwrap();
        assert_eq!(fs::read(&sentinel).unwrap(), b"safe");
        assert!(fs::symlink_metadata(parent.join(format!("{A}.json"))).is_err());
    }
}
