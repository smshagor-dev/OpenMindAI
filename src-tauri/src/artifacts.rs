#[cfg(test)]
use std::fs;
use std::path::{Path, PathBuf};

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

/// One bounded Library result page. A cursor refers to the final row on the
/// current page, ordered by creation timestamp and artifact ID for stable ties.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryPage {
    pub entries: Vec<LibraryEntry>,
    pub next_cursor: Option<String>,
    pub total_count: i64,
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

    /// Keep an active queued or rendering image fresh for crash recovery.
    /// This must not modify a finished image or another artifact kind.
    pub fn touch_generating_image(&self, id: &str) -> Result<bool, AppError> {
        let updated = self.database.connection().execute(
            "UPDATE artifacts SET updated_at = ?1
             WHERE id = ?2 AND kind = 'image' AND status = 'generating'",
            rusqlite::params![Utc::now().to_rfc3339(), id],
        )?;
        Ok(updated != 0)
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

    /// Server-side filtered, keyset-paginated artifact listing. Applying filters
    /// before LIMIT prevents older PDFs from disappearing behind newer images.
    /// The cursor is a validated (timestamp, UUID) pair, never interpolated SQL.
    pub fn list_page(
        &self,
        kind: Option<&str>,
        search: Option<&str>,
        cursor: Option<&str>,
        requested_limit: i64,
    ) -> Result<LibraryPage, AppError> {
        let limit = requested_limit.clamp(1, 100);
        let kind = kind.filter(|value| !value.is_empty() && *value != "all");
        if kind.is_some_and(|value| {
            !matches!(
                value,
                "text" | "markdown" | "pdf" | "docx" | "code" | "image" | "video" | "audio"
            )
        }) {
            return Err(AppError::internal("invalid library artifact kind"));
        }
        let search = search.map(str::trim).filter(|value| !value.is_empty());
        if search.is_some_and(|value| value.chars().count() > 256) {
            return Err(AppError::internal("library search exceeds 256 characters"));
        }
        let cursor = match cursor {
            Some(raw) => {
                let (created_at, id) = raw
                    .split_once('|')
                    .ok_or_else(|| AppError::internal("invalid library cursor"))?;
                if raw.len() > 128
                    || chrono::DateTime::parse_from_rfc3339(created_at).is_err()
                    || Uuid::parse_str(id).is_err()
                {
                    return Err(AppError::internal("invalid library cursor"));
                }
                Some((created_at, id))
            }
            None => None,
        };
        let cursor_time = cursor.map(|value| value.0);
        let cursor_id = cursor.map(|value| value.1);

        let total_count = self.database.connection().query_row(
            "SELECT COUNT(*)
             FROM artifacts AS a
             JOIN conversations AS c ON c.id = a.conversation_id
             WHERE (?1 IS NULL OR a.kind = ?1)
               AND (?2 IS NULL OR instr(lower(a.name), lower(?2)) > 0
                    OR instr(lower(c.title), lower(?2)) > 0)",
            rusqlite::params![kind, search],
            |row| row.get::<_, i64>(0),
        )?;

        let mut statement = self.database.connection().prepare(
            "SELECT a.id, a.conversation_id, c.title, a.message_id,
                    a.name, a.path, a.mime_type, a.kind, a.size_bytes,
                    a.page_count, a.status, a.error, a.created_at, a.updated_at
             FROM artifacts AS a
             JOIN conversations AS c ON c.id = a.conversation_id
             WHERE (?1 IS NULL OR a.kind = ?1)
               AND (?2 IS NULL OR instr(lower(a.name), lower(?2)) > 0
                    OR instr(lower(c.title), lower(?2)) > 0)
               AND (?3 IS NULL OR a.created_at < ?3
                    OR (a.created_at = ?3 AND a.id < ?4))
             ORDER BY a.created_at DESC, a.id DESC
             LIMIT ?5",
        )?;
        let rows = statement.query_map(
            rusqlite::params![kind, search, cursor_time, cursor_id, limit + 1],
            map_library_entry,
        )?;
        let mut entries = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)?;
        let has_more = entries.len() > limit as usize;
        entries.truncate(limit as usize);
        let next_cursor = if has_more {
            entries
                .last()
                .map(|entry| format!("{}|{}", entry.created_at, entry.id))
        } else {
            None
        };
        Ok(LibraryPage {
            entries,
            next_cursor,
            total_count,
        })
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

struct ArtifactDestination {
    directory: cap_std::fs::Dir,
    path: PathBuf,
    filename: String,
    relative: String,
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
        let destination = self.reserve_destination(kind, filename_hint, fallback_title)?;
        Ok((destination.path, destination.filename, destination.relative))
    }

    /// Renderers receive a private staging path and retain a confined destination
    /// handle from reservation through publication; no portable path is reopened.
    pub fn stage_destination(
        &self,
        kind: &str,
        filename_hint: Option<&str>,
        fallback_title: &str,
    ) -> Result<(crate::artifact_output::StagedOutput, String, String), AppError> {
        let destination = self.reserve_destination(kind, filename_hint, fallback_title)?;
        let output = crate::artifact_output::StagedOutput::from_reserved(
            destination.directory,
            &destination.filename,
        )?;
        Ok((output, destination.filename, destination.relative))
    }

    fn reserve_destination(
        &self,
        kind: &str,
        filename_hint: Option<&str>,
        fallback_title: &str,
    ) -> Result<ArtifactDestination, AppError> {
        let subdir = match kind {
            "pdf" | "docx" => "generated/exports",
            "image" => "generated/images",
            "audio" => "generated/audio",
            "video" => "generated/video",
            _ => "generated/files",
        };

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

        let (directory, reservation) =
            crate::artifact_output::reserve_directory(self.root.root(), Path::new(subdir))?;
        let filename = format!("{stem}.{extension}");
        let path = self
            .root
            .root()
            .join(subdir)
            .join(&reservation)
            .join(&filename);
        let relative = format!("{subdir}/{reservation}/{filename}");
        Ok(ArtifactDestination {
            directory,
            path,
            filename,
            relative,
        })
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
    fn image_heartbeat_only_refreshes_unfinished_images() {
        let database = Database::in_memory().unwrap();
        let conversation = crate::chat::ChatRepository::new(&database)
            .create_conversation(Some("Image lease"))
            .unwrap();
        let repo = ArtifactRepository::new(&database);
        let running = repo
            .create(
                &conversation.id,
                None,
                "waiting.png",
                "generated/images/waiting.png",
                "image/png",
                "image",
                "generating",
            )
            .unwrap();
        let ready = repo
            .create(
                &conversation.id,
                None,
                "ready.png",
                "generated/images/ready.png",
                "image/png",
                "image",
                "generating",
            )
            .unwrap();
        let document = repo
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

        let earlier = (Utc::now() - chrono::Duration::hours(5)).to_rfc3339();
        database
            .connection()
            .execute(
                "UPDATE artifacts SET updated_at = ?1
                 WHERE id IN (?2, ?3, ?4)",
                rusqlite::params![earlier, running.id, ready.id, document.id],
            )
            .unwrap();
        repo.set_ready(&ready.id, 100, None).unwrap();

        assert!(repo.touch_generating_image(&running.id).unwrap());
        assert!(!repo.touch_generating_image(&ready.id).unwrap());
        assert!(!repo.touch_generating_image(&document.id).unwrap());
        assert!(!repo.touch_generating_image("unknown").unwrap());
        assert_ne!(repo.find(&running.id).unwrap().updated_at, earlier);
        assert_eq!(repo.find(&running.id).unwrap().status, "generating");
        assert_eq!(repo.find(&document.id).unwrap().updated_at, earlier);
        assert_eq!(repo.find(&ready.id).unwrap().status, "ready");
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

        let page = artifacts.list_page(None, None, None, 10).unwrap();
        let entries = page.entries;
        assert_eq!(page.total_count, 2);
        assert_eq!(entries.len(), 2);
        // Newest first: `second` was created after `first`.
        assert_eq!(entries[0].id, second.id);
        assert_eq!(entries[0].conversation_title, "Export Chat");
        assert_eq!(entries[1].id, first.id);
        assert_eq!(entries[1].conversation_title, "Report Chat");
    }

    #[test]
    fn library_paginates_past_200_and_filters_old_documents_before_limit() {
        let database = Database::in_memory().unwrap();
        let chat_repo = crate::chat::ChatRepository::new(&database);
        let conversation = chat_repo.create_conversation(Some("Archive Chat")).unwrap();
        let artifacts = ArtifactRepository::new(&database);

        for index in 0..225 {
            let is_pdf = index == 0 || index == 11;
            let name = if is_pdf {
                format!("archive-{index:03}.pdf")
            } else {
                format!("image-{index:03}.png")
            };
            let kind = if is_pdf { "pdf" } else { "image" };
            let saved = artifacts
                .create(
                    &conversation.id,
                    None,
                    &name,
                    &format!("generated/files/{name}"),
                    if is_pdf {
                        "application/pdf"
                    } else {
                        "image/png"
                    },
                    kind,
                    "ready",
                )
                .unwrap();
            // Deterministic timestamps make the oldest documents appear well
            // after the first 200 newest rows, independent of runner speed.
            let created = format!(
                "2026-01-01T00:{:02}:{:02}.000+00:00",
                index / 60,
                index % 60
            );
            database
                .connection()
                .execute(
                    "UPDATE artifacts SET created_at = ?1 WHERE id = ?2",
                    rusqlite::params![created, saved.id],
                )
                .unwrap();
        }

        let mut cursor = None;
        let mut ids = std::collections::HashSet::new();
        let mut seen = Vec::new();
        let mut page_count = 0;
        loop {
            let page = artifacts
                .list_page(None, None, cursor.as_deref(), 50)
                .unwrap();
            assert_eq!(page.total_count, 225);
            assert!(page.entries.len() <= 50);
            page_count += 1;
            for entry in page.entries {
                assert!(
                    ids.insert(entry.id),
                    "a Library cursor returned a duplicate"
                );
                seen.push(entry.name);
            }
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
            assert!(page_count < 10, "Library pagination never reached its end");
        }
        assert_eq!(page_count, 5);
        assert_eq!(ids.len(), 225);
        assert_eq!(seen.first().unwrap(), "image-224.png");
        assert_eq!(seen.last().unwrap(), "archive-000.pdf");

        let pdfs = artifacts.list_page(Some("pdf"), None, None, 50).unwrap();
        assert_eq!(pdfs.total_count, 2);
        assert_eq!(pdfs.entries.len(), 2);
        assert!(pdfs.next_cursor.is_none());
        assert_eq!(pdfs.entries[0].name, "archive-011.pdf");
        assert_eq!(pdfs.entries[1].name, "archive-000.pdf");

        let searched = artifacts
            .list_page(Some("pdf"), Some("archive-000"), None, 50)
            .unwrap();
        assert_eq!(searched.total_count, 1);
        assert_eq!(searched.entries[0].name, "archive-000.pdf");
        let by_chat = artifacts
            .list_page(Some("pdf"), Some("ARCHIVE CHAT"), None, 50)
            .unwrap();
        assert_eq!(by_chat.total_count, 2);
        assert!(artifacts
            .list_page(Some("pdf"), Some("missing"), None, 50)
            .unwrap()
            .entries
            .is_empty());
    }

    #[test]
    fn library_cursor_is_stable_for_identical_creation_timestamps() {
        let database = Database::in_memory().unwrap();
        let chat = crate::chat::ChatRepository::new(&database)
            .create_conversation(Some("Same timestamp"))
            .unwrap();
        let artifacts = ArtifactRepository::new(&database);

        for index in 0..34 {
            let saved = artifacts
                .create(
                    &chat.id,
                    None,
                    &format!("item-{index}.md"),
                    &format!("generated/files/item-{index}.md"),
                    "text/markdown",
                    "markdown",
                    "ready",
                )
                .unwrap();
            database
                .connection()
                .execute(
                    "UPDATE artifacts SET created_at = ?1 WHERE id = ?2",
                    rusqlite::params!["2026-01-01T00:00:00.000+00:00", saved.id],
                )
                .unwrap();
        }
        let mut cursor = None;
        let mut ids = std::collections::HashSet::new();
        loop {
            let page = artifacts
                .list_page(Some("markdown"), None, cursor.as_deref(), 7)
                .unwrap();
            assert_eq!(page.total_count, 34);
            assert!(!page.entries.is_empty());
            for entry in page.entries {
                assert!(ids.insert(entry.id), "duplicate item with same timestamp");
            }
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert_eq!(ids.len(), 34);
        assert!(artifacts
            .list_page(None, None, Some("unsafe-cursor"), 50)
            .is_err());
        assert!(artifacts
            .list_page(None, None, Some("2026-01-01T00:00:00Z|not-a-uuid"), 50)
            .is_err());
        assert!(artifacts
            .list_page(None, Some(&"a".repeat(257)), None, 50)
            .is_err());
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
        assert!(relative1.starts_with("generated/files/"));
        assert!(relative1.ends_with("/Project-Report.md"));

        let (path2, filename2, _) = manager
            .resolve_destination("markdown", Some("Project Report!!.md"), "fallback")
            .unwrap();
        assert_eq!(filename2, "Project-Report.md");
        assert_ne!(path1, path2);
        assert!(!path1.exists());
        assert!(!path2.exists());
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
        assert!(relative.starts_with("generated/exports/"));
        assert!(relative.ends_with("/Quarterly-Research-Summary.pdf"));
    }
    #[test]
    fn rendered_destinations_keep_private_staging_and_portable_metadata_separate() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let manager = ArtifactManager::new(&root);
        let (output, filename, relative) = manager
            .stage_destination("pdf", Some("Project Report!!.pdf"), "fallback")
            .unwrap();
        assert_eq!(filename, "Project-Report.pdf");
        assert!(relative.starts_with("generated/exports/"));
        assert!(relative.ends_with("/Project-Report.pdf"));
        assert_eq!(output.path.file_name().unwrap(), filename.as_str());
        assert!(!output.path.starts_with(root.root()));
        let destination = root.resolve_relative(&relative).unwrap();
        assert!(!destination.exists());
        fs::write(&output.path, b"%PDF-fixture").unwrap();
        assert_eq!(output.commit_for("pdf").unwrap(), 12);
        assert_eq!(fs::read(&destination).unwrap(), b"%PDF-fixture");
        let private = output.path.clone();
        drop(output);
        assert!(!private.exists());
        assert!(destination.is_file());
    }

    #[test]
    fn reserved_text_outputs_publish_through_portable_capabilities() {
        let dir = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(dir.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let manager = ArtifactManager::new(&root);
        let (first, _, relative) = manager
            .resolve_destination("text", Some("same.txt"), "same")
            .unwrap();
        root.write_new_relative(&relative, b"first completed text")
            .unwrap();
        assert!(root.write_new_relative(&relative, b"overwrite").is_err());
        assert_eq!(fs::read(&first).unwrap(), b"first completed text");
        assert_eq!(fs::read_dir(first.parent().unwrap()).unwrap().count(), 1);
        let (second, _, relative) = manager
            .resolve_destination("text", Some("same.txt"), "same")
            .unwrap();
        root.write_new_relative(&relative, b"second completed text")
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read(second).unwrap(), b"second completed text");
        assert_eq!(fs::read(first).unwrap(), b"first completed text");
    }
}
