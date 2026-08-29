use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use uuid::Uuid;

use crate::{app_error::AppError, database::Database};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
    pub pinned: bool,
    pub active_model_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub status: String,
    pub model_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

const PROFILE_CONTEXT_MARKER: &str = "[open-mind-ai-profile-context]";

pub struct ChatRepository<'a> {
    database: &'a Database,
}

impl<'a> ChatRepository<'a> {
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    pub fn list_conversations(&self) -> Result<Vec<Conversation>, AppError> {
        let mut statement = self.database.connection().prepare(
            "SELECT id, title, created_at, updated_at, archived_at, pinned, active_model_id
             FROM conversations
             WHERE archived_at IS NULL
             ORDER BY pinned DESC, updated_at DESC",
        )?;
        let rows = statement.query_map([], map_conversation)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    pub fn create_conversation(&self, title: Option<&str>) -> Result<Conversation, AppError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let title = title
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("New conversation");
        self.database.connection().execute(
            "INSERT INTO conversations (id, title, created_at, updated_at, pinned)
             VALUES (?1, ?2, ?3, ?4, 0)",
            params![id, title, now, now],
        )?;
        self.find_conversation(&id)
    }

    pub fn find_conversation(&self, id: &str) -> Result<Conversation, AppError> {
        self.database
            .connection()
            .query_row(
                "SELECT id, title, created_at, updated_at, archived_at, pinned, active_model_id
             FROM conversations WHERE id = ?1",
                params![id],
                map_conversation,
            )
            .map_err(AppError::from)
    }

