use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{app_error::AppError, database::Database};

const SETTINGS_KEY: &str = "app.google";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredGoogleCredentials {
    client_id: String,
    client_secret: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleCredentialsStatus {
    pub client_id: String,
    pub has_secret: bool,
}

pub struct GoogleRepository<'a> {
    database: &'a Database,
}

impl<'a> GoogleRepository<'a> {
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    pub fn get_status(&self) -> Result<Option<GoogleCredentialsStatus>, AppError> {
        Ok(self.get_stored()?.map(|stored| GoogleCredentialsStatus {
            client_id: stored.client_id,
            has_secret: !stored.client_secret.is_empty(),
        }))
    }

    fn get_stored(&self) -> Result<Option<StoredGoogleCredentials>, AppError> {
        let value: Option<String> = self
            .database
            .connection()
            .query_row(
                "SELECT value_json FROM app_settings WHERE key = ?1",
                params![SETTINGS_KEY],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.and_then(|value| serde_json::from_str(&value).ok()))
    }

    pub fn save(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> Result<GoogleCredentialsStatus, AppError> {
        let stored = StoredGoogleCredentials {
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
        };
        let payload = serde_json::to_string(&stored)
            .map_err(|error| AppError::internal(error.to_string()))?;
        let now = Utc::now().to_rfc3339();
        self.database.connection().execute(
            "INSERT INTO app_settings (key, value_json, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
            params![SETTINGS_KEY, payload, now],
        )?;
        Ok(GoogleCredentialsStatus {
            client_id: stored.client_id,
            has_secret: !stored.client_secret.is_empty(),
        })
    }

    pub fn clear(&self) -> Result<(), AppError> {
        self.database.connection().execute(
            "DELETE FROM app_settings WHERE key = ?1",
            params![SETTINGS_KEY],
        )?;
        Ok(())
    }
}
