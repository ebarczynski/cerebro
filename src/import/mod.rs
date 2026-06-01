//! Document import module for Cerebro.
//!
//! Converts external document formats (PDF, PPTX, ODS, HTML) into Markdown
//! notes that can be stored in the vault.

use std::path::Path;

use anyhow::{bail, Context, Result};
use calamine::Reader;

/// Supported import formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportFormat {
    Pdf,
    Pptx,
    Ods,
    Html,
}

impl ImportFormat {
    /// Detect format from file extension.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "pdf" => Some(Self::Pdf),
            "pptx" => Some(Self::Pptx),
            "ods" => Some(Self::Ods),
            "html" | "htm" => Some(Self::Html),
            _ => None,
        }
    }

    /// Detect format from a file path.
    pub fn from_path(path: &Path) -> Option<Self> {
        path.extension()
            .and_then(|e| e.to_str())
            .and_then(Self::from_extension)
    }

    /// Human-readable format name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Pptx => "PPTX",
            Self::Ods => "ODS",
            Self::Html => "HTML",
        }
    }
}

/// Result of an import operation.
#[derive(Debug)]
pub struct ImportResult {
    /// Suggested note title (derived from filename or document metadata).
    pub title: String,
    /// Generated Markdown content.
    pub markdown: String,
    /// The format that was imported.
    pub format: ImportFormat,
    /// Suggested tags (includes the format name).
    pub tags: Vec<String>,
}

/// Import a file as a Markdown note.
///
/// Detects the format from the file extension and converts accordingly.
pub fn import_file(path: &Path) -> Result<ImportResult> {
    let format = ImportFormat::from_path(path).ok_or_else(|| {
        anyhow::anyhow!(
            "Unsupported file format: {:?}. Supported: .pdf, .pptx, .ods, .html/.htm",
            path.extension()
        )
    })?;

    if !path.exists() {
        bail!("File not found: {}", path.display());
    }

    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;

    let title = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Imported Note")
        .to_string();

    let markdown = match format {
        ImportFormat::Pdf => convert_pdf(&data)?,
        ImportFormat::Pptx => convert_pptx(&data)?,
        ImportFormat::Ods => convert_ods(&data)?,
        ImportFormat::Html => convert_html(&data)?,
    };

    let tags = vec![
        format!("import/{}", format.name().to_lowercase()),
        "imported".to_string(),
    ];

    Ok(ImportResult {
        title,
        markdown,
        format,
        tags,
    })
}

// ── PDF → Markdown ───────────────────────────────────────────────

fn convert_pdf(data: &[u8]) -> Result<String> {
    let text = pdf_extract::extract_text_from_mem(data).context("extracting text from PDF")?;

    let text = text.trim();

    if text.is_empty() {
        return Ok(
            "*This PDF contains no extractable text (possibly scanned/image-based).*".to_string(),
        );
    }

    // Convert to markdown paragraphs
    let mut lines = Vec::new();
    let mut paragraph = String::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !paragraph.is_empty() {
                lines.push(paragraph.trim().to_string());
                paragraph.clear();
            }
        } else {
            paragraph.push_str(trimmed);
            paragraph.push(' ');
        }
    }
    if !paragraph.is_empty() {
        lines.push(paragraph.trim().to_string());
    }

    Ok(lines.join("\n\n"))
}

// ── PPTX → Markdown ──────────────────────────────────────────────

fn convert_pptx(data: &[u8]) -> Result<String> {
    // pptx-to-md requires a file path, so write to a temp file
    let tmp_dir = tempfile::tempdir().context("creating temp dir for PPTX")?;
    let tmp_path = tmp_dir.path().join("input.pptx");
    std::fs::write(&tmp_path, data).context("writing temp PPTX file")?;

    let mut container =
        pptx_to_md::PptxContainer::open(&tmp_path, pptx_to_md::ParserConfig::default())
            .context("parsing PPTX file")?;

    let slides = container.parse_all().context("parsing PPTX slides")?;

    let mut md_parts: Vec<String> = Vec::new();
    for (i, slide) in slides.iter().enumerate() {
        if let Some(slide_md) = slide.convert_to_md() {
            if !md_parts.is_empty() {
                md_parts.push(format!("\n---\n\n## Slide {}\n", i + 1));
            }
            md_parts.push(slide_md);
        }
    }

    if md_parts.is_empty() {
        return Ok("*This PPTX file contains no extractable text.*".to_string());
    }

    Ok(md_parts.join("\n\n"))
}

// ── ODS → Markdown ───────────────────────────────────────────────

