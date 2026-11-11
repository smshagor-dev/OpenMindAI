//! Durable, account-local Studio outbox. A potentially successful POST is never
//! silently submitted twice after an interrupted connection.
use crate::app_error::AppError;
use crate::AppState;
use chrono::Utc;
use rusqlite::params;
use serde::Serialize;
use serde_json::Value;
use tauri::State;
use uuid::Uuid;

const MAX_QUEUE_ITEMS: i64 = 100;
const MAX_PAYLOAD_BYTES: usize = 20_000;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StudioQueuedRequest {
    id: String,
    kind: String,
    request_key: String,
    payload: Value,
    status: String,
    note: Option<String>,
    created_at: String,
}

fn queue_error(message: &str) -> AppError {
    AppError::internal(format!("Studio outbox: {message}"))
}

fn validate(kind: &str, payload: &Value) -> Result<(), AppError> {
    match kind {
        "scene3d" => {
            let prompt = payload.get("prompt").and_then(Value::as_str).unwrap_or("");
            let mode = payload.get("mode").and_then(Value::as_str).unwrap_or("");
            if prompt.len() < 5
                || prompt.len() > 8_000
                || !matches!(mode, "object" | "architecture")
            {
                return Err(queue_error("Invalid 3D scene request"));
            }
        }
        _ => return Err(queue_error("Unsupported queued task")),
    }
    Ok(())
}

fn ensure_table(db: &crate::database::Database) -> Result<(), AppError> {
    db.connection().execute_batch(
        "CREATE TABLE IF NOT EXISTS studio_outbox (
          id TEXT PRIMARY KEY NOT NULL,
          kind TEXT NOT NULL,
          request_key TEXT NOT NULL UNIQUE,
          payload_json TEXT NOT NULL,
          status TEXT NOT NULL CHECK(status IN ('queued','sending','review','completed')),
          note TEXT,
          created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_studio_outbox_status ON studio_outbox(status, created_at);",
    )?;
    Ok(())
}

fn read_rows(db: &crate::database::Database) -> Result<Vec<StudioQueuedRequest>, AppError> {
    let mut statement = db.connection().prepare(
        "SELECT id, kind, request_key, payload_json, status, note, created_at
         FROM studio_outbox WHERE status <> 'completed'
         ORDER BY created_at, id LIMIT 100",
    )?;
    let records = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;
    let mut list = Vec::new();
    for record in records {
        let (id, kind, request_key, raw, status, note, created_at) = record?;
        let payload =
            serde_json::from_str(&raw).map_err(|_| queue_error("Stored payload is invalid"))?;
        list.push(StudioQueuedRequest {
            id,
            kind,
            request_key,
            payload,
            status,
            note,
            created_at,
        });
    }
    Ok(list)
}

#[tauri::command]
pub(crate) fn studio_outbox_list(
    state: State<'_, AppState>,
) -> Result<Vec<StudioQueuedRequest>, AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| queue_error("Database unavailable"))?;
    ensure_table(&db)?;
    read_rows(&db)
}

#[tauri::command]
pub(crate) fn studio_outbox_enqueue(
    kind: String,
    request_key: String,
    payload: Value,
    state: State<'_, AppState>,
) -> Result<Vec<StudioQueuedRequest>, AppError> {
    validate(&kind, &payload)?;
    Uuid::parse_str(&request_key).map_err(|_| queue_error("Invalid idempotency key"))?;
    let encoded =
        serde_json::to_string(&payload).map_err(|_| queue_error("Invalid JSON payload"))?;
    if encoded.len() > MAX_PAYLOAD_BYTES {
        return Err(queue_error("Request exceeds the 20 KB outbox limit"));
    }
    let db = state
        .database
        .lock()
        .map_err(|_| queue_error("Database unavailable"))?;
    ensure_table(&db)?;
    let pending: i64 = db.connection().query_row(
        "SELECT COUNT(*) FROM studio_outbox WHERE status <> 'completed'",
        [],
        |row| row.get(0),
    )?;
    if pending >= MAX_QUEUE_ITEMS {
        return Err(queue_error("Outbox is full; review pending requests"));
    }
    db.connection().execute(
        "INSERT OR IGNORE INTO studio_outbox
         (id, kind, request_key, payload_json, status, note, created_at)
         VALUES (?1, ?2, ?3, ?4, 'queued', NULL, ?5)",
        params![
            Uuid::new_v4().to_string(),
            kind,
            request_key,
            encoded,
            Utc::now().to_rfc3339()
        ],
    )?;
    read_rows(&db)
}

/// App startup only: unknown outcomes from a previous process require review.
#[tauri::command]
pub(crate) fn studio_outbox_recover(state: State<'_, AppState>) -> Result<(), AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| queue_error("Database unavailable"))?;
    ensure_table(&db)?;
    db.connection().execute(
        "UPDATE studio_outbox SET status = 'review',
         note = 'The app exited while submitting this request. Verify the remote job before retrying.'
         WHERE status = 'sending'",
        [],
    )?;
    Ok(())
}

#[tauri::command]
pub(crate) fn studio_outbox_claim(
    id: String,
    state: State<'_, AppState>,
) -> Result<bool, AppError> {
    Uuid::parse_str(&id).map_err(|_| queue_error("Invalid queued request id"))?;
    let db = state
        .database
        .lock()
        .map_err(|_| queue_error("Database unavailable"))?;
    ensure_table(&db)?;
    Ok(db.connection().execute(
        "UPDATE studio_outbox SET status = 'sending', note = NULL WHERE id = ?1 AND status = 'queued'",
        params![id],
    )? == 1)
}

#[tauri::command]
pub(crate) fn studio_outbox_finish(
    id: String,
    status: String,
    note: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    Uuid::parse_str(&id).map_err(|_| queue_error("Invalid queued request id"))?;
    if !matches!(status.as_str(), "completed" | "review") {
        return Err(queue_error("Invalid outbox completion status"));
    }
    let note = note
        .unwrap_or_default()
        .chars()
        .take(300)
        .collect::<String>();
    let db = state
        .database
        .lock()
        .map_err(|_| queue_error("Database unavailable"))?;
    ensure_table(&db)?;
    if db.connection().execute(
        "UPDATE studio_outbox SET status = ?1, note = ?2 WHERE id = ?3 AND status = 'sending'",
        params![status, note, id],
    )? == 0
    {
        return Err(queue_error("Request was not claimed or already completed"));
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn studio_outbox_retry(id: String, state: State<'_, AppState>) -> Result<(), AppError> {
    Uuid::parse_str(&id).map_err(|_| queue_error("Invalid queued request id"))?;
    let db = state
        .database
        .lock()
        .map_err(|_| queue_error("Database unavailable"))?;
    ensure_table(&db)?;
    db.connection().execute(
        "UPDATE studio_outbox SET status = 'queued'
         WHERE id = ?1 AND kind = 'scene3d' AND status = 'review'",
        params![id],
    )?;
    // Cloud 3D idempotency is preserved by the unchanged request_key.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_task_kind_is_bounded_and_does_not_accept_arbitrary_commands() {
        assert!(validate(
            "scene3d",
            &serde_json::json!({"prompt":"A ceramic sphere","mode":"object"})
        )
        .is_ok());
        assert!(validate("command", &serde_json::json!({"shell":"rm -rf /"})).is_err());
        assert!(validate(
            "scene3d",
            &serde_json::json!({"prompt":"short","mode":"admin"})
        )
        .is_err());
    }
}
