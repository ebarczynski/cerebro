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

    // 2. Extract wikilinks from body
    let wikilinks = extract_wikilinks(&split.body);

    // 3. Extract inline tags from body
    let inline_tags = extract_inline_tags(&split.body);

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
}
