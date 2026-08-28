use std::{fs, path::PathBuf};

use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use crate::{app_error::AppError, database::Database, portable_root::PortableRootManager};

#[path = "media_preflight.rs"]
pub(crate) mod media_preflight;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub id: String,
    pub conversation_id: String,
    pub message_id: Option<String>,
    pub name: String,
    pub path: String,
    pub mime_type: String,
    pub kind: String,
    pub size_bytes: i64,
    pub page_count: Option<i64>,
    pub status: String,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryEntry {
    pub id: String,
    pub conversation_id: String,
    pub conversation_title: String,
    pub message_id: Option<String>,
    pub name: String,
    pub path: String,
    pub mime_type: String,
    pub kind: String,
    pub size_bytes: i64,
    pub page_count: Option<i64>,
    pub status: String,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub struct ArtifactRepository<'a> {
    database: &'a Database,
}

impl<'a> ArtifactRepository<'a> {
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &self,
        conversation_id: &str,
        message_id: Option<&str>,
        name: &str,
        path: &str,
        mime_type: &str,
        kind: &str,
        status: &str,
    ) -> Result<Artifact, AppError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        self.database.connection().execute(
            "INSERT INTO artifacts
             (id, conversation_id, message_id, name, path, mime_type, kind, size_bytes, page_count, status, error, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, NULL, ?8, NULL, ?9, ?9)",
            rusqlite::params![id, conversation_id, message_id, name, path, mime_type, kind, status, now],
        )?;
        self.find(&id)
    }

    pub fn set_ready(
        &self,
        id: &str,
        size_bytes: i64,
        page_count: Option<i64>,
    ) -> Result<(), AppError> {
        self.database.connection().execute(
            "UPDATE artifacts SET status = 'ready', size_bytes = ?1, page_count = ?2, error = NULL, updated_at = ?3 WHERE id = ?4",
            rusqlite::params![size_bytes, page_count, Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    pub fn set_failed(&self, id: &str, error: &str) -> Result<(), AppError> {
        self.database.connection().execute(
            "UPDATE artifacts SET status = 'failed', error = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![error, Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    pub fn list_for_conversation(&self, conversation_id: &str) -> Result<Vec<Artifact>, AppError> {
        let mut statement = self.database.connection().prepare(
            "SELECT id, conversation_id, message_id, name, path, mime_type, kind, size_bytes,
                    page_count, status, error, created_at, updated_at
             FROM artifacts
             WHERE conversation_id = ?1
             ORDER BY created_at ASC",
        )?;
        let rows = statement.query_map(rusqlite::params![conversation_id], map_artifact)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    pub fn find(&self, id: &str) -> Result<Artifact, AppError> {
        self.database
            .connection()
            .query_row(
                "SELECT id, conversation_id, message_id, name, path, mime_type, kind, size_bytes,
                        page_count, status, error, created_at, updated_at
                 FROM artifacts WHERE id = ?1",
                rusqlite::params![id],
                map_artifact,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => AppError::ArtifactNotFound(id.to_string()),
                other => AppError::from(other),
            })
    }

    /// Lists artifacts across every conversation, newest first, joined with the owning
    /// conversation's title for display in the Library view.
    pub fn list_all(&self, limit: i64) -> Result<Vec<LibraryEntry>, AppError> {
        let mut statement = self.database.connection().prepare(
            "SELECT artifacts.id, artifacts.conversation_id, conversations.title, artifacts.message_id,
                    artifacts.name, artifacts.path, artifacts.mime_type, artifacts.kind, artifacts.size_bytes,
                    artifacts.page_count, artifacts.status, artifacts.error, artifacts.created_at, artifacts.updated_at
             FROM artifacts
             JOIN conversations ON conversations.id = artifacts.conversation_id
             ORDER BY artifacts.created_at DESC
             LIMIT ?1",
        )?;
        let rows = statement.query_map(rusqlite::params![limit], map_library_entry)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }
}

fn map_artifact(row: &rusqlite::Row<'_>) -> rusqlite::Result<Artifact> {
    Ok(Artifact {
        id: row.get(0)?,
        conversation_id: row.get(1)?,
        message_id: row.get(2)?,
        name: row.get(3)?,
        path: row.get(4)?,
        mime_type: row.get(5)?,
        kind: row.get(6)?,
        size_bytes: row.get(7)?,
        page_count: row.get(8)?,
        status: row.get(9)?,
        error: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn map_library_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<LibraryEntry> {
    Ok(LibraryEntry {
        id: row.get(0)?,
        conversation_id: row.get(1)?,
        conversation_title: row.get(2)?,
        message_id: row.get(3)?,
        name: row.get(4)?,
        path: row.get(5)?,
        mime_type: row.get(6)?,
        kind: row.get(7)?,
        size_bytes: row.get(8)?,
        page_count: row.get(9)?,
        status: row.get(10)?,
        error: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

pub struct ArtifactManager<'a> {
    root: &'a PortableRootManager,
}

impl<'a> ArtifactManager<'a> {
    pub fn new(root: &'a PortableRootManager) -> Self {
        Self { root }
    }

    /// Resolves a collision-free destination path for a new artifact and returns
    /// (absolute path, final filename, path relative to the portable root).
    pub fn resolve_destination(
        &self,
        kind: &str,
        filename_hint: Option<&str>,
        fallback_title: &str,
    ) -> Result<(PathBuf, String, String), AppError> {
        if let Some(report) = media_preflight::preflight(self.root, kind)? {
            report.ensure_ready()?;
        }

        let subdir = match kind {
            "pdf" | "docx" => "generated/exports",
            "image" => "generated/images",
            "audio" => "generated/audio",
            "video" => "generated/video",
            _ => "generated/files",
        };
        let dir = self.root.resolve_relative(subdir)?;
        fs::create_dir_all(&dir)?;

        let (stem, extension) = match filename_hint.filter(|hint| !hint.trim().is_empty()) {
            Some(hint) => split_stem_extension(hint, kind),
            None => (
                sanitize_filename(fallback_title),
                default_extension(kind).to_string(),
            ),
        };
        let stem = if stem.is_empty() {
            "artifact".to_string()
        } else {
            stem
        };

        let mut filename = format!("{stem}.{extension}");
        let mut counter = 1;
        while dir.join(&filename).exists() {
            filename = format!("{stem}-{counter}.{extension}");
            counter += 1;
        }
        let path = dir.join(&filename);
        let relative = format!("{subdir}/{filename}");
        Ok((path, filename, relative))
    }
}

pub fn mime_type_for(kind: &str) -> &'static str {
    match kind {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "markdown" => "text/markdown",
        "image" => "image/png",
        "audio" => "audio/wav",
        "video" => "video/webm",
        _ => "text/plain",
    }
}

fn default_extension(kind: &str) -> &'static str {
    match kind {
        "text" => "txt",
        "markdown" => "md",
        "pdf" => "pdf",
        "docx" => "docx",
        "image" => "png",
        "audio" => "wav",
        "video" => "webm",
        _ => "txt",
    }
}

fn split_stem_extension(hint: &str, kind: &str) -> (String, String) {
    // Split the raw hint into stem/extension *before* sanitizing, so the dash-collapsing
    // in `sanitize_filename` never runs across the extension boundary (it otherwise leaves a
    // stray trailing dash, e.g. "Report!!.md" -> "Report-.md" instead of "Report.md").
    let trimmed = hint.trim();
    if let Some(dot_index) = trimmed.rfind('.') {
        let (raw_stem, raw_ext) = trimmed.split_at(dot_index);
        let ext = sanitize_filename(raw_ext.trim_start_matches('.'));
        if !ext.is_empty() {
            return (sanitize_filename(raw_stem), ext);
        }
    }
    (
        sanitize_filename(trimmed),
        default_extension(kind).to_string(),
    )
}

fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let collapsed = cleaned
        .split(['-', ' '])
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    collapsed.trim_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_lists_and_updates_artifacts() {
        let database = Database::in_memory().unwrap();
        let repo = ArtifactRepository::new(&database);
        let conversation = crate::chat::ChatRepository::new(&database)
            .create_conversation(Some("Artifacts"))
            .unwrap();

        let artifact = repo
            .create(
                &conversation.id,
                None,
                "README.md",
                "generated/files/README.md",
                "text/markdown",
                "markdown",
                "generating",
            )
            .unwrap();
        assert_eq!(artifact.status, "generating");
        assert_eq!(artifact.size_bytes, 0);

        repo.set_ready(&artifact.id, 128, Some(3)).unwrap();
        let ready = repo.find(&artifact.id).unwrap();
        assert_eq!(ready.status, "ready");
        assert_eq!(ready.size_bytes, 128);
        assert_eq!(ready.page_count, Some(3));

        let listed = repo.list_for_conversation(&conversation.id).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, artifact.id);
    }

    #[test]
    fn set_failed_records_error() {
        let database = Database::in_memory().unwrap();
        let repo = ArtifactRepository::new(&database);
        let conversation = crate::chat::ChatRepository::new(&database)
            .create_conversation(Some("Artifacts"))
            .unwrap();
        let artifact = repo
            .create(
                &conversation.id,
                None,
                "report.pdf",
                "generated/exports/report.pdf",
                "application/pdf",
                "pdf",
                "generating",
            )
            .unwrap();

        repo.set_failed(&artifact.id, "renderer crashed").unwrap();
        let failed = repo.find(&artifact.id).unwrap();
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.error.as_deref(), Some("renderer crashed"));
    }

    #[test]
    fn find_missing_returns_not_found() {
        let database = Database::in_memory().unwrap();
        let repo = ArtifactRepository::new(&database);
        let error = repo.find("missing-id").unwrap_err();
        assert!(matches!(error, AppError::ArtifactNotFound(_)));
    }

    #[test]
    fn list_all_joins_conversation_titles_newest_first() {
        let database = Database::in_memory().unwrap();
        let chat_repo = crate::chat::ChatRepository::new(&database);
        let artifacts = ArtifactRepository::new(&database);

        let conversation_a = chat_repo.create_conversation(Some("Report Chat")).unwrap();
        let first = artifacts
            .create(
                &conversation_a.id,
                None,
                "readme.md",
                "generated/files/readme.md",
                "text/markdown",
                "markdown",
                "ready",
            )
            .unwrap();

        let conversation_b = chat_repo.create_conversation(Some("Export Chat")).unwrap();
        let second = artifacts
            .create(
                &conversation_b.id,
                None,
                "summary.pdf",
                "generated/exports/summary.pdf",
                "application/pdf",
                "pdf",
                "ready",
            )
            .unwrap();

        let entries = artifacts.list_all(10).unwrap();
        assert_eq!(entries.len(), 2);
        // Newest first: `second` was created after `first`.
        assert_eq!(entries[0].id, second.id);
        assert_eq!(entries[0].conversation_title, "Export Chat");
        assert_eq!(entries[1].id, first.id);
        assert_eq!(entries[1].conversation_title, "Report Chat");
    }

    #[test]
    fn resolves_collision_free_destination_with_sanitized_name() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let manager = ArtifactManager::new(&root);

        let (path1, filename1, relative1) = manager
            .resolve_destination("markdown", Some("Project Report!!.md"), "fallback")
            .unwrap();
        assert_eq!(filename1, "Project-Report.md");
        assert_eq!(relative1, "generated/files/Project-Report.md");
        fs::write(&path1, b"content").unwrap();

        let (path2, filename2, _) = manager
            .resolve_destination("markdown", Some("Project Report!!.md"), "fallback")
            .unwrap();
        assert_eq!(filename2, "Project-Report-1.md");
        assert_ne!(path1, path2);
    }

    #[test]
    fn falls_back_to_title_and_default_extension() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let manager = ArtifactManager::new(&root);

        let (_, filename, relative) = manager
            .resolve_destination("pdf", None, "Quarterly Research Summary")
            .unwrap();
        assert_eq!(filename, "Quarterly-Research-Summary.pdf");
        assert_eq!(relative, "generated/exports/Quarterly-Research-Summary.pdf");
    }
}