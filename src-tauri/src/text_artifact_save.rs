//! Roll back ordinary text-export failures through the held destination handle.
//! Durable intents bridge a process interruption between filesystem and SQLite commits.
use crate::app_error::AppError;
use cap_std::{ambient_authority, fs::Dir};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::{
    io::{self, Read, Write},
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

const JOURNAL: &str = "generated/files/.text-export-journal";
#[derive(serde::Serialize, serde::Deserialize)]
struct Intent {
    id: String,
    relative: String,
}

fn sync_dir(dir: &Dir) -> io::Result<()> {
    #[cfg(unix)]
    dir.open(".")?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}
fn lock(journal: &Dir) -> Result<std::fs::File, AppError> {
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = journal.open_with("writer.lock", &options)?.into_std();
    if !file.metadata()?.is_file() {
        return Err(AppError::internal("text export lock is not a regular file"));
    }
    file.lock()?;
    Ok(file)
}
fn validate_intent(intent: &Intent) -> Result<(), AppError> {
    if uuid::Uuid::parse_str(&intent.id).is_err() {
        return Err(AppError::internal("invalid text intent identity"));
    }
    let path = Path::new(&intent.relative);
    if !path.starts_with("generated/files")
        || path.components().count() != 4
        || path
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
    {
        return Err(AppError::internal("invalid text intent path"));
    }
    Ok(())
}
fn write_intent(journal: &Dir, intent: &Intent) -> Result<(), AppError> {
    validate_intent(intent)?;
    let name = format!("{}.json", intent.id);
    let pending = format!("{}.new", intent.id);
    let mut options = cap_std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = journal.open_with(&pending, &options)?;
    let result = (|| -> Result<(), AppError> {
        file.write_all(
            &serde_json::to_vec(intent).map_err(|e| AppError::internal(e.to_string()))?,
        )?;
        file.sync_all()?;
        drop(file);
        journal.rename(&pending, journal, &name)?;
        sync_dir(journal)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = journal.remove_file(&pending);
    }
    result
}
fn forget(journal: &Dir, id: &str) -> io::Result<()> {
    journal.remove_file(format!("{id}.json"))?;
    sync_dir(journal)
}
fn remove_if_present(parent: &Dir, name: &str) -> io::Result<()> {
    match parent.remove_file(name) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Called after opening/migrating the DB. Lock ordering matches save: OS lock,
/// then SQLite writer transaction. Recovery never races a live exporting process.
pub(crate) fn recover(connection: &Connection, root: &Path) -> Result<usize, AppError> {
    let root = Dir::open_ambient_dir(root, ambient_authority())?;
    let journal = match root.open_dir(JOURNAL) {
        Ok(dir) => dir,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.into()),
    };
    let _lock = lock(&journal)?;
    let tx = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    let mut recovered = 0;
    let mut failure = None;
    for entry in journal.entries()? {
        let entry = entry?;
        let name = entry.file_name();
        let path = Path::new(&name);
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if uuid::Uuid::parse_str(id).is_err() {
            continue;
        }
        let result = (|| -> Result<(), AppError> {
            if path.extension().is_some_and(|s| s == "new") {
                journal.remove_file(&name)?;
                sync_dir(&journal)?;
                return Ok(());
            }
            if !path.extension().is_some_and(|s| s == "json") {
                return Ok(());
            }
            if !entry.file_type()?.is_file() {
                return Err(AppError::internal("text intent is not a regular file"));
            }
            let mut file = journal.open(&name)?;
            let mut bytes = Vec::new();
            (&mut file).take(65537).read_to_end(&mut bytes)?;
            if bytes.len() > 65536 {
                return Err(AppError::internal("oversized text intent"));
            }
            let intent: Intent = serde_json::from_slice(&bytes)
                .map_err(|_| AppError::internal("malformed text intent"))?;
            validate_intent(&intent)?;
            if intent.id != id {
                return Err(AppError::internal("text intent identity mismatch"));
            }
            let row: Option<(String, String)> = tx
                .query_row("SELECT path,status FROM artifacts WHERE id=?1", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()?;
            if let Some((relative, status)) = &row {
                if relative != &intent.relative || status != "ready" {
                    return Err(AppError::internal(
                        "text intent conflicts with registered artifact",
                    ));
                }
            }
            let path = Path::new(&intent.relative);
            let parent = match root.open_dir(path.parent().unwrap()) {
                Ok(parent) => Some(parent),
                Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            if let Some(parent) = parent {
                remove_if_present(&parent, &format!(".pending-{id}"))?;
                if row.is_none() {
                    remove_if_present(&parent, path.file_name().unwrap().to_str().unwrap())?;
                }
                sync_dir(&parent)?;
            }
            drop(file);
            forget(&journal, id)?;
            recovered += 1;
            Ok(())
        })();
        if let Err(error) = result {
            failure = Some(error);
        }
    }
    tx.commit()?;
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(recovered)
}
#[cfg(test)]
fn checkpoint(stage: &str) {
    if std::env::var("OPENMIND_TEST_TEXT_CRASH_STAGE")
        .ok()
        .as_deref()
        == Some(stage)
    {
        let root = std::env::var_os("OPENMIND_TEST_TEXT_CRASH_ROOT").unwrap();
        std::fs::write(Path::new(&root).join("paused"), stage).unwrap();
        loop {
            std::thread::park_timeout(std::time::Duration::from_secs(1));
        }
    }
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
        sync_dir(&self.parent)?;
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
    let root = Dir::open_ambient_dir(root, ambient_authority())?;
    root.create_dir_all(JOURNAL)?;
    let journal = root.open_dir(JOURNAL)?;
    sync_dir(&root.open_dir("generated/files")?)?;
    let _lock = lock(&journal)?;
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
    let mut output = OutputRollback {
        parent: root.open_dir(relative.parent().unwrap())?,
        name: name.to_owned(),
        pending: format!(".pending-{id}"),
        pending_created: false,
        published: false,
        armed: true,
    };
    // Verify collision-free entries before persisting authority to clean them.
    // Same-user mutations after this point remain outside the trust boundary.
    for name in [&output.name, &output.pending] {
        match output.parent.symlink_metadata(name) {
            Ok(_) => {
                return Err(AppError::internal(
                    "text artifact destination already exists",
                ))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    write_intent(
        &journal,
        &Intent {
            id: id.clone(),
            relative: input.relative.to_owned(),
        },
    )?;
    #[cfg(test)]
    checkpoint("intent");
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
        #[cfg(test)]
        checkpoint("write");
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
        #[cfg(test)]
        checkpoint("rename");
        #[cfg(unix)]
        output.parent.open(".")?.sync_all()?;
        tx.commit()?;
        #[cfg(test)]
        checkpoint("commit");
        Ok(())
    })();
    if let Err(error) = result {
        if let Err(cleanup) = output.cleanup() {
            return Err(AppError::internal(format!(
                "{error}; text artifact cleanup also failed: {cleanup}"
            )));
        }
        forget(&journal, &id)?;
        return Err(error);
    }
    output.armed = false;
    // A committed row/output remains valid even if intent cleanup fails; next
    // recovery retains it. Do not report an export failure after commit.
    if let Err(error) = forget(&journal, &id) {
        tracing::warn!(%error,"text export intent cleanup deferred");
    }
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
    fn crash_writer_child() {
        let Some(root) = std::env::var_os("OPENMIND_TEST_TEXT_CRASH_ROOT") else {
            return;
        };
        let root = Path::new(&root);
        if std::env::var("OPENMIND_TEST_TEXT_MODE").ok().as_deref() == Some("probe") {
            let dir = Dir::open_ambient_dir(root.join(JOURNAL), ambient_authority()).unwrap();
            let mut options = cap_std::fs::OpenOptions::new();
            options.read(true).write(true);
            let file = dir.open_with("writer.lock", &options).unwrap().into_std();
            assert!(matches!(
                file.try_lock(),
                Err(std::fs::TryLockError::WouldBlock)
            ));
            return;
        }
        let db = Connection::open(root.join("fixture.db")).unwrap();
        db.pragma_update(None, "foreign_keys", "ON").unwrap();
        save(&db, root, input()).unwrap();
    }
    #[test]
    fn killed_process_recovers_uncommitted_outputs_and_preserves_committed_bytes() {
        use std::{
            process::Command,
            time::{Duration, Instant},
        };
        for stage in ["intent", "write", "rename", "commit"] {
            let (root, db) = fixture();
            drop(db);
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "text_artifact_save::tests::crash_writer_child",
                    "--nocapture",
                ])
                .env("OPENMIND_TEST_TEXT_CRASH_ROOT", root.path())
                .env("OPENMIND_TEST_TEXT_CRASH_STAGE", stage)
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(20);
            while !root.path().join("paused").exists() {
                if let Some(status) = child.try_wait().unwrap() {
                    panic!("child exited before {stage}: {status}");
                }
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("child never reached {stage}");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let probe = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "text_artifact_save::tests::crash_writer_child",
                    "--nocapture",
                ])
                .env("OPENMIND_TEST_TEXT_CRASH_ROOT", root.path())
                .env("OPENMIND_TEST_TEXT_MODE", "probe")
                .status()
                .unwrap();
            assert!(probe.success(), "cross-process recovery exclusion failed");
            child.kill().unwrap();
            child.wait().unwrap();
            let db = Connection::open(root.path().join("fixture.db")).unwrap();
            assert_eq!(recover(&db, root.path()).unwrap(), 1);
            let count = db
                .query_row("SELECT count(*) FROM artifacts", [], |r| r.get::<_, i64>(0))
                .unwrap();
            if stage == "commit" {
                assert_eq!(count, 1);
                assert_eq!(
                    fs::read(root.path().join(input().relative)).unwrap(),
                    input().content
                );
            } else {
                assert_eq!(count, 0);
                assert_empty(root.path(), &db);
            }
            assert_eq!(
                recover(&db, root.path()).unwrap(),
                0,
                "recovery must be idempotent"
            );
            assert_eq!(
                fs::read_dir(root.path().join(JOURNAL)).unwrap().count(),
                1,
                "only lock remains"
            );
        }
    }
    #[test]
    fn malformed_or_conflicting_intents_do_not_delete_user_files() {
        let (root, db) = fixture();
        let journal_path = root.path().join(JOURNAL);
        fs::create_dir_all(&journal_path).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let outside = tempfile::NamedTempFile::new().unwrap();
        fs::write(outside.path(), b"sentinel").unwrap();
        fs::write(
            journal_path.join(format!("{id}.json")),
            serde_json::to_vec(&Intent {
                id: id.clone(),
                relative: outside.path().to_string_lossy().into_owned(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(recover(&db, root.path()).is_err());
        assert_eq!(fs::read(outside.path()).unwrap(), b"sentinel");
        assert!(journal_path.join(format!("{id}.json")).exists());
        let partial = uuid::Uuid::new_v4().to_string();
        fs::write(journal_path.join(format!("{partial}.new")), b"incomplete").unwrap();
        assert!(recover(&db, root.path()).is_err());
        assert!(!journal_path.join(format!("{partial}.new")).exists());
    }

    #[test]
    fn conflicting_registered_identity_is_preserved_for_review() {
        let (root, db) = fixture();
        let journal_path = root.path().join(JOURNAL);
        fs::create_dir_all(&journal_path).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        fs::write(root.path().join(input().relative), b"existing user bytes").unwrap();
        db.execute("INSERT INTO artifacts(id,conversation_id,name,path,mime_type,kind,status,created_at,updated_at) VALUES(?1,'owner','other','generated/files/other/other.txt','text/plain','text','ready','now','now')",[&id]).unwrap();
        fs::write(
            journal_path.join(format!("{id}.json")),
            serde_json::to_vec(&Intent {
                id: id.clone(),
                relative: input().relative.into(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(recover(&db, root.path()).is_err());
        assert_eq!(
            fs::read(root.path().join(input().relative)).unwrap(),
            b"existing user bytes"
        );
        assert!(journal_path.join(format!("{id}.json")).exists());
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
