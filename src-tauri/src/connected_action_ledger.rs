//! Durable receipts for approved connected-app mutations. Unknown outcomes are
//! never automatically dispatched again; a new approval is a new action.
use crate::app_error::AppError;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
const PREFIX: &str = "connected_agent.execution.";
const LATEST: &str = "connected_agent.execution_latest.";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Execution {
    pub id: String,
    pub conversation_id: String,
    pub approved_action: Value,
    pub status: String,
    pub outcome: Value,
    pub updated_at: String,
}

pub(crate) fn claim(
    db: &Connection,
    conversation_id: &str,
    pending_key: &str,
    expected: &Value,
) -> Result<Execution, AppError> {
    let tx = db.unchecked_transaction()?;
    let raw: Option<String> = tx
        .query_row(
            "SELECT value_json FROM app_settings WHERE key = ?1",
            [pending_key],
            |r| r.get(0),
        )
        .optional()?;
    let raw = raw.ok_or_else(|| {
        AppError::internal("approval was already consumed; inspect its execution receipt")
    })?;
    let actual: Value =
        serde_json::from_str(&raw).map_err(|e| AppError::internal(e.to_string()))?;
    if &actual != expected {
        return Err(AppError::internal("approval changed before dispatch"));
    }
    let record = Execution {
        id: uuid::Uuid::new_v4().to_string(),
        conversation_id: conversation_id.into(),
        approved_action: actual,
        status: "executing".into(),
        outcome: Value::Null,
        updated_at: Utc::now().to_rfc3339(),
    };
    store(
        &tx,
        &format!("{PREFIX}{}", record.id),
        &serde_json::to_value(&record).map_err(|e| AppError::internal(e.to_string()))?,
    )?;
    store(
        &tx,
        &format!("{LATEST}{conversation_id}"),
        &Value::String(record.id.clone()),
    )?;
    tx.execute("DELETE FROM app_settings WHERE key = ?1", [pending_key])?;
    tx.commit()?;
    Ok(record)
}

fn store(db: &Connection, key: &str, value: &Value) -> Result<(), AppError> {
    db.execute("INSERT INTO app_settings(key,value_json,updated_at) VALUES(?1,?2,?3) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json,updated_at=excluded.updated_at", params![key, value.to_string(), Utc::now().to_rfc3339()])?;
    Ok(())
}

pub(crate) fn finish(
    db: &Connection,
    record: &Execution,
    succeeded: bool,
    outcome: Value,
) -> Result<(), AppError> {
    let mut record = record.clone();
    record.status = if succeeded { "completed" } else { "unknown" }.into();
    record.outcome = outcome;
    record.updated_at = Utc::now().to_rfc3339();
    store(
        db,
        &format!("{PREFIX}{}", record.id),
        &serde_json::to_value(record).map_err(|e| AppError::internal(e.to_string()))?,
    )
}

pub(crate) async fn dispatch(
    execution: &Execution,
    operation: impl std::future::Future<Output = Result<Value, AppError>>,
    cancellation: &tokio_util::sync::CancellationToken,
    save: impl Fn(bool, Value) -> Result<(), AppError>,
) -> Result<Value, AppError> {
    let result = tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(AppError::InferenceCancelled("Connected action cancelled; remote outcome may be unknown".into())),
        result = operation => result,
    };
    match result {
        Ok(value) => {
            save(
                true,
                serde_json::json!({
                    "executionId": execution.id,
                    "remoteId": value.get("id"),
                    "url": value.get("html_url").or_else(|| value.get("url")),
                    "summary": value.to_string().chars().take(12_000).collect::<String>(),
                }),
            )?;
            Ok(value)
        }
        Err(error) => {
            save(
                false,
                Value::String(
                    "Cancelled, remote failure or response lost; verify provider before repeating"
                        .into(),
                ),
            )?;
            Err(error)
        }
    }
}

pub(crate) fn latest(db: &Connection, conversation: &str) -> Result<Option<Execution>, AppError> {
    let raw: Option<String> = db.query_row("SELECT e.value_json FROM app_settings p JOIN app_settings e ON e.key = ?1 || json_extract(p.value_json, '$') WHERE p.key = ?2", params![PREFIX, format!("{LATEST}{conversation}")], |r| r.get(0)).optional()?;
    raw.map(|raw| {
        serde_json::from_str(&raw)
            .map_err(|e| AppError::internal(format!("invalid action receipt: {e}")))
    })
    .transpose()
}

pub(crate) fn recover_interrupted(db: &Connection) -> Result<(), AppError> {
    db.execute("UPDATE app_settings SET value_json=json_set(value_json,'$.status','unknown','$.outcome','Application restarted before an outcome was recorded','$.updatedAt',?1),updated_at=?1 WHERE substr(key,1,length(?2))=?2 AND json_valid(value_json) AND json_extract(value_json,'$.status')='executing'", params![Utc::now().to_rfc3339(), PREFIX])?;
    Ok(())
}

