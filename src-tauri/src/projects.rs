use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Utc;
use sha2::{Digest, Sha256};
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
/// Single-request IPC and SQLite BLOB bound; large files need a streaming API.
const MAX_PROJECT_BINARY_BYTES: usize = 1_500_000;

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

/// Lightweight catalog row: no instructions, conversations or file bodies.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub updated_at: String,
    pub conversation_count: i64,
    pub file_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectListPage {
    pub projects: Vec<ProjectSummary>,
    pub next_cursor: Option<String>,
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
    /// SHA-256 of retained binary bytes (absent for readable text and legacy metadata).
    pub sha256: Option<String>,
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

/// Filename is display metadata, never an OS path. The encoded payload is
/// accepted only on this dedicated bounded attachment endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBinaryFileInput {
    pub project_id: String,
    pub name: String,
    pub mime_type: Option<String>,
    pub size_bytes: i64,
    pub data_base64: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBinaryFileDownload {
    pub name: String,
    pub mime_type: Option<String>,
    pub size_bytes: i64,
    pub sha256: String,
    pub data_base64: String,
}

type BinaryProjectFileRow = (String, Option<String>, i64, Vec<u8>, String);

pub struct ProjectRepository<'a> {
    database: &'a Database,
}

impl<'a> ProjectRepository<'a> {
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    /// A single bounded metadata-only SQL query replaces the previous
    /// N+1 conversation and full-file queries for every catalog entry.
    /// A single selected project is loaded on demand via find_project.
    pub fn list_projects(
        &self,
        cursor: Option<&str>,
        requested_limit: i64,
    ) -> Result<ProjectListPage, AppError> {
        let limit = requested_limit.clamp(1, 100);
        let cursor = match cursor {
            Some(raw) => {
                let (updated_at, id) = raw
                    .split_once('|')
                    .ok_or_else(|| AppError::internal("invalid project catalog cursor"))?;
                if raw.len() > 128
                    || chrono::DateTime::parse_from_rfc3339(updated_at).is_err()
                    || Uuid::parse_str(id).is_err()
                {
                    return Err(AppError::internal("invalid project catalog cursor"));
                }
                Some((updated_at, id))
            }
            None => None,
        };
        let (after_time, after_id) = match cursor {
            Some((time, id)) => (Some(time), Some(id)),
            None => (None, None),
        };
        let mut statement = self.database.connection().prepare(
            "SELECT p.id, p.name, p.updated_at,
                    (SELECT COUNT(*) FROM project_conversations pc WHERE pc.project_id = p.id),
                    (SELECT COUNT(*) FROM project_files pf WHERE pf.project_id = p.id)
             FROM projects p
             WHERE (?1 IS NULL OR p.updated_at < ?1
                    OR (p.updated_at = ?1 AND p.id < ?2))
             ORDER BY p.updated_at DESC, p.id DESC
             LIMIT ?3",
        )?;
        let rows = statement.query_map(params![after_time, after_id, limit + 1], |row| {
            Ok(ProjectSummary {
                id: row.get(0)?,
                name: row.get(1)?,
                updated_at: row.get(2)?,
                conversation_count: row.get(3)?,
                file_count: row.get(4)?,
            })
        })?;
        let mut projects = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)?;
        let has_more = projects.len() > limit as usize;
        projects.truncate(limit as usize);
        let next_cursor = if has_more {
            projects
                .last()
                .map(|project| format!("{}|{}", project.updated_at, project.id))
        } else {
            None
        };
        Ok(ProjectListPage {
            projects,
            next_cursor,
        })
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

