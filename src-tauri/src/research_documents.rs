use std::{fs, io::Read, path::PathBuf};

use chrono::Utc;
use lopdf::{Document, LoadOptions};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::State;
use uuid::Uuid;

use crate::{app_error::AppError, AppState};

const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
const MAX_EXTRACTED_CHARS: usize = 800_000;
const CHUNK_CHARS: usize = 2_400;
const CHUNK_OVERLAP: usize = 240;
const MAX_CHUNKS: usize = 1_000;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResearchDocument {
    pub id: String,
    pub name: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub content_sha256: String,
    pub chunk_count: usize,
    pub imported_at: String,
    pub extraction_metadata_json: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResearchFigureAsset {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub content_sha256: String,
    pub width_px: u32,
    pub height_px: u32,
    pub caption: String,
    pub alt_text: String,
    pub imported_at: String,
}

struct FigureInput {
    name: String,
    media_type: &'static str,
    bytes: Vec<u8>,
    sha256: String,
    width: u32,
    height: u32,
}

/// Immutable, integrity-checked source bytes captured for one export snapshot.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedResearchFigure {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
    pub sha256: String,
    pub width: u32,
    pub height: u32,
    pub caption: String,
    pub alt_text: String,
}

const MAX_FIGURE_BYTES: u64 = 5 * 1024 * 1024;

/// Resolve only the canonical `research-figure:<uuid>` form against the active
/// project. The bytes are copied and revalidated while the caller holds the DB
/// lock, giving the export a stable snapshot even if later edits occur.
pub(crate) fn resolve_research_figure(
    connection: &rusqlite::Connection,
    project_id: &str,
    asset_ref: &str,
) -> Result<ResolvedResearchFigure, AppError> {
    let id = asset_ref.strip_prefix("research-figure:").ok_or_else(|| {
        AppError::Internal("Use a registered figure reference (research-figure:<asset-id>).".into())
    })?;
    let parsed = Uuid::parse_str(id).map_err(|_| {
        AppError::Internal("Figure reference has an invalid or unsafe asset ID.".into())
    })?;
    if parsed.to_string() != id.to_ascii_lowercase() {
        return Err(AppError::Internal(
            "Figure asset ID is not canonical.".into(),
        ));
    }
    let row = connection.query_row(
        "SELECT id,name,media_type,content_blob,content_sha256,width_px,height_px,caption,alt_text,source_run_id
         FROM research_figure_assets WHERE id=?1 AND project_id=?2",
        params![id, project_id],
        |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,Vec<u8>>(3)?,row.get::<_,String>(4)?,row.get::<_,i64>(5)?,row.get::<_,i64>(6)?,row.get::<_,String>(7)?,row.get::<_,String>(8)?,row.get::<_,Option<String>>(9)?)),
    ).optional()?.ok_or_else(|| AppError::Internal("This figure is missing or does not belong to the active Research Studio project. Re-link a project-owned figure.".into()))?;
    let (id, name, media_type, bytes, sha256, width, height, caption, alt_text, source_run_id) =
        row;
    if bytes.is_empty()
        || bytes.len() as u64 > MAX_FIGURE_BYTES
        || format!("{:x}", Sha256::digest(&bytes)) != sha256
    {
        return Err(AppError::Internal("Registered figure bytes failed their SHA-256 or size integrity check; re-register the source file.".into()));
    }
    if let Some(run_id) = source_run_id.as_deref() {
        let belongs: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM research_runs WHERE id=?1 AND project_id=?2)",
            params![run_id, project_id],
            |row| row.get(0),
        )?;
        if !belongs {
            return Err(AppError::Internal(
                "Figure source run provenance no longer matches this project.".into(),
            ));
        }
    }
    let (actual_type, actual_width, actual_height) = match media_type.as_str() {
        "image/svg+xml" => {
            let (w, h) = validate_svg(&bytes)?;
            ("image/svg+xml", w, h)
        }
        "image/png" | "image/jpeg" => {
            let image = image::load_from_memory(&bytes)
                .map_err(|_| AppError::Internal("Registered raster figure is corrupted.".into()))?;
            let (w, h) = (image.width(), image.height());
            (
                if media_type == "image/png" {
                    "image/png"
                } else {
                    "image/jpeg"
                },
                w,
                h,
            )
        }
        _ => {
            return Err(AppError::Internal(
                "Registered figure has an unsupported MIME type.".into(),
            ))
        }
    };
    if actual_type != media_type
        || width != actual_width as i64
        || height != actual_height as i64
        || !(1..=4096).contains(&actual_width)
        || !(1..=4096).contains(&actual_height)
    {
        return Err(AppError::Internal(
            "Registered figure MIME type or dimensions do not match its source bytes.".into(),
        ));
    }
    Ok(ResolvedResearchFigure {
        id,
        project_id: project_id.to_string(),
        name,
        media_type,
        bytes,
        sha256,
        width: actual_width,
        height: actual_height,
        caption,
        alt_text,
    })
}

/// Capture the unique project-owned assets referenced by Markdown in first-use
/// order while the caller holds the authoritative database snapshot lock.
pub(crate) fn resolve_markdown_figures(
    connection: &rusqlite::Connection,
    project_id: &str,
    markdown: &str,
) -> Result<Vec<ResolvedResearchFigure>, AppError> {
    let mut options = pulldown_cmark::Options::empty();
    options.insert(pulldown_cmark::Options::ENABLE_TABLES);
    let mut seen = std::collections::HashSet::new();
    let mut figures = Vec::new();
    for event in pulldown_cmark::Parser::new_ext(markdown, options) {
        if let pulldown_cmark::Event::Start(pulldown_cmark::Tag::Image { dest_url, .. }) = event {
            let reference = dest_url.to_string();
            if seen.insert(reference.clone()) {
                figures.push(resolve_research_figure(connection, project_id, &reference)?);
            }
        }
    }
    Ok(figures)
}

