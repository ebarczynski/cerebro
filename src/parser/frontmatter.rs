//! YAML frontmatter extraction and simple parsing.
//!
//! Handles `---`-delimited YAML blocks at the top of markdown files.
//! Does not pull in a full YAML crate — instead performs lightweight
//! key-value extraction for common fields (`title`, `tags`, `date`)
//! and stores everything else as raw strings.

use std::collections::HashMap;

/// Parsed frontmatter with common convenience fields.
#[derive(Debug, Clone, Default)]
pub struct Frontmatter {
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub date: Option<String>,
    /// Any other key-value pairs found in the frontmatter.
    pub extra: HashMap<String, String>,
}

/// Result of splitting raw content into frontmatter block + body.
#[derive(Debug, Clone)]
pub struct SplitResult {
    /// The raw YAML text between the `---` delimiters (empty string if none).
    pub yaml_block: String,
    /// Everything after the closing `---`.
    pub body: String,
}

/// Split a full document into optional YAML frontmatter and the remaining body.
///
/// Accepts both `---\n` and `---\r\n` line endings. The opening `---` must be
/// the very first line of the document.
pub fn split_frontmatter(content: &str) -> SplitResult {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return SplitResult {
            yaml_block: String::new(),
            body: content.to_owned(),
        };
    }

    // Find the line boundary after the opening ---
    let after_first = match trimmed.find(['\n', '\r']) {
        Some(pos) => &trimmed[pos..],
        None => {
            return SplitResult {
                yaml_block: String::new(),
                body: content.to_owned(),
            }
        }
    };

    // Skip past the newline(s) after opening ---
    let yaml_start = after_first.trim_start_matches(['\n', '\r']).as_ptr() as usize;
    // Absolute offset from start of `content`
    let abs_yaml_start = yaml_start - content.as_ptr() as usize;

    // Now search for the closing ---
    let rest = &content[abs_yaml_start..];
    for line in rest.lines() {
        let trimmed_line = line.trim();
        if trimmed_line == "---" {
            let closing_offset = line.as_ptr() as usize - content.as_ptr() as usize;
            let yaml_block = content[abs_yaml_start..closing_offset].to_owned();
            // body starts after the closing ---
            let body_start = closing_offset + line.len();
            let body = content[body_start..].trim_start_matches(['\n', '\r']).to_owned();
            return SplitResult { yaml_block, body };
        }
        // If the first non-empty line is not `---`, keep going —
        // but if we hit an empty yaml_block and a non-dash line, abort.
        if line.trim().is_empty() {
            continue;
        }
    }

    // No closing --- found => treat entire content as body
    SplitResult {
        yaml_block: String::new(),
        body: content.to_owned(),
    }
}

/// Parse a raw YAML block into a [`Frontmatter`].
///
/// This is a *simple* parser that handles the most common patterns:
/// - `key: value`
/// - `key: "quoted value"`
/// - `tags: [t1, t2, t3]`
/// - `tags:\n  - t1\n  - t2`
pub fn parse_frontmatter(yaml: &str) -> Frontmatter {
    let mut fm = Frontmatter::default();
    let mut in_tags_list = false;

    for raw_line in yaml.lines() {
        let line = raw_line.trim();

        if line.is_empty() {
            in_tags_list = false;
            continue;
        }

        // List item under a previous `tags:` key
        if in_tags_list {
            if let Some(tag) = line.strip_prefix("- ") {
                fm.tags.push(tag.trim().trim_matches('"').trim_matches('\'').to_owned());
                continue;
            }
            // Not a list item -> tags list ended
            in_tags_list = false;
        }

        // Key: value
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            let value = value.trim();

            match key {
                "title" => {
                    fm.title = Some(unquote(value));
                }
                "date" => {
                    fm.date = Some(unquote(value));
                }
                "tags" => {
                    if value.is_empty() {
                        // tags: followed by a newline — list starts next lines
                        in_tags_list = true;
                    } else if value.starts_with('[') && value.ends_with(']') {
                        // Inline array: [t1, t2]
                        let inner = &value[1..value.len() - 1];
                        fm.tags = inner
                            .split(',')
                            .map(|t| unquote(t.trim()))
                            .filter(|t| !t.is_empty())
                            .collect();
                    } else {
                        // Single tag on one line
                        fm.tags.push(unquote(value));
                    }
                }
                _ => {
                    if !value.is_empty() {
                        fm.extra.insert(key.to_owned(), unquote(value));
                    }
                }
            }
        }
    }

    fm
}

/// Strip matching quotes from a value.
fn unquote(s: &str) -> String {
    if (s.starts_with('"') && s.ends_with('"'))
        || (s.starts_with('\'') && s.ends_with('\''))
    {
        s[1..s.len() - 1].to_owned()
    } else {
        s.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_frontmatter() {
        let result = split_frontmatter("# Hello\nbody");
        assert!(result.yaml_block.is_empty());
        assert_eq!(result.body, "# Hello\nbody");
    }

    #[test]
    fn simple_frontmatter() {
        let doc = "---\ntitle: My Note\ndate: 2025-01-01\n---\n# Body\n";
        let split = split_frontmatter(doc);
        assert!(split.yaml_block.contains("title: My Note"));
        assert_eq!(split.body, "# Body\n");

        let fm = parse_frontmatter(&split.yaml_block);
        assert_eq!(fm.title.as_deref(), Some("My Note"));
        assert_eq!(fm.date.as_deref(), Some("2025-01-01"));
    }

    #[test]
    fn tags_inline_array() {
        let yaml = "title: T\ntags: [rust, code, \"quoted\"]";
        let fm = parse_frontmatter(yaml);
        assert_eq!(fm.tags, vec!["rust", "code", "quoted"]);
    }

    #[test]
    fn tags_list() {
        let yaml = "title: T\ntags:\n  - rust\n  - code";
        let fm = parse_frontmatter(yaml);
        assert_eq!(fm.tags, vec!["rust", "code"]);
    }

    #[test]
    fn extra_fields() {
        let yaml = "title: T\nauthor: Ed\nrating: \"5\"";
        let fm = parse_frontmatter(yaml);
        assert_eq!(fm.extra.get("author").unwrap(), "Ed");
        assert_eq!(fm.extra.get("rating").unwrap(), "5");
    }
}