    pub fn rename_conversation(&self, id: &str, title: &str) -> Result<(), AppError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::internal("conversation title cannot be empty"));
        }
        self.touch_conversation_with(
            id,
            "UPDATE conversations SET title = ?1, updated_at = ?2 WHERE id = ?3",
            params![title, Utc::now().to_rfc3339(), id],
        )
    }

    pub fn set_pinned(&self, id: &str, pinned: bool) -> Result<(), AppError> {
        self.touch_conversation_with(
            id,
            "UPDATE conversations SET pinned = ?1, updated_at = ?2 WHERE id = ?3",
            params![pinned as i64, Utc::now().to_rfc3339(), id],
        )
    }

    pub fn set_active_model(&self, id: &str, model_id: Option<&str>) -> Result<(), AppError> {
        self.touch_conversation_with(
            id,
            "UPDATE conversations SET active_model_id = ?1, updated_at = ?2 WHERE id = ?3",
            params![model_id, Utc::now().to_rfc3339(), id],
        )
    }

    pub fn archive_conversation(&self, id: &str) -> Result<(), AppError> {
        let now = Utc::now().to_rfc3339();
        self.touch_conversation_with(
            id,
            "UPDATE conversations SET archived_at = ?1, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )
    }

    pub fn delete_conversation(&self, id: &str) -> Result<(), AppError> {
        let changed = self
            .database
            .connection()
            .execute("DELETE FROM conversations WHERE id = ?1", params![id])?;
        if changed == 0 {
            return Err(AppError::internal(format!(
                "conversation not found: {id}"
            )));
        }
        Ok(())
    }

    pub fn list_messages(&self, conversation_id: &str) -> Result<Vec<Message>, AppError> {
        let mut statement = self.database.connection().prepare(
            "SELECT id, conversation_id, role, content, status, model_id, created_at, updated_at
             FROM messages
             WHERE conversation_id = ?1
             ORDER BY created_at ASC",
        )?;
        let rows = statement.query_map(params![conversation_id], map_message)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    pub fn add_message(
        &self,
        conversation_id: &str,
        role: &str,
        content: &str,
        status: &str,
        model_id: Option<&str>,
    ) -> Result<Message, AppError> {
        validate_message_role(role)?;
        validate_message_status(status)?;

        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        self.database.connection().execute(
            "INSERT INTO messages (id, conversation_id, role, content, status, model_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, conversation_id, role, content, status, model_id, now, now],
        )?;
        self.database.connection().execute(
            "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
            params![now, conversation_id],
        )?;
        self.find_message(&id)
    }

    pub fn upsert_profile_context(
        &self,
        conversation_id: &str,
        content: Option<&str>,
    ) -> Result<(), AppError> {
        let existing_id: Option<String> = self
            .database
            .connection()
            .query_row(
                "SELECT id FROM messages
                 WHERE conversation_id = ?1 AND role = 'system' AND content LIKE ?2
                 LIMIT 1",
                params![conversation_id, format!("{PROFILE_CONTEXT_MARKER}%")],
                |row| row.get(0),
            )
            .optional()?;

        match (
            existing_id,
            content.filter(|value| !value.trim().is_empty()),
        ) {
            (Some(id), Some(content)) => {
                self.database.connection().execute(
                    "UPDATE messages SET content = ?1, status = 'completed', updated_at = ?2 WHERE id = ?3",
                    params![format!("{PROFILE_CONTEXT_MARKER}\n{content}"), Utc::now().to_rfc3339(), id],
                )?;
            }
            (None, Some(content)) => {
                self.add_message(
                    conversation_id,
                    "system",
                    &format!("{PROFILE_CONTEXT_MARKER}\n{content}"),
                    "completed",
                    None,
                )?;
            }
            (Some(id), None) => {
                self.database
                    .connection()
                    .execute("DELETE FROM messages WHERE id = ?1", params![id])?;
            }
            (None, None) => {}
        }
        Ok(())
    }

    pub fn append_message_chunk(&self, message_id: &str, chunk: &str) -> Result<(), AppError> {
        let now = Utc::now().to_rfc3339();
        let changed = self.database.connection().execute(
            "UPDATE messages SET content = content || ?1, updated_at = ?2 WHERE id = ?3",
            params![chunk, now, message_id],
        )?;
        if changed == 0 {
            return Err(AppError::internal(format!(
                "message not found: {message_id}"
            )));
        }
        Ok(())
    }

    pub fn set_message_status(&self, message_id: &str, status: &str) -> Result<(), AppError> {
        validate_message_status(status)?;
        let changed = self.database.connection().execute(
            "UPDATE messages SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status, Utc::now().to_rfc3339(), message_id],
        )?;
        if changed == 0 {
            return Err(AppError::internal(format!(
                "message not found: {message_id}"
            )));
        }
        Ok(())
    }

    pub fn delete_message(&self, message_id: &str) -> Result<(), AppError> {
        let changed = self
            .database
            .connection()
            .execute("DELETE FROM messages WHERE id = ?1", params![message_id])?;
        if changed == 0 {
            return Err(AppError::internal(format!(
                "message not found: {message_id}"
            )));
        }
        Ok(())
    }

    fn find_message(&self, id: &str) -> Result<Message, AppError> {
        self.database
            .connection()
            .query_row(
                "SELECT id, conversation_id, role, content, status, model_id, created_at, updated_at
             FROM messages WHERE id = ?1",
                params![id],
                map_message,
            )
            .map_err(AppError::from)
    }

    fn touch_conversation_with<P: rusqlite::Params>(
        &self,
        id: &str,
        sql: &str,
        params: P,
    ) -> Result<(), AppError> {
        let changed = self.database.connection().execute(sql, params)?;
        if changed == 0 {
            return Err(AppError::internal(format!("conversation not found: {id}")));
        }
        Ok(())
    }
}

fn validate_message_role(role: &str) -> Result<(), AppError> {
    if matches!(role, "system" | "user" | "assistant" | "tool") {
        Ok(())
    } else {
        Err(AppError::internal("invalid message role"))
    }
}

fn validate_message_status(status: &str) -> Result<(), AppError> {
    if matches!(
        status,
        "completed" | "cancelled" | "failed" | "streaming" | "pending"
    ) {
        Ok(())
    } else {
        Err(AppError::internal("invalid message status"))
    }
}

fn map_conversation(row: &rusqlite::Row<'_>) -> rusqlite::Result<Conversation> {
    Ok(Conversation {
        id: row.get(0)?,
        title: row.get(1)?,
        created_at: row.get(2)?,
        updated_at: row.get(3)?,
        archived_at: row.get(4)?,
        pinned: row.get::<_, i64>(5)? != 0,
        active_model_id: row.get(6)?,
    })
}

fn map_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<Message> {
    Ok(Message {
        id: row.get(0)?,
        conversation_id: row.get(1)?,
        role: row.get(2)?,
        content: row.get(3)?,
        status: row.get(4)?,
        model_id: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_conversation_and_messages() {
        let database = Database::in_memory().unwrap();
        let repo = ChatRepository::new(&database);
        let conversation = repo.create_conversation(Some("Test")).unwrap();
        repo.add_message(&conversation.id, "user", "hello", "completed", None)
            .unwrap();
        let assistant = repo
            .add_message(&conversation.id, "assistant", "", "streaming", None)
            .unwrap();
        repo.append_message_chunk(&assistant.id, "world").unwrap();
        repo.set_message_status(&assistant.id, "cancelled").unwrap();
        let messages = repo.list_messages(&conversation.id).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].content, "world");
        assert_eq!(messages[1].status, "cancelled");
    }

    #[test]
    fn supports_all_streaming_statuses() {
        let database = Database::in_memory().unwrap();
        let repo = ChatRepository::new(&database);
        let conversation = repo.create_conversation(Some("Statuses")).unwrap();

        for status in ["pending", "streaming", "completed", "cancelled", "failed"] {
            let message = repo
                .add_message(&conversation.id, "assistant", "partial", status, None)
                .unwrap();
            assert_eq!(message.status, status);
        }
    }

    #[test]
    fn rejects_invalid_message_role_and_status() {
        let database = Database::in_memory().unwrap();
        let repo = ChatRepository::new(&database);
        let conversation = repo.create_conversation(Some("Validation")).unwrap();

        assert!(matches!(
            repo.add_message(&conversation.id, "invalid", "hello", "completed", None)
                .unwrap_err(),
            AppError::Internal(_)
        ));
        assert!(matches!(
            repo.add_message(&conversation.id, "user", "hello", "unknown", None)
                .unwrap_err(),
            AppError::Internal(_)
        ));
    }

    #[test]
    fn reloads_history_from_file_database() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("openmind_ai.db");
        let conversation_id;

        {
            let database = Database::open(path.clone()).unwrap();
            let repo = ChatRepository::new(&database);
            let conversation = repo.create_conversation(Some("Restart")).unwrap();
            conversation_id = conversation.id.clone();
            repo.add_message(&conversation.id, "user", "hello", "completed", None)
                .unwrap();
            repo.add_message(&conversation.id, "assistant", "saved", "failed", None)
                .unwrap();
        }

        let database = Database::open(path).unwrap();
        let repo = ChatRepository::new(&database);
        let conversations = repo.list_conversations().unwrap();
        let messages = repo.list_messages(&conversation_id).unwrap();

        assert_eq!(conversations.len(), 1);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].content, "saved");
        assert_eq!(messages[1].status, "failed");
    }

    #[test]
    fn sets_and_clears_active_model() {
        let database = Database::in_memory().unwrap();
        let repo = ChatRepository::new(&database);
        let conversation = repo.create_conversation(Some("Model binding")).unwrap();
        assert!(conversation.active_model_id.is_none());

        database
            .connection()
            .execute(
                "INSERT INTO model_registry
                 (id, name, path, format, size_bytes, capabilities, enabled, created_at, updated_at)
                 VALUES ('model-1', 'Test Model', 'models/llm/test.gguf', 'gguf', 0, '[]', 1, 'now', 'now')",
                [],
            )
            .unwrap();

        repo.set_active_model(&conversation.id, Some("model-1"))
            .unwrap();
        let updated = repo.find_conversation(&conversation.id).unwrap();
        assert_eq!(updated.active_model_id.as_deref(), Some("model-1"));

        repo.set_active_model(&conversation.id, None).unwrap();
        let cleared = repo.find_conversation(&conversation.id).unwrap();
        assert!(cleared.active_model_id.is_none());
    }

    #[test]
    fn missing_message_mutations_fail() {
        let database = Database::in_memory().unwrap();
        let repo = ChatRepository::new(&database);

        assert!(matches!(
            repo.append_message_chunk("missing", "chunk").unwrap_err(),
            AppError::Internal(_)
        ));
        assert!(matches!(
            repo.set_message_status("missing", "failed").unwrap_err(),
            AppError::Internal(_)
        ));
    }

    #[test]
    fn delete_message_removes_row() {
        let database = Database::in_memory().unwrap();
        let repo = ChatRepository::new(&database);
        let conversation = repo.create_conversation(Some("Delete")).unwrap();
        let message = repo
            .add_message(&conversation.id, "assistant", "stale", "failed", None)
            .unwrap();

        repo.delete_message(&message.id).unwrap();
        assert!(repo.list_messages(&conversation.id).unwrap().is_empty());
        assert!(matches!(
            repo.delete_message(&message.id).unwrap_err(),
            AppError::Internal(_)
        ));
    }

    #[test]
    fn supports_conversation_operations() {
        let database = Database::in_memory().unwrap();
        let repo = ChatRepository::new(&database);
        let conversation = repo.create_conversation(Some("Ops")).unwrap();

        repo.rename_conversation(&conversation.id, "Renamed")
            .unwrap();
        repo.set_pinned(&conversation.id, true).unwrap();
        let pinned = repo.find_conversation(&conversation.id).unwrap();
        assert_eq!(pinned.title, "Renamed");
        assert!(pinned.pinned);

        repo.archive_conversation(&conversation.id).unwrap();
        assert!(repo.list_conversations().unwrap().is_empty());

        repo.delete_conversation(&conversation.id).unwrap();
        assert!(repo.list_messages(&conversation.id).unwrap().is_empty());
    }

    #[test]
    fn missing_conversation_mutations_fail() {
        let database = Database::in_memory().unwrap();
        let repo = ChatRepository::new(&database);

        assert!(matches!(
            repo.archive_conversation("missing").unwrap_err(),
            AppError::Internal(_)
        ));
        assert!(matches!(
            repo.delete_conversation("missing").unwrap_err(),
            AppError::Internal(_)
        ));
    }
}