pub(crate) fn explanation(record: &Execution) -> String {
    let provider = record
        .approved_action
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or("connected app");
    if record.status == "completed" {
        format!(
            "Action {} completed. It will not be executed again by Confirm. Recorded receipt: {}",
            record.id, record.outcome
        )
    } else {
        format!("Action {} has an {} outcome. It may already have changed data in {}. Confirm will not repeat it. Check the provider's recent activity or records first. To risk a duplicate, make a new explicit request and approve the new action separately.", record.id, record.status, provider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE app_settings(key TEXT PRIMARY KEY,value_json TEXT NOT NULL,updated_at TEXT NOT NULL);").unwrap();
        db
    }
    fn pending(db: &Connection) -> Value {
        let value = serde_json::json!({"provider":"fake","action":"send","params":{"text":"exact approved text"}});
        store(db, "pending", &value).unwrap();
        value
    }
    #[test]
    fn approved_action_is_claimed_once_and_retained_after_uncertain_commit() {
        let db = database();
        let expected = pending(&db);
        let record = claim(&db, "chat", "pending", &expected).unwrap();
        let mut remote_commits = 0;
        remote_commits += 1; // Fake remote accepts the write, then its response is lost.
        finish(&db, &record, false, Value::String("response lost".into())).unwrap();
        assert!(claim(&db, "chat", "pending", &expected).is_err());
        assert_eq!(remote_commits, 1);
        let recovered = latest(&db, "chat").unwrap().unwrap();
        assert_eq!(recovered.status, "unknown");
        assert_eq!(recovered.approved_action, expected);
        assert!(explanation(&recovered).contains("Check the provider"));
    }
    #[test]
    fn cancellation_restart_and_failure_before_send_never_reapprove() {
        for sent in [false, true] {
            let db = database();
            let expected = pending(&db);
            let record = claim(&db, "chat", "pending", &expected).unwrap();
            let remote_count = usize::from(sent);
            recover_interrupted(&db).unwrap();
            let recovered = latest(&db, "chat").unwrap().unwrap();
            assert_eq!(recovered.id, record.id);
            assert_eq!(recovered.status, "unknown");
            assert!(claim(&db, "chat", "pending", &expected).is_err());
            assert_eq!(remote_count, usize::from(sent));
        }
    }
    #[test]
    fn successful_receipt_and_explicit_new_approval_have_independent_ids() {
        let db = database();
        let expected = pending(&db);
        let first = claim(&db, "chat", "pending", &expected).unwrap();
        finish(
            &db,
            &first,
            true,
            serde_json::json!({"remoteId":"fake-123"}),
        )
        .unwrap();
        assert!(explanation(&latest(&db, "chat").unwrap().unwrap()).contains("fake-123"));
        pending(&db);
        let second = claim(&db, "chat", "pending", &expected).unwrap();
        assert_ne!(first.id, second.id);
        let count: i64 = db
            .query_row(
                "SELECT count(*) FROM app_settings WHERE key LIKE 'connected_agent.execution.%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }
    #[test]
    fn ledger_write_failure_rolls_back_the_approval_claim() {
        let db = database();
        let expected = pending(&db);
        db.execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON app_settings WHEN NEW.key LIKE 'connected_agent.execution.%' BEGIN SELECT RAISE(ABORT,'injected disk failure'); END;").unwrap();
        assert!(claim(&db, "chat", "pending", &expected).is_err());
        assert!(latest(&db, "chat").unwrap().is_none());
        let raw: String = db
            .query_row(
                "SELECT value_json FROM app_settings WHERE key='pending'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&raw).unwrap(), expected);
    }
    #[tokio::test]
    async fn actual_dispatch_handles_pre_send_cancel_and_post_commit_response_loss() {
        use std::cell::Cell;
        for cancel_before_send in [false, true] {
            let db = database();
            let expected = pending(&db);
            let record = claim(&db, "chat", "pending", &expected).unwrap();
            let commits = Cell::new(0);
            let token = tokio_util::sync::CancellationToken::new();
            if cancel_before_send {
                token.cancel();
            }
            let operation = async {
                commits.set(commits.get() + 1);
                Err(AppError::Internal(
                    "fake provider committed, response lost".into(),
                ))
            };
            let result = dispatch(&record, operation, &token, |ok, outcome| {
                finish(&db, &record, ok, outcome)
            })
            .await;
            assert!(result.is_err());
            assert_eq!(commits.get(), usize::from(!cancel_before_send));
            assert_eq!(latest(&db, "chat").unwrap().unwrap().status, "unknown");
            assert!(claim(&db, "chat", "pending", &expected).is_err());
        }
    }
    #[tokio::test]
    async fn actual_dispatch_persists_receipt_before_returning_success() {
        let db = database();
        let expected = pending(&db);
        let record = claim(&db, "chat", "pending", &expected).unwrap();
        dispatch(
            &record,
            async { Ok(serde_json::json!({"id":"remote-123"})) },
            &tokio_util::sync::CancellationToken::new(),
            |ok, outcome| finish(&db, &record, ok, outcome),
        )
        .await
        .unwrap();
        let saved = latest(&db, "chat").unwrap().unwrap();
        assert_eq!(saved.status, "completed");
        assert_eq!(saved.outcome["remoteId"], "remote-123");
    }
    #[tokio::test]
    async fn dropped_dispatch_retains_provenance_across_a_real_database_reopen() {
        use std::sync::{Arc, Mutex};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("actions.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE app_settings(key TEXT PRIMARY KEY,value_json TEXT NOT NULL,updated_at TEXT NOT NULL);").unwrap();
        let expected = pending(&db);
        let record = claim(&db, "chat", "pending", &expected).unwrap();
        let id = record.id.clone();
        let shared = Arc::new(Mutex::new(db));
        let owner = shared.clone();
        let (sent, seen) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            dispatch(
                &record,
                async {
                    sent.send(()).unwrap(); // Fake provider accepted the request.
                    std::future::pending::<Result<Value, AppError>>().await
                },
                &tokio_util::sync::CancellationToken::new(),
                |ok, outcome| finish(&owner.lock().unwrap(), &record, ok, outcome),
            )
            .await
        });
        seen.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        drop(shared);
        let reopened = Connection::open(path).unwrap();
        recover_interrupted(&reopened).unwrap();
        let saved = latest(&reopened, "chat").unwrap().unwrap();
        assert_eq!(saved.id, id);
        assert_eq!(saved.status, "unknown");
        assert_eq!(saved.approved_action, expected);
        assert!(claim(&reopened, "chat", "pending", &expected).is_err());
    }
}