fn figure_dimensions(value: &str) -> Option<u32> {
    let numeric = value.trim().trim_end_matches("px");
    let parsed = numeric.parse::<f64>().ok()?;
    (parsed.is_finite() && parsed >= 1.0 && parsed <= 4096.0).then_some(parsed.round() as u32)
}

fn validate_svg(bytes: &[u8]) -> Result<(u32, u32), AppError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| AppError::Internal("SVG must be UTF-8 XML.".into()))?;
    if text.to_ascii_lowercase().contains("<!doctype")
        || text.to_ascii_lowercase().contains("<!entity")
        || text.to_ascii_lowercase().contains("<?xml-stylesheet")
        || text.len() > MAX_FIGURE_BYTES as usize
    {
        return Err(AppError::Internal(
            "SVG DTD/entities are not supported.".into(),
        ));
    }
    let document = roxmltree::Document::parse(text)
        .map_err(|_| AppError::Internal("SVG XML is malformed.".into()))?;
    let root = document.root_element();
    if root.tag_name().name() != "svg"
        || root.tag_name().namespace() != Some("http://www.w3.org/2000/svg")
    {
        return Err(AppError::Internal("Figure must be an SVG document.".into()));
    }
    let width = root.attribute("width").and_then(figure_dimensions);
    let height = root.attribute("height").and_then(figure_dimensions);
    let (width, height) = match (width, height) {
        (Some(width), Some(height)) => (width, height),
        _ => {
            let values = root
                .attribute("viewBox")
                .unwrap_or_default()
                .split_ascii_whitespace()
                .collect::<Vec<_>>();
            if values.len() != 4 {
                return Err(AppError::Internal(
                    "SVG requires bounded width and height or viewBox dimensions.".into(),
                ));
            }
            (figure_dimensions(values[2]), figure_dimensions(values[3])).into_option_pair()?
        }
    };
    let allowed_elements = ["svg", "g", "rect", "line", "circle", "text"];
    let allowed_attributes = [
        "xmlns",
        "width",
        "height",
        "viewBox",
        "x",
        "y",
        "x1",
        "y1",
        "x2",
        "y2",
        "cx",
        "cy",
        "r",
        "rx",
        "ry",
        "fill",
        "stroke",
        "stroke-width",
        "font-family",
        "font-size",
        "font-weight",
        "text-anchor",
    ];
    let mut nodes = 0usize;
    for node in document.descendants().filter(roxmltree::Node::is_element) {
        nodes += 1;
        if nodes > 20_000 || !allowed_elements.contains(&node.tag_name().name()) {
            return Err(AppError::Internal(
                "SVG contains unsupported or excessive XML content.".into(),
            ));
        }
        for attribute in node.attributes() {
            let value = attribute.value().to_ascii_lowercase();
            if attribute.name() == "font-family"
                && !matches!(
                    value.as_str(),
                    "arial,sans-serif" | "arial, sans-serif" | "sans-serif" | "dejavu sans"
                )
            {
                return Err(AppError::Internal(
                    "SVG font is unavailable in the deterministic bundled font set.".into(),
                ));
            }
            if !allowed_attributes.contains(&attribute.name())
                || value.contains("url(")
                || value.contains("javascript:")
                || value.contains("data:")
                || value.contains("http:")
                || value.contains("https:")
            {
                return Err(AppError::Internal(
                    "SVG contains an unsupported attribute or external/active reference.".into(),
                ));
            }
        }
    }
    Ok((width, height))
}

trait OptionPair<T> {
    fn into_option_pair(self) -> Result<(T, T), AppError>;
}
impl<T> OptionPair<T> for (Option<T>, Option<T>) {
    fn into_option_pair(self) -> Result<(T, T), AppError> {
        match self {
            (Some(a), Some(b)) => Ok((a, b)),
            _ => Err(AppError::Internal(
                "SVG dimensions are invalid or exceed 4096 px.".into(),
            )),
        }
    }
}

