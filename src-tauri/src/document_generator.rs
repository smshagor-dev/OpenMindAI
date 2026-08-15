use std::path::Path;

use genpdf::{elements, style, Element as _};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::app_error::AppError;

const REGULAR_FONT: &[u8] = include_bytes!("../assets/fonts/DejaVuSans-Regular.ttf");
const BOLD_FONT: &[u8] = include_bytes!("../assets/fonts/DejaVuSans-Bold.ttf");
const ITALIC_FONT: &[u8] = include_bytes!("../assets/fonts/DejaVuSans-Italic.ttf");
const BOLD_ITALIC_FONT: &[u8] = include_bytes!("../assets/fonts/DejaVuSans-BoldItalic.ttf");
const MONO_REGULAR_FONT: &[u8] = include_bytes!("../assets/fonts/DejaVuSansMono-Regular.ttf");
const MONO_BOLD_FONT: &[u8] = include_bytes!("../assets/fonts/DejaVuSansMono-Bold.ttf");
const MONO_ITALIC_FONT: &[u8] = include_bytes!("../assets/fonts/DejaVuSansMono-Italic.ttf");
const MONO_BOLD_ITALIC_FONT: &[u8] =
    include_bytes!("../assets/fonts/DejaVuSansMono-BoldItalic.ttf");

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
                // Paragraph/Run text in genpdf and docx-rs has no concept of an embedded line
                // break (it isn't a glyph either font can render) -- collapse to a space so a
                // hard break inside a paragraph degrades to normal word-wrapped text instead of
                // showing a missing-glyph box.
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

pub fn generate_pdf(markdown: &str, title: &str, dest: &Path) -> Result<PdfMeta, AppError> {
    let blocks = parse_markdown_blocks(markdown);

    let font_family = genpdf::fonts::FontFamily {
        regular: load_font(REGULAR_FONT)?,
        bold: load_font(BOLD_FONT)?,
        italic: load_font(ITALIC_FONT)?,
        bold_italic: load_font(BOLD_ITALIC_FONT)?,
    };
    let mut doc = genpdf::Document::new(font_family);
    doc.set_title(title);
    doc.set_font_size(11);
    doc.set_line_spacing(1.25);

    let mono_family = doc.add_font_family(genpdf::fonts::FontFamily {
        regular: load_font(MONO_REGULAR_FONT)?,
        bold: load_font(MONO_BOLD_FONT)?,
        italic: load_font(MONO_ITALIC_FONT)?,
        bold_italic: load_font(MONO_BOLD_ITALIC_FONT)?,
    });

    let mut decorator = genpdf::SimplePageDecorator::new();
    decorator.set_margins(20);
    doc.set_page_decorator(decorator);

    doc.push(elements::Paragraph::new(style::StyledString::new(
        title.to_string(),
        style::Style::new().bold().with_font_size(20),
    )));
    doc.push(elements::Break::new(1.5));

    for block in blocks {
        push_pdf_block(&mut doc, mono_family, block);
    }

    doc.render_to_file(dest)
        .map_err(|error| AppError::ArtifactGenerationFailed(error.to_string()))?;

    let page_count = count_pdf_pages(dest)?;
    Ok(PdfMeta { page_count })
}

fn load_font(bytes: &[u8]) -> Result<genpdf::fonts::FontData, AppError> {
    genpdf::fonts::FontData::new(bytes.to_vec(), None)
        .map_err(|error| AppError::ArtifactGenerationFailed(error.to_string()))
}

fn push_pdf_block(
    doc: &mut genpdf::Document,
    mono_family: genpdf::fonts::FontFamily<genpdf::fonts::Font>,
    block: Block,
) {
    match block {
        Block::Heading { level, text } => {
            let size = match level {
                1 => 18,
                2 => 16,
                3 => 14,
                4 => 13,
                _ => 12,
            };
            doc.push(elements::Break::new(0.5));
            doc.push(elements::Paragraph::new(style::StyledString::new(
                text,
                style::Style::new().bold().with_font_size(size),
            )));
            doc.push(elements::Break::new(0.5));
        }
        Block::Paragraph { text } => {
            doc.push(elements::Paragraph::new(text));
            doc.push(elements::Break::new(0.6));
        }
        Block::UnorderedList { items } => {
            let mut list = elements::UnorderedList::new();
            for item in items {
                list.push(elements::Paragraph::new(item));
            }
            doc.push(list);
            doc.push(elements::Break::new(0.6));
        }
        Block::OrderedList { start, items } => {
            let mut list = elements::OrderedList::with_start(start.max(1) as usize);
            for item in items {
                list.push(elements::Paragraph::new(item));
            }
            doc.push(list);
            doc.push(elements::Break::new(0.6));
        }
        Block::Code { code } => {
            let mono_style = style::Style::new()
                .with_font_family(mono_family)
                .with_font_size(9);
            let mut layout = elements::LinearLayout::vertical();
            for line in code.lines() {
                layout.push(elements::Text::new(style::StyledString::new(
                    line.to_string(),
                    mono_style,
                )));
            }
            doc.push(layout.padded(3).framed());
            doc.push(elements::Break::new(0.6));
        }
        Block::Table { headers, rows } => {
            let column_count = headers.len();
            if column_count == 0 {
                return;
            }
            let mut table = elements::TableLayout::new(vec![1; column_count]);
            table.set_cell_decorator(elements::FrameCellDecorator::new(true, true, false));

            let mut header_row = table.row();
            for header in &headers {
                header_row = header_row.element(
                    elements::Paragraph::new(style::StyledString::new(
                        header.clone(),
                        style::Style::new().bold(),
                    ))
                    .padded(1),
                );
            }
            let _ = header_row.push();

            for row in rows {
                let mut table_row = table.row();
                for index in 0..column_count {
                    let cell_text = row.get(index).cloned().unwrap_or_default();
                    table_row = table_row.element(elements::Paragraph::new(cell_text).padded(1));
                }
                let _ = table_row.push();
            }
            doc.push(table);
            doc.push(elements::Break::new(0.6));
        }
        Block::Rule => {
            doc.push(elements::Paragraph::new(style::StyledString::new(
                "\u{2014}".repeat(40),
                style::Style::new().with_color(style::Color::Greyscale(170)),
            )));
            doc.push(elements::Break::new(0.4));
        }
    }
}

fn count_pdf_pages(path: &Path) -> Result<i64, AppError> {
    let document = lopdf::Document::load(path)
        .map_err(|error| AppError::ArtifactGenerationFailed(error.to_string()))?;
    Ok(document.get_pages().len() as i64)
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
    fn generates_real_multi_section_pdf() {
        let temp = tempfile::tempdir().unwrap();
        let dest = temp.path().join("report.pdf");
        let meta = generate_pdf(SAMPLE_MARKDOWN, "Report Title", &dest).unwrap();
        assert!(dest.exists());
        assert!(std::fs::metadata(&dest).unwrap().len() > 0);
        assert!(meta.page_count >= 1);
    }

    #[test]
    fn generates_real_docx_document() {
        let temp = tempfile::tempdir().unwrap();
        let dest = temp.path().join("report.docx");
        generate_docx(SAMPLE_MARKDOWN, "Report Title", &dest).unwrap();
        assert!(dest.exists());
        let size = std::fs::metadata(&dest).unwrap().len();
        assert!(size > 0);

        // A valid DOCX is a ZIP archive: verify the local-file-header signature.
        let bytes = std::fs::read(&dest).unwrap();
        assert_eq!(&bytes[0..4], b"PK\x03\x04");
    }
}
