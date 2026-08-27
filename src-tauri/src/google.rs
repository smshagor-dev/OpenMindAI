use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::github::secret_store;
use crate::{app_error::AppError, database::Database};

const SETTINGS_KEY: &str = "app.google";
const SECRET_SLOT: &str = "google-client-secret";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredGoogleCredentials {
    client_id: String,
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
        let Some((client_id, legacy_secret)) = self.get_stored()? else {
            return Ok(None);
        };

        if let Some(secret) = legacy_secret {
            self.migrate_legacy(&client_id, &secret)?;
        }

        Ok(Some(GoogleCredentialsStatus {
            client_id,
            has_secret: secret_store::get_secret(SECRET_SLOT)?.is_some(),
        }))
    }

    fn get_stored(&self) -> Result<Option<(String, Option<String>)>, AppError> {
        let value: Option<String> = self
            .database
            .connection()
            .query_row(
                "SELECT value_json FROM app_settings WHERE key = ?1",
                params![SETTINGS_KEY],
                |row| row.get(0),
            )
            .optional()?;

        let Some(value) = value else {
            return Ok(None);
        };
        let payload: serde_json::Value = serde_json::from_str(&value)
            .map_err(|error| AppError::internal(format!("invalid Google settings: {error}")))?;
        let client_id = payload
            .get("clientId")
            .or_else(|| payload.get("client_id"))
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| AppError::internal("Google settings are missing a client ID"))?
            .to_string();
        let legacy_secret = payload
            .get("clientSecret")
            .or_else(|| payload.get("client_secret"))
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        Ok(Some((client_id, legacy_secret)))
    }

    fn persist_client_id(&self, client_id: &str) -> Result<(), AppError> {
        let payload = serde_json::to_string(&StoredGoogleCredentials {
            client_id: client_id.to_string(),
        })
        .map_err(|error| AppError::internal(error.to_string()))?;
        let now = Utc::now().to_rfc3339();
        self.database.connection().execute(
            "INSERT INTO app_settings (key, value_json, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
            params![SETTINGS_KEY, payload, now],
        )?;
        Ok(())
    }

    fn migrate_legacy(&self, client_id: &str, client_secret: &str) -> Result<(), AppError> {
        secret_store::set_secret(SECRET_SLOT, client_secret)?;
        self.persist_client_id(client_id)?;
        tracing::info!("migrated Google client secret from SQLite to the OS credential store");
        Ok(())
    }

    pub fn save(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> Result<GoogleCredentialsStatus, AppError> {
        let client_id = client_id.trim();
        if client_id.is_empty() {
            return Err(AppError::internal("Google client ID cannot be empty"));
        }

        if client_secret.is_empty() {
            secret_store::delete_secret(SECRET_SLOT)?;
        } else {
            secret_store::set_secret(SECRET_SLOT, client_secret)?;
        }
        self.persist_client_id(client_id)?;

        Ok(GoogleCredentialsStatus {
            client_id: client_id.to_string(),
            has_secret: !client_secret.is_empty(),
        })
    }

    pub fn clear(&self) -> Result<(), AppError> {
        secret_store::delete_secret(SECRET_SLOT)?;
        self.database.connection().execute(
            "DELETE FROM app_settings WHERE key = ?1",
            params![SETTINGS_KEY],
        )?;
        Ok(())
    }
}
