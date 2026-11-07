//! Markdown parser for the Cerebro knowledge base.
//!
//! Extracts frontmatter, wikilinks, tags, and plain text from markdown notes.

pub mod frontmatter;
pub mod tags;
pub mod wikilinks;

use std::collections::HashMap;

use serde_json::Value;

pub use frontmatter::{parse_frontmatter, split_frontmatter, Frontmatter};
pub use tags::{extract_inline_tags, merge_tags, normalize_tag};
pub use wikilinks::{extract_transclusions, extract_wikilinks, Transclusion, Wikilink};

/// Complete parsed result for a single note.
#[derive(Debug, Clone)]
pub struct ParsedNote {
    /// Title extracted from frontmatter, or first `# Heading` if absent.
    pub title: Option<String>,
    /// Parsed frontmatter key-value pairs (all values as serde_json::Value strings).
    pub frontmatter: HashMap<String, Value>,
    /// All tags (merged from frontmatter + inline), normalized.
    pub tags: Vec<String>,
    /// All wikilinks found in the note body.
    pub wikilinks: Vec<Wikilink>,
    /// Body text with YAML frontmatter stripped.
    pub body_text: String,
}

/// Parse a markdown note and extract all metadata.
///
/// This is the main entry point for note parsing.
pub fn extract_links_and_metadata(content: &str) -> ParsedNote {
    // 1. Split frontmatter from body
    let split = split_frontmatter(content);
    let fm = if split.yaml_block.is_empty() {
        Frontmatter::default()
    } else {
        parse_frontmatter(&split.yaml_block)
    };

    // 2. Extract wikilinks and inline tags from the body with fenced code
    // blocks blanked out: `[[links]]` and `#tags` inside code samples are
    // code, not knowledge-graph edges (grill finding F7). Line numbers are
    // preserved by replacing fence contents with empty lines.
    let stripped = strip_fenced_code(&split.body);
    let wikilinks = extract_wikilinks(&stripped);

    // 3. Extract inline tags from body
    let inline_tags = extract_inline_tags(&stripped);

    // 4. Merge frontmatter tags with inline tags
    let tags = merge_tags(&fm.tags, &inline_tags);

    // 5. Determine title: frontmatter > first heading > None
    let title = fm.title.clone().or_else(|| extract_first_heading(&split.body));

    // 6. Build frontmatter HashMap<String, Value>
    let mut frontmatter_map = HashMap::new();
    if let Some(t) = &fm.title {
        frontmatter_map.insert("title".to_owned(), Value::String(t.clone()));
    }
    if let Some(d) = &fm.date {
        frontmatter_map.insert("date".to_owned(), Value::String(d.clone()));
    }
    if !fm.tags.is_empty() {
        frontmatter_map.insert(
            "tags".to_owned(),
            Value::Array(fm.tags.iter().map(|t| Value::String(t.clone())).collect()),
        );
    }
    for (k, v) in &fm.extra {
        frontmatter_map.insert(k.clone(), Value::String(v.clone()));
    }

    ParsedNote {
        title,
        frontmatter: frontmatter_map,
        tags,
        wikilinks,
        body_text: split.body,
    }
}

/// Extract the first ATX heading (`# …`) from markdown text.
fn extract_first_heading(body: &str) -> Option<String> {
    for line in body.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix('#') {
            let pound_count = 1 + rest.chars().take_while(|c| *c == '#').count();
            if pound_count >= 1 && pound_count <= 6 {
                let heading = trimmed[pound_count..].trim();
                if !heading.is_empty() {
                    return Some(heading.to_owned());
                }
            }
        }
    }
    None
}

/// Blank out fenced code blocks while PRESERVING the line count, so line
/// numbers extracted from the stripped text still map onto the original
/// document (grill finding F7).
fn strip_fenced_code(body: &str) -> String {
    let mut out: Vec<&str> = Vec::with_capacity(body.lines().count());
    let mut in_fence = false;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if in_fence {
            if trimmed.starts_with("```") {
                in_fence = false;
            }
            out.push("");
        } else if trimmed.starts_with("```") {
            in_fence = true;
            out.push("");
        } else {
            out.push(line);
        }
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_note_parsing() {
        let doc = r#"---
title: My Note
tags: [rust, code]
date: 2025-01-15
---

# My Note

This is a note about #rust programming.

See [[API Reference]] and [[Design Decisions|design]] for more.
"#;
        let parsed = extract_links_and_metadata(doc);
        assert_eq!(parsed.title.as_deref(), Some("My Note"));
        assert!(parsed.tags.contains(&"rust".to_owned()));
        assert!(parsed.tags.contains(&"code".to_owned()));
        assert_eq!(parsed.wikilinks.len(), 2);
        assert_eq!(parsed.wikilinks[0].target, "API Reference");
        assert_eq!(parsed.wikilinks[1].target, "Design Decisions");
        assert!(parsed.body_text.contains("This is a note"));
        assert!(!parsed.body_text.contains("title: My Note"));
    }

    #[test]
    fn no_frontmatter_note() {
        let doc = "# Just a Heading\n\nSome #tagged content with [[Link]].\n";
        let parsed = extract_links_and_metadata(doc);
        assert_eq!(parsed.title.as_deref(), Some("Just a Heading"));
        assert!(parsed.tags.contains(&"tagged".to_owned()));
        assert_eq!(parsed.wikilinks.len(), 1);
    }

    #[test]
    fn links_inside_fenced_code_are_not_extracted() {
        // Grill finding F7: `[[links]]` in code samples must not become
        // knowledge-graph edges; links OUTSIDE the fence still are.
        let doc = "Real [[Outside Link]] here.\n\n```rust\nlet s = \"[[Code Link]]\";\nlet t = fake();\n```\n\nAnother [[Second Real]] link.";
        let parsed = extract_links_and_metadata(doc);
        let targets: Vec<&str> = parsed.wikilinks.iter().map(|w| w.target.as_str()).collect();
        assert!(targets.contains(&"Outside Link"));
        assert!(targets.contains(&"Second Real"));
        assert!(!targets.contains(&"Code Link"));
        // Line numbers must still map onto the original document
        let second = parsed
            .wikilinks
            .iter()
            .find(|w| w.target == "Second Real")
            .unwrap();
        assert_eq!(second.line_number, 8);
    }

    #[test]
    fn tags_inside_fenced_code_are_not_extracted() {
        let doc = "Real #realtag here\n\n```bash\necho \"#faketag\"\n```";
        let parsed = extract_links_and_metadata(doc);
        assert!(parsed.tags.contains(&"realtag".to_owned()));
        assert!(!parsed.tags.contains(&"faketag".to_owned()));
    }
}
