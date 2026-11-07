# 0001 — Markdown files are the source of truth; SQLite is a derived index

Date: 2026-11-06
Status: Accepted (de facto since first release; recorded during grill review)

## Context

A knowledge base needs one authoritative representation of its content.
Options considered: database-only (content lives in SQLite), file-only
(no index), or a hybrid.

## Decision

Plain `.md` files inside the vault's notes directory are the source of truth.
The SQLite database (metadata, FTS5 index, link graph, tag counts) is a
derived, rebuildable index. Any notebook can be reconstructed from the files
alone via reindex. Sync and Obsidian import/export operate on files, never on
the database.

## Consequences

- Vault is portable and human-readable; git-friendly; Obsidian-compatible.
- Index can be discarded and rebuilt at any time (`cerebro reindex`).
- Every write path must keep file and index in agreement, or tolerate
  short-lived drift that reindex repairs. Crash-safety of multi-step writes
  is a permanent concern of this architecture.
- External tools (editors, Obsidian) can modify notes behind Cerebro's back;
  the file watcher/reindex path must reconcile, not fight them.
