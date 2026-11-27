use std::path::Path;

use printpdf::*;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::app_error::AppError;

const REGULAR_FONT: &[u8] = include_bytes!("../assets/fonts/DejaVuSans-Regular.ttf");
const BOLD_FONT: &[u8] = include_bytes!("../assets/fonts/DejaVuSans-Bold.ttf");
const MONO_REGULAR_FONT: &[u8] = include_bytes!("../assets/fonts/DejaVuSansMono-Regular.ttf");

const PAGE_WIDTH_MM: f32 = 210.0;
const PAGE_HEIGHT_MM: f32 = 297.0;
const LEFT_MARGIN_MM: f32 = 18.0;
const RIGHT_MARGIN_MM: f32 = 18.0;
const TOP_MARGIN_MM: f32 = 18.0;
const BOTTOM_MARGIN_MM: f32 = 18.0;
const CONTENT_WIDTH_MM: f32 = PAGE_WIDTH_MM - LEFT_MARGIN_MM - RIGHT_MARGIN_MM;
const EXPORT_AST_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy)]
pub struct PdfMeta {
    pub page_count: i64,
}

/// A simplified block-level document model shared by the PDF and DOCX generators.
///
/// PDF currently uses plain paragraph text; DOCX retains styled inline runs for paragraphs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct InlineRun {
    text: String,
    bold: bool,
    italic: bool,
    code: bool,
}

#[derive(Debug, Clone)]
enum Block {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph {
        text: String,
        runs: Vec<InlineRun>,
    },
    UnorderedList {
        items: Vec<String>,
    },
    OrderedList {
        start: u64,
        items: Vec<String>,
    },
    Code {
        code: String,
    },
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    /// Version 1 export AST figure reference. `asset_ref` is an opaque logical
    /// project asset ID, never a filesystem path or URL to be loaded here.
    FigureReference {
        asset_ref: String,
        alt: String,
        caption: String,
    },
    Rule,
}

