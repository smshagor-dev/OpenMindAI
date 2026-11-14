//! Roll back ordinary text-export failures through the held destination handle.
//! SQLite and filesystem commits are separate: process/power loss still needs a journal.
use crate::app_error::AppError;
use cap_std::{ambient_authority, fs::Dir};
use rusqlite::{params, Connection, Transaction, TransactionBehavior};
use std::{
    io::{self, Write},
    path::{Component, Path},
};

pub(crate) struct TextArtifactWrite<'a> {
    pub conversation_id: &'a str,
    pub message_id: Option<&'a str>,
    pub name: &'a str,
    pub relative: &'a str,
    pub mime_type: &'a str,
    pub kind: &'a str,
    pub content: &'a [u8],
}

pub(crate) fn save(
    connection: &Connection,
    root: &Path,
    input: TextArtifactWrite<'_>,
) -> Result<String, AppError> {
    save_with(connection, root, input, |file, content| {
        file.write_all(content)
    })
}

struct OutputRollback {
    parent: Dir,
    name: String,
    pending: String,
    pending_created: bool,
    published: bool,
    armed: bool,
}
impl OutputRollback {
    fn cleanup(&mut self) -> io::Result<()> {
        let mut failure = None;
        for name in [
            self.pending_created.then_some(self.pending.as_str()),
            self.published.then_some(self.name.as_str()),
        ]
        .into_iter()
        .flatten()
        {
            if let Err(error) = self.parent.remove_file(name) {
                if error.kind() != io::ErrorKind::NotFound {
                    failure = Some(error);
                }
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        self.armed = false;
        Ok(())
    }
}
impl Drop for OutputRollback {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.cleanup();
        }
    }
}

