# 0002 — Full-text search is SQLite FTS5 only; Tantivy is dropped

Date: 2026-11-06
Status: Accepted (de facto since first release; recorded during grill review;
supersedes the plan doc's "Tantivy + FTS5 dual search" stack)

## Context

The original plan (`docs/plans/2026-05-31-cerebro-knowledge-base.md`) called
for two search engines: SQLite FTS5 for exact keyword search and Tantivy for
relevance search (BM25 + recency boost, faceting), combined into a hybrid
mode. Tantivy would live in a per-profile `tantivy/` directory.

## Decision

Search is implemented exclusively with SQLite FTS5 (standalone content table,
porter+unicode61 tokenizer, BM25 ranking via `fts.rank`, `snippet()` for
excerpts). Tantivy is not used, its index directory does not exist, and the
`tantivy` dependency is not present.

## Consequences

- One index to maintain, one write path per note, simpler crash behavior.
- No faceted/field-boosted search; ranking is FTS5 BM25 only, no recency
  boost. Accepted until a demonstrated need appears.
- The plan document's performance targets that assumed Tantivy (faceted
  search, hybrid modes) are obsolete as written.
- `.gitignore` entries and docs referencing `tantivy/` are stale.
- If hybrid/semantic ranking is ever needed, prefer the Knowledge Palace
  connector (pgvector) over reintroducing Tantivy — revisit only if local,
  offline semantic search becomes a requirement (tripwire: user cannot reach
  KP and needs semantic search locally).
