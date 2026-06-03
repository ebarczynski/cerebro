//! Document import module for Cerebro.
//!
//! Converts external document formats (PDF, PPTX, ODS, XLSX, XLS, DOCX, DOC,
//! ODT, HTML) into Markdown notes that can be stored in the vault.

use std::path::Path;

use anyhow::{bail, Context, Result};

/// Supported import formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportFormat {
    Pdf,
    Pptx,
    Docx,
    Doc,
    Ods,
    Xlsx,
    Xls,
    Odt,
    Html,
    /// Plain text / code file — imported as-is with syntax hint in code block
    Text,
    /// Markdown file — imported as-is
    Markdown,
    /// JSON file — pretty-printed in code block
    Json,
}

impl ImportFormat {
    /// Detect format from file extension.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "pdf" => Some(Self::Pdf),
            "pptx" => Some(Self::Pptx),
            "docx" => Some(Self::Docx),
            "doc" => Some(Self::Doc),
            "ods" => Some(Self::Ods),
            "xlsx" => Some(Self::Xlsx),
            "xls" => Some(Self::Xls),
            "odt" => Some(Self::Odt),
            "html" | "htm" => Some(Self::Html),
            "md" | "markdown" => Some(Self::Markdown),
            "json" | "jsonl" => Some(Self::Json),
            // All other text/code formats
            "txt" | "text" | "csv" | "tsv" | "yaml" | "yml" | "toml" | "ini" | "cfg"
            | "xml" | "css" | "scss" | "less" | "log" | "rst" | "asciidoc" | "adoc"
            | "js" | "ts" | "jsx" | "tsx" | "py" | "rb" | "rs" | "go" | "java"
            | "c" | "cpp" | "h" | "hpp" | "sh" | "bash" | "zsh" | "fish"
            | "sql" | "graphql" | "dockerfile" | "makefile"
            => Some(Self::Text),
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
            Self::Docx => "DOCX",
            Self::Doc => "DOC",
            Self::Ods => "ODS",
            Self::Xlsx => "XLSX",
            Self::Xls => "XLS",
            Self::Odt => "ODT",
            Self::Html => "HTML",
            Self::Markdown => "Markdown",
            Self::Json => "JSON",
            Self::Text => "Text",
        }
    }

    /// All supported file extensions for display.
    pub fn supported_extensions() -> &'static [&'static str] {
        &[
            "md", "txt", "json", "csv", "yaml", "yml", "toml",
            "pdf", "pptx", "docx", "doc", "ods", "xlsx", "xls", "odt",
            "html", "htm", "xml", "rst",
            "py", "js", "ts", "rs", "go", "java", "c", "cpp", "sh",
            "sql", "log",
        ]
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
            "Unsupported file format: {:?}. Supported: {}",
            path.extension(),
            ImportFormat::supported_extensions().join(", ")
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
        ImportFormat::Docx => convert_docx(&data)?,
        ImportFormat::Doc => convert_doc(path)?,
        ImportFormat::Ods => convert_spreadsheet::<calamine::Ods<_>>(&data)?,
        ImportFormat::Xlsx => convert_spreadsheet::<calamine::Xlsx<_>>(&data)?,
        ImportFormat::Xls => convert_spreadsheet::<calamine::Xls<_>>(&data)?,
        ImportFormat::Odt => convert_odt(&data)?,
        ImportFormat::Html => convert_html(&data)?,
        ImportFormat::Markdown => convert_markdown(&data)?,
        ImportFormat::Json => convert_json(&data)?,
        ImportFormat::Text => convert_text(&data, path)?,
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

// ── DOCX → Markdown (via ZIP + XML) ──────────────────────────────

fn convert_docx(data: &[u8]) -> Result<String> {
    // DOCX is a ZIP containing word/document.xml with the content
    let reader = std::io::Cursor::new(data.to_vec());
    let mut archive = zip::ZipArchive::new(reader).context("opening DOCX as ZIP")?;

    let mut doc_xml = String::new();
    let mut found = false;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        if file.name() == "word/document.xml" {
            use std::io::Read;
            file.read_to_string(&mut doc_xml)
                .context("reading word/document.xml from DOCX")?;
            found = true;
            break;
        }
    }

    if !found {
        bail!("DOCX file does not contain word/document.xml");
    }

    let document = scraper::Html::parse_document(&doc_xml);
    let mut md = String::new();

    // Find all paragraphs (<w:p>) and tables (<w:tbl>) in document order
    // We walk the body's direct children
    let star_sel = scraper::Selector::parse("*").expect("valid selector");
    let body_sel = scraper::Selector::parse("w\\:body, body").ok();

    let root = if let Some(sel) = &body_sel {
        document.select(sel).next()
    } else {
        None
    };

    let root_el = match root {
        Some(el) => el,
        None => {
            // Fallback: just process the whole document
            let html_sel = scraper::Selector::parse("html").ok();
            match html_sel.and_then(|s| document.select(&s).next()) {
                Some(el) => el,
                None => return Ok("*This DOCX document has no parseable body.*".to_string()),
            }
        }
    };

    for el in root_el.select(&star_sel) {
        let tag = el.value().name();

        match tag {
            // Paragraph
            "w:p" | "p" => {
                // Check for heading style
                let mut heading_level = 0usize;

                // Look for w:pStyle in child elements
                let pstyle_sel = scraper::Selector::parse("w\\:pStyle, pStyle").ok();
                if let Some(sel) = &pstyle_sel {
                    if let Some(style_el) = el.select(sel).next() {
                        if let Some(val) = style_el.value().attr("w:val") {
                            heading_level = match val {
                                "Heading1" | "heading 1" | "Titre1" => 1,
                                "Heading2" | "heading 2" | "Titre2" => 2,
                                "Heading3" | "heading 3" | "Titre3" => 3,
                                "Heading4" | "heading 4" => 4,
                                "Heading5" | "heading 5" => 5,
                                "Heading6" | "heading 6" => 6,
                                _ => 0,
                            };
                        }
                    }
                }

                // Extract text from all <w:t> elements inside this paragraph
                let t_sel = scraper::Selector::parse("w\\:t, t").ok();
                let text: String = match &t_sel {
                    Some(sel) => el.select(sel).map(|t_el| t_el.text().collect::<String>()).collect::<Vec<_>>().join(""),
                    None => el.text().collect(),
                };

                let trimmed = text.trim();
                if trimmed.is_empty() {
                    continue;
                }

                if heading_level > 0 {
                    md.push_str(&format!(
                        "\n\n{} {}\n",
                        "#".repeat(heading_level),
                        trimmed
                    ));
                } else {
                    md.push_str(&format!("\n\n{}\n", trimmed));
                }
            }
            // Table
            "w:tbl" | "tbl" => {
                md.push_str("\n\n");
                let tr_sel = scraper::Selector::parse("w\\:tr, tr").ok();
                let tc_sel = scraper::Selector::parse("w\\:tc, tc").ok();
                let t_sel = scraper::Selector::parse("w\\:t, t").ok();

                if let (Some(tr_s), Some(tc_s), Some(t_s)) = (&tr_sel, &tc_sel, &t_sel) {
                    for (ri, tr) in el.select(tr_s).enumerate() {
                        let cells: Vec<String> = tr
                            .select(tc_s)
                            .map(|tc| {
                                tc.select(t_s)
                                    .map(|t_el| t_el.text().collect::<String>())
                                    .collect::<Vec<_>>()
                                    .join("")
                                    .trim()
                                    .to_string()
                            })
                            .collect();

                        if !cells.is_empty() {
                            md.push_str(&format!("| {} |\n", cells.join(" | ")));
                            if ri == 0 {
                                md.push_str(&format!(
                                    "|{}|\n",
                                    "| --- ".repeat(cells.len())
                                ));
                            }
                        }
                    }
                }
                md.push('\n');
            }
            _ => {}
        }
    }

    // Clean up excessive blank lines
    while md.contains("\n\n\n") {
        md = md.replace("\n\n\n", "\n\n");
    }

    let result = md.trim().to_string();

    if result.is_empty() {
        return Ok("*This DOCX document contains no extractable text.*".to_string());
    }

    Ok(result)
}

