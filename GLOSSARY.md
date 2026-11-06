# Cerebro Glossary

Terms used across Cerebro's code, CLI, MCP tools, and GUI. Definitions only —
no implementation details.

## Vault

A self-contained knowledge domain: a directory of Markdown notes plus a derived
search/link index. Everything in Cerebro lives inside exactly one vault.

## Profile

A named vault with its own notes directory and own database. Profiles are how
personal and professional knowledge are kept separated. No note, tag, link, or
search result ever crosses a profile boundary.

## Note

A single Markdown file inside a vault's notes directory. The file — not the
database row — is the note. Titles come from frontmatter or the filename.

## Frontmatter

YAML metadata at the top of a note (`title`, `tags`, …). Derived metadata
(tags, links) is always re-derivable from the note file.

## Wikilink

An in-note reference of the form `[[Target]]`, optionally with a heading
(`[[Target#Section]]`) or display alias (`[[Target|alias]]`). Links may resolve
to an existing note or stay unresolved.

## Transclusion

An embedding directive `![[Target]]` that pulls another note's content into the
rendering note. Visually similar to a wikilink; semantically different — it
includes content rather than referencing it.

## Backlink

A link from another note that points TO the current note. The reverse edge of a
wikilink, always derivable from the full link graph.

## Keyword link

An automatic, computed connection between two notes that share tags or title
words and are not already wikilinked. Never authored by a human; always marked
as inferred in the UI.

## Broken link

A wikilink whose target note does not exist in the same vault.

## Capture

A quick, one-command append of a thought to the vault's inbox without opening
an editor or choosing a location.

## Daily note

A per-date note (one per day) auto-created on demand from a template, used as
the journaling entry point.

## Import

Converting an external artifact (an Obsidian vault, a PDF, DOCX, PPTX,
spreadsheet, HTML page) into a Markdown note inside a vault.

## Sync

Replicating a vault's notes to a git remote. Notes are the unit of sync; the
derived index never syncs.

## Publish

Sending a note's rendered content to an external system (e.g. Confluence) as a
page. Publishing is one-directional; the vault copy remains the source of truth.

## MCP tool

A capability the Cerebro MCP server exposes to AI clients over JSON-RPC 2.0.
Tool names are part of the public contract with MCP clients.
