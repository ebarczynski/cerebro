//! Document import module for Cerebro.
//!
//! Converts external document formats (PDF, PPTX, ODS, XLSX, XLS, DOCX, DOC,
//! ODT, HTML) into Markdown notes that can be stored in the vault.

use std::fmt::Write as _;
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

/// Every file extension the importer accepts, sorted. Anything here that is
/// not a document format is imported as plain text / source code.
const ACCEPTED_EXTENSIONS: &[&str] = &[
    "adoc", "asciidoc", "bash", "c", "cfg", "cpp", "css", "csv", "dockerfile", "doc", "docx",
    "fish", "go", "graphql", "h", "hpp", "htm", "html", "ini", "java", "js", "json", "jsonl",
    "jsx", "less", "log", "makefile", "markdown", "md", "odt", "ods", "pdf", "pptx", "py", "rb",
    "rs", "rst", "scss", "sh", "sql", "text", "toml", "ts", "tsv", "tsx", "txt", "xml", "xls",
    "xlsx", "yaml", "yml", "zsh",
];

impl ImportFormat {
    /// Detect format from file extension.
    pub fn from_extension(ext: &str) -> Option<Self> {
        let ext = ext.to_ascii_lowercase();
        match ext.as_str() {
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
            // Everything else that is accepted — text, configuration, source code
            other => ACCEPTED_EXTENSIONS.contains(&other).then_some(Self::Text),
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

    /// All supported file extensions, for display. This is the same list
    /// that decides what [`Self::from_extension`] accepts.
    pub fn supported_extensions() -> &'static [&'static str] {
        ACCEPTED_EXTENSIONS
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
        // pptx-to-md 2.x returns Result<String> per slide (was Option<String> in 0.x).
        if let Ok(slide_md) = slide.convert_to_md() {
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

// ── DOCX → Markdown (via ZIP + XML) ──

/// Is `element` nested inside an element with one of these tag names?
fn inside_element(element: scraper::ElementRef, names: &[&str]) -> bool {
    element.ancestors().any(|ancestor| {
        matches!(ancestor.value(), scraper::Node::Element(inner) if names.contains(&inner.name()))
    })
}

fn convert_docx(data: &[u8]) -> Result<String> {
    // DOCX is a ZIP containing word/document.xml with the content
    let reader = std::io::Cursor::new(data.to_vec());
    let mut archive = zip::ZipArchive::new(reader).context("opening DOCX as ZIP")?;

    let mut doc_xml = String::new();
    let mut found = false;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        // zip 9.x: name() returns Result<Cow<str>> (was &str in 8.x).
        let name = file.name().map_err(|e| anyhow::anyhow!("invalid entry name at index {}: {}", i, e))?;
        if name == "word/document.xml" {
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
                // A paragraph inside a table cell is rendered with the table.
                if inside_element(el, &["w:tbl", "tbl"]) {
                    continue;
                }
                // Check for heading style
                let mut heading_level = 0usize;

                // Look for w:pStyle in child elements
                let pstyle_sel = scraper::Selector::parse("w\\:pStyle, pStyle").ok();
                if let Some(sel) = &pstyle_sel
                    && let Some(style_el) = el.select(sel).next()
                    && let Some(val) = style_el.value().attr("w:val")
                {
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
                let tr_sel = scraper::Selector::parse("w\\:tr, tr").expect("valid selector");
                let tc_sel = scraper::Selector::parse("w\\:tc, tc").expect("valid selector");
                let t_sel = scraper::Selector::parse("w\\:t, t").expect("valid selector");
                let rows: Vec<Vec<String>> = el
                    .select(&tr_sel)
                    .map(|tr| {
                        tr.select(&tc_sel)
                            .map(|tc| table_cell(&tc.select(&t_sel).map(|t| t.text().collect::<String>()).collect::<String>()))
                            .collect()
                    })
                    .collect();
                md.push_str(&markdown_table(&rows));
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

    // Every sheet becomes its own section; empty sheets are skipped.
    let mut md = String::new();
    for sheet_name in workbook.sheet_names() {
        let range = workbook
            .worksheet_range(&sheet_name)
            .map_err(|e| anyhow::anyhow!("reading sheet '{}': {:?}", sheet_name, e))?;
        let rows: Vec<Vec<String>> = range
            .rows()
            .map(|row| row.iter().map(|cell| table_cell(&cell.to_string())).collect())
            .collect();
        if rows.iter().all(|row| row.iter().all(String::is_empty)) {
            continue;
        }
        let _ = write!(md, "## Sheet: {sheet_name}\n{}\n", markdown_table(&rows));
    }

    if md.is_empty() {
        return Ok("*This spreadsheet is empty.*".to_string());
    }
    Ok(tidy_markdown(&md))
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
        // zip 9.x: name() returns Result<Cow<str>> (was &str in 8.x).
        let name = file
            .name()
            .map_err(|e| anyhow::anyhow!("invalid entry name at index {}: {}", i, e))?
            .to_string();
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

    // Parse the XML and extract text with heading levels.
    // ODT uses <text:h text:outline-level="N"> for headings, <text:p> for
    // paragraphs, <text:span> for inline text, and
    // table:table > table:table-row > table:table-cell > text:p for tables.
    // ODT uses namespaces heavily. The scraper might not handle namespaced
    // selectors, so we fall back to tag-name matching on all elements.
    let document = scraper::Html::parse_document(&content_xml);

    let star_sel = scraper::Selector::parse("*").expect("valid selector");

    let mut md = String::new();
    let mut table: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();

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
                    let _ = write!(md, "\n\n{} {}\n", "#".repeat(level), text.trim());
                }
            }
            // ODT paragraph
            "text:p" | "p" => {
                let text: String = el.text().collect();
                if inside_element(el, &["table:table", "table"]) {
                    // part of the cell we are in
                    if let Some(cell) = row.last_mut() {
                        if !cell.is_empty() {
                            cell.push(' ');
                        }
                        cell.push_str(text.trim());
                    }
                } else if !text.trim().is_empty() {
                    md.push_str(&format!("\n\n{}\n", text.trim()));
                }
            }
            // ODT table structure: the table ends where it ends
            "table:table" | "table" => {
                if !row.is_empty() {
                    table.push(std::mem::take(&mut row));
                }
                md.push_str(&markdown_table(&table));
                table.clear();
            }
            "table:table-row" | "tr" => {
                if !row.is_empty() {
                    table.push(std::mem::take(&mut row));
                }
            }
            "table:table-cell" | "td" | "th" => row.push(String::new()),
            _ => {}
        }
    }

    // A table that is not closed in the XML still gets rendered.
    if !row.is_empty() {
        table.push(row);
    }
    md.push_str(&markdown_table(&table));

    let result = tidy_markdown(&md);
    if result.is_empty() {
        return Ok("*This ODT document contains no extractable text.*".to_string());
    }
    Ok(result)
}

// ── HTML → Markdown ──────────────────────────────────────────────

/// Elements that get a line (or more) of their own.
fn is_block_element(name: &str) -> bool {
    matches!(
        name,
        "address" | "article" | "aside" | "blockquote" | "dd" | "details" | "div" | "dl" | "dt"
            | "fieldset" | "figcaption" | "figure" | "form" | "h1" | "h2" | "h3" | "h4" | "h5"
            | "h6" | "header" | "hr" | "main" | "ol" | "p" | "pre" | "section" | "summary"
            | "table" | "ul"
    )
}

/// Page furniture that is never part of the note.
fn is_boilerplate(name: &str) -> bool {
    matches!(
        name,
        "footer" | "head" | "iframe" | "nav" | "noscript" | "script" | "style" | "svg"
            | "template" | "title"
    )
}

fn convert_html(data: &[u8]) -> Result<String> {
    let html = std::str::from_utf8(data).context("HTML file is not valid UTF-8")?;
    let document = scraper::Html::parse_document(html);

    let mut md = String::new();

    // The page title leads the note.
    let title_sel = scraper::Selector::parse("title").expect("valid selector");
    if let Some(title) = document.select(&title_sel).next() {
        let title = inline_children(title);
        if !title.trim().is_empty() {
            let _ = writeln!(md, "# {}\n", title.trim());
        }
    }

    // Every element is rendered exactly once: block elements recurse into
    // their children, inline elements are rendered in place.
    let body_sel = scraper::Selector::parse("body").expect("valid selector");
    let body = document
        .select(&body_sel)
        .next()
        .unwrap_or_else(|| document.root_element());
    render_block_children(body, &mut md);

    let result = tidy_markdown(&md);
    if result.is_empty() {
        Ok("*This HTML document contains no extractable content.*".to_string())
    } else {
        Ok(result)
    }
}

/// Render the content of `parent` as a sequence of blocks. Loose text and
/// inline elements between two blocks form a paragraph.
fn render_block_children(parent: scraper::ElementRef, md: &mut String) {
    let mut inline = String::new();
    for child in parent.children() {
        match child.value() {
            scraper::Node::Text(text) => inline.push_str(&collapse_whitespace(text)),
            scraper::Node::Element(element) => {
                if is_boilerplate(element.name()) {
                    continue;
                }
                let child_el = scraper::ElementRef::wrap(child).expect("element node");
                if is_block_element(element.name()) {
                    push_paragraph(&inline, md);
                    inline.clear();
                    render_block(child_el, md);
                } else {
                    render_inline_children(child_el, &mut inline);
                }
            }
            _ => {}
        }
    }
    push_paragraph(&inline, md);
}

fn render_block(element: scraper::ElementRef, md: &mut String) {
    let name = element.value().name();
    if is_boilerplate(name) {
        return;
    }
    let heading = match name {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    };
    if let Some(level) = heading {
        let text = inline_children(element).trim().to_string();
        if !text.is_empty() {
            let _ = write!(md, "\n\n{} {text}\n\n", "#".repeat(level));
        }
        return;
    }
    match name {
        "p" | "dt" | "dd" | "figcaption" | "summary" => push_paragraph(&inline_children(element), md),
        "blockquote" => {
            let mut inner = String::new();
            render_block_children(element, &mut inner);
            let quoted = tidy_markdown(&inner)
                .lines()
                .map(|line| if line.is_empty() { ">".to_string() } else { format!("> {line}") })
                .collect::<Vec<_>>()
                .join("\n");
            if !quoted.is_empty() {
                let _ = write!(md, "\n\n{quoted}\n\n");
            }
        }
        "pre" => {
            let code: String = element.text().collect::<Vec<_>>().join("");
            push_code_block(&code, &code_language(element), md);
        }
        "ul" | "ol" => render_list(element, 0, md),
        "table" => {
            let tr_sel = scraper::Selector::parse("tr").expect("valid selector");
            let cell_sel = scraper::Selector::parse("td,th").expect("valid selector");
            let rows: Vec<Vec<String>> = element
                .select(&tr_sel)
                .map(|tr| {
                    tr.select(&cell_sel)
                        .map(|cell| table_cell(&inline_children(cell)))
                        .collect()
                })
                .collect();
            push_table(&rows, md);
        }
        "hr" => md.push_str("\n\n---\n\n"),
        // div, section, article, body, … — and anything we do not know
        _ => render_block_children(element, md),
    }
}

fn render_inline_children(element: scraper::ElementRef, out: &mut String) {
    for child in element.children() {
        match child.value() {
            scraper::Node::Text(text) => out.push_str(&collapse_whitespace(text)),
            scraper::Node::Element(inner) => {
                let name = inner.name();
                let child_el = scraper::ElementRef::wrap(child).expect("element node");
                match name {
                    "br" => out.push_str("\\\n"),
                    "img" => {
                        let src = inner.attr("src").unwrap_or("");
                        let alt = collapse_whitespace(inner.attr("alt").unwrap_or(""));
                        let _ = write!(out, "![{alt}]({src})");
                    }
                    "a" => {
                        // The link's own text (never its children's marks) is
                        // the link label; the spaces around it stay outside.
                        let text: String = child_el.text().collect::<Vec<_>>().join("");
                        let text = collapse_whitespace(&text);
                        let (href, text) = match inner.attr("href") {
                            Some(href) if !href.is_empty() => (href.to_string(), text),
                            _ => (String::new(), text),
                        };
                        pad_like(&text, out, |label| {
                            if href.is_empty() {
                                label.to_string()
                            } else {
                                let target = if href.contains([' ', '(', ')']) {
                                    format!("<{href}>")
                                } else {
                                    href.clone()
                                };
                                format!("[{label}]({target})")
                            }
                        });
                    }
                    "strong" | "b" => wrap_inline(child_el, "**", out),
                    "em" | "i" => wrap_inline(child_el, "*", out),
                    "del" | "s" | "strike" => wrap_inline(child_el, "~~", out),
                    "code" | "kbd" | "samp" => {
                        let code: String = child_el.text().collect::<Vec<_>>().join("");
                        let code = code.trim();
                        if !code.is_empty() {
                            let fence = if code.contains('`') { "``" } else { "`" };
                            let _ = write!(out, "{fence}{code}{fence}");
                        }
                    }
                    _ if is_boilerplate(name) => {}
                    _ => render_inline_children(child_el, out),
                }
            }
            _ => {}
        }
    }
}

/// Write `wrap(text.trim())`, keeping one space on each side where `text`
/// had whitespace, so "Read <a>the docs</a> now" stays one sentence.
fn pad_like(text: &str, out: &mut String, wrap: impl FnOnce(&str) -> String) {
    let had_leading = text.starts_with(char::is_whitespace);
    let had_trailing = text.ends_with(char::is_whitespace);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        if had_leading || had_trailing {
            out.push(' ');
        }
        return;
    }
    if had_leading {
        out.push(' ');
    }
    out.push_str(&wrap(trimmed));
    if had_trailing {
        out.push(' ');
    }
}

fn wrap_inline(element: scraper::ElementRef, marks: &str, out: &mut String) {
    let inner = inline_children(element);
    pad_like(&inner, out, |trimmed| format!("{marks}{trimmed}{marks}"));
}

fn inline_children(element: scraper::ElementRef) -> String {
    let mut out = String::new();
    render_inline_children(element, &mut out);
    out
}

fn render_list(list: scraper::ElementRef, depth: usize, md: &mut String) {
    let indent = "    ".repeat(depth);
    let ordered = list.value().name() == "ol";
    let start: usize = list
        .value()
        .attr("start")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let mut index = 0usize;
    for item in list.children().filter_map(scraper::ElementRef::wrap) {
        if item.value().name() != "li" {
            continue;
        }
        let marker = if ordered {
            format!("{}. ", start + index)
        } else {
            "- ".to_string()
        };
        index += 1;

        // The item's own text, and the blocks nested in it (sub-lists, …).
        let mut text = String::new();
        let mut blocks = String::new();
        for child in item.children() {
            match child.value() {
                scraper::Node::Text(t) => text.push_str(&collapse_whitespace(t)),
                scraper::Node::Element(_) => {
                    let el = scraper::ElementRef::wrap(child).expect("element node");
                    let name = el.value().name();
                    if name == "ul" || name == "ol" {
                        render_list(el, 0, &mut blocks);
                    } else if is_block_element(name) {
                        if matches!(name, "p" | "dd" | "dt" | "figcaption" | "summary") {
                            text.push(' ');
                            text.push_str(&inline_children(el));
                        } else {
                            render_block(el, &mut blocks);
                        }
                    } else {
                        render_inline_children(el, &mut text);
                    }
                }
                _ => {}
            }
        }

        if text.trim().is_empty() && blocks.trim().is_empty() {
            continue;
        }
        let continuation = format!("{indent}{}", " ".repeat(marker.len()));
        let first_line = text.lines().next().unwrap_or("").trim();
        let _ = writeln!(md, "{indent}{marker}{first_line}");
        for line in text.lines().skip(1) {
            let line = line.trim();
            if !line.is_empty() {
                let _ = writeln!(md, "{continuation}{line}");
            }
        }
        for line in blocks.lines() {
            if line.trim().is_empty() {
                md.push('\n');
            } else {
                let _ = writeln!(md, "{continuation}{}", line.trim_end());
            }
        }
    }
}

/// The `language-…` class of a `<pre>` or its `<code>`, as a fence hint.
fn code_language(pre: scraper::ElementRef) -> String {
    let mut classes = Vec::new();
    if let Some(class) = pre.value().attr("class") {
        classes.push(class.to_string());
    }
    let code_sel = scraper::Selector::parse("code").expect("valid selector");
    if let Some(code) = pre.select(&code_sel).next()
        && let Some(class) = code.value().attr("class")
    {
        classes.push(class.to_string());
    }
    classes
        .iter()
        .flat_map(|class| class.split_whitespace())
        .find_map(|token| token.strip_prefix("language-"))
        .unwrap_or("")
        .to_string()
}

/// Re-flow text outside fenced code blocks: no trailing spaces, and never
/// more than one blank line between two lines.
fn tidy_markdown(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    for (index, segment) in md.split("```").enumerate() {
        if index > 0 {
            out.push_str("```");
        }
        if index % 2 == 1 {
            // inside a fenced code block: keep the layout
            out.push_str(segment);
        } else {
            tidy_plain(segment, &mut out);
        }
    }
    out.trim().to_string()
}

fn tidy_plain(text: &str, out: &mut String) {
    let mut blanks = 0usize;
    for raw in text.split('\n') {
        let line = raw.trim_end();
        if line.is_empty() {
            blanks += 1;
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
            if blanks > 0 {
                out.push('\n');
            }
        }
        out.push_str(line);
        blanks = 0;
    }
}

fn push_paragraph(inline: &str, md: &mut String) {
    let text = inline.trim();
    if !text.is_empty() {
        let _ = write!(md, "\n\n{text}\n\n");
    }
}

fn push_code_block(code: &str, language: &str, md: &mut String) {
    let code = code.trim_end();
    if code.is_empty() {
        return;
    }
    let fence = if code.contains("```") { "~~~~" } else { "```" };
    let _ = write!(md, "\n\n{fence}{language}\n{code}\n{fence}\n\n");
}

/// One table cell on one line, with the pipe character escaped.
fn table_cell(text: &str) -> String {
    text.replace(['\n', '\r'], " ").replace('|', "\\|").trim().to_string()
}

fn push_table(rows: &[Vec<String>], md: &mut String) {
    md.push_str(&markdown_table(rows));
}

/// A GitHub-flavored Markdown table with a well-formed delimiter row.
fn markdown_table(rows: &[Vec<String>]) -> String {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    if columns == 0 {
        return String::new();
    }
    let mut out = String::from("\n\n");
    for (index, row) in rows.iter().enumerate() {
        let cells: Vec<&str> = (0..columns)
            .map(|column| row.get(column).map(String::as_str).unwrap_or(""))
            .collect();
        let _ = writeln!(out, "| {} |", cells.join(" | "));
        if index == 0 {
            let _ = writeln!(out, "| {} |", vec!["---"; columns].join(" | "));
        }
    }
    out.push('\n');
    out
}

/// Runs of whitespace are a single space, as in HTML.
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !in_space {
                out.push(' ');
                in_space = true;
            }
        } else {
            out.push(ch);
            in_space = false;
        }
    }
    out
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
