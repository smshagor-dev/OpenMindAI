use std::{
    io,
    path::{Component, Path, PathBuf},
};

use chrono::Utc;
use rusqlite::{functions::FunctionFlags, params, Connection, Error as SqliteError};
use uuid::Uuid;

use crate::app_error::AppError;
use crate::portable_root::CURRENT_SCHEMA_VERSION;

const SCHEMA_VERSION_KEY: &str = "schema.version";
const ARTIFACT_DELETE_FUNCTION: &str = "openmind_delete_artifact";

struct Migration {
    number: u32,
    name: &'static str,
    sql: &'static str,
}

/// Applied in order, each exactly once (tracked in `schema_migrations`) —
/// not idempotently replayed on every open. The two existing files still
/// use `CREATE TABLE/INDEX IF NOT EXISTS`, which is harmless to run once
/// more on a pre-existing database created before this tracking table
/// existed; new migrations don't need that idempotency, since this runner
/// guarantees they only ever execute a single time per database.
const MIGRATIONS: &[Migration] = &[
    Migration {
        number: 1,
        name: "001_initial",
        sql: include_str!("../migrations/001_initial.sql"),
    },
    Migration {
        number: 2,
        name: "002_artifacts",
        sql: include_str!("../migrations/002_artifacts.sql"),
    },
    Migration {
        number: 3,
        name: "003_projects",
        sql: include_str!("../migrations/003_projects.sql"),
    },
    Migration {
        number: 4,
        name: "004_project_file_ingestion",
        sql: include_str!("../migrations/004_project_file_ingestion.sql"),
    },
    Migration {
        number: 5,
        name: "005_artifact_cleanup_and_media_kinds",
        sql: include_str!("../migrations/005_artifact_cleanup_and_media_kinds.sql"),
    },
    Migration {
        number: 6,
        name: "006_single_project_per_conversation",
        sql: include_str!("../migrations/006_single_project_per_conversation.sql"),
    },
];

pub struct Database {
    connection: Connection,
}

impl Database {
    pub fn open(path: PathBuf) -> Result<Self, AppError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let artifact_root = artifact_root_for_database(&path);
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        register_artifact_cleanup(&connection, artifact_root)?;
        let mut database = Self { connection };
        database.migrate()?;
        database.recover_interrupted_chat_messages()?;
        database.ensure_local_profile()?;
        Ok(database)
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self, AppError> {
        let connection = Connection::open_in_memory()?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        // In-memory repository tests have no portable root. Register the same
        // SQL function as a no-op so the production trigger remains testable
        // without granting tests arbitrary filesystem deletion authority.
        register_artifact_cleanup(&connection, None)?;
        let mut database = Self { connection };
        database.migrate()?;
        database.ensure_local_profile()?;
        Ok(database)
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    #[cfg(test)]
    pub fn pragma_string(&self, name: &str) -> Result<String, AppError> {
        self.connection
            .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
            .map_err(AppError::from)
    }

    #[cfg(test)]
    pub fn pragma_i64(&self, name: &str) -> Result<i64, AppError> {
        self.connection
            .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
            .map_err(AppError::from)
    }

    fn migrate(&mut self) -> Result<(), AppError> {
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                number INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at TEXT NOT NULL
            )",
        )?;

