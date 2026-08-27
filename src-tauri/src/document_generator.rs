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

#[derive(Debug, Clone, Copy)]
pub struct PdfMeta {
    pub page_count: i64,
}

/// A simplified block-level document model shared by the PDF and DOCX generators.
///
/// Inline formatting (bold/italic/inline-code runs within a paragraph) is intentionally
/// flattened to plain text; only block structure (headings, paragraphs, lists, code blocks,
/// tables, rules) is preserved. This keeps both renderers small and predictable.
#[derive(Debug, Clone)]
enum Block {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph {
        text: String,
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
    let mut in_code_block = false;

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
                    in_paragraph = true;
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if in_paragraph {
                    let text = text_buffer.trim().to_string();
                    if !text.is_empty() {
                        blocks.push(Block::Paragraph { text });
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
                    });
                }
                text_buffer.clear();
            }
            Event::Rule => blocks.push(Block::Rule),
            Event::Text(text) | Event::Code(text) => {
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
                }
            }
            Event::HardBreak => {
                if in_item {
                    item_buffer.push(' ');
                } else {
                    text_buffer.push(' ');
                }
            }
            _ => {}
        }
    }

    blocks
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
            Block::Paragraph { text } => {
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
                self.push_wrapped(
                    &headers.join("  |  "),
                    PdfFontKind::Bold,
                    9.5,
                    0.8,
                );
                self.push_wrapped(
                    &headers.iter().map(|_| "--------").collect::<Vec<_>>().join("-+-"),
                    PdfFontKind::Mono,
                    8.0,
                    0.5,
                );
                for row in rows {
                    let normalized = (0..headers.len())
                        .map(|index| row.get(index).cloned().unwrap_or_default())
                        .collect::<Vec<_>>();
                    self.push_wrapped(
                        &normalized.join("  |  "),
                        PdfFontKind::Regular,
                        9.5,
                        0.7,
                    );
                }
                self.add_gap(1.4);
            }
            Block::Rule => {
                self.push_wrapped(
                    &"\u{2014}".repeat(45),
                    PdfFontKind::Regular,
                    9.0,
                    1.5,
                );
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

            let proposed_len = current.chars().count() + usize::from(!current.is_empty()) + word.chars().count();
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
    let blocks = parse_markdown_blocks(markdown);
    let mut warnings = Vec::new();
    let mut doc = PdfDocument::new(title);

    let regular = ParsedFont::from_bytes(REGULAR_FONT, 0, &mut warnings).ok_or_else(|| {
        AppError::ArtifactGenerationFailed("could not parse bundled regular PDF font".to_string())
    })?;
    let bold = ParsedFont::from_bytes(BOLD_FONT, 0, &mut warnings).ok_or_else(|| {
        AppError::ArtifactGenerationFailed("could not parse bundled bold PDF font".to_string())
    })?;
    let mono = ParsedFont::from_bytes(MONO_REGULAR_FONT, 0, &mut warnings).ok_or_else(|| {
        AppError::ArtifactGenerationFailed("could not parse bundled monospace PDF font".to_string())
    })?;

    let fonts = PdfFonts {
        regular: doc.add_font(&regular),
        bold: doc.add_font(&bold),
        mono: doc.add_font(&mono),
    };
    let mut layout = PdfLayout::new(fonts);
    layout.push_wrapped(title, PdfFontKind::Bold, 20.0, 5.0);
    for block in blocks {
        layout.push_block(block);
    }
    let pages = layout.finish();
    let page_count = pages.len() as i64;
    if pages.is_empty() {
        return Err(AppError::ArtifactGenerationFailed(
            "PDF content produced no pages".to_string(),
        ));
    }

    doc.with_pages(pages);
    let bytes = doc.save(
        &PdfSaveOptions {
            subset_fonts: true,
            ..Default::default()
        },
        &mut warnings,
    );
    std::fs::write(dest, bytes)
        .map_err(|error| AppError::ArtifactGenerationFailed(error.to_string()))?;

    if !warnings.is_empty() {
        tracing::debug!(warning_count = warnings.len(), "PDF generated with warnings");
    }
    Ok(PdfMeta { page_count })
}

pub fn generate_docx(markdown: &str, title: &str, dest: &Path) -> Result<(), AppError> {
    let blocks = parse_markdown_blocks(markdown);

    let mut docx = docx_rs::Docx::new().add_paragraph(
        docx_rs::Paragraph::new().add_run(docx_rs::Run::new().add_text(title).bold().size(36)),
    );
    docx = docx.add_paragraph(docx_rs::Paragraph::new());

    for block in blocks {
        docx = push_docx_block(docx, block);
    }

    let file = std::fs::File::create(dest)
        .map_err(|error| AppError::ArtifactGenerationFailed(error.to_string()))?;
    docx.pack(file)
        .map_err(|error| AppError::ArtifactGenerationFailed(error.to_string()))?;
    Ok(())
}

fn push_docx_block(docx: docx_rs::Docx, block: Block) -> docx_rs::Docx {
    use docx_rs::{Paragraph, Run, Table, TableCell, TableRow};

    match block {
        Block::Heading { level, text } => {
            let size = match level {
                1 => 32,
                2 => 28,
                3 => 26,
                4 => 24,
                _ => 22,
            };
            docx.add_paragraph(
                Paragraph::new().add_run(Run::new().add_text(text).bold().size(size)),
            )
        }
        Block::Paragraph { text } => {
            docx.add_paragraph(Paragraph::new().add_run(Run::new().add_text(text).size(22)))
        }
        Block::UnorderedList { items } => {
            let mut updated = docx;
            for item in items {
                updated = updated.add_paragraph(
                    Paragraph::new()
                        .add_run(Run::new().add_text(format!("\u{2022}  {item}")).size(22)),
                );
            }
            updated
        }
        Block::OrderedList { start, items } => {
            let mut updated = docx;
            for (index, item) in items.into_iter().enumerate() {
                let number = start + index as u64;
                updated = updated.add_paragraph(
                    Paragraph::new()
                        .add_run(Run::new().add_text(format!("{number}.  {item}")).size(22)),
                );
            }
            updated
        }
        Block::Code { code } => {
            let mut updated = docx;
            for line in code.lines() {
                updated = updated
                    .add_paragraph(Paragraph::new().add_run(Run::new().add_text(line).size(20)));
            }
            updated
        }
        Block::Table { headers, rows } => {
            if headers.is_empty() {
                return docx;
            }
            let mut table_rows = vec![TableRow::new(
                headers
                    .iter()
                    .map(|header| {
                        TableCell::new().add_paragraph(
                            Paragraph::new().add_run(Run::new().add_text(header).bold()),
                        )
                    })
                    .collect(),
            )];
            for row in rows {
                table_rows.push(TableRow::new(
                    row.iter()
                        .map(|cell| {
                            TableCell::new()
                                .add_paragraph(Paragraph::new().add_run(Run::new().add_text(cell)))
                        })
                        .collect(),
                ));
            }
            docx.add_table(Table::new(table_rows))
        }
        Block::Rule => {
            docx.add_paragraph(Paragraph::new().add_run(Run::new().add_text("\u{2014}".repeat(40))))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn generates_real_docx_document() {
        let temp = tempfile::tempdir().unwrap();
        let dest = temp.path().join("report.docx");
        generate_docx(SAMPLE_MARKDOWN, "Report Title", &dest).unwrap();
        assert!(dest.exists());
        let size = std::fs::metadata(&dest).unwrap().len();
        assert!(size > 0);

        let bytes = std::fs::read(&dest).unwrap();
        assert_eq!(&bytes[0..4], b"PK\x03\x04");
    }
}