fn save_with(
    connection: &Connection,
    root: &Path,
    input: TextArtifactWrite<'_>,
    write: impl FnOnce(&mut cap_std::fs::File, &[u8]) -> io::Result<()>,
) -> Result<String, AppError> {
    if !matches!(input.kind, "text" | "markdown" | "code") {
        return Err(AppError::internal("unsupported text artifact kind"));
    }
    let relative = Path::new(input.relative);
    if !relative.starts_with("generated/files")
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(AppError::internal("invalid text artifact destination"));
    }
    let name = relative
        .file_name()
        .and_then(|part| part.to_str())
        .ok_or_else(|| AppError::internal("text artifact has no filename"))?;
    let tx = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    let owner_exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1)",
        [input.conversation_id],
        |row| row.get(0),
    )?;
    if !owner_exists {
        return Err(AppError::internal(
            "text artifact conversation does not exist",
        ));
    }
    if let Some(message) = input.message_id {
        let matches: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE id=?1 AND conversation_id=?2)",
            params![message, input.conversation_id],
            |row| row.get(0),
        )?;
        if !matches {
            return Err(AppError::internal(
                "text artifact message does not belong to its conversation",
            ));
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let size = i64::try_from(input.content.len())
        .map_err(|_| AppError::internal("text artifact is too large"))?;
    // Insert and validate constraints before opening any output. The ready row
    // remains invisible to other connections until publication and commit succeed.
    tx.execute("INSERT INTO artifacts (id,conversation_id,message_id,name,path,mime_type,kind,size_bytes,page_count,status,error,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,NULL,'ready',NULL,?9,?9)", params![id,input.conversation_id,input.message_id,input.name,input.relative,input.mime_type,input.kind,size,now])?;
    let root = Dir::open_ambient_dir(root, ambient_authority())?;
    let mut output = OutputRollback {
        parent: root.open_dir(relative.parent().unwrap())?,
        name: name.to_owned(),
        pending: format!(".pending-{id}"),
        pending_created: false,
        published: false,
        armed: true,
    };
    let result = (|| -> Result<(), AppError> {
        let mut options = cap_std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = output.parent.open_with(&output.pending, &options)?;
        output.pending_created = true;
        write(&mut file, input.content)?;
        file.sync_all()?;
        drop(file);
        match output.parent.symlink_metadata(&output.name) {
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "text artifact destination already exists",
                )
                .into())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        output
            .parent
            .rename(&output.pending, &output.parent, &output.name)?;
        output.published = true;
        #[cfg(unix)]
        output.parent.open(".")?.sync_all()?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(error) = result {
        if let Err(cleanup) = output.cleanup() {
            return Err(AppError::internal(format!(
                "{error}; text artifact cleanup also failed: {cleanup}"
            )));
        }
        return Err(error);
    }
    output.armed = false;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn fixture() -> (tempfile::TempDir, Connection) {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("generated/files/reserved")).unwrap();
        let db = Connection::open(root.path().join("fixture.db")).unwrap();
        db.execute_batch(include_str!("../migrations/001_initial.sql"))
            .unwrap();
        db.execute_batch(include_str!("../migrations/002_artifacts.sql"))
            .unwrap();
        db.execute_batch("INSERT INTO conversations(id,title,created_at,updated_at) VALUES ('owner','Owner','now','now'),('other','Other','now','now'); INSERT INTO messages(id,conversation_id,role,content,status,created_at,updated_at) VALUES ('message','owner','user','hello','completed','now','now');").unwrap();
        (root, db)
    }
    fn input() -> TextArtifactWrite<'static> {
        TextArtifactWrite {
            conversation_id: "owner",
            message_id: Some("message"),
            name: "report.txt",
            relative: "generated/files/reserved/report.txt",
            mime_type: "text/plain",
            kind: "text",
            content: b"exact text\n",
        }
    }
    fn assert_empty(root: &Path, db: &Connection) {
        assert_eq!(
            fs::read_dir(root.join("generated/files/reserved"))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM artifacts", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn success_publishes_exact_bytes_and_ready_metadata_including_empty_text() {
        for content in [b"exact text\n".as_slice(), b""] {
            let (root, db) = fixture();
            let mut request = input();
            request.content = content;
            let id = save(&db, root.path(), request).unwrap();
            assert_eq!(
                fs::read(root.path().join(input().relative)).unwrap(),
                content
            );
            let row = db
                .query_row(
                    "SELECT status,size_bytes FROM artifacts WHERE id=?1",
                    [id],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
                )
                .unwrap();
            assert_eq!(row, ("ready".into(), content.len() as i64));
            assert_eq!(
                fs::read_dir(root.path().join("generated/files/reserved"))
                    .unwrap()
                    .count(),
                1
            );
        }
    }
    #[test]
    fn missing_owners_and_cross_conversation_messages_create_no_files() {
        for (conversation, message) in [
            ("missing", Some("message")),
            ("owner", Some("missing")),
            ("other", Some("message")),
        ] {
            let (root, db) = fixture();
            let mut request = input();
            request.conversation_id = conversation;
            request.message_id = message;
            assert!(save(&db, root.path(), request).is_err());
            assert_empty(root.path(), &db);
        }
    }
    #[test]
    fn rejected_insert_creates_no_output() {
        let (root, db) = fixture();
        db.execute_batch("CREATE TRIGGER reject_artifact BEFORE INSERT ON artifacts BEGIN SELECT RAISE(ABORT,'injected database failure'); END;").unwrap();
        assert!(save(&db, root.path(), input()).is_err());
        assert_empty(root.path(), &db);
    }
    #[test]
    fn deferred_database_commit_failure_removes_published_file() {
        let (root, db) = fixture();
        db.execute_batch("CREATE TABLE deferred_guard(artifact TEXT REFERENCES artifacts(id), missing TEXT REFERENCES conversations(id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER reject_commit AFTER INSERT ON artifacts BEGIN INSERT INTO deferred_guard VALUES(NEW.id,'missing'); END;").unwrap();
        assert!(save(&db, root.path(), input()).is_err());
        assert_empty(root.path(), &db);
        assert!(db.is_autocommit());
    }
    #[test]
    fn partial_disk_write_and_permission_errors_remove_staging_and_row() {
        for kind in [io::ErrorKind::StorageFull, io::ErrorKind::PermissionDenied] {
            let (root, db) = fixture();
            assert!(save_with(&db, root.path(), input(), |file, _| {
                file.write_all(b"partial")?;
                Err(io::Error::new(kind, "injected writer failure"))
            })
            .is_err());
            assert_empty(root.path(), &db);
        }
    }
    #[test]
    fn unwinding_writer_rolls_back_staging_and_transaction() {
        let (root, db) = fixture();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = save_with(&db, root.path(), input(), |file, _| {
                file.write_all(b"partial")?;
                panic!("injected unwind");
            });
        }));
        assert!(result.is_err());
        assert_empty(root.path(), &db);
    }
    #[test]
    fn unowned_pending_entry_is_never_removed_by_rollback() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(".pending-collision"), b"another owner").unwrap();
        let mut rollback = OutputRollback {
            parent: Dir::open_ambient_dir(root.path(), ambient_authority()).unwrap(),
            name: "final".into(),
            pending: ".pending-collision".into(),
            pending_created: false,
            published: false,
            armed: true,
        };
        rollback.cleanup().unwrap();
        assert_eq!(
            fs::read(root.path().join(".pending-collision")).unwrap(),
            b"another owner"
        );
    }

    #[test]
    fn ready_row_is_not_visible_before_publication_commits() {
        let (root, db) = fixture();
        save_with(&db, root.path(), input(), |file, bytes| {
            let reader = Connection::open(root.path().join("fixture.db")).unwrap();
            assert_eq!(
                reader
                    .query_row("SELECT count(*) FROM artifacts", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                0
            );
            file.write_all(bytes)
        })
        .unwrap();
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM artifacts WHERE status='ready'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }

    #[test]
    fn collision_preserves_existing_file_and_rolls_back_row() {
        let (root, db) = fixture();
        fs::write(root.path().join(input().relative), b"existing").unwrap();
        assert!(save(&db, root.path(), input()).is_err());
        assert_eq!(
            fs::read(root.path().join(input().relative)).unwrap(),
            b"existing"
        );
        assert_eq!(
            fs::read_dir(root.path().join("generated/files/reserved"))
                .unwrap()
                .count(),
            1
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM artifacts", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn replaced_parent_cannot_redirect_rollback_after_database_failure() {
        let (root, db) = fixture();
        let outside = tempfile::tempdir().unwrap();
        db.execute_batch("CREATE TABLE deferred_guard(missing TEXT REFERENCES conversations(id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER reject_commit AFTER INSERT ON artifacts BEGIN INSERT INTO deferred_guard VALUES('missing'); END;").unwrap();
        let result = save_with(&db, root.path(), input(), |file, bytes| {
            let parent = root.path().join("generated/files/reserved");
            let moved = root.path().join("held");
            match fs::rename(&parent, &moved) {
                Ok(()) => {
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(outside.path(), &parent).unwrap();
                    #[cfg(windows)]
                    fs::create_dir(&parent).unwrap();
                }
                Err(error) => {
                    #[cfg(windows)]
                    assert!(matches!(error.raw_os_error(), Some(5 | 32)), "{error}");
                    #[cfg(not(windows))]
                    panic!("directory replacement failed: {error}");
                }
            }
            fs::write(outside.path().join("report.txt"), b"outside sentinel").unwrap();
            file.write_all(bytes)
        });
        assert!(result.is_err());
        let held = root.path().join("held");
        let actual = if held.exists() {
            held
        } else {
            root.path().join("generated/files/reserved")
        };
        assert_eq!(fs::read_dir(actual).unwrap().count(), 0);
        assert_eq!(
            fs::read(outside.path().join("report.txt")).unwrap(),
            b"outside sentinel"
        );
        #[cfg(unix)]
        fs::remove_file(root.path().join("generated/files/reserved")).unwrap();
    }
}
