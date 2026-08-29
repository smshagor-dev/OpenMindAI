use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{app_error::AppError, chat::Conversation, database::Database};

const MAX_PROJECT_NAME_CHARS: usize = 120;
const MAX_PROJECT_INSTRUCTIONS_CHARS: usize = 20_000;
const MAX_PROJECT_FILE_NAME_CHARS: usize = 255;
const MAX_PROJECT_FILE_TEXT_CHARS: usize = 120_000;
const MAX_PROJECT_FILE_ERROR_CHARS: usize = 2_000;
const MAX_PROJECT_MIME_TYPE_CHARS: usize = 255;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub instructions: String,
    pub created_at: String,
    pub updated_at: String,
    pub conversation_ids: Vec<String>,
    pub files: Vec<ProjectFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFile {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub size_bytes: i64,
    pub mime_type: Option<String>,
    pub content_text: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub added_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFileInput {
    pub project_id: String,
    pub name: String,
    pub size_bytes: i64,
    pub mime_type: Option<String>,
    pub content_text: Option<String>,
    pub status: String,
    pub error: Option<String>,
}

pub struct ProjectRepository<'a> {
    database: &'a Database,
}

impl<'a> ProjectRepository<'a> {
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    pub fn list_projects(&self) -> Result<Vec<Project>, AppError> {
        let mut statement = self.database.connection().prepare(
            "SELECT id, name, instructions, created_at, updated_at
             FROM projects
             ORDER BY updated_at DESC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;

        let mut projects = Vec::new();
        for row in rows {
            let (id, name, instructions, created_at, updated_at) = row?;
            projects.push(Project {
                conversation_ids: self.conversation_ids(&id)?,
                files: self.list_files(&id)?,
                id,
                name,
                instructions,
                created_at,
                updated_at,
            });
        }
        Ok(projects)
    }

    pub fn create_project(&self, name: &str) -> Result<Project, AppError> {
        let name = sanitize_name(name)?;
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        self.database.connection().execute(
            "INSERT INTO projects (id, name, instructions, created_at, updated_at)
             VALUES (?1, ?2, '', ?3, ?3)",
            params![id, name, now],
        )?;
        self.find_project(&id)
    }

    pub fn update_project(
        &self,
        id: &str,
        name: &str,
        instructions: &str,
    ) -> Result<Project, AppError> {
        let name = sanitize_name(name)?;
        let instructions = take_chars(instructions.trim(), MAX_PROJECT_INSTRUCTIONS_CHARS);
        let changed = self.database.connection().execute(
            "UPDATE projects SET name = ?1, instructions = ?2, updated_at = ?3 WHERE id = ?4",
            params![name, instructions, Utc::now().to_rfc3339(), id],
        )?;
        if changed == 0 {
            return Err(AppError::internal(format!("project not found: {id}")));
        }
        self.find_project(id)
    }

    pub fn delete_project(&self, id: &str) -> Result<(), AppError> {
        let changed = self
            .database
            .connection()
            .execute("DELETE FROM projects WHERE id = ?1", params![id])?;
        if changed == 0 {
            return Err(AppError::internal(format!("project not found: {id}")));
        }
        Ok(())
    }

    pub fn link_conversation(
        &self,
        project_id: &str,
        conversation_id: &str,
    ) -> Result<(), AppError> {
        let now = Utc::now().to_rfc3339();
        let transaction = self.database.connection().unchecked_transaction()?;
        let previous_project: Option<String> = transaction
            .query_row(
                "SELECT project_id FROM project_conversations WHERE conversation_id = ?1 LIMIT 1",
                params![conversation_id],
                |row| row.get(0),
            )
            .optional()?;

        transaction.execute(
            "DELETE FROM project_conversations WHERE conversation_id = ?1",
            params![conversation_id],
        )?;
        transaction.execute(
            "INSERT INTO project_conversations (project_id, conversation_id, created_at)
             VALUES (?1, ?2, ?3)",
            params![project_id, conversation_id, now],
        )?;
        transaction.execute(
            "UPDATE projects SET updated_at = ?1 WHERE id = ?2",
            params![now, project_id],
        )?;
        if let Some(previous_project) = previous_project.filter(|id| id != project_id) {
            transaction.execute(
                "UPDATE projects SET updated_at = ?1 WHERE id = ?2",
                params![now, previous_project],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn unlink_conversation(
        &self,
        project_id: &str,
        conversation_id: &str,
    ) -> Result<(), AppError> {
        self.database.connection().execute(
            "DELETE FROM project_conversations WHERE project_id = ?1 AND conversation_id = ?2",
            params![project_id, conversation_id],
        )?;
        self.touch_project(project_id)?;
        Ok(())
    }

    pub fn add_file(&self, input: &ProjectFileInput) -> Result<ProjectFile, AppError> {
        if input.size_bytes < 0 {
            return Err(AppError::internal("project file size cannot be negative"));
        }
        let name = sanitize_file_name(&input.name)?;
        let mime_type = sanitize_mime_type(input.mime_type.as_deref())?;
        let content_text = input
            .content_text
            .as_deref()
            .map(|value| take_chars(value, MAX_PROJECT_FILE_TEXT_CHARS));
        let error = input
            .error
            .as_deref()
            .map(|value| take_chars(value.trim(), MAX_PROJECT_FILE_ERROR_CHARS))
            .filter(|value| !value.is_empty());
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        self.database.connection().execute(
            "INSERT INTO project_files (id, project_id, name, size_bytes, mime_type, content_text, status, error, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                input.project_id,
                name,
                input.size_bytes,
                mime_type.as_deref(),
                content_text.as_deref(),
                sanitize_file_status(&input.status),
                error.as_deref(),
                now
            ],
        )?;
        self.touch_project(&input.project_id)?;
        self.find_file(&id)
    }

    pub fn delete_file(&self, project_id: &str, file_id: &str) -> Result<(), AppError> {
        self.database.connection().execute(
            "DELETE FROM project_files WHERE project_id = ?1 AND id = ?2",
            params![project_id, file_id],
        )?;
        self.touch_project(project_id)?;
        Ok(())
    }

    pub fn project_for_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Option<Project>, AppError> {
        let project_id = self
            .database
            .connection()
            .query_row(
                "SELECT project_id FROM project_conversations
                 WHERE conversation_id = ?1
                 ORDER BY created_at DESC
                 LIMIT 1",
                params![conversation_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        project_id.map(|id| self.find_project(&id)).transpose()
    }

    pub fn find_project(&self, id: &str) -> Result<Project, AppError> {
        let (id, name, instructions, created_at, updated_at) =
            self.database.connection().query_row(
                "SELECT id, name, instructions, created_at, updated_at FROM projects WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )?;
        Ok(Project {
            conversation_ids: self.conversation_ids(&id)?,
            files: self.list_files(&id)?,
            id,
            name,
            instructions,
            created_at,
            updated_at,
        })
    }

    fn conversation_ids(&self, project_id: &str) -> Result<Vec<String>, AppError> {
        let mut statement = self.database.connection().prepare(
            "SELECT conversation_id FROM project_conversations WHERE project_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = statement.query_map(params![project_id], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    fn list_files(&self, project_id: &str) -> Result<Vec<ProjectFile>, AppError> {
        let mut statement = self.database.connection().prepare(
            "SELECT id, project_id, name, size_bytes, mime_type, content_text, status, error, added_at
             FROM project_files
             WHERE project_id = ?1
             ORDER BY added_at DESC",
        )?;
        let rows = statement.query_map(params![project_id], map_project_file)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    fn find_file(&self, id: &str) -> Result<ProjectFile, AppError> {
        self.database.connection().query_row(
            "SELECT id, project_id, name, size_bytes, mime_type, content_text, status, error, added_at FROM project_files WHERE id = ?1",
            params![id],
            map_project_file,
        ).map_err(AppError::from)
    }

    fn touch_project(&self, id: &str) -> Result<(), AppError> {
        let changed = self.database.connection().execute(
            "UPDATE projects SET updated_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), id],
        )?;
        if changed == 0 {
            return Err(AppError::internal(format!("project not found: {id}")));
        }
        Ok(())
    }
}

pub fn project_context_message(project: &Project) -> Option<String> {
    let instructions = project.instructions.trim();
    let file_context = project_file_context(&project.files);
    if instructions.is_empty() && file_context.is_empty() {
        return None;
    }

    let mut sections = vec![format!(
        "[open-mind-ai-project-context]\nProject: {}",
        project.name
    )];
    if !instructions.is_empty() {
        sections.push(format!("Instructions:\n{instructions}"));
    }
    if !file_context.is_empty() {
        sections.push(format!("Project files:\n{file_context}"));
    }
    Some(sections.join("\n\n"))
}

fn sanitize_name(name: &str) -> Result<String, AppError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::internal("project name cannot be empty"));
    }
    Ok(take_chars(trimmed, MAX_PROJECT_NAME_CHARS))
}

fn sanitize_file_name(name: &str) -> Result<String, AppError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::internal("project file name cannot be empty"));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(AppError::internal("project file name contains control characters"));
    }
    Ok(take_chars(trimmed, MAX_PROJECT_FILE_NAME_CHARS))
}

fn sanitize_mime_type(value: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if value.chars().any(char::is_control) {
        return Err(AppError::internal("project file MIME type contains control characters"));
    }
    Ok(Some(take_chars(value, MAX_PROJECT_MIME_TYPE_CHARS)))
}

fn sanitize_file_status(status: &str) -> &'static str {
    match status {
        "ready" => "ready",
        "tracked" => "tracked",
        "skipped" => "skipped",
        "failed" => "failed",
        _ => "tracked",
    }
}

fn project_file_context(files: &[ProjectFile]) -> String {
    const MAX_FILES: usize = 8;
    const MAX_FILE_CHARS: usize = 4_000;
    const MAX_TOTAL_CHARS: usize = 16_000;

    let mut total_chars = 0usize;
    let mut snippets = Vec::new();
    for file in files
        .iter()
        .filter(|file| file.status == "ready")
        .filter_map(|file| {
            file.content_text
                .as_deref()
                .map(|content| (file, content.trim()))
        })
        .filter(|(_, content)| !content.is_empty())
        .take(MAX_FILES)
    {
        if total_chars >= MAX_TOTAL_CHARS {
            break;
        }
        let (file, content) = file;
        let remaining = MAX_TOTAL_CHARS.saturating_sub(total_chars);
        let limit = MAX_FILE_CHARS.min(remaining);
        let content_chars = content.chars().count();
        let snippet = take_chars(content, limit);
        let snippet_chars = snippet.chars().count();
        total_chars += snippet_chars;
        let suffix = if snippet_chars < content_chars {
            "\n[truncated]"
        } else {
            ""
        };
        let mime = file.mime_type.as_deref().unwrap_or("text/plain");
        snippets.push(format!(
            "--- {} ({}) ---\n{}{}",
            file.name, mime, snippet, suffix
        ));
    }

    snippets.join("\n\n")
}

fn take_chars(content: &str, limit: usize) -> String {
    content.chars().take(limit).collect()
}

fn map_project_file(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectFile> {
    Ok(ProjectFile {
        id: row.get(0)?,
        project_id: row.get(1)?,
        name: row.get(2)?,
        size_bytes: row.get(3)?,
        mime_type: row.get(4)?,
        content_text: row.get(5)?,
        status: row.get(6)?,
        error: row.get(7)?,
        added_at: row.get(8)?,
    })
}

#[allow(dead_code)]
fn _conversation_type_check(_: Conversation) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert_conversation(database: &Database, id: &str, title: &str) {
        database
            .connection()
            .execute(
                "INSERT INTO conversations (id, title, created_at, updated_at, pinned) VALUES (?1, ?2, 'now', 'now', 0)",
                params![id, title],
            )
            .unwrap();
    }

    #[test]
    fn project_crud_links_and_files() {
        let database = Database::in_memory().unwrap();
        let repo = ProjectRepository::new(&database);
        let project = repo.create_project("Launch").unwrap();
        let project = repo
            .update_project(&project.id, "Launch Plan", "Be concise")
            .unwrap();
        assert_eq!(project.name, "Launch Plan");
        assert!(project_context_message(&project)
            .unwrap()
            .contains("Be concise"));

        insert_conversation(&database, "c1", "Chat");
        repo.link_conversation(&project.id, "c1").unwrap();
        repo.add_file(&ProjectFileInput {
            project_id: project.id.clone(),
            name: "brief.md".to_string(),
            size_bytes: 42,
            mime_type: Some("text/markdown".to_string()),
            content_text: Some("# Brief\nShip the polished version.".to_string()),
            status: "ready".to_string(),
            error: None,
        })
        .unwrap();
        let listed = repo.list_projects().unwrap();
        assert_eq!(listed[0].conversation_ids, vec!["c1"]);
        assert_eq!(listed[0].files[0].name, "brief.md");
        let context = project_context_message(&listed[0]).unwrap();
        assert!(context.contains("Ship the polished version."));
        repo.unlink_conversation(&project.id, "c1").unwrap();
        repo.delete_project(&project.id).unwrap();
        assert!(repo.list_projects().unwrap().is_empty());
    }

    #[test]
    fn moving_conversation_between_projects_keeps_single_owner() {
        let database = Database::in_memory().unwrap();
        let repo = ProjectRepository::new(&database);
        let alpha = repo.create_project("Alpha").unwrap();
        let beta = repo.create_project("Beta").unwrap();
        insert_conversation(&database, "move-me", "Move me");

        repo.link_conversation(&alpha.id, "move-me").unwrap();
        repo.link_conversation(&beta.id, "move-me").unwrap();

        let alpha = repo.find_project(&alpha.id).unwrap();
        let beta = repo.find_project(&beta.id).unwrap();
        assert!(alpha.conversation_ids.is_empty());
        assert_eq!(beta.conversation_ids, vec!["move-me"]);
        assert_eq!(
            repo.project_for_conversation("move-me").unwrap().unwrap().id,
            beta.id
        );
    }

    #[test]
    fn project_inputs_are_bounded_and_validated() {
        let database = Database::in_memory().unwrap();
        let repo = ProjectRepository::new(&database);
        let project = repo.create_project(&"P".repeat(200)).unwrap();
        assert_eq!(project.name.chars().count(), MAX_PROJECT_NAME_CHARS);

        let project = repo
            .update_project(&project.id, &project.name, &"I".repeat(MAX_PROJECT_INSTRUCTIONS_CHARS + 10))
            .unwrap();
        assert_eq!(
            project.instructions.chars().count(),
            MAX_PROJECT_INSTRUCTIONS_CHARS
        );

        assert!(repo
            .add_file(&ProjectFileInput {
                project_id: project.id.clone(),
                name: "bad\nname.txt".to_string(),
                size_bytes: 1,
                mime_type: Some("text/plain".to_string()),
                content_text: Some("text".to_string()),
                status: "ready".to_string(),
                error: None,
            })
            .is_err());
        assert!(repo
            .add_file(&ProjectFileInput {
                project_id: project.id,
                name: "ok.txt".to_string(),
                size_bytes: -1,
                mime_type: Some("text/plain".to_string()),
                content_text: Some("text".to_string()),
                status: "ready".to_string(),
                error: None,
            })
            .is_err());
    }

    #[test]
    fn context_limits_count_unicode_characters_not_bytes() {
        let database = Database::in_memory().unwrap();
        let repo = ProjectRepository::new(&database);
        let project = repo.create_project("Unicode").unwrap();
        repo.add_file(&ProjectFileInput {
            project_id: project.id.clone(),
            name: "unicode.txt".to_string(),
            size_bytes: 32_000,
            mime_type: Some("text/plain".to_string()),
            content_text: Some("🙂".repeat(5_000)),
            status: "ready".to_string(),
            error: None,
        })
        .unwrap();
        let project = repo.find_project(&project.id).unwrap();
        let context = project_context_message(&project).unwrap();
        assert!(context.contains("[truncated]"));
        assert!(context.chars().filter(|character| *character == '🙂').count() <= 4_000);
    }
}
