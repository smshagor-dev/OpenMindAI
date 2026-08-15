use std::path::PathBuf;

use chrono::Utc;
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::app_error::AppError;
use crate::portable_root::CURRENT_SCHEMA_VERSION;

const SCHEMA_VERSION_KEY: &str = "schema.version";

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
];

pub struct Database {
    connection: Connection,
}

impl Database {
    pub fn open(path: PathBuf) -> Result<Self, AppError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let mut database = Self { connection };
        database.migrate()?;
        database.ensure_local_profile()?;
        Ok(database)
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self, AppError> {
        let connection = Connection::open_in_memory()?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
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