    /// Stores original binary bytes with the portable SQLite profile.
    /// No filesystem paths are built from names; database snapshots and
    /// cascading deletes therefore include the exact payload automatically.
    pub fn add_binary_file(
        &self,
        input: &ProjectBinaryFileInput,
    ) -> Result<ProjectFile, AppError> {
        if input.size_bytes < 0 || input.size_bytes as usize > MAX_PROJECT_BINARY_BYTES {
            return Err(AppError::internal("binary project attachment exceeds 1.5 MB limit"));
        }
        // Reject overlong payloads *before* decoding to bound IPC allocations.
        const MAX_ENCODED_BYTES: usize = MAX_PROJECT_BINARY_BYTES.div_ceil(3) * 4;
        if input.data_base64.len() > MAX_ENCODED_BYTES {
            return Err(AppError::internal("encoded project attachment exceeds size limit"));
        }
        let bytes = STANDARD
            .decode(&input.data_base64)
            .map_err(|_| AppError::internal("invalid project attachment base64"))?;
        if bytes.len() != input.size_bytes as usize {
            return Err(AppError::internal("binary attachment length does not match file size"));
        }
        let name = sanitize_file_name(&input.name)?;
        if name == "." || name == ".." || name.contains('/') || name.contains('\\') {
            return Err(AppError::internal("binary attachment name cannot contain path separators"));
        }
        let mime_type = sanitize_mime_type(input.mime_type.as_deref())?;
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let transaction = self.database.connection().unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO project_files
             (id, project_id, name, size_bytes, mime_type, content_text, status, error,
              added_at, content_blob, content_sha256)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, 'attached', NULL, ?6, ?7, ?8)",
            params![
                id,
                input.project_id,
                name,
                input.size_bytes,
                mime_type.as_deref(),
                now,
                bytes,
                digest
            ],
        )?;
        transaction.execute(
            "UPDATE projects SET updated_at = ?1 WHERE id = ?2",
            params![now, input.project_id],
        )?;
        transaction.commit()?;
        self.find_file(&id)
    }

    /// Returns bytes only for a file owned by this project. Verify every
    /// read against the persisted SHA-256 to detect corruption or mismatch.
    pub fn read_binary_file(
        &self,
        project_id: &str,
        file_id: &str,
    ) -> Result<ProjectBinaryFileDownload, AppError> {
        let record: Option<BinaryProjectFileRow> = self
            .database
            .connection()
            .query_row(
                "SELECT name, mime_type, size_bytes, content_blob, content_sha256
                 FROM project_files
                 WHERE id = ?1 AND project_id = ?2 AND status = 'attached'",
                params![file_id, project_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .optional()?;
        let (name, mime_type, size_bytes, bytes, expected) =
            record.ok_or_else(|| AppError::internal("project attachment not found"))?;
        if size_bytes < 0
            || size_bytes as usize > MAX_PROJECT_BINARY_BYTES
            || bytes.len() != size_bytes as usize
            || format!("{:x}", Sha256::digest(&bytes)) != expected
        {
            return Err(AppError::internal("project attachment failed integrity verification"));
        }
        Ok(ProjectBinaryFileDownload {
            name,
            mime_type,
            size_bytes,
            sha256: expected,
            data_base64: STANDARD.encode(&bytes),
        })
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
            "SELECT id, project_id, name, size_bytes, mime_type, content_text, status, error, added_at, content_sha256
             FROM project_files
             WHERE project_id = ?1
             ORDER BY added_at DESC",
        )?;
        let rows = statement.query_map(params![project_id], map_project_file)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    fn find_file(&self, id: &str) -> Result<ProjectFile, AppError> {
        self.database.connection().query_row(
            "SELECT id, project_id, name, size_bytes, mime_type, content_text, status, error, added_at, content_sha256 FROM project_files WHERE id = ?1",
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
    project_context_message_for_prompt(project, None)
}

/// Select relevant retained text evidence for this individual user request.
/// Uploading a file marks it readable, not permanently included in every
/// model prompt. Updates and idle project previews may use the generic view.
pub fn project_context_message_for_prompt(
    project: &Project,
    prompt: Option<&str>,
) -> Option<String> {
    let instructions = project.instructions.trim();
    let file_context = project_file_context(&project.files, prompt);
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

/// Each project file remains stored and addressable. Only the best-scoring
/// subset is sent to a given model request, with an explicit omission count.
fn project_file_context(files: &[ProjectFile], prompt: Option<&str>) -> String {
    const MAX_FILES: usize = 8;
    const MAX_FILE_CHARS: usize = 4_000;
    const MAX_TOTAL_CHARS: usize = 16_000;
    // Reserve space for the selection/disclosure line, even at the full cap.
    const MAX_SECTION_CHARS: usize = MAX_TOTAL_CHARS - 256;

    let terms = project_query_terms(prompt.unwrap_or(""));
    let mut candidates = files
        .iter()
        .filter(|file| file.status == "ready")
        .filter_map(|file| {
            file.content_text
                .as_deref()
                .map(str::trim)
                .filter(|content| !content.is_empty())
                .map(|content| (file, content))
        })
        .enumerate()
        .map(|(recency, (file, content))| {
            let score = project_file_relevance(&file.name, content, &terms);
            (score, recency, file, content)
        })
        .collect::<Vec<_>>();

    // Stable ordering: relevant older files win, then preserve the file-list's
    // newest-first order when the query does not identify a specific file.
    candidates.sort_by(|left, right| {
        right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1))
    });
    if candidates.is_empty() {
        return String::new();
    }

    let total = candidates.len();
    let mut sections = Vec::new();
    let mut used_chars = 0usize;
    for (_, _, file, content) in candidates.into_iter().take(MAX_FILES) {
        let mime = file.mime_type.as_deref().unwrap_or("text/plain");
        let header = format!("--- {} ({}) ---\n", file.name, mime);
        let separator_chars = if sections.is_empty() { 0 } else { 2 };
        let overhead = header.chars().count() + separator_chars;
        if used_chars + overhead >= MAX_SECTION_CHARS {
            break;
        }
        let capacity = (MAX_SECTION_CHARS - used_chars - overhead)
            .saturating_sub("\n[truncated]".len())
            .min(MAX_FILE_CHARS);
        if capacity == 0 {
            break;
        }
        let snippet = take_chars(content, capacity);
        let truncated = snippet.chars().count() < content.chars().count();
        let suffix = if truncated { "\n[truncated]" } else { "" };
        let section = format!("{header}{snippet}{suffix}");
        used_chars += section.chars().count() + separator_chars;
        sections.push(section);
    }

    let selected = sections.len();
    let omitted = total - selected;
    let mut output = format!(
        "[Project file selection: {selected} of {total} ready text files included for this request; {omitted} omitted. Ready means stored and searchable, not guaranteed in every prompt.]"
    );
    if !sections.is_empty() {
        output.push_str("\n\n");
        output.push_str(&sections.join("\n\n"));
    }
    // The total includes headers, truncation markers and the omission note.
    take_chars(&output, MAX_TOTAL_CHARS)
}

/// Filename references rank above incidental body hits; older project files
/// are eligible before any top-N limit is applied. Query terms are bounded to
/// avoid scanning files repeatedly for an unbounded user-supplied query.
fn project_query_terms(prompt: &str) -> Vec<String> {
    let stop = [
        "about", "could", "explain", "from", "give", "have", "help",
        "into", "latest", "need", "please", "project", "show",
        "tell", "that", "them", "there", "these", "this", "what",
        "when", "where", "which", "with", "your",
    ];
    let mut terms = Vec::new();
    for word in prompt
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
    {
        if word.chars().count() < 3 || word.chars().count() > 80 || stop.contains(&word) {
            continue;
        }
        if !terms.iter().any(|existing| existing == word) {
            terms.push(word.to_string());
            if terms.len() >= 16 {
                break;
            }
        }
    }
    terms
}

fn project_file_relevance(name: &str, content: &str, terms: &[String]) -> usize {
    if terms.is_empty() {
        return 0;
    }
    let name = name.to_lowercase();
    let text = content.to_lowercase();
    let mut score = 0usize;
    for term in terms {
        if name.contains(term) {
            score += 20;
        }
        if text.contains(term) {
            score += 3;
        }
    }
    score
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
        sha256: row.get(9)?,
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
    fn binary_attachment_is_verified_project_scoped_and_deleted_with_its_project() {
        let database = Database::in_memory().unwrap();
        let repo = ProjectRepository::new(&database);
        let first = repo.create_project("Archives").unwrap();
        let second = repo.create_project("Other").unwrap();
        let original = vec![0, 255, 0x89, 0x50, 0x4e, 0x47, 0, 0, 19];
        let record = repo
            .add_binary_file(&ProjectBinaryFileInput {
                project_id: first.id.clone(),
                name: "diagram.png".to_string(),
                size_bytes: original.len() as i64,
                mime_type: Some("image/png".to_string()),
                data_base64: STANDARD.encode(&original),
            })
            .unwrap();
        assert_eq!(record.status, "attached");
        assert_eq!(record.content_text, None);
        let expected_sha = format!("{:x}", Sha256::digest(&original));
        assert_eq!(record.sha256.as_deref(), Some(expected_sha.as_str()));
        let listed = repo.list_projects(None, 50).unwrap();
        let json = serde_json::to_string(&listed).unwrap();
        assert!(!json.contains("diagram.png"));
        assert!(!json.contains("iVBOR"));
        let selected = repo.find_project(&first.id).unwrap();
        assert_eq!(selected.files.len(), 1);
        assert_eq!(selected.files[0].status, "attached");
        let manifest = serde_json::to_string(&selected).unwrap();
        assert!(!manifest.contains("dataBase64"));
        assert!(!manifest.contains("contentBlob"));

        let copy = repo.read_binary_file(&first.id, &record.id).unwrap();
        assert_eq!(STANDARD.decode(copy.data_base64).unwrap(), original);
        assert_eq!(copy.name, "diagram.png");
        assert_eq!(copy.sha256, record.sha256.unwrap());
        assert!(repo.read_binary_file(&second.id, &record.id).is_err());

        // A corrupted payload is never returned to users as a verified copy.
        database
            .connection()
            .execute(
                "UPDATE project_files SET content_blob = ?1 WHERE id = ?2",
                params![vec![1u8; 9], record.id],
            )
            .unwrap();
        assert!(repo
            .read_binary_file(&first.id, &record.id)
            .unwrap_err()
            .to_string()
            .contains("integrity"));
        repo.delete_project(&first.id).unwrap();
        let leftovers: i64 = database
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM project_files WHERE id = ?1",
                params![record.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(leftovers, 0, "cascading project deletion must remove binary blobs");
        assert!(repo.read_binary_file(&first.id, &record.id).is_err());
    }

    #[test]
    fn binary_upload_rejects_oversized_invalid_and_path_like_inputs() {
        let database = Database::in_memory().unwrap();
        let repo = ProjectRepository::new(&database);
        let project = repo.create_project("Checks").unwrap();
        let mut input = ProjectBinaryFileInput {
            project_id: project.id.clone(),
            name: "../outside.png".to_string(),
            size_bytes: 3,
            mime_type: Some("image/png".to_string()),
            data_base64: STANDARD.encode([1u8, 2, 3]),
        };
        assert!(repo.add_binary_file(&input).is_err());
        input.name = "valid.png".to_string();
        input.size_bytes = 4;
        assert!(repo.add_binary_file(&input).is_err(), "claimed size mismatch");
        input.size_bytes = 3;
        input.data_base64 = "%%%invalid%%%".to_string();
        assert!(repo.add_binary_file(&input).is_err(), "invalid base64");
        input.size_bytes = MAX_PROJECT_BINARY_BYTES as i64 + 1;
        input.data_base64 = "AA==".to_string();
        assert!(repo.add_binary_file(&input).is_err(), "oversized declared length");
        input.size_bytes = 3;
        input.data_base64 = "A".repeat(MAX_PROJECT_BINARY_BYTES.div_ceil(3) * 4 + 1);
        assert!(repo.add_binary_file(&input).is_err(), "oversized encoded payload");
        assert!(repo.find_project(&project.id).unwrap().files.is_empty());
    }

    #[test]
    fn attached_bytes_survive_portable_move_and_database_backup() {
        let dir = tempfile::tempdir().unwrap();
        let old_root = crate::portable_root::PortableRootManager::from_root(
            dir.path().join("old-portable-root"),
        );
        old_root.ensure_directories().unwrap();
        let database = Database::open(old_root.database_path()).unwrap();
        let repo = ProjectRepository::new(&database);
        let project = repo.create_project("Portable").unwrap();
        let original: Vec<u8> = (0..=255).cycle().take(64_000).collect();
        let record = repo
            .add_binary_file(&ProjectBinaryFileInput {
                project_id: project.id.clone(),
                name: "record.bin".to_string(),
                size_bytes: original.len() as i64,
                mime_type: Some("application/octet-stream".to_string()),
                data_base64: STANDARD.encode(&original),
            })
            .unwrap();
        // Only the database connection needs explicit closure before moving
        // the portable root; the repository is a lightweight borrowed view.
        drop(database);
        let new_root = crate::portable_root::PortableRootManager::from_root(
            dir.path().join("relocated-root"),
        );
        std::fs::rename(old_root.root(), new_root.root()).unwrap();
        let relocated_database = Database::open(new_root.database_path()).unwrap();
        let copied = ProjectRepository::new(&relocated_database)
            .read_binary_file(&project.id, &record.id)
            .unwrap();
        assert_eq!(STANDARD.decode(copied.data_base64).unwrap(), original);

        let backup = crate::maintenance::backup_database(&new_root, &relocated_database).unwrap();
        let snapshot = new_root
            .resolve_relative("backups")
            .unwrap()
            .join(backup.name);
        let recovered = Database::open(snapshot).unwrap();
        let restored_copy = ProjectRepository::new(&recovered)
            .read_binary_file(&project.id, &record.id)
            .unwrap();
        assert_eq!(STANDARD.decode(restored_copy.data_base64).unwrap(), original);
    }

    #[test]
    fn large_project_catalog_is_paged_without_serializing_file_contents() {
        let database = Database::in_memory().unwrap();
        let repo = ProjectRepository::new(&database);
        let huge_text = "LARGE_PRIVATE_CONTENT_NOT_FOR_SIDEBAR ".repeat(3_000);
        let mut created_ids = Vec::new();
        for index in 0..125 {
            let project = repo
                .create_project(&format!("Project {index:03}"))
                .unwrap();
            database
                .connection()
                .execute(
                    "INSERT INTO project_files
                     (id, project_id, name, size_bytes, mime_type, content_text, status, added_at)
                     VALUES (?1, ?2, 'long-notes.md', ?3, 'text/markdown', ?4, 'ready', '2026-01-01')",
                    params![
                        Uuid::new_v4().to_string(),
                        project.id,
                        huge_text.len() as i64,
                        huge_text,
                    ],
                )
                .unwrap();
            database
                .connection()
                .execute(
                    "UPDATE projects SET updated_at = ?1 WHERE id = ?2",
                    params!["2026-01-01T00:00:00+00:00", project.id],
                )
                .unwrap();
            created_ids.push(project.id);
        }
        let chat_id = Uuid::new_v4().to_string();
        insert_conversation(&database, &chat_id, "Linked chat");
        database
            .connection()
            .execute(
                "INSERT INTO project_conversations (project_id, conversation_id, created_at)
                 VALUES (?1, ?2, '2026-01-01')",
                params![created_ids[0], chat_id],
            )
            .unwrap();

        let mut cursor = None;
        let mut seen = std::collections::HashSet::new();
        let mut pages = 0;
        loop {
            let page = repo.list_projects(cursor.as_deref(), 50).unwrap();
            assert!(page.projects.len() <= 50);
            pages += 1;
            let serialized = serde_json::to_string(&page).unwrap();
            assert!(!serialized.contains("LARGE_PRIVATE_CONTENT_NOT_FOR_SIDEBAR"));
            assert!(!serialized.contains("long-notes.md"));
            assert!(
                serialized.len() < 16_000,
                "catalog unexpectedly included large files"
            );
            for summary in page.projects {
                assert!(
                    seen.insert(summary.id.clone()),
                    "duplicate from timestamp-tied pages"
                );
                assert_eq!(summary.file_count, 1);
                assert_eq!(
                    summary.conversation_count,
                    if summary.id == created_ids[0] { 1 } else { 0 }
                );
            }
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
            assert!(pages < 5, "catalog cursor did not reach the final page");
        }
        assert_eq!(pages, 3);
        assert_eq!(seen.len(), 125);

        // Only a requested detail query can expose the stored text, preserving
        // existing project chat-context behavior and metadata.
        let selected = repo.find_project(&created_ids[0]).unwrap();
        assert_eq!(selected.conversation_ids, vec![chat_id]);
        assert_eq!(
            selected.files[0].content_text.as_deref(),
            Some(huge_text.as_str())
        );
        assert!(repo
            .list_projects(Some("bad cursor"), 50)
            .is_err());
        assert!(repo
            .list_projects(Some("2026-01-01T00:00:00Z|not-a-uuid"), 50)
            .is_err());
        assert_eq!(
            repo.list_projects(None, 100_000).unwrap().projects.len(),
            100
        );
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
        let listed = repo.list_projects(None, 50).unwrap();
        assert_eq!(listed.projects[0].conversation_count, 1);
        assert_eq!(listed.projects[0].file_count, 1);
        let selected = repo.find_project(&project.id).unwrap();
        assert_eq!(selected.conversation_ids, vec!["c1"]);
        assert_eq!(selected.files[0].name, "brief.md");
        let context = project_context_message(&selected).unwrap();
        assert!(context.contains("Ship the polished version."));
        repo.unlink_conversation(&project.id, "c1").unwrap();
        repo.delete_project(&project.id).unwrap();
        assert!(repo.list_projects(None, 50).unwrap().projects.is_empty());
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
    fn older_ninth_project_file_is_retrieved_for_relevant_question() {
        let database = Database::in_memory().unwrap();
        let repo = ProjectRepository::new(&database);
        let project = repo.create_project("Architecture").unwrap();

        for index in 0..12 {
            let old = index == 0;
            let name = if old {
                "legacy-auth-rollback.md".to_string()
            } else {
                format!("new-module-{index:02}.txt")
            };
            let content = if old {
                "AUTH_ROLLBACK_MARKER: the legacy authentication plan uses a reversible token swap."
                    .to_string()
            } else {
                format!("UNRELATED_MARKER_{index} ordinary local module content")
            };
            let saved = repo
                .add_file(&ProjectFileInput {
                    project_id: project.id.clone(),
                    name,
                    size_bytes: content.len() as i64,
                    mime_type: Some("text/plain".into()),
                    content_text: Some(content),
                    status: "ready".into(),
                    error: None,
                })
                .unwrap();
            database
                .connection()
                .execute(
                    "UPDATE project_files SET added_at = ?1 WHERE id = ?2",
                    params![format!("2026-01-01T00:00:{index:02}Z"), saved.id],
                )
                .unwrap();
        }

        let snapshot = repo.find_project(&project.id).unwrap();
        assert_eq!(snapshot.files.len(), 12);
        // Prove why the old fixed newest-first take(8) was incorrect.
        let generic = project_context_message(&snapshot).unwrap();
        assert!(!generic.contains("AUTH_ROLLBACK_MARKER"));

        let relevant = project_context_message_for_prompt(
            &snapshot,
            Some("How does the legacy auth rollback plan work?"),
        )
        .unwrap();
        assert!(relevant.contains("AUTH_ROLLBACK_MARKER"));
        assert!(relevant.contains("legacy-auth-rollback.md"));
        assert!(relevant.contains("8 of 12 ready text files included"));
        assert!(relevant.contains("4 omitted"));
        assert!(relevant.contains("Ready means stored and searchable"));
        assert!(relevant.chars().count() <= 17_000);
    }

    #[test]
    fn changed_question_reselects_stored_evidence_and_delete_is_immediate() {
        let database = Database::in_memory().unwrap();
        let repo = ProjectRepository::new(&database);
        let project = repo.create_project("Research").unwrap();
        let first = repo
            .add_file(&ProjectFileInput {
                project_id: project.id.clone(),
                name: "older-robotics.txt".into(),
                size_bytes: 30,
                mime_type: Some("text/plain".into()),
                content_text: Some("ROBOTICS_UNIQUE_MARKER navigation control".into()),
                status: "ready".into(),
                error: None,
            })
            .unwrap();
        for index in 0..10 {
            let body = format!("DATASET_MARKER_{index} ordinary notes");
            repo.add_file(&ProjectFileInput {
                project_id: project.id.clone(),
                name: format!("dataset-{index}.txt"),
                size_bytes: body.len() as i64,
                mime_type: Some("text/plain".into()),
                content_text: Some(body),
                status: "ready".into(),
                error: None,
            })
            .unwrap();
        }
        let project = repo.find_project(&project.id).unwrap();
        let robot = project_context_message_for_prompt(&project, Some("robotics navigation"))
            .unwrap();
        assert!(robot.contains("ROBOTICS_UNIQUE_MARKER"));
        let data = project_context_message_for_prompt(&project, Some("dataset-9 notes"))
            .unwrap();
        assert!(data.contains("DATASET_MARKER_9"));

        repo.delete_file(&project.id, &first.id).unwrap();
        let latest = repo.find_project(&project.id).unwrap();
        let after_delete =
            project_context_message_for_prompt(&latest, Some("robotics navigation")).unwrap();
        assert!(!after_delete.contains("ROBOTICS_UNIQUE_MARKER"));

        let replacement = repo
            .add_file(&ProjectFileInput {
                project_id: project.id.clone(),
                name: "robotics-revised.txt".into(),
                size_bytes: 24,
                mime_type: Some("text/plain".into()),
                content_text: Some("REVISED_ROBOTICS_MARKER updated sensors".into()),
                status: "ready".into(),
                error: None,
            })
            .unwrap();
        let updated = repo.find_project(&project.id).unwrap();
        let new_context =
            project_context_message_for_prompt(&updated, Some("robotics sensors")).unwrap();
        assert!(new_context.contains("REVISED_ROBOTICS_MARKER"));
        assert!(!new_context.contains("ROBOTICS_UNIQUE_MARKER"));
        assert_eq!(replacement.status, "ready");
    }

    #[test]
    fn project_context_stays_within_budget_and_discloses_omitted_large_files() {
        let database = Database::in_memory().unwrap();
        let repo = ProjectRepository::new(&database);
        let project = repo.create_project("Bounded Context").unwrap();
        for index in 0..16 {
            let body = format!("VECTOR_{index} {}", "λ".repeat(20_000));
            repo.add_file(&ProjectFileInput {
                project_id: project.id.clone(),
                name: format!("vector-{index}.txt"),
                size_bytes: body.len() as i64,
                mime_type: Some("text/plain".into()),
                content_text: Some(body),
                status: "ready".into(),
                error: None,
            })
            .unwrap();
        }
        let project = repo.find_project(&project.id).unwrap();
        let context = project_context_message_for_prompt(&project, Some("vector")).unwrap();
        let context_section = context.split_once("Project files:\n").unwrap().1;
        assert!(context_section.chars().count() <= 16_000);
        assert!(context_section.contains("omitted"));
        assert!(context_section.contains("[truncated]"));
        assert!(context.contains('λ'));
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