fn extract_figure(path: PathBuf) -> Result<FigureInput, AppError> {
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > MAX_FIGURE_BYTES
    {
        return Err(AppError::Internal(
            "Select a regular figure file between 1 byte and 5 MB.".into(),
        ));
    }
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
        .map(str::to_owned)
        .ok_or_else(|| AppError::Internal("Figure filename is invalid.".into()))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(&path)?
        .take(MAX_FIGURE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != metadata.len() {
        return Err(AppError::Internal(
            "Figure changed while it was being imported.".into(),
        ));
    }
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let (media_type, width, height) = match extension.as_str() {
        "svg" => {
            let (w, h) = validate_svg(&bytes)?;
            ("image/svg+xml", w, h)
        }
        "png" | "jpg" | "jpeg" => {
            let image = image::load_from_memory(&bytes)
                .map_err(|_| AppError::Internal("Raster figure is malformed.".into()))?;
            let (w, h) = (image.width(), image.height());
            if w == 0 || h == 0 || w > 4096 || h > 4096 {
                return Err(AppError::Internal(
                    "Figure dimensions must be 1–4096 px.".into(),
                ));
            }
            (
                if extension == "png" {
                    "image/png"
                } else {
                    "image/jpeg"
                },
                w,
                h,
            )
        }
        _ => {
            return Err(AppError::Internal(
                "Figures must be SVG, PNG, or JPEG.".into(),
            ))
        }
    };
    Ok(FigureInput {
        name,
        media_type,
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        bytes,
        width,
        height,
    })
}

struct ExtractedDocument {
    name: String,
    media_type: &'static str,
    bytes: Vec<u8>,
    text: String,
    metadata_json: String,
    sha256: String,
}

fn extract_file(path: PathBuf) -> Result<ExtractedDocument, AppError> {
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(AppError::Internal(
            "Select a regular local PDF, DOCX, or Markdown document.".into(),
        ));
    }
    if metadata.len() == 0 || metadata.len() > MAX_FILE_BYTES {
        return Err(AppError::Internal(
            "Research documents must be between 1 byte and 25 MB.".into(),
        ));
    }
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty() && !value.chars().any(char::is_control))
        .map(str::to_owned)
        .ok_or_else(|| {
            AppError::Internal("The selected document has an invalid filename.".into())
        })?;
    if name.chars().count() > 255 {
        return Err(AppError::Internal(
            "Research document filenames are limited to 255 characters.".into(),
        ));
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(&path)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != metadata.len() || bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(AppError::Internal(
            "The selected document changed or exceeded the 25 MB import limit.".into(),
        ));
    }
    let (media_type, text, mut details) = match extension.as_str() {
        "md" | "markdown" => (
            "text/markdown",
            normalize_text(std::str::from_utf8(&bytes).map_err(|_| {
                AppError::Internal("Markdown documents must be valid UTF-8.".into())
            })?),
            serde_json::json!({"extractor":"utf8-markdown","provenance":"user-selected local file"}),
        ),
        "docx" => {
            let (text, paragraphs) = extract_docx(&bytes)?;
            (
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                text,
                serde_json::json!({"extractor":"local-docx-xml","paragraphCount":paragraphs,"provenance":"user-selected local file"}),
            )
        }
        "pdf" => {
            let (text, pages_total, pages_extracted) = extract_pdf(&bytes)?;
            (
                "application/pdf",
                text,
                serde_json::json!({"extractor":"local-lopdf-text","pagesTotal":pages_total,"pagesExtracted":pages_extracted,"provenance":"user-selected local file"}),
            )
        }
        "csv" => {
            let input = std::str::from_utf8(&bytes)
                .map_err(|_| AppError::Internal("CSV documents must be valid UTF-8.".into()))?;
            let (text, rows, columns) = extract_csv(input)?;
            (
                "text/csv",
                text,
                serde_json::json!({"extractor":"bounded-csv","rows":rows,"columns":columns,"preview":preview_text(input),"provenance":"user-selected local file"}),
            )
        }
        "json" => {
            let input = std::str::from_utf8(&bytes)
                .map_err(|_| AppError::Internal("JSON documents must be valid UTF-8.".into()))?;
            let value: serde_json::Value = serde_json::from_str(input)
                .map_err(|_| AppError::Internal("JSON document is malformed.".into()))?;
            let normalized = serde_json::to_string_pretty(&value)
                .map_err(|_| AppError::Internal("JSON document could not be normalized.".into()))?;
            let preview = normalized.chars().take(2_000).collect::<String>();
            (
                "application/json",
                normalize_text(&normalized),
                serde_json::json!({"extractor":"serde-json","rootType":json_type(&value),"preview":preview,"provenance":"user-selected local file"}),
            )
        }
        "xlsx" => {
            let (text, rows, columns) = extract_xlsx(&bytes)?;
            (
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                text.clone(),
                serde_json::json!({"extractor":"bounded-xlsx-xml","rows":rows,"columns":columns,"preview":preview_text(&text),"provenance":"user-selected local file"}),
            )
        }
        _ => {
            return Err(AppError::Internal(
                "Research Studio accepts PDF, DOCX, Markdown, CSV, JSON, and XLSX files.".into(),
            ));
        }
    };
    if text.trim().chars().count() < 20 {
        return Err(AppError::Internal(
            "The selected document has too little extractable text to index.".into(),
        ));
    }
    // Keep source aliases with the project document. Test packs and real
    // studies can contain byte-identical files under distinct method names;
    // SHA deduplication should retain every human-readable origin label.
    details["sourceLabels"] = serde_json::json!([name.clone()]);
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    Ok(ExtractedDocument {
        name,
        media_type,
        bytes,
        text,
        metadata_json: serde_json::to_string(&details).unwrap_or_else(|_| "{}".into()),
        sha256,
    })
}

fn preview_text(text: &str) -> String {
    text.chars().take(2_000).collect()
}

fn json_type(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn extract_csv(input: &str) -> Result<(String, usize, usize), AppError> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let delimiter = [',', '\t', ';']
        .into_iter()
        .max_by_key(|delimiter| {
            input
                .lines()
                .next()
                .unwrap_or_default()
                .matches(*delimiter)
                .count()
        })
        .unwrap_or(',');
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                cell.push('"');
            }
            '"' => quoted = !quoted,
            ch if ch == delimiter && !quoted => {
                row.push(std::mem::take(&mut cell));
            }
            '\n' if !quoted => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            '\r' if !quoted => {}
            _ => cell.push(ch),
        }
        if rows.len() > 100_000 || cell.len() > MAX_EXTRACTED_CHARS {
            return Err(AppError::Internal(
                "CSV exceeds the row or cell extraction limit.".into(),
            ));
        }
    }
    if quoted {
        return Err(AppError::Internal(
            "CSV contains an unterminated quoted field.".into(),
        ));
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    if rows.is_empty() {
        return Err(AppError::Internal("CSV contains no rows.".into()));
    }
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    if columns == 0 || columns > 10_000 {
        return Err(AppError::Internal(
            "CSV column count is outside the supported limit.".into(),
        ));
    }
    let text = rows
        .iter()
        .map(|r| r.join("\t"))
        .collect::<Vec<_>>()
        .join("\n");
    Ok((normalize_text(&text), rows.len(), columns))
}