fn convert_ods(data: &[u8]) -> Result<String> {
    let cursor = std::io::Cursor::new(data.to_vec());
    let mut workbook: calamine::Ods<_> = calamine::open_workbook_from_rs(cursor)
        .map_err(|e| anyhow::anyhow!("opening ODS: {}", e))?;

    let sheet_names = workbook.sheet_names();
    let sheet_name = sheet_names
        .first()
        .ok_or_else(|| anyhow::anyhow!("ODS file has no sheets"))?
        .clone();

    let range = workbook
        .worksheet_range(&sheet_name)
        .map_err(|e| anyhow::anyhow!("reading ODS sheet '{}': {}", sheet_name, e))?;

    let mut md = format!("## Sheet: {}\n\n", sheet_name);

    let mut rows = range.rows().peekable();

    if let Some(header_row) = rows.peek() {
        let col_count = header_row.len();
        if col_count == 0 {
            return Ok("*This spreadsheet is empty.*".to_string());
        }

        // Header row
        let header: Vec<String> = header_row.iter().map(|cell| cell.to_string()).collect();
        md.push_str(&format!("| {} |\n", header.join(" | ")));
        md.push_str(&format!("|{}|\n", "| --- ".repeat(col_count)));

        rows.next(); // skip header

        for row in rows {
            let cells: Vec<String> = row.iter().map(|cell| cell.to_string()).collect();
            md.push_str(&format!("| {} |\n", cells.join(" | ")));
        }
    }

    Ok(md)
}

// ── HTML → Markdown ──────────────────────────────────────────────

fn convert_html(data: &[u8]) -> Result<String> {
    let html = std::str::from_utf8(data).context("HTML file is not valid UTF-8")?;

    let document = scraper::Html::parse_document(html);

    let mut md = String::new();

    // Extract title
    let title_sel = scraper::Selector::parse("title").expect("valid selector");
    if let Some(title_el) = document.select(&title_sel).next() {
        let title = title_el.text().collect::<String>().trim().to_string();
        if !title.is_empty() {
            md.push_str(&format!("# {}\n\n", title));
        }
    }

    // Extract body text — use select() on body to get element refs
    let body_sel = scraper::Selector::parse("body").expect("valid selector");
    if let Some(body_el) = document.select(&body_sel).next() {
        // Walk the tree via ElementRef's inner_html + text, simplified approach:
        // Process direct children of body
        process_element_children(&document, body_el, &mut md, 0);
    }

    // Clean up excessive blank lines
    while md.contains("\n\n\n") {
        md = md.replace("\n\n\n", "\n\n");
    }

    let result = md.trim().to_string();

    if result.is_empty() {
        return Ok("*This HTML document contains no extractable content.*".to_string());
    }

    Ok(result)
}

/// Process child elements of an ElementRef by iterating with select("*").
fn process_element_children(
    doc: &scraper::Html,
    parent: scraper::element_ref::ElementRef,
    md: &mut String,
    depth: usize,
) {
    if depth > 50 {
        return;
    }

    let all_sel = scraper::Selector::parse("*").expect("valid selector");

    for child in parent.select(&all_sel) {
        let tag = child.value().name();
        match tag {
            "h1" => {
                md.push_str("\n\n# ");
                md.push_str(&collect_text(child));
                md.push_str("\n\n");
            }
            "h2" => {
                md.push_str("\n\n## ");
                md.push_str(&collect_text(child));
                md.push_str("\n\n");
            }
            "h3" => {
                md.push_str("\n\n### ");
                md.push_str(&collect_text(child));
                md.push_str("\n\n");
            }
            "h4" => {
                md.push_str("\n\n#### ");
                md.push_str(&collect_text(child));
                md.push_str("\n\n");
            }
            "h5" => {
                md.push_str("\n\n##### ");
                md.push_str(&collect_text(child));
                md.push_str("\n\n");
            }
            "h6" => {
                md.push_str("\n\n###### ");
                md.push_str(&collect_text(child));
                md.push_str("\n\n");
            }
            "p" => {
                let text = collect_text(child);
                if !text.trim().is_empty() {
                    md.push_str("\n\n");
                    md.push_str(&text);
                    md.push_str("\n\n");
                }
            }
            "br" => {
                md.push_str("  \n");
            }
            "hr" => {
                md.push_str("\n\n---\n\n");
            }
            "strong" | "b" => {
                md.push_str("**");
                md.push_str(&collect_text(child));
                md.push_str("**");
            }
            "em" | "i" => {
                md.push_str("*");
                md.push_str(&collect_text(child));
                md.push_str("*");
            }
            "code" => {
                md.push_str("`");
                md.push_str(&collect_text(child));
                md.push_str("`");
            }
            "pre" => {
                md.push_str("\n\n```\n");
                md.push_str(&collect_text(child));
                md.push_str("\n```\n\n");
            }
            "a" => {
                let href = child.value().attr("href").unwrap_or("");
                let text = collect_text(child);
                md.push_str(&format!("[{}]({})", text, href));
            }
            "img" => {
                let src = child.value().attr("src").unwrap_or("");
                let alt = child.value().attr("alt").unwrap_or("");
                md.push_str(&format!("![{}]({})", alt, src));
            }
            "ul" | "ol" => {
                md.push('\n');
                process_list(doc, child, md, depth, tag == "ol");
                md.push('\n');
            }
            "table" => {
                process_table(child, md);
            }
            // Skip script, style, head, nav, footer
            "script" | "style" | "nav" | "footer" | "head" => {}
            _ => {
                // For other elements, recurse into their children
                process_element_children(doc, child, md, depth + 1);
            }
        }
    }
}

