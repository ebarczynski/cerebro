//! Wikilink extraction from markdown content.
//!
//! Supports Obsidian-style wikilinks:
//! - `[[Target]]`
//! - `[[Target|Alias]]`
//! - `[[Target#heading]]`
//! - `[[Target#heading|Alias]]`
//!
//! And note transclusions:
//! - `![[Target]]`
//! - `![[Target#heading]]`

use regex::Regex;
use std::sync::LazyLock;

/// A single wikilink extracted from a document.
#[derive(Debug, Clone)]
pub struct Wikilink {
    /// The target note name (without heading anchor).
    pub target: String,
    /// Optional heading anchor within the target note.
    pub heading: Option<String>,
    /// Optional display alias.
    pub alias: Option<String>,
    /// 1-indexed line number where the link appears.
    pub line_number: usize,
}

/// A transclusion link — `![[Target]]` which embeds another note's content.
#[derive(Debug, Clone)]
pub struct Transclusion {
    /// The target note name.
    pub target: String,
    /// Optional heading anchor to embed only a section.
    pub heading: Option<String>,
    /// 1-indexed line number.
    pub line_number: usize,
}

// Patterns for [[...]] wikilinks
static WIKILINK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\[([^\]]+)\]\]").unwrap());

// Regex for transclusions: ![[...]]
static TRANSCLUSION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"!\[\[([^\]]+)\]\]").unwrap());

/// Extract all wikilinks from a markdown document.
///
/// Returns wikilinks in order of appearance, with 1-indexed line numbers.
/// Transclusion links (`![[...]]`) are excluded from wikilink results.
pub fn extract_wikilinks(content: &str) -> Vec<Wikilink> {
    let mut links = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let line_number = idx + 1;
        for cap in WIKILINK_RE.captures_iter(line) {
            let m = cap.get(0).unwrap();
            let start = m.start();
            // Skip if preceded by '!' (transclusion)
            if start > 0 && line.as_bytes().get(start - 1) == Some(&b'!') {
                continue;
            }
            let inner = &cap[1];
            if let Some(wl) = parse_wikilink_inner(inner, line_number) {
                links.push(wl);
            }
        }
    }

    links
}

/// Extract all transclusion links from markdown content.
///
/// Returns transclusions in order of appearance, with 1-indexed line numbers.
pub fn extract_transclusions(content: &str) -> Vec<Transclusion> {
    let mut transclusions = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let line_number = idx + 1;
        for cap in TRANSCLUSION_RE.captures_iter(line) {
            let inner = &cap[1];
            let inner = inner.trim();
            if inner.is_empty() {
                continue;
            }

            let (target, heading) = if let Some(hash_pos) = inner.find('#') {
                let t = inner[..hash_pos].trim().to_owned();
                let h = inner[hash_pos + 1..].trim().to_owned();
                (t, if h.is_empty() { None } else { Some(h) })
            } else {
                (inner.to_owned(), None)
            };

            if !target.is_empty() {
                transclusions.push(Transclusion {
                    target,
                    heading,
                    line_number,
                });
            }
        }
    }
    transclusions
}

/// Parse the inner content of a `[[…]]` block.
fn parse_wikilink_inner(inner: &str, line_number: usize) -> Option<Wikilink> {
    let inner = inner.trim();
    if inner.is_empty() {
        return None;
    }

    // Split alias: Target#heading|Alias  or  Target|Alias
    let (target_part, alias) = if let Some(pipe_pos) = inner.find('|') {
        let target = &inner[..pipe_pos];
        let alias_str = inner[pipe_pos + 1..].trim();
        (
            target,
            if alias_str.is_empty() {
                None
            } else {
                Some(alias_str.to_owned())
            },
        )
    } else {
        (inner, None)
    };

    // Split heading: Target#heading
    let (target, heading) = if let Some(hash_pos) = target_part.find('#') {
        let t = target_part[..hash_pos].trim();
        let h = target_part[hash_pos + 1..].trim();
        (
            t.to_owned(),
            if h.is_empty() {
                None
            } else {
                Some(h.to_owned())
            },
        )
    } else {
        (target_part.trim().to_owned(), None)
    };

    if target.is_empty() {
        return None;
    }

    Some(Wikilink {
        target,
        heading,
        alias,
        line_number,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_link() {
        let links = extract_wikilinks("See [[My Note]] for details.");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "My Note");
        assert!(links[0].alias.is_none());
        assert!(links[0].heading.is_none());
        assert_eq!(links[0].line_number, 1);
    }

    #[test]
    fn link_with_alias() {
        let links = extract_wikilinks("See [[My Note|the note]]!");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "My Note");
        assert_eq!(links[0].alias.as_deref(), Some("the note"));
    }

    #[test]
    fn link_with_heading() {
        let links = extract_wikilinks("[[API Reference#Authentication]]");
        assert_eq!(links[0].target, "API Reference");
        assert_eq!(links[0].heading.as_deref(), Some("Authentication"));
    }

    #[test]
    fn link_with_heading_and_alias() {
        let links = extract_wikilinks("[[API Reference#Auth|auth section]]");
        assert_eq!(links[0].target, "API Reference");
        assert_eq!(links[0].heading.as_deref(), Some("Auth"));
        assert_eq!(links[0].alias.as_deref(), Some("auth section"));
    }

    #[test]
    fn multiple_links_on_different_lines() {
        let content = "[[A]]\nSome text\n[[B|beta]] [[C]]";
        let links = extract_wikilinks(content);
        assert_eq!(links.len(), 3);
        assert_eq!(links[0].line_number, 1);
        assert_eq!(links[1].line_number, 3);
        assert_eq!(links[2].line_number, 3);
    }

    #[test]
    fn no_links() {
        let links = extract_wikilinks("Just regular text with no links.");
        assert!(links.is_empty());
    }

    // ── Transclusion tests ──────────────────────────────────────

    #[test]
    fn simple_transclusion() {
        let tcs = extract_transclusions("Some text\n![[My Note]]\nMore text");
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].target, "My Note");
        assert!(tcs[0].heading.is_none());
        assert_eq!(tcs[0].line_number, 2);
    }

    #[test]
    fn transclusion_with_heading() {
        let tcs = extract_transclusions("![[API Reference#Authentication]]");
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].target, "API Reference");
        assert_eq!(tcs[0].heading.as_deref(), Some("Authentication"));
    }

    #[test]
    fn transclusion_excluded_from_wikilinks() {
        let content = "See [[Regular Link]] and ![[Transcluded Note]] here.";
        let links = extract_wikilinks(content);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "Regular Link");

        let tcs = extract_transclusions(content);
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].target, "Transcluded Note");
    }

    #[test]
    fn multiple_transclusions() {
        let content = "![[A]]\nText\n![[B#Section]]\n![[C]]";
        let tcs = extract_transclusions(content);
        assert_eq!(tcs.len(), 3);
        assert_eq!(tcs[0].target, "A");
        assert_eq!(tcs[1].target, "B");
        assert_eq!(tcs[1].heading.as_deref(), Some("Section"));
        assert_eq!(tcs[2].target, "C");
    }

    #[test]
    fn empty_transclusion_ignored() {
        let tcs = extract_transclusions("![[ ]]");
        assert!(tcs.is_empty());
    }
}