        for migration in MIGRATIONS {
            let already_applied: bool = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE number = ?1)",
                params![migration.number],
                |row| row.get(0),
            )?;
            if already_applied {
                continue;
            }

            // One transaction per migration: if `migration.sql` fails
            // partway, nothing from it commits and it isn't marked applied,
            // so the next `open()` retries it cleanly instead of getting
            // stuck half-migrated.
            let tx = self.connection.transaction()?;
            tx.execute_batch(migration.sql)?;
            tx.execute(
                "INSERT INTO schema_migrations (number, name, applied_at) VALUES (?1, ?2, ?3)",
                params![migration.number, migration.name, Utc::now().to_rfc3339()],
            )?;
            tx.commit()?;
        }

        self.record_schema_version()?;
        Ok(())
    }

    /// A process exit can interrupt inference after the assistant row has
    /// already been persisted as `pending` or `streaming`. There is no live
    /// generation left after a fresh process start, so those states must not
    /// survive indefinitely and block retry/regenerate UX.
    fn recover_interrupted_chat_messages(&self) -> Result<usize, AppError> {
        let now = Utc::now().to_rfc3339();
        self.connection
            .execute(
                "UPDATE messages
                 SET status = 'failed', updated_at = ?1
                 WHERE status IN ('pending', 'streaming')",
                params![now],
            )
            .map_err(AppError::from)
    }

    /// Records the schema version this build understands. Kept independent
    /// from `app_version` (Cargo package version) — application releases and
    /// database schema revisions evolve on different timelines.
    fn record_schema_version(&self) -> Result<(), AppError> {
        let now = Utc::now().to_rfc3339();
        self.connection.execute(
            "INSERT INTO app_settings (key, value_json, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
            params![
                SCHEMA_VERSION_KEY,
                CURRENT_SCHEMA_VERSION.to_string(),
                now
            ],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn schema_version(&self) -> Result<u32, AppError> {
        let raw: String = self.connection.query_row(
            "SELECT value_json FROM app_settings WHERE key = ?1",
            params![SCHEMA_VERSION_KEY],
            |row| row.get(0),
        )?;
        raw.parse()
            .map_err(|_| AppError::internal("stored schema version is not a valid integer"))
    }

    fn ensure_local_profile(&self) -> Result<(), AppError> {
        let count: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM app_profiles", [], |row| row.get(0))?;
        if count == 0 {
            let now = Utc::now().to_rfc3339();
            self.connection.execute(
                "INSERT INTO app_profiles (id, display_name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
                params![Uuid::new_v4().to_string(), "Local User", now, now],
            )?;
        }
        Ok(())
    }
}

fn artifact_root_for_database(path: &Path) -> Option<PathBuf> {
    let database_dir = path.parent()?;
    if database_dir.file_name()?.to_string_lossy() != "database" {
        return None;
    }
    let data_dir = database_dir.parent()?;
    if data_dir.file_name()?.to_string_lossy() != "data" {
        return None;
    }
    data_dir.parent().map(Path::to_path_buf)
}

fn register_artifact_cleanup(
    connection: &Connection,
    root: Option<PathBuf>,
) -> Result<(), AppError> {
    connection.create_scalar_function(
        ARTIFACT_DELETE_FUNCTION,
        1,
        FunctionFlags::SQLITE_UTF8,
        move |context| {
            let relative: String = context.get(0)?;
            let Some(root) = root.as_ref() else {
                return Ok(0_i64);
            };
            let relative_path = Path::new(&relative);
            let unsafe_path = relative_path.is_absolute()
                || !relative_path.starts_with("generated")
                || relative_path.components().any(|component| {
                    matches!(
                        component,
                        Component::ParentDir | Component::RootDir | Component::Prefix(_)
                    )
                });
            if unsafe_path {
                return Err(SqliteError::UserFunctionError(Box::new(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "artifact cleanup path escaped the generated directory",
                ))));
            }

            let path = root.join(relative_path);
            if !path.exists() {
                return Ok(0_i64);
            }
            let canonical_root = std::fs::canonicalize(root)
                .map_err(|error| SqliteError::UserFunctionError(Box::new(error)))?;
            let canonical_path = std::fs::canonicalize(&path)
                .map_err(|error| SqliteError::UserFunctionError(Box::new(error)))?;
            if !canonical_path.starts_with(&canonical_root) {
                return Err(SqliteError::UserFunctionError(Box::new(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "artifact cleanup resolved outside the portable root",
                ))));
            }

            match std::fs::remove_file(&path) {
                Ok(()) => Ok(1_i64),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(0_i64),
                Err(error) => Err(SqliteError::UserFunctionError(Box::new(error))),
            }
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enables_wal_and_foreign_keys() {
        let temp = tempfile::tempdir().unwrap();
        let database = Database::open(temp.path().join("openmind_ai.db")).unwrap();

        assert_eq!(database.pragma_string("journal_mode").unwrap(), "wal");
        assert_eq!(database.pragma_i64("foreign_keys").unwrap(), 1);
    }

    #[test]
    fn records_current_schema_version() {
        let database = Database::in_memory().unwrap();
        assert_eq!(database.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn migrations_are_recorded_and_not_reapplied() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("openmind_ai.db");
        {
            let _database = Database::open(path.clone()).unwrap();
        }
        // Reopening must not error re-running a non-idempotent migration --
        // there isn't one yet, but this is the regression test for when
        // there is.
        let database = Database::open(path).unwrap();
        let applied: i64 = database
            .connection
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(applied, MIGRATIONS.len() as i64);
    }

    #[test]
    fn pre_existing_database_without_tracking_table_still_opens() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("openmind_ai.db");
        {
            // Simulates a database created before `schema_migrations`
            // existed: the tables are already there, but nothing tracks it.
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(include_str!("../migrations/001_initial.sql"))
                .unwrap();
            connection
                .execute_batch(include_str!("../migrations/002_artifacts.sql"))
                .unwrap();
        }
        let database = Database::open(path).unwrap();
        let applied: i64 = database
            .connection
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(applied, MIGRATIONS.len() as i64);
    }

    #[test]
    fn interrupted_chat_messages_are_failed_on_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("openmind_ai.db");
        let conversation_id = Uuid::new_v4().to_string();
        let message_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        {
            let database = Database::open(path.clone()).unwrap();
            database
                .connection
                .execute(
                    "INSERT INTO conversations (id, title, created_at, updated_at, pinned)
                     VALUES (?1, 'Interrupted', ?2, ?2, 0)",
                    params![conversation_id, now],
                )
                .unwrap();
            database
                .connection
                .execute(
                    "INSERT INTO messages
                     (id, conversation_id, role, content, status, created_at, updated_at)
                     VALUES (?1, ?2, 'assistant', 'partial', 'streaming', ?3, ?3)",
                    params![message_id, conversation_id, now],
                )
                .unwrap();
        }

        let database = Database::open(path).unwrap();
        let status: String = database
            .connection
            .query_row(
                "SELECT status FROM messages WHERE id = ?1",
                params![message_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "failed");
    }

    #[test]
    fn conversation_delete_cleans_generated_artifact_file() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("OpenMindAI");
        let database_dir = root.join("data/database");
        let generated_dir = root.join("generated/files");
        std::fs::create_dir_all(&database_dir).unwrap();
        std::fs::create_dir_all(&generated_dir).unwrap();
        let artifact_path = generated_dir.join("chat-note.txt");
        std::fs::write(&artifact_path, b"generated chat artifact").unwrap();

        let database = Database::open(database_dir.join("openmind_ai.db")).unwrap();
        let chat = crate::chat::ChatRepository::new(&database);
        let conversation = chat.create_conversation(Some("Cleanup")).unwrap();
        crate::artifacts::ArtifactRepository::new(&database)
            .create(
                &conversation.id,
                None,
                "chat-note.txt",
                "generated/files/chat-note.txt",
                "text/plain",
                "text",
                "ready",
            )
            .unwrap();

        chat.delete_conversation(&conversation.id).unwrap();

        assert!(!artifact_path.exists());
        let artifact_count: i64 = database
            .connection
            .query_row("SELECT COUNT(*) FROM artifacts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(artifact_count, 0);
    }

    #[test]
    fn artifact_cleanup_rejects_paths_outside_generated_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("OpenMindAI");
        let database_dir = root.join("data/database");
        std::fs::create_dir_all(&database_dir).unwrap();
        let outside_path = root.join("keep.txt");
        std::fs::write(&outside_path, b"keep").unwrap();

        let database = Database::open(database_dir.join("openmind_ai.db")).unwrap();
        let chat = crate::chat::ChatRepository::new(&database);
        let conversation = chat.create_conversation(Some("Guarded cleanup")).unwrap();
        crate::artifacts::ArtifactRepository::new(&database)
            .create(
                &conversation.id,
                None,
                "keep.txt",
                "../keep.txt",
                "text/plain",
                "text",
                "ready",
            )
            .unwrap();

        assert!(chat.delete_conversation(&conversation.id).is_err());
        assert!(outside_path.exists());
        assert!(chat.find_conversation(&conversation.id).is_ok());
    }

    #[test]
    fn artifact_schema_accepts_audio_and_video_kinds() {
        let database = Database::in_memory().unwrap();
        let chat = crate::chat::ChatRepository::new(&database);
        let conversation = chat.create_conversation(Some("Media")).unwrap();
        let artifacts = crate::artifacts::ArtifactRepository::new(&database);

        artifacts
            .create(
                &conversation.id,
                None,
                "voice.wav",
                "generated/audio/voice.wav",
                "audio/wav",
                "audio",
                "generating",
            )
            .unwrap();
        artifacts
            .create(
                &conversation.id,
                None,
                "clip.webm",
                "generated/video/clip.webm",
                "video/webm",
                "video",
                "generating",
            )
            .unwrap();
    }

    #[test]
    fn unavailable_database_returns_controlled_error() {
        let temp = tempfile::tempdir().unwrap();
        let directory_path = temp.path().join("not-a-database.db");
        std::fs::create_dir_all(&directory_path).unwrap();

        let error = match Database::open(directory_path) {
            Ok(_) => panic!("opening a directory as a database should fail"),
            Err(error) => error,
        };
        assert!(matches!(error, AppError::DatabaseInitFailed(_)));
    }
}