fn process_list(
    doc: &scraper::Html,
    list_el: scraper::element_ref::ElementRef,
    md: &mut String,
    depth: usize,
    ordered: bool,
) {
    let li_sel = scraper::Selector::parse("li").expect("valid selector");
    let mut counter = 1;

    for li in list_el.select(&li_sel) {
        let prefix = if ordered {
            let p = format!("{}. ", counter);
            counter += 1;
            p
        } else {
            "- ".to_string()
        };
        md.push_str(&prefix);
        md.push_str(&collect_text(li));
        md.push('\n');
    }
    let _ = (doc, depth);
}

fn process_table(table_el: scraper::element_ref::ElementRef, md: &mut String) {
    md.push_str("\n\n");

    let tr_sel = scraper::Selector::parse("tr").expect("valid selector");
    let td_sel = scraper::Selector::parse("td,th").expect("valid selector");

    for tr in table_el.select(&tr_sel) {
        let cells: Vec<String> = tr
            .select(&td_sel)
            .map(|cell| collect_text(cell).trim().to_string())
            .collect();

        if !cells.is_empty() {
            md.push_str(&format!("| {} |\n", cells.join(" | ")));
        }
    }

    md.push('\n');
}

/// Collect all text content from an element and its descendants.
fn collect_text(el: scraper::element_ref::ElementRef) -> String {
    el.text().collect::<Vec<_>>().join(" ")
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_detection() {
        assert_eq!(ImportFormat::from_extension("pdf"), Some(ImportFormat::Pdf));
        assert_eq!(ImportFormat::from_extension("PDF"), Some(ImportFormat::Pdf));
        assert_eq!(
            ImportFormat::from_extension("pptx"),
            Some(ImportFormat::Pptx)
        );
        assert_eq!(ImportFormat::from_extension("ods"), Some(ImportFormat::Ods));
        assert_eq!(
            ImportFormat::from_extension("html"),
            Some(ImportFormat::Html)
        );
        assert_eq!(
            ImportFormat::from_extension("htm"),
            Some(ImportFormat::Html)
        );
        assert_eq!(ImportFormat::from_extension("docx"), None);
    }

    #[test]
    fn test_html_to_markdown() {
        let html = b"<html><body><h1>Hello</h1><p>This is <strong>bold</strong> and <em>italic</em>.</p><ul><li>Item 1</li><li>Item 2</li></ul></body></html>";
        let result = convert_html(html).unwrap();
        assert!(result.contains("# Hello"));
        assert!(result.contains("**bold**"));
        assert!(result.contains("*italic*"));
        assert!(result.contains("- Item 1"));
        assert!(result.contains("- Item 2"));
    }

    #[test]
    fn test_html_links() {
        let html =
            b"<html><body><p><a href=\"https://example.com\">Click here</a></p></body></html>";
        let result = convert_html(html).unwrap();
        assert!(result.contains("[Click here](https://example.com)"));
    }

    #[test]
    fn test_html_table() {
        let html = b"<html><body><table><tr><th>Name</th><th>Age</th></tr><tr><td>Alice</td><td>30</td></tr></table></body></html>";
        let result = convert_html(html).unwrap();
        assert!(result.contains("| Name | Age |"));
        assert!(result.contains("| Alice | 30 |"));
    }

    #[test]
    fn test_import_unsupported_format() {
        let path = Path::new("document.docx");
        assert!(ImportFormat::from_path(path).is_none());
    }

    #[test]
    fn test_import_missing_file() {
        let path = Path::new("/tmp/nonexistent_test_file_12345.pdf");
        let result = import_file(path);
        assert!(result.is_err());
    }
}