fn extract_xlsx(bytes: &[u8]) -> Result<(String, usize, usize), AppError> {
    if !bytes.starts_with(b"PK\x03\x04") {
        return Err(AppError::Internal(
            "XLSX is not a ZIP-based Office workbook.".into(),
        ));
    }
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| AppError::Internal("XLSX package is invalid.".into()))?;
    if archive.len() == 0 || archive.len() > 2_048 {
        return Err(AppError::Internal(
            "XLSX contains an invalid number of package parts.".into(),
        ));
    }
    for required in ["[Content_Types].xml", "xl/workbook.xml"] {
        if archive.by_name(required).is_err() {
            return Err(AppError::Internal(
                "XLSX is missing required Office workbook parts.".into(),
            ));
        }
    }
    let mut expanded = 0u64;
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|_| AppError::Internal("XLSX package entry is invalid.".into()))?;
        expanded = expanded.saturating_add(entry.size());
        if entry.size() > 16 * 1024 * 1024 || expanded > 64 * 1024 * 1024 {
            return Err(AppError::Internal(
                "XLSX expanded package exceeds the 64 MB extraction limit.".into(),
            ));
        }
    }
    let mut shared = Vec::new();
    if let Ok(mut entry) = archive.by_name("xl/sharedStrings.xml") {
        let mut xml = String::new();
        entry
            .read_to_string(&mut xml)
            .map_err(|_| AppError::Internal("XLSX shared strings are malformed.".into()))?;
        let doc = roxmltree::Document::parse(&xml)
            .map_err(|_| AppError::Internal("XLSX shared strings XML is malformed.".into()))?;
        for item in doc
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "si")
        {
            shared.push(
                item.descendants()
                    .filter(|n| n.is_element() && n.tag_name().name() == "t")
                    .filter_map(|n| n.text())
                    .collect::<String>(),
            );
        }
    }
    let sheet = archive
        .by_name("xl/worksheets/sheet1.xml")
        .map_err(|_| AppError::Internal("XLSX has no readable first worksheet.".into()))?;
    let mut xml = String::new();
    sheet
        .take(16 * 1024 * 1024 + 1)
        .read_to_string(&mut xml)
        .map_err(|_| AppError::Internal("XLSX worksheet XML is malformed.".into()))?;
    if xml.len() > 16 * 1024 * 1024 {
        return Err(AppError::Internal(
            "XLSX worksheet exceeds the 16 MB limit.".into(),
        ));
    }
    let doc = roxmltree::Document::parse(&xml)
        .map_err(|_| AppError::Internal("XLSX worksheet XML is malformed.".into()))?;
    let mut rows = Vec::new();
    let mut max_columns = 0usize;
    for row_node in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "row")
    {
        let mut row = Vec::new();
        for cell in row_node
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "c")
        {
            let reference = cell.attribute("r").unwrap_or("");
            let col = reference
                .chars()
                .take_while(|c| c.is_ascii_alphabetic())
                .fold(0usize, |v, c| {
                    v * 26 + (c.to_ascii_uppercase() as usize - 'A' as usize + 1)
                })
                .saturating_sub(1);
            if col > 10_000 {
                return Err(AppError::Internal(
                    "XLSX column index exceeds the supported limit.".into(),
                ));
            }
            while row.len() <= col {
                row.push(String::new());
            }
            let value = cell
                .children()
                .find(|n| n.is_element() && n.tag_name().name() == "v")
                .and_then(|n| n.text())
                .unwrap_or("");
            row[col] = if cell.attribute("t") == Some("s") {
                value
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| shared.get(i).cloned())
                    .ok_or_else(|| {
                        AppError::Internal("XLSX references an invalid shared string.".into())
                    })?
            } else if cell.attribute("t") == Some("inlineStr") {
                cell.descendants()
                    .find(|n| n.is_element() && n.tag_name().name() == "t")
                    .and_then(|n| n.text())
                    .unwrap_or("")
                    .to_owned()
            } else {
                value.to_owned()
            };
        }
        max_columns = max_columns.max(row.len());
        rows.push(row);
        if rows.len() > 100_000 {
            return Err(AppError::Internal(
                "XLSX exceeds the 100,000 row extraction limit.".into(),
            ));
        }
    }
    if rows.is_empty() {
        return Err(AppError::Internal(
            "XLSX first worksheet contains no rows.".into(),
        ));
    }
    let text = rows
        .iter()
        .map(|r| r.join("\t"))
        .collect::<Vec<_>>()
        .join("\n");
    Ok((normalize_text(&text), rows.len(), max_columns))
}