// ── DOC → Markdown (via LibreOffice) ──────────────────────────────

fn convert_doc(path: &Path) -> Result<String> {
    // .doc is the legacy binary format — requires LibreOffice to convert
    let libreoffice = std::process::Command::new("libreoffice")
        .arg("--version")
        .output();

    match libreoffice {
        Ok(output) if output.status.success() => {
            // Convert .doc → .docx via LibreOffice
            let tmp_dir = tempfile::tempdir().context("creating temp dir for DOC conversion")?;
            let output_dir = tmp_dir.path();

            let status = std::process::Command::new("libreoffice")
                .args([
                    "--headless",
                    "--convert-to",
                    "docx",
                    &path.to_string_lossy(),
                    "--outdir",
                    &output_dir.to_string_lossy(),
                ])
                .output()
                .context("running LibreOffice conversion")?;

            if !status.status.success() {
                let stderr = String::from_utf8_lossy(&status.stderr);
                bail!("LibreOffice conversion failed: {}", stderr);
            }

            // Find the converted file
            let docx_name = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
                + ".docx";
            let docx_path = output_dir.join(&docx_name);

            if !docx_path.exists() {
                bail!(
                    "LibreOffice conversion output not found at {}",
                    docx_path.display()
                );
            }

            let docx_data = std::fs::read(&docx_path).context("reading converted DOCX")?;
            convert_docx(&docx_data)
        }
        _ => {
            bail!(
                "Legacy .doc format requires LibreOffice to be installed. \
                 Install it with: sudo apt install libreoffice-writer"
            )
        }
    }
}

