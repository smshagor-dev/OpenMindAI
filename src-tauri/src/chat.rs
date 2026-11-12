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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkedConversation {
    pub conversation: Conversation,
    pub messages: Vec<Message>,
}

/// Bounded server-side results; no message bodies are ever sent to search UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSearchPage {
    pub conversations: Vec<Conversation>,
    pub next_cursor: Option<String>,
}

const PROFILE_CONTEXT_MARKER: &str = "[open-mind-ai-profile-context]";
const PROJECT_CONTEXT_MARKER: &str = "[open-mind-ai-project-context]";

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

    /// Searches chat titles and indexed local message text without materializing
    /// every conversation history in the renderer. Result pages have a stable
    /// (updated_at, id) ordering; the query is never interpolated into SQL.
    pub fn search_conversations(
        &self,
        query: &str,
        cursor: Option<&str>,
        requested_limit: i64,
    ) -> Result<ChatSearchPage, AppError> {
        let query = query.trim();
        if query.chars().count() > 256 {
            return Err(AppError::internal("chat search query exceeds 256 characters"));
        }
        let limit = requested_limit.clamp(1, 40);
        let cursor = match cursor {
            Some(raw) => {
                let (updated_at, id) = raw
                    .split_once('|')
                    .ok_or_else(|| AppError::internal("invalid chat search cursor"))?;
                if raw.len() > 128
                    || chrono::DateTime::parse_from_rfc3339(updated_at).is_err()
                    || Uuid::parse_str(id).is_err()
                {
                    return Err(AppError::internal("invalid chat search cursor"));
                }
                Some((updated_at, id))
            }
            None => None,
        };

        // FTS5 trigram queries are exact quoted phrases. Even user-supplied
        // quotes and wildcard characters are literal text, not FTS operators.
        // Queries below three Unicode characters use a bounded-result SQLite
        // substring fallback because trigram indexes cannot match 1-2 chars.
        let search_condition = if query.is_empty() {
            "1 = 1"
        } else if query.chars().count() < 3 {
            "instr(lower(c.title), lower(?1)) > 0
             OR EXISTS (
                SELECT 1 FROM messages m
                WHERE m.conversation_id = c.id AND m.role != 'system'
                  AND instr(lower(m.content), lower(?1)) > 0
             )"
        } else {
            "instr(lower(c.title), lower(?1)) > 0
             OR c.id IN (
                SELECT m.conversation_id
                FROM chat_message_fts
                JOIN messages m ON m.rowid = chat_message_fts.rowid
                WHERE chat_message_fts MATCH ?2 AND m.role != 'system'
             )"
        };
        let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
        let (cursor_time, cursor_id) = match cursor {
            Some((time, id)) => (Some(time), Some(id)),
            None => (None, None),
        };
        let sql = format!(
            "SELECT c.id, c.title, c.created_at, c.updated_at, c.archived_at,
                    c.pinned, c.active_model_id
             FROM conversations c
             WHERE c.archived_at IS NULL
               AND ({search_condition})
               AND (?3 IS NULL OR c.updated_at < ?3
                    OR (c.updated_at = ?3 AND c.id < ?4))
             ORDER BY c.updated_at DESC, c.id DESC
             LIMIT ?5"
        );
        let mut statement = self.database.connection().prepare(&sql)?;
        let rows = statement.query_map(
            params![query, fts_query, cursor_time, cursor_id, limit + 1],
            map_conversation,
        )?;
        let mut conversations = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)?;
        let has_more = conversations.len() > limit as usize;
        conversations.truncate(limit as usize);
        let next_cursor = if has_more {
            conversations
                .last()
                .map(|chat| format!("{}|{}", chat.updated_at, chat.id))
        } else {
            None
        };
        Ok(ChatSearchPage {
            conversations,
            next_cursor,
        })
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

    /// Fork the text context before a user turn without changing any source row.
    /// Generated files/media remain owned by the source conversation, rather
    /// than sharing artifact paths whose deletion could affect both chats.
    pub fn fork_for_edit(
        &self,
        conversation_id: &str,
        message_id: &str,
    ) -> Result<ForkedConversation, AppError> {
        let transaction = self.database.connection().unchecked_transaction()?;
        let source = self.find_conversation(conversation_id)?;
        let target: Option<(String, String, i64)> = transaction.query_row(
            "SELECT role, created_at, rowid FROM messages WHERE id = ?1 AND conversation_id = ?2",
            params![message_id, conversation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional()?;
        let Some((role, target_created_at, target_rowid)) = target else {
            return Err(AppError::internal(
                "the edited message does not belong to this conversation",
            ));
        };
        if role != "user" {
            return Err(AppError::internal("only user messages can be edited"));
        }
        let busy: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE conversation_id = ?1 AND status IN ('pending', 'streaming'))",
            params![conversation_id],
            |row| row.get(0),
        )?;
        if busy {
            return Err(AppError::internal(
                "stop the active generation before editing this conversation",
            ));
        }
        let prefix = {
            let mut statement = transaction.prepare(
                "SELECT id, conversation_id, role, content, status, model_id, created_at, updated_at
                 FROM messages WHERE conversation_id = ?1
                   AND (role = 'system' OR created_at < ?2 OR (created_at = ?2 AND rowid < ?3))
                 ORDER BY created_at ASC, rowid ASC",
            )?;
            let rows = statement.query_map(
                params![conversation_id, target_created_at, target_rowid],
                map_message,
            )?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let branch_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        transaction.execute(
            "INSERT INTO conversations (id, title, created_at, updated_at, pinned, active_model_id)
             VALUES (?1, ?2, ?3, ?3, 0, ?4)",
            params![
                branch_id,
                format!("{} (edited)", source.title),
                now,
                source.active_model_id
            ],
        )?;
        for message in prefix {
            transaction.execute(
                "INSERT INTO messages (id, conversation_id, role, content, status, model_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![Uuid::new_v4().to_string(), branch_id, message.role, message.content,
                    message.status, message.model_id, message.created_at, message.updated_at],
            )?;
        }
        transaction.execute(
            "INSERT INTO conversation_settings (conversation_id, settings_json, updated_at)
             SELECT ?1, settings_json, ?2 FROM conversation_settings WHERE conversation_id = ?3",
            params![branch_id, now, conversation_id],
        )?;
        transaction.execute(
            "INSERT INTO project_conversations (project_id, conversation_id, created_at)
             SELECT project_id, ?1, ?2 FROM project_conversations WHERE conversation_id = ?3",
            params![branch_id, now, conversation_id],
        )?;
        let fork = ForkedConversation {
            conversation: self.find_conversation(&branch_id)?,
            messages: self.list_messages(&branch_id)?,
        };
        transaction.commit()?;
        Ok(fork)
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
            return Err(AppError::internal(format!("conversation not found: {id}")));
        }
        Ok(())
    }

    pub fn list_messages(&self, conversation_id: &str) -> Result<Vec<Message>, AppError> {
        let mut statement = self.database.connection().prepare(
            "SELECT id, conversation_id, role, content, status, model_id, created_at, updated_at
             FROM messages
             WHERE conversation_id = ?1
             ORDER BY created_at ASC, rowid ASC",
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

    /// Create a separate streaming candidate without changing the source reply.
    pub fn begin_regeneration(
        &self,
        conversation_id: &str,
        target_id: &str,
        model_id: Option<&str>,
    ) -> Result<Message, AppError> {
        let tx = self.database.connection().unchecked_transaction()?;
        let target = self.find_message(target_id)?;
        if target.conversation_id != conversation_id || target.role != "assistant" {
            return Err(AppError::internal(
                "regeneration target must be an assistant in this conversation",
            ));
        }
        let candidate =
            self.add_message(conversation_id, "assistant", "", "streaming", model_id)?;
        tx.execute(
            "UPDATE messages SET regenerates_message_id = ?1 WHERE id = ?2",
            params![target_id, candidate.id],
        )?;
        let candidate = self.find_message(&candidate.id)?;
        tx.commit()?;
        Ok(candidate)
    }

    /// Exclude the reply being regenerated and later turns from model context,
    /// while keeping system instructions and all durable history intact.
    pub fn history_before_regeneration(
        &self,
        conversation_id: &str,
        target_id: &str,
    ) -> Result<Vec<Message>, AppError> {
        let messages = self.list_messages(conversation_id)?;
        let index = messages
            .iter()
            .position(|message| message.id == target_id && message.role == "assistant")
            .ok_or_else(|| {
                AppError::internal("assistant message not found in this conversation")
            })?;
        Ok(messages
            .into_iter()
            .enumerate()
            .filter(|(position, message)| *position < index || message.role == "system")
            .map(|(_, message)| message)
            .collect())
    }

    pub fn messages_for_generation(
        &self,
        conversation_id: &str,
        assistant_id: &str,
    ) -> Result<Vec<Message>, AppError> {
        let target: Option<String> = self.database.connection().query_row(
            "SELECT regenerates_message_id FROM messages WHERE id = ?1 AND conversation_id = ?2 AND role = 'assistant'",
            params![assistant_id, conversation_id], |row| row.get(0),
        )?;
        match target {
            Some(target) => self.history_before_regeneration(conversation_id, &target),
            None => self.list_messages(conversation_id),
        }
    }

    pub fn upsert_profile_context(
        &self,
        conversation_id: &str,
        content: Option<&str>,
    ) -> Result<(), AppError> {
        // This method is also used by the project-context sync path. Keep the
        // two system slots independent so refreshing project instructions can
        // never overwrite (or delete) the user's profile/custom instructions.
        // UserProfile::to_system_context() always returns Some; None therefore
        // means "remove project context" for the sync path.
        let marker = match content {
            Some(value) if value.trim_start().starts_with(PROJECT_CONTEXT_MARKER) => {
                PROJECT_CONTEXT_MARKER
            }
            Some(_) => PROFILE_CONTEXT_MARKER,
            None => PROJECT_CONTEXT_MARKER,
        };
        let existing_id: Option<String> = self
            .database
            .connection()
            .query_row(
                "SELECT id FROM messages
                 WHERE conversation_id = ?1 AND role = 'system' AND content LIKE ?2
                 LIMIT 1",
                params![conversation_id, format!("{marker}%")],
                |row| row.get(0),
            )
            .optional()?;

        match (
            existing_id,
            content.filter(|value| !value.trim().is_empty()),
        ) {
            (Some(id), Some(content)) => {
                let stored = if content.trim_start().starts_with(marker) {
                    content.to_string()
                } else {
                    format!("{marker}\n{content}")
                };
                self.database.connection().execute(
                    "UPDATE messages SET content = ?1, status = 'completed', updated_at = ?2 WHERE id = ?3",
                    params![stored, Utc::now().to_rfc3339(), id],
                )?;
            }
            (None, Some(content)) => {
                let stored = if content.trim_start().starts_with(marker) {
                    content.to_string()
                } else {
                    format!("{marker}\n{content}")
                };
                self.add_message(conversation_id, "system", &stored, "completed", None)?;
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
        // Publishing completion and retiring the source reply are one SQLite
        // transaction. Any write/constraint failure rolls both operations back.
        let tx = self.database.connection().unchecked_transaction()?;
        let changed = tx.execute(
            "UPDATE messages SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status, Utc::now().to_rfc3339(), message_id],
        )?;
        if changed == 0 {
            return Err(AppError::internal(format!(
                "message not found: {message_id}"
            )));
        }
        if status == "completed" {
            tx.execute(
                "DELETE FROM messages WHERE id = (
                    SELECT regenerates_message_id FROM messages
                    WHERE id = ?1 AND length(trim(content)) > 0
                 ) AND role = 'assistant' AND conversation_id = (
                    SELECT conversation_id FROM messages WHERE id = ?1
                 )",
                params![message_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_message(&self, message_id: &str) -> Result<(), AppError> {
        let target: Option<(String, String, i64)> = self
            .database
            .connection()
            .query_row(
                "SELECT conversation_id, role, rowid FROM messages WHERE id = ?1",
                params![message_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((conversation_id, role, row_id)) = target else {
            return Err(AppError::internal(format!(
                "message not found: {message_id}"
            )));
        };

        // Explicitly deleting a user turn truncates the following branch. Editing
        // uses fork_for_edit and never invokes this destructive operation. A single
        // SQL statement removes that turn and every later non-system message,
        // so the conversation cannot be left half-truncated if a process or
        // request fails between individual deletes. Assistant deletion remains
        // single-row behavior for regenerate/retry flows.
        let changed = if role == "user" {
            self.database.connection().execute(
                "DELETE FROM messages
                 WHERE conversation_id = ?1 AND rowid >= ?2 AND role != 'system'",
                params![conversation_id, row_id],
            )?
        } else {
            self.database
                .connection()
                .execute("DELETE FROM messages WHERE id = ?1", params![message_id])?
        };
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
    fn deleting_user_turn_atomically_truncates_following_branch() {
        let database = Database::in_memory().unwrap();
        let repo = ChatRepository::new(&database);
        let conversation = repo.create_conversation(Some("Branch edit")).unwrap();
        repo.upsert_profile_context(&conversation.id, Some("profile context"))
            .unwrap();
        let first_user = repo
            .add_message(&conversation.id, "user", "first", "completed", None)
            .unwrap();
        repo.add_message(&conversation.id, "assistant", "one", "completed", None)
            .unwrap();
        let second_user = repo
            .add_message(&conversation.id, "user", "second", "completed", None)
            .unwrap();
        repo.add_message(&conversation.id, "assistant", "two", "completed", None)
            .unwrap();

        repo.delete_message(&second_user.id).unwrap();
        let messages = repo.list_messages(&conversation.id).unwrap();
        assert_eq!(
            messages
                .iter()
                .filter(|message| message.role == "system")
                .count(),
            1
        );
        assert_eq!(
            messages
                .iter()
                .filter(|message| message.role != "system")
                .count(),
            2
        );
        assert!(messages.iter().any(|message| message.id == first_user.id));
        assert!(!messages.iter().any(|message| message.id == second_user.id));
    }

    #[test]
    fn profile_and_project_context_are_independent() {
        let database = Database::in_memory().unwrap();
        let repo = ChatRepository::new(&database);
        let conversation = repo.create_conversation(Some("Context")).unwrap();

        repo.upsert_profile_context(&conversation.id, Some("profile instructions"))
            .unwrap();
        repo.upsert_profile_context(
            &conversation.id,
            Some("[open-mind-ai-project-context]\nProject: Demo"),
        )
        .unwrap();
        let messages = repo.list_messages(&conversation.id).unwrap();
        assert_eq!(
            messages
                .iter()
                .filter(|message| message.role == "system")
                .count(),
            2
        );
        assert!(messages
            .iter()
            .any(|message| message.content.starts_with(PROFILE_CONTEXT_MARKER)));
        assert!(messages
            .iter()
            .any(|message| message.content.starts_with(PROJECT_CONTEXT_MARKER)));

        repo.upsert_profile_context(&conversation.id, None).unwrap();
        let messages = repo.list_messages(&conversation.id).unwrap();
        assert_eq!(
            messages
                .iter()
                .filter(|message| message.role == "system")
                .count(),
            1
        );
        assert!(messages
            .iter()
            .any(|message| message.content.starts_with(PROFILE_CONTEXT_MARKER)));
        assert!(!messages
            .iter()
            .any(|message| message.content.starts_with(PROJECT_CONTEXT_MARKER)));
    }

    #[test]
    fn search_230_chats_returns_bounded_pages_and_oldest_message_hits() {
        let database = Database::in_memory().unwrap();
        let repo = ChatRepository::new(&database);
        let mut oldest = String::new();
        for index in 0..230 {
            let conversation = repo
                .create_conversation(Some(&format!("Report {index:03}")))
                .unwrap();
            if index == 0 {
                oldest = conversation.id.clone();
            }
            let body = if index == 0 {
                "RARE_STORED_ARCHIVE_MARKER in the very oldest chat".to_string()
            } else {
                format!("ordinary chat evidence {index:03}")
            };
            repo.add_message(&conversation.id, "user", &body, "completed", None)
                .unwrap();
            // Monotonic timestamps make page ordering deterministic.
            let timestamp = format!(
                "2026-01-01T{:02}:{:02}:{:02}+00:00",
                index / 3600,
                (index / 60) % 60,
                index % 60
            );
            database
                .connection()
                .execute(
                    "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
                    params![timestamp, conversation.id],
                )
                .unwrap();
        }

        let mut cursor = None;
        let mut seen = std::collections::HashSet::new();
        let mut pages = 0;
        loop {
            let page = repo
                .search_conversations("", cursor.as_deref(), 40)
                .unwrap();
            assert!(page.conversations.len() <= 40);
            pages += 1;
            for chat in page.conversations {
                assert!(seen.insert(chat.id), "duplicate chat across search pages");
            }
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
            assert!(pages <= 10, "chat pagination stalled");
        }
        assert_eq!(pages, 6);
        assert_eq!(seen.len(), 230);
        assert!(seen.contains(&oldest));

        let match_page = repo
            .search_conversations("stored_archive", None, 40)
            .unwrap();
        assert_eq!(match_page.conversations.len(), 1);
        assert_eq!(match_page.conversations[0].id, oldest);
        assert!(match_page.next_cursor.is_none());
        let no_match = repo.search_conversations("a-not-existing-token", None, 40).unwrap();
        assert!(no_match.conversations.is_empty());
        assert!(repo.search_conversations(&"x".repeat(257), None, 40).is_err());
        assert!(repo
            .search_conversations("", Some("not-a-real-cursor"), 40)
            .is_err());
    }

    #[test]
    fn chat_search_index_tracks_streams_edits_deletions_and_ignores_system_rows() {
        let database = Database::in_memory().unwrap();
        let repo = ChatRepository::new(&database);
        let chat = repo.create_conversation(Some("Session")).unwrap();
        let original = repo
            .add_message(
                &chat.id,
                "user",
                "an ORIGINAL_TRIGRAM_SENTINEL inside a chat message",
                "completed",
                None,
            )
            .unwrap();

        assert_eq!(
            repo.search_conversations("trigram_sentinel", None, 40)
                .unwrap()
                .conversations
                .len(),
            1
        );
        // Streaming appends use UPDATE OF content; the trigger must refresh
        // the indexed text for the next search without a manual index pass.
        repo.append_message_chunk(&original.id, " NEW_STREAM_CHUNK").unwrap();
        assert_eq!(
            repo.search_conversations("stream_chunk", None, 40)
                .unwrap()
                .conversations
                .len(),
            1
        );

        // Pending/streaming assistant text is mutable and must not trigger a
        // full FTS re-index on each token. It becomes searchable on completion.
        let assistant = repo
            .add_message(
                &chat.id,
                "assistant",
                "INFLIGHT_NOT_INDEXED_TOKEN",
                "streaming",
                None,
            )
            .unwrap();
        repo.append_message_chunk(&assistant.id, " MORE_STREAM_TEXT").unwrap();
        assert!(repo
            .search_conversations("inflight_not_indexed", None, 40)
            .unwrap()
            .conversations
            .is_empty());
        assert!(repo
            .search_conversations("more_stream_text", None, 40)
            .unwrap()
            .conversations
            .is_empty());
        repo.set_message_status(&assistant.id, "completed").unwrap();
        assert_eq!(
            repo.search_conversations("inflight_not_indexed", None, 40)
                .unwrap()
                .conversations
                .len(),
            1
        );

        database
            .connection()
            .execute(
                "UPDATE messages SET content = 'replaced evidence' WHERE id = ?1",
                params![original.id],
            )
            .unwrap();
        assert!(repo
            .search_conversations("trigram_sentinel", None, 40)
            .unwrap()
            .conversations
            .is_empty());
        assert_eq!(
            repo.search_conversations("placed ev", None, 40)
                .unwrap()
                .conversations
                .len(),
            1
        );
        // FTS syntax and wildcard-looking text must be treated literally.
        assert!(repo.search_conversations("\" or *", None, 40).is_ok());
        assert!(repo.search_conversations("zz%", None, 40).unwrap().conversations.is_empty());

        repo.add_message(
            &chat.id,
            "system",
            "SYSTEM_ONLY_SECRET_WORD",
            "completed",
            None,
        )
        .unwrap();
        assert!(repo.search_conversations("system_only_secret", None, 40)
            .unwrap()
            .conversations
            .is_empty());
        assert_eq!(
            repo.search_conversations("ss", None, 40)
                .unwrap()
                .conversations
                .len(),
            1,
            "two-character queries still work with the SQL fallback"
        );

        repo.archive_conversation(&chat.id).unwrap();
        assert!(repo.search_conversations("replaced", None, 40)
            .unwrap()
            .conversations
            .is_empty());
        repo.delete_conversation(&chat.id).unwrap();
        assert!(repo.search_conversations("replaced", None, 40)
            .unwrap()
            .conversations
            .is_empty());
    }

    #[test]
    fn historical_chat_messages_become_searchable_on_database_migration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data/database/openmindai.db");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(include_str!("../migrations/001_initial.sql"))
                .unwrap();
            conn.execute(
                "INSERT INTO conversations (id, title, created_at, updated_at, pinned)
                 VALUES ('preexisting', 'Old chat', '2026-01-01', '2026-01-01', 0)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO messages (id, conversation_id, role, content, status, created_at, updated_at)
                 VALUES ('old-message', 'preexisting', 'user', 'HISTORICAL_FTS_EVIDENCE_NOT_NEW',
                         'completed', '2026-01-01', '2026-01-01')",
                [],
            )
            .unwrap();
        }
        let migrated = Database::open(path).unwrap();
        let page = ChatRepository::new(&migrated)
            .search_conversations("historical_fts_evidence", None, 40)
            .unwrap();
        assert_eq!(page.conversations.len(), 1);
        assert_eq!(page.conversations[0].id, "preexisting");
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

    fn edit_fixture(database: &Database) -> (Conversation, Message) {
        let repo = ChatRepository::new(database);
        let source = repo.create_conversation(Some("Original chat")).unwrap();
        repo.upsert_profile_context(&source.id, Some("profile instructions"))
            .unwrap();
        repo.add_message(&source.id, "user", "earlier prompt", "completed", None)
            .unwrap();
        repo.add_message(&source.id, "assistant", "earlier answer", "completed", None)
            .unwrap();
        let target = repo
            .add_message(
                &source.id,
                "user",
                "original prompt with attachment text",
                "completed",
                None,
            )
            .unwrap();
        repo.add_message(
            &source.id,
            "assistant",
            "original answer",
            "completed",
            None,
        )
        .unwrap();
        (source, target)
    }

    fn saved_messages(repo: &ChatRepository<'_>, conversation_id: &str) -> serde_json::Value {
        serde_json::to_value(repo.list_messages(conversation_id).unwrap()).unwrap()
    }

    #[test]
    fn editing_forks_prefix_with_independent_ids_and_preserves_source() {
        let database = Database::in_memory().unwrap();
        let (source, target) = edit_fixture(&database);
        let repo = ChatRepository::new(&database);
        let now = Utc::now().to_rfc3339();
        database.connection().execute(
            "INSERT INTO model_registry (id, name, path, format, created_at, updated_at) VALUES ('edit-model', 'test', 'edit.gguf', 'gguf', ?1, ?1)",
            params![now],
        ).unwrap();
        repo.set_active_model(&source.id, Some("edit-model"))
            .unwrap();
        repo.set_pinned(&source.id, true).unwrap();
        database.connection().execute(
            "INSERT INTO projects (id, name, instructions, created_at, updated_at) VALUES ('edit-project', 'project', 'instructions', ?1, ?1)",
            params![now],
        ).unwrap();
        database.connection().execute(
            "INSERT INTO project_conversations (project_id, conversation_id, created_at) VALUES ('edit-project', ?1, ?2)",
            params![source.id, now],
        ).unwrap();
        database.connection().execute(
            "INSERT INTO conversation_settings (conversation_id, settings_json, updated_at) VALUES (?1, '{\"testSetting\":true}', ?2)",
            params![source.id, now],
        ).unwrap();
        let before = saved_messages(&repo, &source.id);
        let source_before =
            serde_json::to_value(repo.find_conversation(&source.id).unwrap()).unwrap();
        let fork = repo.fork_for_edit(&source.id, &target.id).unwrap();
        assert_ne!(fork.conversation.id, source.id);
        assert_eq!(fork.conversation.title, "Original chat (edited)");
        assert_eq!(
            fork.conversation.active_model_id.as_deref(),
            Some("edit-model")
        );
        assert!(!fork.conversation.pinned);
        let text = fork
            .messages
            .iter()
            .filter(|m| m.role != "system")
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>();
        assert_eq!(text, ["earlier prompt", "earlier answer"]);
        let original_ids = repo
            .list_messages(&source.id)
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect::<Vec<_>>();
        assert!(fork
            .messages
            .iter()
            .all(|m| m.conversation_id == fork.conversation.id && !original_ids.contains(&m.id)));
        assert!(fork.messages.iter().any(|m| m.role == "system"));
        let (project, settings): (String, String) = database.connection().query_row(
            "SELECT p.project_id, s.settings_json FROM project_conversations p JOIN conversation_settings s ON s.conversation_id = p.conversation_id WHERE p.conversation_id = ?1",
            params![fork.conversation.id], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(project, "edit-project");
        assert_eq!(settings, r#"{"testSetting":true}"#);
        assert_eq!(saved_messages(&repo, &source.id), before);
        assert_eq!(
            serde_json::to_value(repo.find_conversation(&source.id).unwrap()).unwrap(),
            source_before
        );
    }

    #[test]
    fn invalid_edit_targets_and_active_generations_do_not_create_branches() {
        let database = Database::in_memory().unwrap();
        let (source, target) = edit_fixture(&database);
        let repo = ChatRepository::new(&database);
        let other = repo.create_conversation(Some("Other chat")).unwrap();
        let messages = repo.list_messages(&source.id).unwrap();
        let assistant = messages.iter().find(|m| m.role == "assistant").unwrap();
        let before = saved_messages(&repo, &source.id);
        assert!(repo.fork_for_edit(&other.id, &target.id).is_err());
        assert!(repo.fork_for_edit(&source.id, "missing").is_err());
        assert!(repo.fork_for_edit("missing", &target.id).is_err());
        assert!(repo.fork_for_edit(&source.id, &assistant.id).is_err());
        for status in ["pending", "streaming"] {
            repo.set_message_status(&assistant.id, status).unwrap();
            assert!(repo.fork_for_edit(&source.id, &target.id).is_err());
        }
        repo.set_message_status(&assistant.id, "completed").unwrap();
        assert_eq!(repo.list_conversations().unwrap().len(), 2);
        // Only the status test's updated_at may change; all original IDs/content remain.
        let after = saved_messages(&repo, &source.id);
        assert_eq!(
            after.as_array().unwrap().len(),
            before.as_array().unwrap().len()
        );
        assert!(messages.iter().all(|m| repo
            .list_messages(&source.id)
            .unwrap()
            .iter()
            .any(|n| n.id == m.id && n.content == m.content)));
    }

    #[test]
    fn edit_fork_rolls_back_partial_prefix_after_database_failure() {
        let database = Database::in_memory().unwrap();
        let (source, target) = edit_fixture(&database);
        let repo = ChatRepository::new(&database);
        let before = saved_messages(&repo, &source.id);
        database
            .connection()
            .execute_batch(
                "CREATE TRIGGER reject_fork_assistant BEFORE INSERT ON messages
             WHEN NEW.role = 'assistant' BEGIN SELECT RAISE(ABORT, 'fixture copy failure'); END;",
            )
            .unwrap();
        assert!(repo.fork_for_edit(&source.id, &target.id).is_err());
        assert_eq!(repo.list_conversations().unwrap().len(), 1);
        let count: usize = database
            .connection()
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, before.as_array().unwrap().len());
        assert_eq!(saved_messages(&repo, &source.id), before);
    }

    #[test]
    fn edit_fork_rolls_back_after_project_copy_failure() {
        let database = Database::in_memory().unwrap();
        let (source, target) = edit_fixture(&database);
        let repo = ChatRepository::new(&database);
        let now = Utc::now().to_rfc3339();
        database.connection().execute(
            "INSERT INTO projects (id, name, created_at, updated_at) VALUES ('p', 'project', ?1, ?1)", params![now],
        ).unwrap();
        database.connection().execute(
            "INSERT INTO project_conversations (project_id, conversation_id, created_at) VALUES ('p', ?1, ?2)", params![source.id, now],
        ).unwrap();
        database
            .connection()
            .execute_batch(
                "CREATE TRIGGER reject_fork_project BEFORE INSERT ON project_conversations
             BEGIN SELECT RAISE(ABORT, 'fixture project failure'); END;",
            )
            .unwrap();
        let before = saved_messages(&repo, &source.id);
        assert!(repo.fork_for_edit(&source.id, &target.id).is_err());
        assert_eq!(repo.list_conversations().unwrap().len(), 1);
        let count: usize = database
            .connection()
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, before.as_array().unwrap().len());
        assert_eq!(saved_messages(&repo, &source.id), before);
    }

    #[test]
    fn edit_prefix_uses_visible_order_with_equal_timestamps() {
        let database = Database::in_memory().unwrap();
        let (source, target) = edit_fixture(&database);
        let repo = ChatRepository::new(&database);
        database.connection().execute(
            "UPDATE messages SET created_at = '2020-01-01T00:00:00Z' WHERE conversation_id = ?1 AND role != 'system'", params![source.id],
        ).unwrap();
        let fork = repo.fork_for_edit(&source.id, &target.id).unwrap();
        let contents = fork
            .messages
            .iter()
            .filter(|m| m.role != "system")
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>();
        assert_eq!(contents, ["earlier prompt", "earlier answer"]);
    }

    #[test]
    fn failed_cancelled_and_interrupted_edits_preserve_original_files_after_restart() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data/database/openmind_ai.db");
        let generated = temp.path().join("generated/files/original.txt");
        let media = temp.path().join("data/media/original.png");
        let index_dir = temp.path().join("data/media-index");
        std::fs::create_dir_all(generated.parent().unwrap()).unwrap();
        std::fs::create_dir_all(media.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&index_dir).unwrap();
        std::fs::write(&generated, b"original artifact bytes").unwrap();
        std::fs::write(&media, b"original media bytes").unwrap();
        let (source_id, target_id, before);
        {
            let database = Database::open(path.clone()).unwrap();
            let (source, target) = edit_fixture(&database);
            source_id = source.id;
            target_id = target.id;
            let repo = ChatRepository::new(&database);
            before = saved_messages(&repo, &source_id);
            std::fs::write(
                index_dir.join(format!("{target_id}.json")),
                b"original media index bytes",
            )
            .unwrap();
            let now = Utc::now().to_rfc3339();
            database.connection().execute(
                "INSERT INTO artifacts (id, conversation_id, message_id, name, path, mime_type, kind, status, created_at, updated_at)
                 VALUES ('original-artifact', ?1, ?2, 'original.txt', 'generated/files/original.txt', 'text/plain', 'text', 'ready', ?3, ?3)",
                params![source_id, target_id, now],
            ).unwrap();
            for outcome in [
                "startup-failure",
                "failed",
                "cancelled",
                "interrupted",
                "artifact-failure",
            ] {
                let fork = repo.fork_for_edit(&source_id, &target_id).unwrap();
                if outcome != "startup-failure" {
                    repo.add_message(
                        &fork.conversation.id,
                        "user",
                        "edited request",
                        "completed",
                        None,
                    )
                    .unwrap();
                    let assistant = repo
                        .add_message(
                            &fork.conversation.id,
                            "assistant",
                            "partial replacement",
                            "streaming",
                            None,
                        )
                        .unwrap();
                    if outcome != "interrupted" {
                        repo.set_message_status(
                            &assistant.id,
                            if outcome == "cancelled" {
                                "cancelled"
                            } else {
                                "failed"
                            },
                        )
                        .unwrap();
                    }
                    if outcome == "artifact-failure" {
                        database.connection().execute_batch(
                            "CREATE TRIGGER reject_new_artifact BEFORE INSERT ON artifacts BEGIN SELECT RAISE(ABORT, 'fixture artifact failure'); END;",
                        ).unwrap();
                        assert!(database.connection().execute(
                            "INSERT INTO artifacts (id, conversation_id, message_id, name, path, mime_type, kind, status, created_at, updated_at)
                             VALUES ('failed-artifact', ?1, ?2, 'new.txt', 'generated/files/new.txt', 'text/plain', 'text', 'ready', ?3, ?3)",
                            params![fork.conversation.id, assistant.id, now],
                        ).is_err());
                    }
                }
                assert_eq!(saved_messages(&repo, &source_id), before);
            }
        }
        let reopened = Database::open(path).unwrap();
        let repo = ChatRepository::new(&reopened);
        assert_eq!(saved_messages(&repo, &source_id), before);
        let (artifact_count, artifact_message): (usize, String) = reopened
            .connection()
            .query_row(
                "SELECT COUNT(*), message_id FROM artifacts WHERE id = 'original-artifact'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(artifact_count, 1);
        assert_eq!(artifact_message, target_id);
        assert_eq!(
            std::fs::read(generated).unwrap(),
            b"original artifact bytes"
        );
        assert_eq!(std::fs::read(media).unwrap(), b"original media bytes");
        assert_eq!(
            std::fs::read(index_dir.join(format!("{target_id}.json"))).unwrap(),
            b"original media index bytes"
        );
    }
    #[test]
    fn regeneration_keeps_original_through_failed_cancelled_and_empty_candidates() {
        let db = Database::in_memory().unwrap();
        let repo = ChatRepository::new(&db);
        let chat = repo.create_conversation(Some("Regenerate")).unwrap();
        repo.add_message(&chat.id, "user", "question", "completed", None)
            .unwrap();
        let original = repo
            .add_message(
                &chat.id,
                "assistant",
                "valuable original",
                "completed",
                None,
            )
            .unwrap();
        // Model-selection/startup errors occur before begin_regeneration: no
        // source row is touched. A later partial stream also cannot retire it.
        for status in ["failed", "cancelled", "completed"] {
            let candidate = repo
                .begin_regeneration(&chat.id, &original.id, None)
                .unwrap();
            if status != "completed" {
                repo.append_message_chunk(&candidate.id, "partial").unwrap();
            }
            assert_eq!(
                repo.find_message(&original.id).unwrap().content,
                "valuable original"
            );
            repo.set_message_status(&candidate.id, status).unwrap();
            assert_eq!(
                repo.find_message(&original.id).unwrap().content,
                "valuable original"
            );
            assert_eq!(repo.find_message(&candidate.id).unwrap().status, status);
        }
    }

    #[test]
    fn successful_regeneration_retires_only_its_source_and_keeps_artifacts() {
        let db = Database::in_memory().unwrap();
        let repo = ChatRepository::new(&db);
        let chat = repo.create_conversation(Some("Regenerate")).unwrap();
        repo.add_message(&chat.id, "user", "question", "completed", None)
            .unwrap();
        let original = repo
            .add_message(&chat.id, "assistant", "original", "completed", None)
            .unwrap();
        let later = repo
            .add_message(&chat.id, "user", "later turn", "completed", None)
            .unwrap();
        let artifact = crate::artifacts::ArtifactRepository::new(&db)
            .create(
                &chat.id,
                Some(&original.id),
                "evidence.txt",
                "generated/files/evidence.txt",
                "text/plain",
                "text",
                "ready",
            )
            .unwrap();
        let candidate = repo
            .begin_regeneration(&chat.id, &original.id, None)
            .unwrap();
        repo.append_message_chunk(&candidate.id, "replacement")
            .unwrap();
        repo.set_message_status(&candidate.id, "completed").unwrap();
        assert!(repo.find_message(&original.id).is_err());
        assert_eq!(
            repo.find_message(&candidate.id).unwrap().content,
            "replacement"
        );
        assert_eq!(repo.find_message(&later.id).unwrap().content, "later turn");
        assert!(crate::artifacts::ArtifactRepository::new(&db)
            .find(&artifact.id)
            .is_ok());
        repo.set_message_status(&candidate.id, "completed").unwrap(); // idempotent
    }

    #[test]
    fn regeneration_context_excludes_source_and_later_turns_but_keeps_system_context() {
        let db = Database::in_memory().unwrap();
        let repo = ChatRepository::new(&db);
        let chat = repo.create_conversation(None).unwrap();
        let user = repo
            .add_message(&chat.id, "user", "question", "completed", None)
            .unwrap();
        let original = repo
            .add_message(&chat.id, "assistant", "do not echo this", "completed", None)
            .unwrap();
        repo.add_message(&chat.id, "user", "later question", "completed", None)
            .unwrap();
        let system = repo
            .add_message(&chat.id, "system", "current policy", "completed", None)
            .unwrap();
        let candidate = repo
            .begin_regeneration(&chat.id, &original.id, None)
            .unwrap();
        let context = repo
            .messages_for_generation(&chat.id, &candidate.id)
            .unwrap();
        assert_eq!(
            context.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec![user.id.as_str(), system.id.as_str()]
        );
        assert_eq!(repo.list_messages(&chat.id).unwrap().len(), 5);
        let other = repo.create_conversation(None).unwrap();
        assert!(repo
            .begin_regeneration(&other.id, &original.id, None)
            .is_err());
        assert!(repo.begin_regeneration(&chat.id, &user.id, None).is_err());
        assert_eq!(repo.list_messages(&chat.id).unwrap().len(), 5);
    }

    #[test]
    fn failed_completion_transaction_preserves_original_and_candidate_status() {
        let db = Database::in_memory().unwrap();
        let repo = ChatRepository::new(&db);
        let chat = repo.create_conversation(None).unwrap();
        let original = repo
            .add_message(&chat.id, "assistant", "original", "completed", None)
            .unwrap();
        let candidate = repo
            .begin_regeneration(&chat.id, &original.id, None)
            .unwrap();
        repo.append_message_chunk(&candidate.id, "replacement")
            .unwrap();
        db.connection().execute_batch("CREATE TRIGGER injected_failure BEFORE DELETE ON messages BEGIN SELECT RAISE(ABORT, 'injected storage failure'); END;").unwrap();
        assert!(repo.set_message_status(&candidate.id, "completed").is_err());
        assert_eq!(
            repo.find_message(&candidate.id).unwrap().status,
            "streaming"
        );
        assert_eq!(repo.find_message(&original.id).unwrap().content, "original");
    }

    #[test]
    fn interrupted_regeneration_survives_reopening_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("openmind_ai.db");
        let (original_id, candidate_id) = {
            let db = Database::open(path.clone()).unwrap();
            let repo = ChatRepository::new(&db);
            let chat = repo.create_conversation(None).unwrap();
            let original = repo
                .add_message(&chat.id, "assistant", "saved original", "completed", None)
                .unwrap();
            let candidate = repo
                .begin_regeneration(&chat.id, &original.id, None)
                .unwrap();
            repo.append_message_chunk(&candidate.id, "interrupted partial")
                .unwrap();
            (original.id, candidate.id)
        };
        let db = Database::open(path).unwrap();
        let repo = ChatRepository::new(&db);
        assert_eq!(
            repo.find_message(&original_id).unwrap().content,
            "saved original"
        );
        assert_eq!(repo.find_message(&candidate_id).unwrap().status, "failed");
    }
    #[test]
    fn failed_candidate_creation_rolls_back_without_touching_the_source() {
        let db = Database::in_memory().unwrap();
        let repo = ChatRepository::new(&db);
        let chat = repo.create_conversation(None).unwrap();
        let original = repo
            .add_message(&chat.id, "assistant", "original", "completed", None)
            .unwrap();
        db.connection().execute_batch("CREATE TRIGGER injected_candidate_failure BEFORE UPDATE OF regenerates_message_id ON messages BEGIN SELECT RAISE(ABORT, 'injected write failure'); END;").unwrap();
        assert!(repo
            .begin_regeneration(&chat.id, &original.id, None)
            .is_err());
        let messages = repo.list_messages(&chat.id).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, original.id);
        assert_eq!(messages[0].content, "original");
    }

    #[test]
    fn original_artifact_file_and_link_survive_cancel_and_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data/database/openmind_ai.db");
        let output = dir.path().join("generated/files/original.txt");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        std::fs::write(&output, b"precious original bytes").unwrap();
        let (original_id, artifact_id) = {
            let db = Database::open(path.clone()).unwrap();
            let repo = ChatRepository::new(&db);
            let chat = repo.create_conversation(None).unwrap();
            let original = repo
                .add_message(&chat.id, "assistant", "original", "completed", None)
                .unwrap();
            let artifact = crate::artifacts::ArtifactRepository::new(&db)
                .create(
                    &chat.id,
                    Some(&original.id),
                    "original.txt",
                    "generated/files/original.txt",
                    "text/plain",
                    "text",
                    "ready",
                )
                .unwrap();
            let cancelled = repo
                .begin_regeneration(&chat.id, &original.id, None)
                .unwrap();
            repo.append_message_chunk(&cancelled.id, "cancelled partial")
                .unwrap();
            repo.set_message_status(&cancelled.id, "cancelled").unwrap();
            let interrupted = repo
                .begin_regeneration(&chat.id, &original.id, None)
                .unwrap();
            repo.append_message_chunk(&interrupted.id, "interrupted partial")
                .unwrap();
            (original.id, artifact.id)
        };
        let db = Database::open(path).unwrap();
        let artifact = crate::artifacts::ArtifactRepository::new(&db)
            .find(&artifact_id)
            .unwrap();
        assert_eq!(artifact.message_id.as_deref(), Some(original_id.as_str()));
        assert_eq!(
            ChatRepository::new(&db)
                .find_message(&original_id)
                .unwrap()
                .content,
            "original"
        );
        assert_eq!(std::fs::read(output).unwrap(), b"precious original bytes");
    }
}