fn normalize_text(text: &str) -> String {
    text.chars()
        .filter(|value| !value.is_control() || matches!(value, '\n' | '\r' | '\t'))
        .take(MAX_EXTRACTED_CHARS)
        .collect::<String>()
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

fn extract_docx(bytes: &[u8]) -> Result<(String, usize), AppError> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|_| AppError::Internal("DOCX is not a readable Office document.".into()))?;
    if archive.len() == 0 || archive.len() > 2_048 {
        return Err(AppError::Internal(
            "DOCX contains an invalid number of package parts.".into(),
        ));
    }
    let entry = archive
        .by_name("word/document.xml")
        .map_err(|_| AppError::Internal("DOCX is missing its main document part.".into()))?;
    if entry.size() > 16 * 1024 * 1024 {
        return Err(AppError::Internal(
            "DOCX main document part exceeds the 16 MB extraction limit.".into(),
        ));
    }
    let mut xml = String::with_capacity(entry.size() as usize);
    entry
        .take(16 * 1024 * 1024 + 1)
        .read_to_string(&mut xml)
        .map_err(|_| {
            AppError::Internal("DOCX main document part is not valid UTF-8 XML.".into())
        })?;
    if xml.len() > 16 * 1024 * 1024 {
        return Err(AppError::Internal(
            "DOCX main document part exceeded its extraction limit.".into(),
        ));
    }
    let document = roxmltree::Document::parse(&xml)
        .map_err(|_| AppError::Internal("DOCX main document XML is malformed.".into()))?;
    let mut output = String::new();
    let mut paragraphs = 0usize;
    for node in document.descendants().filter(roxmltree::Node::is_element) {
        match node.tag_name().name() {
            "t" => {
                if let Some(text) = node.text() {
                    output.push_str(text);
                }
            }
            "tab" => output.push('\t'),
            "br" | "cr" => output.push('\n'),
            "p" => {
                output.push('\n');
                paragraphs += 1;
            }
            _ => {}
        }
        if output.chars().count() >= MAX_EXTRACTED_CHARS {
            break;
        }
    }
    Ok((normalize_text(&output), paragraphs))
}

fn extract_pdf(bytes: &[u8]) -> Result<(String, usize, usize), AppError> {
    if !bytes.starts_with(b"%PDF-") {
        return Err(AppError::Internal(
            "Selected file does not have a PDF header.".into(),
        ));
    }
    let document = Document::load_mem_with_options(
        bytes,
        LoadOptions::with_max_decompressed_size(32 * 1024 * 1024),
    )
    .map_err(|_| {
        AppError::Internal("PDF could not be parsed within the local extraction budget.".into())
    })?;
    if document.is_encrypted() {
        return Err(AppError::Internal(
            "Encrypted PDF documents cannot be indexed.".into(),
        ));
    }
    let pages = document.get_pages().keys().copied().collect::<Vec<_>>();
    if pages.is_empty() {
        return Err(AppError::Internal("PDF has no readable pages.".into()));
    }
    let total = pages.len();
    let mut text = String::new();
    let mut extracted = 0usize;
    for page in pages.into_iter().take(200) {
        let content = document
            .extract_text_with_limit(&[page], 2 * 1024 * 1024)
            .map_err(|_| AppError::Internal("A PDF page could not be safely decoded.".into()))?;
        let content = content.split_whitespace().collect::<Vec<_>>().join(" ");
        if !content.is_empty() {
            text.push_str(&format!("\n[Page {page}] {content}\n"));
            extracted += 1;
        }
        if text.chars().count() >= MAX_EXTRACTED_CHARS {
            break;
        }
    }
    Ok((normalize_text(&text), total, extracted))
}

fn split_chunks(text: &str) -> Vec<(usize, usize, String)> {
    let chars = text.char_indices().collect::<Vec<_>>();
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < chars.len() && chunks.len() < MAX_CHUNKS {
        let end = (start + CHUNK_CHARS).min(chars.len());
        let byte_start = chars[start].0;
        let byte_end = if end == chars.len() {
            text.len()
        } else {
            chars[end].0
        };
        chunks.push((start, end, text[byte_start..byte_end].to_owned()));
        if end == chars.len() {
            break;
        }
        start = end.saturating_sub(CHUNK_OVERLAP).max(start + 1);
    }
    chunks
}

#[tauri::command]
pub(crate) async fn import_local_research_document(
    project_id: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<ResearchDocument, AppError> {
    import_local_research_document_inner(project_id, path, &state).await
}

#[tauri::command]
pub(crate) async fn register_local_research_figure(
    project_id: String,
    source_run_id: Option<String>,
    path: String,
    caption: String,
    alt_text: String,
    state: State<'_, AppState>,
) -> Result<ResearchFigureAsset, AppError> {
    let caption = caption.trim().to_owned();
    let alt_text = alt_text.trim().to_owned();
    if caption.is_empty()
        || caption.chars().count() > 1_000
        || alt_text.is_empty()
        || alt_text.chars().count() > 500
    {
        return Err(AppError::Internal(
            "Figure caption and accessible description are required and must fit their limits."
                .into(),
        ));
    }
    let input = tokio::task::spawn_blocking(move || extract_figure(PathBuf::from(path)))
        .await
        .map_err(|_| AppError::Internal("Figure validation worker failed.".into()))??;
    let id = Uuid::new_v4().to_string();
    let imported_at = Utc::now().to_rfc3339();
    let database = state
        .database
        .lock()
        .map_err(|_| AppError::Internal("Database lock is unavailable".into()))?;
    let connection = database.connection();
    let project_exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM research_projects WHERE id=?1)",
        params![project_id],
        |row| row.get(0),
    )?;
    if !project_exists {
        return Err(AppError::Internal("Research project was not found.".into()));
    }
    if let Some(run_id) = source_run_id.as_deref() {
        let belongs: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM research_runs WHERE id=?1 AND project_id=?2)",
            params![run_id, project_id],
            |row| row.get(0),
        )?;
        if !belongs {
            return Err(AppError::Internal(
                "Figure provenance run does not belong to this research project.".into(),
            ));
        }
    }
    if let Some(existing) = connection.query_row(
        "SELECT id,project_id,name,media_type,size_bytes,content_sha256,width_px,height_px,caption,alt_text,imported_at FROM research_figure_assets WHERE project_id=?1 AND content_sha256=?2",
        params![project_id, input.sha256],
        |row| Ok(ResearchFigureAsset { id: row.get(0)?, project_id: row.get(1)?, name: row.get(2)?, media_type: row.get(3)?, size_bytes: row.get::<_,i64>(4)?.max(0) as u64, content_sha256: row.get(5)?, width_px: row.get::<_,i64>(6)?.max(0) as u32, height_px: row.get::<_,i64>(7)?.max(0) as u32, caption: row.get(8)?, alt_text: row.get(9)?, imported_at: row.get(10)? }),
    ).optional()? { return Ok(existing); }
    connection.execute(
        "INSERT INTO research_figure_assets (id,project_id,source_run_id,name,media_type,size_bytes,content_blob,content_sha256,width_px,height_px,caption,alt_text,provenance_json,imported_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![id, project_id, source_run_id, input.name, input.media_type, input.bytes.len() as i64, input.bytes, input.sha256, input.width as i64, input.height as i64, caption, alt_text, serde_json::json!({"origin":"user-selected local figure","validation":"bounded SVG allowlist or image decoder","sha256":"source bytes"}).to_string(), imported_at],
    )?;
    Ok(ResearchFigureAsset {
        id,
        project_id,
        name: input.name,
        media_type: input.media_type.into(),
        size_bytes: input.bytes.len() as u64,
        content_sha256: input.sha256,
        width_px: input.width,
        height_px: input.height,
        caption,
        alt_text,
        imported_at,
    })
}