// ── Spreadsheet → Markdown (ODS, XLSX, XLS) ─────────────────────

fn convert_spreadsheet<R: calamine::Reader<std::io::Cursor<Vec<u8>>>>(
    data: &[u8],
) -> Result<String> {
    let cursor = std::io::Cursor::new(data.to_vec());
    let mut workbook: R = calamine::open_workbook_from_rs(cursor)
        .map_err(|e| anyhow::anyhow!("opening spreadsheet: {:?}", e))?;

    let sheet_names = workbook.sheet_names();
    let sheet_name = sheet_names
        .first()
        .ok_or_else(|| anyhow::anyhow!("Spreadsheet has no sheets"))?
        .clone();

    let range = workbook
        .worksheet_range(&sheet_name)
        .map_err(|e| anyhow::anyhow!("reading sheet '{}': {:?}", sheet_name, e))?;

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

// ── ODT → Markdown ───────────────────────────────────────────────

fn convert_odt(data: &[u8]) -> Result<String> {
    // ODT is a ZIP file containing content.xml with the document text
    let reader = std::io::Cursor::new(data.to_vec());
    let mut archive = zip::ZipArchive::new(reader).context("opening ODT as ZIP")?;

    let mut content_xml = String::new();
    let mut found = false;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_string();
        if name == "content.xml" {
            use std::io::Read;
            file.read_to_string(&mut content_xml)
                .context("reading content.xml from ODT")?;
            found = true;
            break;
        }
    }

    if !found {
        bail!("ODT file does not contain content.xml");
    }

    // Parse the XML and extract text with heading levels
    // ODT uses <text:h text:outline-level="N"> for headings
    // and <text:p> for paragraphs, <text:span> for inline text
    let document = scraper::Html::parse_document(&content_xml);

    let mut md = String::new();

    // ODT uses namespaces heavily. The scraper might not handle namespaced selectors,
    // so we fall back to tag-name matching on all elements.
    let star_sel = scraper::Selector::parse("*").expect("valid selector");

    let mut in_table = false;
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut current_row: Vec<String> = Vec::new();

    for el in document.select(&star_sel) {
        let tag = el.value().name();

        match tag {
            // ODT heading
            "text:h" | "h" => {
                let level = el
                    .value()
                    .attr("text:outline-level")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(1)
                    .min(6);
                let text: String = el.text().collect();
                if !text.trim().is_empty() {
                    md.push_str(&format!(
                        "\n\n{} {}\n",
                        "#".repeat(level),
                        text.trim()
                    ));
                }
            }
            // ODT paragraph
            "text:p" | "p" => {
                // Check if we're inside a table cell
                let text: String = el.text().collect();
                if in_table {
                    current_row.push(text.trim().to_string());
                } else if !text.trim().is_empty() {
                    md.push_str(&format!("\n\n{}\n", text.trim()));
                }
            }
            // ODT table
            "table:table" | "table" => {
                in_table = true;
                table_rows.clear();
            }
            "table:table-row" | "tr" => {
                if !current_row.is_empty() {
                    table_rows.push(std::mem::take(&mut current_row));
                }
            }
            "table:table-cell" | "td" | "th" => {
                // Text is collected via text:p above
            }
            _ => {}
        }
    }

    // Flush last table row
    if !current_row.is_empty() {
        table_rows.push(current_row);
    }

    // Render table if we found one
    if !table_rows.is_empty() {
        md.push_str("\n\n");
        for (ri, row) in table_rows.iter().enumerate() {
            md.push_str(&format!("| {} |\n", row.join(" | ")));
            if ri == 0 && !row.is_empty() {
                md.push_str(&format!("|{}|\n", "| --- ".repeat(row.len())));
            }
        }
    }

    // Clean up excessive blank lines
    while md.contains("\n\n\n") {
        md = md.replace("\n\n\n", "\n\n");
    }

    let result = md.trim().to_string();

    if result.is_empty() {
        return Ok(
            "*This ODT document contains no extractable text (namespaced XML may need different parsing).*"
                .to_string(),
        );
    }

    Ok(result)
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

// ── Markdown → Markdown (pass-through) ────────────────────────

fn convert_markdown(data: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(data)
        .context("reading markdown file as UTF-8")?;
    Ok(text.to_string())
}

// ── JSON → Markdown (pretty-printed in code block) ────────────

fn convert_json(data: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(data)
        .context("reading JSON file as UTF-8")?;

    // Try to pretty-print the JSON
    let pretty = if let Ok(val) = serde_json::from_str::<serde_json::Value>(text) {
        serde_json::to_string_pretty(&val).unwrap_or_else(|_| text.to_string())
    } else {
        // JSONL or malformed — just wrap as-is
        text.to_string()
    };

    Ok(format!("```json\n{}\n```", pretty))
}

// ── Text/Code → Markdown (in code block with lang hint) ───────

fn convert_text(data: &[u8], path: &Path) -> Result<String> {
    let text = std::str::from_utf8(data)
        .context("reading text file as UTF-8")?;

    let ext = path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Map extension to syntax highlight language hint
    let lang = match ext.as_str() {
        "py" => "python",
        "js" | "jsx" => "javascript",
        "ts" | "tsx" => "typescript",
        "rs" => "rust",
        "go" => "go",
        "java" => "java",
        "c" | "h" => "c",
        "cpp" | "hpp" => "cpp",
        "sh" | "bash" | "zsh" | "fish" => "bash",
        "sql" => "sql",
        "graphql" => "graphql",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "xml" => "xml",
        "html" | "htm" => "html",
        "css" | "scss" | "less" => "css",
        "csv" | "tsv" => "csv",
        "json" | "jsonl" => "json",
        _ => "",
    };

    Ok(format!("```{}\n{}\n```", lang, text))
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
        assert_eq!(
            ImportFormat::from_extension("docx"),
            Some(ImportFormat::Docx)
        );
        assert_eq!(ImportFormat::from_extension("doc"), Some(ImportFormat::Doc));
        assert_eq!(ImportFormat::from_extension("ods"), Some(ImportFormat::Ods));
        assert_eq!(
            ImportFormat::from_extension("xlsx"),
            Some(ImportFormat::Xlsx)
        );
        assert_eq!(ImportFormat::from_extension("xls"), Some(ImportFormat::Xls));
        assert_eq!(ImportFormat::from_extension("odt"), Some(ImportFormat::Odt));
        assert_eq!(
            ImportFormat::from_extension("html"),
            Some(ImportFormat::Html)
        );
        assert_eq!(
            ImportFormat::from_extension("htm"),
            Some(ImportFormat::Html)
        );
        assert_eq!(ImportFormat::from_extension("rtf"), None);
        assert_eq!(ImportFormat::from_extension("exe"), None);
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
        let path = Path::new("document.xyz");
        assert!(ImportFormat::from_path(path).is_none());
    }

    #[test]
    fn test_import_missing_file() {
        let path = Path::new("/tmp/nonexistent_test_file_12345.pdf");
        let result = import_file(path);
        assert!(result.is_err());
    }

    #[test]
    fn test_supported_extensions() {
        let exts = ImportFormat::supported_extensions();
        assert!(exts.contains(&"pdf"));
        assert!(exts.contains(&"docx"));
        assert!(exts.contains(&"doc"));
        assert!(exts.contains(&"xlsx"));
        assert!(exts.contains(&"xls"));
        assert!(exts.contains(&"odt"));
        assert!(exts.contains(&"pptx"));
        assert!(exts.contains(&"ods"));
        assert!(exts.contains(&"html"));
        // New formats
        assert!(exts.contains(&"md"));
        assert!(exts.contains(&"txt"));
        assert!(exts.contains(&"json"));
        assert!(exts.contains(&"yaml"));
        assert!(exts.contains(&"py"));
        assert!(exts.contains(&"rs"));
        assert!(exts.contains(&"csv"));
    }

    #[test]
    fn test_text_format_detection() {
        assert_eq!(ImportFormat::from_extension("md"), Some(ImportFormat::Markdown));
        assert_eq!(ImportFormat::from_extension("json"), Some(ImportFormat::Json));
        assert_eq!(ImportFormat::from_extension("txt"), Some(ImportFormat::Text));
        assert_eq!(ImportFormat::from_extension("py"), Some(ImportFormat::Text));
        assert_eq!(ImportFormat::from_extension("rs"), Some(ImportFormat::Text));
        assert_eq!(ImportFormat::from_extension("csv"), Some(ImportFormat::Text));
        assert_eq!(ImportFormat::from_extension("yaml"), Some(ImportFormat::Text));
    }

    #[test]
    fn test_convert_markdown() {
        let md = b"# Hello\n\nThis is **markdown**.";
        let result = convert_markdown(md).unwrap();
        assert!(result.contains("# Hello"));
        assert!(result.contains("**markdown**"));
    }

    #[test]
    fn test_convert_json() {
        let json = b"{\"key\": \"value\", \"num\": 42}";
        let result = convert_json(json).unwrap();
        assert!(result.starts_with("```json"));
        assert!(result.contains("\"key\": \"value\""));
        assert!(result.ends_with("```"));
    }

    #[test]
    fn test_convert_text() {
        let text = b"fn main() { println!(\"hello\"); }";
        let path = Path::new("test.rs");
        let result = convert_text(text, path).unwrap();
        assert!(result.starts_with("```rust"));
        assert!(result.contains("fn main()"));
        assert!(result.ends_with("```"));
    }
}