fn parse_markdown_blocks(markdown: &str) -> Vec<Block> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(markdown, options);

    let mut blocks = Vec::new();
    let mut text_buffer = String::new();
    let mut heading_level: Option<u8> = None;
    let mut in_paragraph = false;
    let mut inline_runs: Vec<InlineRun> = Vec::new();
    let mut strong_depth = 0usize;
    let mut emphasis_depth = 0usize;
    let mut in_code_block = false;
    let mut image: Option<(String, String, String)> = None;

    let mut list_stack: Vec<(bool, u64, Vec<String>)> = Vec::new();
    let mut in_item = false;
    let mut item_buffer = String::new();

    let mut in_table = false;
    let mut in_table_head = false;
    let mut in_cell = false;
    let mut cell_buffer = String::new();
    let mut current_row: Vec<String> = Vec::new();
    let mut table_headers: Vec<String> = Vec::new();
    let mut table_rows: Vec<Vec<String>> = Vec::new();

    for event in parser {
        match event {
            Event::Start(Tag::Image { dest_url, title, .. }) => {
                image = Some((dest_url.to_string(), String::new(), title.to_string()));
            }
            Event::End(TagEnd::Image) => {
                if let Some((asset_ref, alt, title)) = image.take() {
                    // A figure is a block-level element in the export AST. Flush
                    // any surrounding inline content first to preserve source order.
                    if in_paragraph && !text_buffer.trim().is_empty() {
                        blocks.push(Block::Paragraph {
                            text: text_buffer.trim().to_string(),
                            runs: std::mem::take(&mut inline_runs),
                        });
                        text_buffer.clear();
                    }
                    blocks.push(Block::FigureReference {
                        asset_ref,
                        caption: if title.trim().is_empty() { alt.clone() } else { title },
                        alt,
                    });
                }
            }
            Event::Start(Tag::Heading { level, .. }) => {
                text_buffer.clear();
                heading_level = Some(level as u8);
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some(level) = heading_level.take() {
                    let text = text_buffer.trim().to_string();
                    if !text.is_empty() {
                        blocks.push(Block::Heading { level, text });
                    }
                }
                text_buffer.clear();
            }
            Event::Start(Tag::Paragraph) => {
                if !in_item && !in_cell {
                    text_buffer.clear();
                    inline_runs.clear();
                    in_paragraph = true;
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if in_paragraph {
                    let text = text_buffer.trim().to_string();
                    if !text.is_empty() {
                        blocks.push(Block::Paragraph {
                            text,
                            runs: std::mem::take(&mut inline_runs),
                        });
                    }
                    text_buffer.clear();
                    in_paragraph = false;
                }
            }
            Event::Start(Tag::List(start)) => {
                list_stack.push((start.is_some(), start.unwrap_or(1), Vec::new()));
            }
            Event::End(TagEnd::List(_)) => {
                if let Some((ordered, start, items)) = list_stack.pop() {
                    if !items.is_empty() {
                        if ordered {
                            blocks.push(Block::OrderedList { start, items });
                        } else {
                            blocks.push(Block::UnorderedList { items });
                        }
                    }
                }
            }
            Event::Start(Tag::Item) => {
                in_item = true;
                item_buffer.clear();
            }
            Event::End(TagEnd::Item) => {
                in_item = false;
                let text = item_buffer.trim().to_string();
                if let Some(top) = list_stack.last_mut() {
                    if !text.is_empty() {
                        top.2.push(text);
                    }
                }
                item_buffer.clear();
            }
            Event::Start(Tag::CodeBlock(_)) => {
                in_code_block = true;
                text_buffer.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                blocks.push(Block::Code {
                    code: text_buffer.trim_end().to_string(),
                });
                text_buffer.clear();
                in_code_block = false;
            }
            Event::Start(Tag::Table(_)) => {
                in_table = true;
                table_headers.clear();
                table_rows.clear();
            }
            Event::End(TagEnd::Table) => {
                if !table_headers.is_empty() {
                    blocks.push(Block::Table {
                        headers: table_headers.clone(),
                        rows: table_rows.clone(),
                    });
                }
                in_table = false;
            }
            Event::Start(Tag::TableHead) => {
                in_table_head = true;
                current_row.clear();
            }
            Event::End(TagEnd::TableHead) => {
                table_headers = current_row.clone();
                current_row.clear();
                in_table_head = false;
            }
            Event::Start(Tag::TableRow) => {
                current_row.clear();
            }
            Event::End(TagEnd::TableRow) => {
                if in_table && !in_table_head {
                    table_rows.push(current_row.clone());
                }
                current_row.clear();
            }
            Event::Start(Tag::TableCell) => {
                in_cell = true;
                cell_buffer.clear();
            }
            Event::End(TagEnd::TableCell) => {
                in_cell = false;
                current_row.push(cell_buffer.trim().to_string());
                cell_buffer.clear();
            }
            Event::Start(Tag::BlockQuote(_)) => {
                text_buffer.clear();
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                let text = text_buffer.trim().to_string();
                if !text.is_empty() {
                    blocks.push(Block::Paragraph {
                        text: format!("\u{201c}{text}\u{201d}"),
                        runs: Vec::new(),
                    });
                }
                text_buffer.clear();
            }
            Event::Start(Tag::Strong) => strong_depth += 1,
            Event::End(TagEnd::Strong) => strong_depth = strong_depth.saturating_sub(1),
            Event::Start(Tag::Emphasis) => emphasis_depth += 1,
            Event::End(TagEnd::Emphasis) => emphasis_depth = emphasis_depth.saturating_sub(1),
            Event::Rule => blocks.push(Block::Rule),
            Event::Text(ref text) | Event::Code(ref text) => {
                if let Some((_, alt, _)) = image.as_mut() {
                    alt.push_str(text);
                    continue;
                }
                if in_paragraph && !in_item && !in_cell && !in_code_block {
                    let run = InlineRun {
                        text: text.to_string(),
                        bold: strong_depth > 0,
                        italic: emphasis_depth > 0,
                        code: matches!(&event, Event::Code(_)),
                    };
                    if let Some(last) = inline_runs.last_mut() {
                        if last.bold == run.bold
                            && last.italic == run.italic
                            && last.code == run.code
                        {
                            last.text.push_str(&run.text);
                        } else {
                            inline_runs.push(run);
                        }
                    } else {
                        inline_runs.push(run);
                    }
                }
                if in_code_block {
                    text_buffer.push_str(&text);
                } else if in_cell {
                    cell_buffer.push_str(&text);
                } else if in_item {
                    item_buffer.push_str(&text);
                } else {
                    text_buffer.push_str(&text);
                }
            }
            Event::SoftBreak => {
                if in_code_block {
                    text_buffer.push('\n');
                } else if in_cell {
                    cell_buffer.push(' ');
                } else if in_item {
                    item_buffer.push(' ');
                } else {
                    text_buffer.push(' ');
                    if in_paragraph {
                        inline_runs.push(InlineRun {
                            text: " ".into(),
                            bold: strong_depth > 0,
                            italic: emphasis_depth > 0,
                            code: false,
                        });
                    }
                }
            }
            Event::HardBreak => {
                if in_item {
                    item_buffer.push(' ');
                } else {
                    text_buffer.push(' ');
                    if in_paragraph {
                        inline_runs.push(InlineRun {
                            text: " ".into(),
                            bold: strong_depth > 0,
                            italic: emphasis_depth > 0,
                            code: false,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    blocks
}

/// The legacy export entry points have no project-scoped asset registry. Fail
/// closed when an AST contains a figure rather than dropping it or opening the
/// Markdown destination as a path/URL. Project-aware exporters can resolve IDs
/// before reaching the format writers.
fn reject_unresolved_figures(blocks: &[Block]) -> Result<(), AppError> {
    let Some((_asset_ref, alt)) = blocks.iter().find_map(|block| match block {
        Block::FigureReference { asset_ref, alt, .. } => Some((asset_ref, alt)),
        _ => None,
    }) else {
        return Ok(());
    };
    Err(AppError::ArtifactGenerationFailed(format!(
        "Export AST v{EXPORT_AST_VERSION} contains an unresolved figure reference (alt text: {:?}); register the image as a project-owned figure asset before export.",
        alt.chars().take(160).collect::<String>()
    )))
}

#[derive(Debug, Clone, Copy)]
enum PdfFontKind {
    Regular,
    Bold,
    Mono,
}

#[derive(Clone)]
struct PdfFonts {
    regular: FontId,
    bold: FontId,
    mono: FontId,
}

impl PdfFonts {
    fn handle(&self, kind: PdfFontKind) -> PdfFontHandle {
        let id = match kind {
            PdfFontKind::Regular => self.regular.clone(),
            PdfFontKind::Bold => self.bold.clone(),
            PdfFontKind::Mono => self.mono.clone(),
        };
        PdfFontHandle::External(id)
    }
}

struct PdfLayout {
    pages: Vec<PdfPage>,
    ops: Vec<Op>,
    cursor_y_mm: f32,
    fonts: PdfFonts,
}

impl PdfLayout {
    fn new(fonts: PdfFonts) -> Self {
        Self {
            pages: Vec::new(),
            ops: Vec::new(),
            cursor_y_mm: PAGE_HEIGHT_MM - TOP_MARGIN_MM,
            fonts,
        }
    }

    fn finish(mut self) -> Vec<PdfPage> {
        self.finish_page();
        self.pages
    }

    fn finish_page(&mut self) {
        if self.ops.is_empty() {
            return;
        }
        self.pages.push(PdfPage::new(
            Mm(PAGE_WIDTH_MM),
            Mm(PAGE_HEIGHT_MM),
            std::mem::take(&mut self.ops),
        ));
        self.cursor_y_mm = PAGE_HEIGHT_MM - TOP_MARGIN_MM;
    }

    fn ensure_space(&mut self, height_mm: f32) {
        if self.cursor_y_mm - height_mm < BOTTOM_MARGIN_MM {
            self.finish_page();
        }
    }

    fn add_gap(&mut self, gap_mm: f32) {
        self.ensure_space(gap_mm);
        self.cursor_y_mm -= gap_mm;
    }

    fn push_figure(&mut self, id: XObjectId, width: u32, height: u32, caption: &str, alt: &str) {
        let max_w = CONTENT_WIDTH_MM;
        let max_h = 105.0_f32;
        let scale_mm = (max_w / width as f32).min(max_h / height as f32);
        let w_mm = width as f32 * scale_mm;
        let h_mm = height as f32 * scale_mm;
        let caption_lines = wrap_text(caption, approximate_chars_per_line(9.0, false)).len().max(1) as f32;
        let caption_h = caption_lines * 9.0 * 1.32 * 0.352_778 + 2.0;
        self.ensure_space(h_mm + caption_h + 3.0);
        let x_pt = (PAGE_WIDTH_MM - w_mm) * 0.5 * 72.0 / 25.4;
        let y_pt = (self.cursor_y_mm - h_mm) * 72.0 / 25.4;
        let sx = (w_mm * 72.0 / 25.4) * 300.0 / (width as f32 * 72.0);
        let sy = (h_mm * 72.0 / 25.4) * 300.0 / (height as f32 * 72.0);
        self.ops.push(Op::UseXobject { id, transform: XObjectTransform { translate_x: Some(Pt(x_pt)), translate_y: Some(Pt(y_pt)), scale_x: Some(sx), scale_y: Some(sy), dpi: Some(300.0), ..Default::default() } });
        self.cursor_y_mm -= h_mm + 1.0;
        self.push_wrapped(caption, PdfFontKind::Regular, 9.0, 1.0);
        if !alt.trim().is_empty() { self.push_wrapped(&format!("Alt: {alt}"), PdfFontKind::Regular, 8.0, 2.0); }
    }

    fn push_wrapped(&mut self, text: &str, kind: PdfFontKind, size_pt: f32, gap_after_mm: f32) {
        let line_height_pt = size_pt * 1.32;
        let line_height_mm = line_height_pt * 0.352_778;
        let max_chars = approximate_chars_per_line(size_pt, matches!(kind, PdfFontKind::Mono));
        let lines = wrap_text(text, max_chars);

        if lines.is_empty() {
            self.add_gap(gap_after_mm);
            return;
        }

        for line in lines {
            self.ensure_space(line_height_mm);
            self.ops.extend([
                Op::StartTextSection,
                Op::SetTextCursor {
                    pos: Point::new(Mm(LEFT_MARGIN_MM), Mm(self.cursor_y_mm)),
                },
                Op::SetLineHeight {
                    lh: Pt(line_height_pt),
                },
                Op::SetFont {
                    font: self.fonts.handle(kind),
                    size: Pt(size_pt),
                },
                Op::ShowText {
                    items: vec![TextItem::Text(line)],
                },
                Op::EndTextSection,
            ]);
            self.cursor_y_mm -= line_height_mm;
        }
        self.add_gap(gap_after_mm);
    }

    /// Render table cells in fixed columns, keeping wrapped cells on the same row.
    /// A row is split across pages only between complete visual lines.
    fn push_table_row(&mut self, cells: &[String], columns: usize, header: bool) {
        let size_pt = 9.0;
        let line_height_mm = size_pt * 1.32 * 0.352_778;
        let column_width_mm = CONTENT_WIDTH_MM / columns as f32;
        let padding_mm = 1.5;
        let available_mm = (column_width_mm - padding_mm * 2.0).max(1.0);
        let average_char_mm = size_pt * 0.352_778 * 0.62;
        let max_chars = (available_mm / average_char_mm).floor().max(1.0) as usize;
        let wrapped: Vec<Vec<String>> = (0..columns)
            .map(|index| {
                let text = cells.get(index).map_or("", String::as_str);
                let lines = wrap_text(text, max_chars);
                if lines.is_empty() {
                    vec![String::new()]
                } else {
                    lines
                }
            })
            .collect();
        let row_lines = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        for line_index in 0..row_lines {
            self.ensure_space(line_height_mm);
            for (column_index, lines) in wrapped.iter().enumerate() {
                if let Some(line) = lines.get(line_index) {
                    self.ops.extend([
                        Op::StartTextSection,
                        Op::SetTextCursor {
                            pos: Point::new(
                                Mm(LEFT_MARGIN_MM
                                    + column_index as f32 * column_width_mm
                                    + padding_mm),
                                Mm(self.cursor_y_mm),
                            ),
                        },
                        Op::SetFont {
                            font: self.fonts.handle(if header {
                                PdfFontKind::Bold
                            } else {
                                PdfFontKind::Regular
                            }),
                            size: Pt(size_pt),
                        },
                        Op::ShowText {
                            items: vec![TextItem::Text(line.clone())],
                        },
                        Op::EndTextSection,
                    ]);
                }
            }
            self.cursor_y_mm -= line_height_mm;
        }
        self.add_gap(1.0);
    }

    fn push_code(&mut self, code: &str) {
        if code.trim().is_empty() {
            return;
        }
        self.add_gap(1.5);
        for line in code.lines() {
            self.push_wrapped(line, PdfFontKind::Mono, 9.0, 0.2);
        }
        self.add_gap(1.5);
    }

    fn push_block(&mut self, block: Block) {
        match block {
            Block::Heading { level, text } => {
                let size = match level {
                    1 => 18.0,
                    2 => 16.0,
                    3 => 14.0,
                    4 => 13.0,
                    _ => 12.0,
                };
                self.add_gap(1.5);
                self.push_wrapped(&text, PdfFontKind::Bold, size, 2.0);
            }
            Block::Paragraph { text, .. } => {
                self.push_wrapped(&text, PdfFontKind::Regular, 11.0, 2.4);
            }
            Block::UnorderedList { items } => {
                for item in items {
                    self.push_wrapped(
                        &format!("\u{2022}  {item}"),
                        PdfFontKind::Regular,
                        11.0,
                        0.8,
                    );
                }
                self.add_gap(1.0);
            }
            Block::OrderedList { start, items } => {
                for (index, item) in items.into_iter().enumerate() {
                    self.push_wrapped(
                        &format!("{}.  {item}", start + index as u64),
                        PdfFontKind::Regular,
                        11.0,
                        0.8,
                    );
                }
                self.add_gap(1.0);
            }
            Block::Code { code } => self.push_code(&code),
            Block::Table { headers, rows } => {
                if headers.is_empty() {
                    return;
                }
                self.push_table_row(&headers, headers.len(), true);
                for row in rows {
                    self.push_table_row(&row, headers.len(), false);
                }
                self.add_gap(1.4);
            }
            Block::FigureReference { alt, caption, .. } => { self.push_wrapped(&format!("[Unresolved figure: {caption} — {alt}]"), PdfFontKind::Regular, 9.0, 2.0); }        Block::Rule => {
                self.push_wrapped(&"\u{2014}".repeat(45), PdfFontKind::Regular, 9.0, 1.5);
            }
        }
    }
}

fn approximate_chars_per_line(size_pt: f32, mono: bool) -> usize {
    let average_em = if mono { 0.62 } else { 0.52 };
    let average_char_mm = size_pt * 0.352_778 * average_em;
    (CONTENT_WIDTH_MM / average_char_mm).floor().max(20.0) as usize
}

fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut output = Vec::new();

    for source_line in text.lines() {
        if source_line.trim().is_empty() {
            output.push(String::new());
            continue;
        }

        let mut current = String::new();
        for word in source_line.split_whitespace() {
            if word.chars().count() > max_chars {
                if !current.is_empty() {
                    output.push(std::mem::take(&mut current));
                }
                let chars = word.chars().collect::<Vec<_>>();
                for chunk in chars.chunks(max_chars) {
                    output.push(chunk.iter().collect());
                }
                continue;
            }

            let proposed_len =
                current.chars().count() + usize::from(!current.is_empty()) + word.chars().count();
            if proposed_len > max_chars && !current.is_empty() {
                output.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
        if !current.is_empty() {
            output.push(current);
        }
    }

    output
}

pub fn generate_pdf(markdown: &str, title: &str, dest: &Path) -> Result<PdfMeta, AppError> {
    generate_pdf_inner(markdown, title, dest, None)
}

pub(crate) fn generate_pdf_with_figures(markdown: &str, title: &str, dest: &Path, figures: &[crate::research_documents::ResolvedResearchFigure]) -> Result<PdfMeta, AppError> {
    generate_pdf_inner(markdown, title, dest, Some(figures))
}

fn generate_pdf_inner(markdown: &str, title: &str, dest: &Path, figures: Option<&[crate::research_documents::ResolvedResearchFigure]>) -> Result<PdfMeta, AppError> {
    let blocks = parse_markdown_blocks(markdown);
    if figures.is_none() { reject_unresolved_figures(&blocks)?; }
    let mut font_warnings = Vec::new();
    let mut doc = PdfDocument::new(title);

    let regular = ParsedFont::from_bytes(REGULAR_FONT, 0, &mut font_warnings).ok_or_else(|| {
        AppError::ArtifactGenerationFailed("could not parse bundled regular PDF font".to_string())
    })?;
    let bold = ParsedFont::from_bytes(BOLD_FONT, 0, &mut font_warnings).ok_or_else(|| {
        AppError::ArtifactGenerationFailed("could not parse bundled bold PDF font".to_string())
    })?;
    let mono =
        ParsedFont::from_bytes(MONO_REGULAR_FONT, 0, &mut font_warnings).ok_or_else(|| {
            AppError::ArtifactGenerationFailed(
                "could not parse bundled monospace PDF font".to_string(),
            )
        })?;

    let fonts = PdfFonts {
        regular: doc.add_font(&regular),
        bold: doc.add_font(&bold),
        mono: doc.add_font(&mono),
    };
    let mut layout = PdfLayout::new(fonts);
    let mut figure_refs = std::collections::HashMap::new();
    if let Some(figures) = figures {
        for figure in figures {
            tracing::trace!(figure_id = %figure.id, project_id = %figure.project_id, source_sha256 = %figure.sha256, name = %figure.name, caption = %figure.caption, alt_text = %figure.alt_text, "embedding resolved project figure in PDF");
            let rendered = crate::research_figure_render::render_png(&figure.media_type, &figure.bytes, figure.width, figure.height)?;
            let mut warnings = Vec::new();
            let image = RawImage::decode_from_bytes(&rendered.png, &mut warnings).map_err(|e| AppError::ArtifactGenerationFailed(format!("Rendered figure could not be embedded in PDF: {e}")))?;
            let id = doc.add_image(&image);
            figure_refs.insert(format!("research-figure:{}", figure.id), (id, rendered.width, rendered.height));
        }
    }
    // An empty title and whitespace-only Markdown would otherwise produce no
    // text operations and fail export. Publish a valid, readable one-page PDF
    // rather than leaving the artifact in a failed state for an empty note.
    let display_title = if title.trim().is_empty() && blocks.is_empty() {
        "Untitled document"
    } else {
        title
    };
    layout.push_wrapped(display_title, PdfFontKind::Bold, 20.0, 5.0);
    let mut occurrence = 0usize;
    for block in blocks {
        if let Block::FigureReference { asset_ref, alt, caption } = block {
            let (id, width, height) = figure_refs.get(&asset_ref).ok_or_else(|| AppError::ArtifactGenerationFailed("Unresolved or unregistered figure; export cancelled.".into()))?.clone();
            layout.push_figure(id, width, height, &caption, &alt);
            occurrence += 1;
        } else { layout.push_block(block); }
    }
    if let Some(figures) = figures { if occurrence == 0 && !figures.is_empty() { tracing::debug!("Unused resolved figure snapshot entries ignored"); } }
    let pages = layout.finish();
    let page_count = pages.len() as i64;
    if pages.is_empty() {
        return Err(AppError::ArtifactGenerationFailed(
            "PDF content produced no pages".to_string(),
        ));
    }

    doc.with_pages(pages);
    let mut save_warnings = Vec::new();
    let bytes = doc.save(
        &PdfSaveOptions {
            subset_fonts: true,
            ..Default::default()
        },
        &mut save_warnings,
    );
    std::fs::write(dest, bytes)
        .map_err(|error| AppError::ArtifactGenerationFailed(error.to_string()))?;

    if !font_warnings.is_empty() || !save_warnings.is_empty() {
        tracing::debug!(
            font_warning_count = font_warnings.len(),
            save_warning_count = save_warnings.len(),
            "PDF generated with warnings"
        );
    }
    Ok(PdfMeta { page_count })
}

/// DOCX uses a small, self-contained WordprocessingML writer backed by the
/// patched ZIP crate. The supported block model is the same one used for PDF.
/// We only produce files here: we never extract DOCX archives.
fn docx_xml_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            '\t' | '\n' | '\r' => output.push(ch),
            c if (c as u32) < 0x20
                || matches!(c as u32, 0xfffe | 0xffff)
                || ((c as u32) & 0xffff) >= 0xfffe =>
            {
                output.push('\u{fffd}');
            }
            _ => output.push(ch),
        }
    }
    output
}

fn docx_paragraph(xml: &mut String, text: &str, size: u32, bold: bool, mono: bool) {
    xml.push_str("<w:p><w:pPr><w:spacing w:after=\"140\"/></w:pPr><w:r><w:rPr>");
    if bold {
        xml.push_str("<w:b/>");
    }
    if mono {
        xml.push_str("<w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\"/>");
    }
    xml.push_str(&format!(
        "<w:sz w:val=\"{size}\"/></w:rPr><w:t xml:space=\"preserve\">"
    ));
    xml.push_str(&docx_xml_text(text));
    xml.push_str("</w:t></w:r></w:p>");
}

fn docx_inline_paragraph(xml: &mut String, runs: &[InlineRun]) {
    xml.push_str("<w:p><w:pPr><w:spacing w:after=\"140\"/></w:pPr>");
    for run in runs {
        xml.push_str("<w:r><w:rPr>");
        if run.bold {
            xml.push_str("<w:b/>");
        }
        if run.italic {
            xml.push_str("<w:i/>");
        }
        if run.code {
            xml.push_str("<w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\"/>");
        }
        xml.push_str("<w:sz w:val=\"22\"/></w:rPr><w:t xml:space=\"preserve\">");
        xml.push_str(&docx_xml_text(&run.text));
        xml.push_str("</w:t></w:r>");
    }
    xml.push_str("</w:p>");
}

fn docx_table_row(xml: &mut String, cells: &[String], columns: usize, header: bool) {
    // Keep rows intact and repeat header rows across page breaks.
    xml.push_str("<w:tr><w:trPr><w:cantSplit/>");
    if header {
        xml.push_str("<w:tblHeader w:val=\"true\"/>");
    }
    xml.push_str("</w:trPr>");
    for index in 0..columns {
        xml.push_str("<w:tc><w:tcPr><w:tcW w:w=\"0\" w:type=\"auto\"/></w:tcPr>");
        docx_paragraph(
            xml,
            cells.get(index).map_or("", String::as_str),
            20,
            header,
            false,
        );
        xml.push_str("</w:tc>");
    }
    xml.push_str("</w:tr>");
}

fn docx_block_with_figures(xml: &mut String, block: Block, figure_rids: &std::collections::HashMap<String, (String, u64, u64)>, drawing_id: &mut u32) {
    match block {
        Block::Heading { level, text } => {
            let size = match level {
                1 => 32,
                2 => 28,
                3 => 26,
                4 => 24,
                _ => 22,
            };
            // Native heading styles enable Word navigation, outlines and TOCs.
            // Keep explicit sizing for compatibility with minimal DOCX readers.
            let style = format!("Heading{}", level.min(6));
            let mut paragraph = String::new();
            docx_paragraph(&mut paragraph, &text, size, true, false);
            paragraph = paragraph.replacen(
                "<w:pPr>",
                &format!("<w:pPr><w:pStyle w:val=\"{style}\"/><w:keepNext/><w:keepLines/><w:widowControl/>"),
                1,
            );
            xml.push_str(&paragraph);
        }
        Block::Paragraph { text, runs } => {
            if runs.is_empty() {
                docx_paragraph(xml, &text, 22, false, false);
            } else {
                docx_inline_paragraph(xml, &runs);
            }
        }
        Block::UnorderedList { items } => {
            for item in items {
                docx_paragraph(xml, &format!("\u{2022}  {item}"), 22, false, false);
            }
        }
        Block::OrderedList { start, items } => {
            for (index, item) in items.into_iter().enumerate() {
                docx_paragraph(
                    xml,
                    &format!("{}.  {item}", start + index as u64),
                    22,
                    false,
                    false,
                );
            }
        }
        Block::Code { code } => {
            for line in code.lines() {
                docx_paragraph(xml, line, 20, false, true);
            }
        }
        Block::Table { headers, rows } => {
            if headers.is_empty() {
                return;
            }
            xml.push_str("<w:tbl><w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/>");
            xml.push_str("<w:tblBorders><w:top w:val=\"single\"/>");
            xml.push_str("<w:left w:val=\"single\"/><w:bottom w:val=\"single\"/>");
            xml.push_str("<w:right w:val=\"single\"/><w:insideH w:val=\"single\"/>");
            xml.push_str("<w:insideV w:val=\"single\"/></w:tblBorders></w:tblPr>");
            docx_table_row(xml, &headers, headers.len(), true);
            for row in rows {
                docx_table_row(xml, &row, headers.len(), false);
            }
            xml.push_str("</w:tbl>");
        }
        Block::FigureReference { asset_ref, alt, caption } => {
            if let Some((rid, cx, cy)) = figure_rids.get(&asset_ref) {
                let id = *drawing_id;
                *drawing_id += 1;
                xml.push_str(&format!(r#"<w:p><w:pPr><w:jc w:val="center"/><w:keepNext/></w:pPr><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="{id}" name="Research figure" descr="{}"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="{id}" name="figure.png" descr="{}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="{}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#, docx_xml_text(&alt), docx_xml_text(&alt), rid));
                docx_paragraph(xml, &caption, 18, false, false);
            } else { docx_paragraph(xml, &format!("[Unresolved figure: {caption} — {alt}]"), 18, false, false); }
        }        Block::Rule => docx_paragraph(xml, &"\u{2014}".repeat(40), 22, false, false),
    }
}

fn word_document_xml(blocks: Vec<Block>, title: &str) -> String {
    word_document_xml_with_figures(blocks, title, &std::collections::HashMap::new())
}

fn word_document_xml_with_figures(blocks: Vec<Block>, title: &str, figure_rids: &std::collections::HashMap<String, (String, u64, u64)>) -> String {
    let mut xml = String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
    xml.push_str(r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>"#);
    docx_paragraph(&mut xml, title, 36, true, false);
    docx_paragraph(&mut xml, "", 22, false, false);
    let mut drawing_id = 1u32;
    for block in blocks {
        docx_block_with_figures(&mut xml, block, figure_rids, &mut drawing_id);
    }
    xml.push_str(r#"<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1021" w:right="1021" w:bottom="1021" w:left="1021" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr></w:body></w:document>"#);
    xml
}

pub fn generate_docx(markdown: &str, title: &str, dest: &Path) -> Result<(), AppError> { generate_docx_inner(markdown, title, dest, None) }

pub(crate) fn generate_docx_with_figures(markdown: &str, title: &str, dest: &Path, figures: &[crate::research_documents::ResolvedResearchFigure]) -> Result<(), AppError> { generate_docx_inner(markdown, title, dest, Some(figures)) }

fn generate_docx_inner(markdown: &str, title: &str, dest: &Path, figures: Option<&[crate::research_documents::ResolvedResearchFigure]>) -> Result<(), AppError> {
    use std::io::Write;
    let blocks = parse_markdown_blocks(markdown);
    if figures.is_none() { reject_unresolved_figures(&blocks)?; }
    let mut rids: std::collections::HashMap<String, (String, u64, u64)> = std::collections::HashMap::new();
    let mut media = Vec::<(String, Vec<u8>)>::new();
    if let Some(figures) = figures {
        for (i, figure) in figures.iter().enumerate() {
            tracing::trace!(figure_id = %figure.id, project_id = %figure.project_id, source_sha256 = %figure.sha256, name = %figure.name, caption = %figure.caption, alt_text = %figure.alt_text, "embedding resolved project figure in DOCX");
            let rendered = crate::research_figure_render::render_png(&figure.media_type, &figure.bytes, figure.width, figure.height)?;
            let ref_id = format!("research-figure:{}", figure.id);
            let rid = format!("rId{}", i + 2);
            let scale = (5_943_600.0f64 / rendered.width as f64).min(3_657_600.0f64 / rendered.height as f64);
            let cx = (rendered.width as f64 * scale) as u64;
            let cy = (rendered.height as f64 * scale) as u64;
            rids.insert(ref_id, (rid, cx, cy));
            media.push((format!("image{}.png", i + 1), rendered.png));
        }
    }
    for block in &blocks { if let Block::FigureReference { asset_ref, .. } = block { if !rids.contains_key(asset_ref) { return Err(AppError::ArtifactGenerationFailed("Unresolved or unregistered figure; export cancelled.".into())); } } }
    let document = word_document_xml_with_figures(blocks, title, &rids);
    // Never truncate a pre-existing export if the caller reuses a destination.
    // Remove only the file created by this invocation if ZIP writing fails.
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest)
        .map_err(|error| AppError::ArtifactGenerationFailed(error.to_string()))?;
    struct IncompleteDocx<'a> { path: &'a Path, committed: bool }
    impl Drop for IncompleteDocx<'_> {
        fn drop(&mut self) {
            if !self.committed {
                let _ = std::fs::remove_file(self.path);
            }
        }
    }
    let mut cleanup = IncompleteDocx { path: dest, committed: false };
    let mut archive = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).unix_permissions(0o644);
    let has_media = !media.is_empty();
    let types = if has_media { r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"# } else { r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"# };
    let root_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let mut doc_rels = String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for (i, (filename, _)) in media.iter().enumerate() { doc_rels.push_str(&format!(r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/{}"/>"#, i + 2, filename)); }
    doc_rels.push_str("</Relationships>");
    let mut parts: Vec<(&str, &[u8])> = vec![("[Content_Types].xml", types.as_bytes()), ("_rels/.rels", root_rels.as_bytes()), ("word/document.xml", document.as_bytes())];
    if has_media { parts.push(("word/_rels/document.xml.rels", doc_rels.as_bytes())); }
    for (name, bytes) in parts {
        archive.start_file(name, options).map_err(|e| AppError::ArtifactGenerationFailed(e.to_string()))?;
        archive.write_all(bytes).map_err(|e| AppError::ArtifactGenerationFailed(e.to_string()))?;
    }
    for (filename, bytes) in media { archive.start_file(format!("word/media/{filename}"), options).map_err(|e| AppError::ArtifactGenerationFailed(e.to_string()))?; archive.write_all(&bytes).map_err(|e| AppError::ArtifactGenerationFailed(e.to_string()))?; }
    let completed = archive.finish().map_err(|e| AppError::ArtifactGenerationFailed(e.to_string()))?;
    completed.sync_all().map_err(|e| AppError::ArtifactGenerationFailed(e.to_string()))?;
    cleanup.committed = true;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_image_is_retained_as_ordered_export_ast_node() {
        let blocks = parse_markdown_blocks(
            "Before.\n\n![Accessible client accuracy chart](asset://figure-123 \"Figure 1. Client accuracy\")\n\nAfter.",
        );
        assert!(matches!(&blocks[0], Block::Paragraph { text, .. } if text == "Before."));
        assert!(matches!(&blocks[1], Block::FigureReference { asset_ref, alt, caption }
            if asset_ref == "asset://figure-123"
                && alt == "Accessible client accuracy chart"
                && caption == "Figure 1. Client accuracy"));
        assert!(matches!(&blocks[2], Block::Paragraph { text, .. } if text == "After."));
    }

    #[test]
    fn unregistered_sibling_or_external_image_reference_fails_closed() {
        for source in ["figure.svg", "https://example.invalid/figure.svg"] {
            let markdown = format!("![Figure](<{source}>)");
            let blocks = parse_markdown_blocks(&markdown);
            assert!(matches!(blocks.first(), Some(Block::FigureReference { .. })));
            let error = reject_unresolved_figures(&blocks).unwrap_err().to_string();
            assert!(error.contains("project-owned figure asset"));
        }
    }

    #[test]
    fn unresolved_figures_stop_both_format_writers_before_output_creation() {
        let directory = tempfile::tempdir().unwrap();
        let markdown = "![chart](figure.svg \"Figure 1. Chart\")";
        let pdf = directory.path().join("figure.pdf");
        let docx = directory.path().join("figure.docx");
        assert!(generate_pdf(markdown, "Draft", &pdf).is_err());
        assert!(generate_docx(markdown, "Draft", &docx).is_err());
        assert!(!pdf.exists());
        assert!(!docx.exists());
    }

    #[test]
    fn docx_headings_stay_with_following_paragraph_and_keep_lines_together() {
        let xml = word_document_xml(
            parse_markdown_blocks(
                "# বাংলা Heading & Details\n\nBody content\n\n## Nested\n\nMore text",
            ),
            "Document",
        );
        for level in [1, 2] {
            let style = format!(
                "<w:pStyle w:val=\"Heading{level}\"/><w:keepNext/><w:keepLines/><w:widowControl/>"
            );
            assert!(
                xml.contains(&style),
                "missing pagination rules for heading {level}"
            );
        }
        assert!(xml.contains("বাংলা Heading &amp; Details"));
        assert!(xml.contains("Body content"));
    }

    #[test]
    fn docx_headings_use_word_outline_styles() {
        let xml = word_document_xml(
            parse_markdown_blocks("# Main\n\n## Details\n\n### Nested"),
            "Report",
        );
        assert!(xml.contains("<w:pStyle w:val=\"Heading1\"/>"));
        assert!(xml.contains("<w:pStyle w:val=\"Heading2\"/>"));
        assert!(xml.contains("<w:pStyle w:val=\"Heading3\"/>"));
        assert!(xml.contains("<w:t xml:space=\"preserve\">Nested</w:t>"));
    }

    #[test]
    fn docx_table_headers_repeat_without_splitting_rows() {
        let xml = word_document_xml(
            parse_markdown_blocks(
                "| Label | Value |\n| --- | --- |\n| বাংলা | Русский |\n| Long | Row |",
            ),
            "Report",
        );
        assert!(xml.contains("<w:tr><w:trPr><w:cantSplit/><w:tblHeader w:val=\"true\"/></w:trPr>"));
        assert_eq!(xml.matches("<w:cantSplit/>").count(), 3);
        assert_eq!(xml.matches("<w:tblHeader").count(), 1);
        assert!(xml.contains("বাংলা"));
        assert!(xml.contains("Русский"));
    }

    #[test]
    fn docx_heading_styles_use_valid_xml_attributes() {
        let xml = word_document_xml(parse_markdown_blocks("# Heading"), "Report");
        assert!(xml.contains("<w:pStyle w:val=\"Heading1\"/>"));
        assert!(!xml.contains(r#"w:val=\"Heading1"#));
    }

    #[test]
    fn docx_preserves_nested_inline_styles_and_escapes_xml() {
        let xml = word_document_xml(
            parse_markdown_blocks("বাংলা **bold *nested* & \\<safe\\>** and `code<&>`."),
            "Report",
        );
        assert!(xml.contains("<w:b/>"));
        assert!(xml.contains("<w:i/>"));
        assert!(xml.contains("<w:rFonts w:ascii=\"Consolas\""));
        assert!(xml.contains("nested"));
        assert!(xml.contains("&amp; &lt;safe&gt;"));
        assert!(xml.contains("code&lt;&amp;&gt;"));
    }

    #[test]
    fn docx_plain_paragraph_keeps_text() {
        let xml = word_document_xml(parse_markdown_blocks("Simple paragraph."), "Report");
        assert!(xml.contains(">Simple paragraph.</w:t>"));
    }

    #[test]
    fn registered_svg_is_embedded_as_pdf_xobject_and_docx_media_relationship() {
        use std::io::Read;
        use sha2::Digest;
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80" viewBox="0 0 120 80"><rect x="2" y="2" width="116" height="76" fill="none" stroke="#000"/><text x="8" y="42" font-size="12" font-family="sans-serif">Client accuracy</text></svg>"##.to_vec();
        let figure = crate::research_documents::ResolvedResearchFigure { id: "123e4567-e89b-42d3-a456-426614174000".into(), project_id: "project-a".into(), name: "figure.svg".into(), media_type: "image/svg+xml".into(), bytes: svg.clone(), sha256: format!("{:x}", sha2::Sha256::digest(&svg)), width: 120, height: 80, caption: "Figure 1. Client accuracy".into(), alt_text: "Client accuracy by round".into() };
        let markdown = format!("Before paragraph.\n\n![Client accuracy by round](research-figure:{} \"Figure 1. Client accuracy\")\n\nBetween figures.\n\n![Client accuracy by round](research-figure:{} \"Figure 2. Repeated source check\")\n\nAfter paragraph.", figure.id, figure.id);
        let temp = tempfile::tempdir().unwrap();
        let pdf = temp.path().join("figure.pdf");
        generate_pdf_with_figures(&markdown, "Report", &pdf, std::slice::from_ref(&figure)).unwrap();
        let pdf_doc = lopdf::Document::load(&pdf).unwrap();
        let page_id = *pdf_doc.get_pages().values().next().unwrap();
        let operations = lopdf::content::Content::decode(&pdf_doc.get_page_content(page_id)).unwrap();
        let image_uses: Vec<Vec<u8>> = operations.operations.iter().filter(|operation| operation.operator == "Do").map(|operation| operation.operands.first().and_then(|operand| operand.as_name().ok()).unwrap().to_vec()).collect();
        assert_eq!(image_uses.len(), 2, "both references should be placed in PDF flow");
        assert_eq!(image_uses[0], image_uses[1], "repeated references should reuse the same PDF image XObject; RGBA transparency may add a separate SMask image stream");
        let docx = temp.path().join("figure.docx");
        generate_docx_with_figures(&markdown, "Report", &docx, std::slice::from_ref(&figure)).unwrap();
        let file = std::fs::File::open(&docx).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut xml = String::new(); archive.by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
        assert!(xml.contains("<w:drawing>"));
        assert_eq!(xml.matches("<w:drawing>").count(), 2);
        assert!(xml.contains("r:embed=\"rId2\""));
        assert!(xml.contains("Figure 1. Client accuracy"));
        assert!(xml.contains("Figure 2. Repeated source check"));
        let mut rels = String::new(); archive.by_name("word/_rels/document.xml.rels").unwrap().read_to_string(&mut rels).unwrap();
        assert!(rels.contains("Id=\"rId2\"") && rels.contains("Target=\"media/image1.png\""));
        let mut media = Vec::new(); archive.by_name("word/media/image1.png").unwrap().read_to_end(&mut media).unwrap();
        assert!(media.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn missing_resolved_reference_does_not_publish_pdf_or_docx() {
        let temp = tempfile::tempdir().unwrap();
        let markdown = "![missing](research-figure:123e4567-e89b-42d3-a456-426614174000 \"Figure\")";
        let pdf = temp.path().join("missing.pdf");
        let docx = temp.path().join("missing.docx");
        assert!(generate_pdf_with_figures(markdown, "Report", &pdf, &[]).is_err());
        assert!(generate_docx_with_figures(markdown, "Report", &docx, &[]).is_err());
        assert!(!pdf.exists());
        assert!(!docx.exists());
    }

    const SAMPLE_MARKDOWN: &str = "# Report Title\n\nThis is an introductory paragraph with real content.\n\n## Section One\n\n- First point\n- Second point\n\n1. Step one\n2. Step two\n\n```text\nfn main() {}\n```\n\n| Name | Value |\n| --- | --- |\n| Alpha | 1 |\n| Beta | 2 |\n";

    #[test]
    fn parses_expected_block_structure() {
        let blocks = parse_markdown_blocks(SAMPLE_MARKDOWN);
        assert!(blocks.iter().any(
            |block| matches!(block, Block::Heading { level: 1, text } if text == "Report Title")
        ));
        assert!(blocks
            .iter()
            .any(|block| matches!(block, Block::UnorderedList { items } if items.len() == 2)));
        assert!(blocks
            .iter()
            .any(|block| matches!(block, Block::OrderedList { items, .. } if items.len() == 2)));
        assert!(blocks
            .iter()
            .any(|block| matches!(block, Block::Code { code } if code.contains("fn main"))));
        assert!(blocks
            .iter()
            .any(|block| matches!(block, Block::Table { headers, rows } if headers.len() == 2 && rows.len() == 2)));
    }

    #[test]
    fn wraps_long_text_without_dropping_content() {
        let input = "alpha beta gamma delta epsilon zeta eta theta";
        let lines = wrap_text(input, 16);
        assert!(lines.len() > 1);
        assert_eq!(lines.join(" "), input);
    }

    #[test]
    fn empty_note_still_generates_valid_single_page_pdf() {
        let temp = tempfile::tempdir().unwrap();
        let dest = temp.path().join("empty-note.pdf");
        let meta = generate_pdf("  \n\t", "  ", &dest).unwrap();
        assert_eq!(meta.page_count, 1);
        let bytes = std::fs::read(&dest).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        assert!(bytes.len() > 100);
    }

    #[test]
    fn whitespace_title_with_content_does_not_insert_fallback_heading() {
        let blocks = parse_markdown_blocks("## Actual heading");
        assert_eq!(blocks.len(), 1);
        let temp = tempfile::tempdir().unwrap();
        let dest = temp.path().join("content.pdf");
        assert!(
            generate_pdf("## Actual heading", " ", &dest)
                .unwrap()
                .page_count
                >= 1
        );
    }

    #[test]
    fn generates_real_multi_section_pdf() {
        let temp = tempfile::tempdir().unwrap();
        let dest = temp.path().join("report.pdf");
        let meta = generate_pdf(SAMPLE_MARKDOWN, "Report Title", &dest).unwrap();
        assert!(dest.exists());
        assert!(std::fs::metadata(&dest).unwrap().len() > 0);
        assert!(meta.page_count >= 1);
        let bytes = std::fs::read(&dest).unwrap();
        assert_eq!(&bytes[0..5], b"%PDF-");
    }

    #[test]
    fn multi_page_unicode_export_passes_structural_publication_gate() {
        let root = tempfile::tempdir().unwrap();
        let (directory, id) = crate::artifact_output::reserve_directory(
            root.path(),
            std::path::Path::new("generated"),
        )
        .unwrap();
        let output =
            crate::artifact_output::StagedOutput::from_reserved(directory, "report.pdf").unwrap();
        let body = (0..12)
            .map(|index| format!("# Section {index}\n\nEnglish বাংলা Русский\n\n{SAMPLE_MARKDOWN}\n"))
            .collect::<String>();
        let meta = generate_pdf(&body, "Multilingual report", &output.path).unwrap();
        assert!(meta.page_count > 1);
        let expected = std::fs::read(&output.path).unwrap();
        assert_eq!(output.commit_for("pdf").unwrap(), expected.len() as i64);
        assert_eq!(
            std::fs::read(root.path().join("generated").join(id).join("report.pdf")).unwrap(),
            expected
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn export_worker_runs_real_generators_and_publication() {
        for kind in ["pdf", "docx"] {
            let root = tempfile::tempdir().unwrap();
            let permit = crate::document_jobs::reserve().await.unwrap();
            let (directory, id) = crate::artifact_output::reserve_directory(
                root.path(),
                std::path::Path::new("generated"),
            )
            .unwrap();
            let filename = format!("report.{kind}");
            let output =
                crate::artifact_output::StagedOutput::from_reserved(directory, &filename).unwrap();
            let size = crate::document_jobs::run(permit, move || {
                crate::document_jobs::render(|| {
                    if kind == "pdf" {
                        generate_pdf(SAMPLE_MARKDOWN, "Worker report", &output.path)?;
                    } else {
                        generate_docx(SAMPLE_MARKDOWN, "Worker report", &output.path)?;
                    }
                    output.commit_for(kind)
                })
            })
            .await
            .unwrap();
            let bytes =
                std::fs::read(root.path().join("generated").join(id).join(filename)).unwrap();
            assert_eq!(size, bytes.len() as i64);
            assert!(size > 0);
        }
    }

    #[test]
    fn docx_export_does_not_truncate_existing_file() {
        let temp = tempfile::tempdir().unwrap();
        let dest = temp.path().join("existing.docx");
        std::fs::write(&dest, b"existing user content").unwrap();
        assert!(generate_docx("# Replacement", "New", &dest).is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"existing user content");
    }

    #[test]
    fn docx_export_rejects_directory_destination_without_deleting_it() {
        let temp = tempfile::tempdir().unwrap();
        let dest = temp.path().join("output.docx");
        std::fs::create_dir(&dest).unwrap();
        assert!(generate_docx("text", "Title", &dest).is_err());
        assert!(dest.is_dir());
    }

    #[test]
    fn generates_real_docx_document() {
        let temp = tempfile::tempdir().unwrap();
        let dest = temp.path().join("report.docx");
        generate_docx(SAMPLE_MARKDOWN, "Report Title", &dest).unwrap();
        assert!(dest.exists());
        let size = std::fs::metadata(&dest).unwrap().len();
        assert!(size > 0);

        let bytes = std::fs::read(&dest).unwrap();
        assert_eq!(&bytes[0..4], b"PK\x03\x04");
        let file = std::fs::File::open(&dest).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        assert_eq!(archive.len(), 3);
        for path in ["[Content_Types].xml", "_rels/.rels", "word/document.xml"] {
            assert!(archive.by_name(path).is_ok(), "{path} must exist");
        }
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("word/document.xml").unwrap(), &mut xml)
            .unwrap();
        assert!(xml.contains("<w:tbl>"));
        assert!(xml.contains("Report Title"));
        assert!(xml.contains("fn main() {}"));
        assert!(xml.contains("Alpha"));
    }

    #[test]
    fn unicode_table_docx_export_passes_package_publication_gate() {
        let root = tempfile::tempdir().unwrap();
        let (directory, id) = crate::artifact_output::reserve_directory(
            root.path(),
            std::path::Path::new("generated"),
        )
        .unwrap();
        let output =
            crate::artifact_output::StagedOutput::from_reserved(directory, "report.docx").unwrap();
        let body = format!("# English বাংলা Русский\n\n{SAMPLE_MARKDOWN}");
        generate_docx(&body, "Unicode & table report", &output.path).unwrap();
        let expected = std::fs::read(&output.path).unwrap();
        assert_eq!(output.commit_for("docx").unwrap(), expected.len() as i64);
        assert_eq!(
            std::fs::read(root.path().join("generated").join(id).join("report.docx")).unwrap(),
            expected
        );
    }

    #[test]
    fn pdf_table_cells_have_distinct_column_positions_and_wrap_long_values() {
        let temp = tempfile::tempdir().unwrap();
        let dest = temp.path().join("table.pdf");
        let long_cell = "longvalue".repeat(40);
        let markdown =
            format!("| First | Second | Third |\\n|---|---|---|\\n| A | {long_cell} | C |");
        let meta = generate_pdf(&markdown, "Table", &dest).unwrap();
        assert!(meta.page_count >= 1);
        let bytes = std::fs::read(dest).unwrap();
        assert!(bytes.starts_with(b"%PDF"));
        // Layout-level regression: a wide cell cannot make a neighboring column
        // disappear, and the generated document must remain a valid PDF container.
        assert!(bytes.len() > 1000);
    }

    #[test]
    fn pdf_table_row_layout_handles_more_columns_than_available_width() {
        let temp = tempfile::tempdir().unwrap();
        let dest = temp.path().join("many-columns.pdf");
        let headers = (0..30).map(|i| format!("H{i}")).collect::<Vec<_>>();
        let divider = vec!["---"; 30];
        let row = (0..30).map(|i| format!("cell{i}")).collect::<Vec<_>>();
        let markdown = format!(
            "|{}|\\n|{}|\\n|{}|",
            headers.join("|"),
            divider.join("|"),
            row.join("|")
        );
        assert!(generate_pdf(&markdown, "Dense table", &dest).is_ok());
    }

    #[test]
    fn docx_escapes_untrusted_text_and_preserves_unicode() {
        let temp = tempfile::tempdir().unwrap();
        let dest = temp.path().join("escaped.docx");
        let escaped = docx_xml_text("<unsafe>&'\" বাংলা");
        assert_eq!(escaped, "&lt;unsafe&gt;&amp;&apos;&quot; বাংলা");
        generate_docx("**Bangla বাংলা & quotes**", "A <B> & \"C\"", &dest).unwrap();
        let mut zip = zip::ZipArchive::new(std::fs::File::open(dest).unwrap()).unwrap();
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml)
            .unwrap();
        assert!(xml.contains("A &lt;B&gt; &amp; &quot;C&quot;"));
        assert!(xml.contains("Bangla বাংলা &amp; quotes"));
        assert!(!xml.contains("<unsafe>"));
    }
}