#[tauri::command]
pub(crate) fn list_local_research_figure_assets(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<ResearchFigureAsset>, AppError> {
    let database = state
        .database
        .lock()
        .map_err(|_| AppError::Internal("Database lock is unavailable".into()))?;
    let mut statement = database.connection().prepare(
        "SELECT id,project_id,name,media_type,size_bytes,content_sha256,width_px,height_px,caption,alt_text,imported_at FROM research_figure_assets WHERE project_id=?1 ORDER BY imported_at,id"
    )?;
    let rows = statement.query_map(params![project_id], |row| {
        Ok(ResearchFigureAsset {
            id: row.get(0)?,
            project_id: row.get(1)?,
            name: row.get(2)?,
            media_type: row.get(3)?,
            size_bytes: row.get::<_, i64>(4)?.max(0) as u64,
            content_sha256: row.get(5)?,
            width_px: row.get::<_, i64>(6)?.max(0) as u32,
            height_px: row.get::<_, i64>(7)?.max(0) as u32,
            caption: row.get(8)?,
            alt_text: row.get(9)?,
            imported_at: row.get(10)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
}

/// Returns a project-owned figure as a `data:` URL for in-editor preview. Reuses
/// the same fail-closed resolver as export: a forged, non-canonical, wrong-
/// project, missing or tampered asset is rejected and never previewed.
#[tauri::command]
pub(crate) fn local_research_figure_data_url(
    project_id: String,
    asset_id: String,
    state: State<'_, AppState>,
) -> Result<String, AppError> {
    let database = state
        .database
        .lock()
        .map_err(|_| AppError::Internal("Database lock is unavailable".into()))?;
    let resolved = resolve_research_figure(
        database.connection(),
        &project_id,
        &format!("research-figure:{}", asset_id.trim()),
    )?;
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&resolved.bytes);
    Ok(format!("data:{};base64,{}", resolved.media_type, encoded))
}

pub(super) async fn import_local_research_document_inner(
    project_id: String,
    path: String,
    state: &AppState,
) -> Result<ResearchDocument, AppError> {
    let path = PathBuf::from(path);
    let extracted = tokio::task::spawn_blocking(move || extract_file(path))
        .await
        .map_err(|_| AppError::Internal("Local document extraction worker failed.".into()))??;
    let document_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let chunks = split_chunks(&extracted.text);
    let database = state
        .database
        .lock()
        .map_err(|_| AppError::Internal("Database lock is unavailable".into()))?;
    let connection = database.connection();
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM research_projects WHERE id=?1)",
        params![project_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(AppError::Internal("Research project was not found.".into()));
    }
    if let Some(mut existing) = connection
        .query_row(
            "SELECT d.id,d.name,d.media_type,d.size_bytes,d.content_sha256,d.imported_at,d.extraction_metadata_json,
                    (SELECT COUNT(*) FROM research_document_chunks c WHERE c.document_id=d.id)
             FROM research_documents d WHERE d.project_id=?1 AND d.content_sha256=?2",
            params![project_id, extracted.sha256],
            |row| {
                Ok(ResearchDocument {
                    id: row.get(0)?, name: row.get(1)?, media_type: row.get(2)?,
                    size_bytes: row.get::<_, i64>(3)?.max(0) as u64,
                    content_sha256: row.get(4)?, imported_at: row.get(5)?, extraction_metadata_json: row.get(6)?,
                    chunk_count: row.get::<_, i64>(7)?.max(0) as usize,
                })
            },
        )
        .optional()?
    {
        let mut metadata: serde_json::Value = serde_json::from_str(&existing.extraction_metadata_json)
            .unwrap_or_else(|_| serde_json::json!({}));
        let mut labels = metadata["sourceLabels"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if let Some(incoming) = serde_json::from_str::<serde_json::Value>(&extracted.metadata_json)
            .ok()
            .and_then(|value| value["sourceLabels"].as_array().cloned())
        {
            for label in incoming {
                if !labels.contains(&label) {
                    labels.push(label);
                }
            }
        }
        metadata["sourceLabels"] = serde_json::Value::Array(labels);
        existing.extraction_metadata_json = serde_json::to_string(&metadata)
            .unwrap_or_else(|_| "{}".into());
        connection.execute(
            "UPDATE research_documents SET extraction_metadata_json=?1 WHERE id=?2",
            params![existing.extraction_metadata_json, existing.id],
        )?;
        return Ok(existing);
    }
    let (document_count, stored_bytes): (i64, i64) = connection.query_row(
        "SELECT COUNT(*),COALESCE(SUM(size_bytes),0) FROM research_documents WHERE project_id=?1",
        params![project_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if document_count >= 40
        || stored_bytes.saturating_add(extracted.bytes.len() as i64) > 200 * 1024 * 1024
    {
        return Err(AppError::Internal(
            "A research project can store up to 40 local documents or 200 MB of source files."
                .into(),
        ));
    }
    let tx = connection.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO research_documents
         (id,project_id,name,media_type,size_bytes,content_blob,content_sha256,extracted_text,extraction_metadata_json,imported_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            document_id, project_id, extracted.name, extracted.media_type,
            extracted.bytes.len() as i64, extracted.bytes, extracted.sha256,
            extracted.text, extracted.metadata_json, now
        ],
    )?;
    for (index, (start, end, content)) in chunks.iter().enumerate() {
        tx.execute(
            "INSERT INTO research_document_chunks (id,document_id,project_id,chunk_index,start_char,end_char,content)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![Uuid::new_v4().to_string(), document_id, project_id, index as i64, *start as i64, *end as i64, content],
        )?;
    }
    tx.execute(
        "UPDATE research_projects SET updated_at=?1 WHERE id=?2",
        params![now, project_id],
    )?;
    tx.commit()?;
    Ok(ResearchDocument {
        id: document_id,
        name: extracted.name,
        media_type: extracted.media_type.into(),
        size_bytes: extracted.bytes.len() as u64,
        content_sha256: extracted.sha256,
        chunk_count: chunks.len(),
        imported_at: now,
        extraction_metadata_json: extracted.metadata_json,
    })
}

#[tauri::command]
pub(crate) fn list_local_research_documents(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<ResearchDocument>, AppError> {
    let database = state
        .database
        .lock()
        .map_err(|_| AppError::Internal("Database lock is unavailable".into()))?;
    let mut statement = database.connection().prepare(
        "SELECT d.id,d.name,d.media_type,d.size_bytes,d.content_sha256,d.imported_at,d.extraction_metadata_json,
                (SELECT COUNT(*) FROM research_document_chunks c WHERE c.document_id=d.id)
         FROM research_documents d WHERE d.project_id=?1 ORDER BY d.imported_at DESC,d.id",
    )?;
    let rows = statement.query_map(params![project_id], |row| {
        Ok(ResearchDocument {
            id: row.get(0)?,
            name: row.get(1)?,
            media_type: row.get(2)?,
            size_bytes: row.get::<_, i64>(3)?.max(0) as u64,
            content_sha256: row.get(4)?,
            imported_at: row.get(5)?,
            extraction_metadata_json: row.get(6)?,
            chunk_count: row.get::<_, i64>(7)?.max(0) as usize,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn svg_figure_allowlist_accepts_static_vector_shapes_and_rejects_active_content() {
        let safe = br##"<svg xmlns="http://www.w3.org/2000/svg" width="900" height="600" viewBox="0 0 900 600"><rect width="100%" height="100%" fill="white"/><line x1="0" y1="0" x2="10" y2="10" stroke="#000"/><text x="2" y="3">Axis</text></svg>"##;
        assert_eq!(validate_svg(safe).unwrap(), (900, 600));
        for unsafe_svg in [
            br#"<!DOCTYPE svg [<!ENTITY x SYSTEM "file:///C:/Windows/win.ini">]><svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">&x;</svg>"#.as_slice(),
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><script>alert(1)</script></svg>"#.as_slice(),
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image href="https://example.invalid/a.png"/></svg>"#.as_slice(),
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="90000" height="10"/>"#.as_slice(),
        ] {
            assert!(validate_svg(unsafe_svg).is_err());
        }
    }

    #[test]
    fn figure_resolution_is_project_scoped_and_rechecks_source_integrity() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE research_figure_assets(id TEXT,project_id TEXT,name TEXT,media_type TEXT,content_blob BLOB,content_sha256 TEXT,width_px INTEGER,height_px INTEGER,caption TEXT,alt_text TEXT,source_run_id TEXT); CREATE TABLE research_runs(id TEXT,project_id TEXT);").unwrap();
        let id = Uuid::new_v4().to_string();
        let project_id = Uuid::new_v4().to_string();
        let bytes = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="white"/></svg>"#.to_vec();
        let hash = format!("{:x}", Sha256::digest(&bytes));
        connection.execute("INSERT INTO research_figure_assets VALUES (?1,?2,'chart.svg','image/svg+xml',?3,?4,10,10,'Figure 1','Chart',NULL)", params![id, project_id, bytes, hash]).unwrap();
        let resolved =
            resolve_research_figure(&connection, &project_id, &format!("research-figure:{id}"))
                .unwrap();
        assert_eq!(resolved.id, id);
        assert_eq!(resolved.width, 10);
        assert!(resolve_research_figure(
            &connection,
            "another-project",
            &format!("research-figure:{id}")
        )
        .is_err());
        assert!(
            resolve_research_figure(&connection, &project_id, "research-figure:../../outside")
                .is_err()
        );
        connection
            .execute(
                "UPDATE research_figure_assets SET content_blob=?1 WHERE id=?2",
                params![b"tampered", id],
            )
            .unwrap();
        assert!(resolve_research_figure(
            &connection,
            &project_id,
            &format!("research-figure:{id}")
        )
        .unwrap_err()
        .to_string()
        .contains("SHA-256"));
    }

    #[test]
    fn markdown_is_normalized_and_chunked_with_character_offsets() {
        let text = ("Research evidence. 🙂\n").repeat(400);
        let chunks = split_chunks(&text);
        assert!(chunks.len() > 1);
        assert_eq!(
            chunks[0].2,
            text.chars().take(CHUNK_CHARS).collect::<String>()
        );
        for (start, end, chunk) in chunks {
            assert!(end > start);
            assert_eq!(chunk.chars().count(), end - start);
        }
    }

    #[test]
    fn imports_markdown_bytes_text_and_local_provenance() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notes.md");
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(b"# Local source\n\nThis document contains enough research evidence to be indexed locally.")
            .unwrap();
        let extracted = extract_file(path).unwrap();
        assert_eq!(extracted.media_type, "text/markdown");
        assert!(extracted.text.contains("Local source"));
        assert_eq!(extracted.sha256.len(), 64);
        assert!(extracted.metadata_json.contains("user-selected local file"));
        assert_eq!(split_chunks(&extracted.text).len(), 1);
    }

    #[test]
    fn extracts_docx_paragraphs_locally() {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut archive = zip::ZipWriter::new(cursor);
        archive
            .start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        archive
            .write_all(b"<w:document xmlns:w=\"urn:test\"><w:body><w:p><w:r><w:t>First paragraph</w:t></w:r></w:p><w:p><w:r><w:t>Second paragraph</w:t></w:r></w:p></w:body></w:document>")
            .unwrap();
        let bytes = archive.finish().unwrap().into_inner();
        let (text, paragraphs) = extract_docx(&bytes).unwrap();
        assert_eq!(paragraphs, 2);
        assert!(text.contains("First paragraph\n"));
        assert!(text.contains("Second paragraph"));
    }

    #[test]
    fn extracts_text_pdf_and_preserves_page_provenance() {
        use lopdf::{dictionary, Stream};

        let mut document = Document::with_version("1.7");
        let pages = document.new_object_id();
        let font = document.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica"
        });
        let resources = dictionary! { "Font" => dictionary! { "F1" => font } };
        let content = Stream::new(
            dictionary! {},
            b"BT /F1 12 Tf 72 720 Td (Local PDF evidence extraction test passage.) Tj ET".to_vec(),
        );
        let content = document.add_object(content);
        let page = document.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => resources, "Contents" => content,
        });
        document.objects.insert(
            pages,
            dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }.into(),
        );
        let root = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        document.trailer.set("Root", root);
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();

        let (text, total, extracted) = extract_pdf(&bytes).unwrap();
        assert_eq!((total, extracted), (1, 1));
        assert!(text.contains("[Page 1]"));
        assert!(text.contains("Local PDF evidence extraction test passage"));
    }

    #[test]
    fn rejects_unsupported_and_symlinked_files() {
        let directory = tempfile::tempdir().unwrap();
        let unsupported = directory.path().join("notes.txt");
        fs::write(&unsupported, "This file type is not supported.").unwrap();
        assert!(extract_file(unsupported).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let target = directory.path().join("target.md");
            fs::write(&target, "A source document with enough content to extract.").unwrap();
            let link = directory.path().join("linked.md");
            symlink(target, &link).unwrap();
            assert!(extract_file(link).is_err());
        }
    }

    #[test]
    fn parses_csv_rows_quotes_and_rejects_unterminated_fields() {
        let (text, rows, columns) =
            extract_csv("name,score\n\"Client, A\",0.91\nClient B,0.82\n").unwrap();
        assert_eq!((rows, columns), (3, 2));
        assert!(text.contains("Client, A\t0.91"));
        assert!(extract_csv("a,b\n\"broken,1").is_err());
    }

    #[test]
    fn validates_and_normalizes_json() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("metrics.json");
        fs::write(&path, br#"{"seed":11,"accuracy":0.9907}"#).unwrap();
        let parsed = extract_file(path).unwrap();
        assert_eq!(parsed.media_type, "application/json");
        assert!(parsed.text.contains("0.9907"));
        assert!(parsed.metadata_json.contains("\"rootType\":\"object\""));

        let bad = directory.path().join("invalid.json");
        fs::write(&bad, b"{bad json").unwrap();
        assert!(extract_file(bad).is_err());
    }

    #[test]
    fn extracts_first_xlsx_sheet_and_rejects_zip_without_office_parts() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut archive = zip::ZipWriter::new(&mut cursor);
            for (name, content) in [
                ("[Content_Types].xml", "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"/>"),
                ("xl/workbook.xml", "<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheets/></workbook>"),
                ("xl/worksheets/sheet1.xml", "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData><row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>metric</t></is></c><c r=\"B1\"><v>0.99</v></c></row></sheetData></worksheet>"),
            ] {
                archive.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
                archive.write_all(content.as_bytes()).unwrap();
            }
            archive.finish().unwrap();
        }
        let (text, rows, columns) = extract_xlsx(cursor.get_ref()).unwrap();
        assert_eq!((rows, columns), (1, 2));
        assert!(text.contains("0.99"));

        let mut bad = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        bad.start_file("notes.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        bad.write_all(b"not a workbook").unwrap();
        assert!(extract_xlsx(&bad.finish().unwrap().into_inner()).is_err());
    }
}
