//! Tag extraction from markdown notes.
//!
//! Tags come from two sources:
//! 1. YAML frontmatter `tags` field (parsed by the frontmatter module)
//! 2. Inline `#tag` occurrences in the body text
//!
//! Inline tags must be preceded by whitespace or be at the start of a line.
//! Markdown headings (`# Heading`) are **not** treated as tags.

use regex::Regex;
use std::sync::LazyLock;

static INLINE_TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[\s.,;:!?()\[\]{}])(#)([\pL\pN_][\pL\pN_./-]*)").unwrap());

/// Normalize a tag: lowercase, trim leading/trailing special chars.
pub fn normalize_tag(tag: &str) -> String {
    let t = tag.trim().to_lowercase();
    // strip leading/trailing dots, slashes, hyphens
    t.trim_matches(|c: char| c == '.' || c == '/' || c == '-')
        .to_owned()
}

/// Extract inline `#tags` from markdown body text.
///
/// Returns a deduplicated list of normalized tags (lowercase).
/// Markdown headings (lines starting with `# `) are excluded.
pub fn extract_inline_tags(body: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for line in body.lines() {
        // Skip heading lines: lines that start with 1-6 # followed by a space
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix('#') {
            let pound_count = 1 + rest.chars().take_while(|c| *c == '#').count();
            if pound_count >= 1 && pound_count <= 6 {
                let after = &trimmed[pound_count..];
                if after.starts_with(' ') || after.starts_with('\t') {
                    continue; // This is a heading line
                }
            }
        }

        for cap in INLINE_TAG_RE.captures_iter(line) {
            let raw_tag = &cap[2];
            let tag = normalize_tag(raw_tag);
            if !tag.is_empty() && seen.insert(tag.clone()) {
                tags.push(tag);
            }
        }
    }

    tags
}

/// Merge frontmatter tags with inline tags, deduplicating.
pub fn merge_tags(frontmatter_tags: &[String], inline_tags: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();

    for t in frontmatter_tags.iter().chain(inline_tags.iter()) {
        let normalized = normalize_tag(t);
        if !normalized.is_empty() && seen.insert(normalized.clone()) {
            result.push(normalized);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_inline_tag() {
        let tags = extract_inline_tags("This is a #test tag");
        assert_eq!(tags, vec!["test"]);
    }

    #[test]
    fn multiple_inline_tags() {
        let tags = extract_inline_tags("Working with #rust and #wasm today");
        assert!(tags.contains(&"rust".to_owned()));
        assert!(tags.contains(&"wasm".to_owned()));
    }

    #[test]
    fn heading_is_not_tag() {
        let tags = extract_inline_tags("# This is a heading\nNot a heading #tag");
        assert!(!tags.contains(&"this".to_owned()));
        assert!(tags.contains(&"tag".to_owned()));
    }

    #[test]
    fn tag_with_slashes() {
        let tags = extract_inline_tags("Filed under #project/cerebro");
        assert!(tags.contains(&"project/cerebro".to_owned()));
    }

    #[test]
    fn merge_dedup() {
        let fm = vec!["rust".to_owned(), "Code".to_owned()];
        let inline = vec!["RUST".to_owned(), "test".to_owned()];
        let merged = merge_tags(&fm, &inline);
        assert_eq!(merged.len(), 3);
        assert!(merged.contains(&"rust".to_owned()));
        assert!(merged.contains(&"code".to_owned()));
        assert!(merged.contains(&"test".to_owned()));
    }

    #[test]
    fn no_tags_in_pure_text() {
        let tags = extract_inline_tags("Just some regular text without any tags.");
        assert!(tags.is_empty());
    }
}
