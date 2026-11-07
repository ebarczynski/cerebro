-- Migration 001: Initial schema
-- Notes, links, tags, FTS
-- (daily_notes/sync_log/kp_cache removed — never written by any code path;
-- see docs/adr/0003 and the 2026-11-06 grill audit)

CREATE TABLE IF NOT EXISTS notes (
    id          TEXT PRIMARY KEY,
    title       TEXT NOT NULL,
    path        TEXT NOT NULL UNIQUE,
    content_hash TEXT NOT NULL,
    frontmatter TEXT,
    tags        TEXT NOT NULL DEFAULT '[]',
    word_count  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
    deleted_at  TEXT
);

CREATE TABLE IF NOT EXISTS links (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    source_id   TEXT NOT NULL REFERENCES notes(id),
    target_id   TEXT REFERENCES notes(id),
    target_ref  TEXT NOT NULL,
    context     TEXT,
    line_number INTEGER,
    UNIQUE(source_id, target_ref, line_number)
);

CREATE TABLE IF NOT EXISTS tags (
    name        TEXT PRIMARY KEY,
    note_count  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS note_tags (
    note_id     TEXT NOT NULL REFERENCES notes(id),
    tag         TEXT NOT NULL REFERENCES tags(name),
    PRIMARY KEY (note_id, tag)
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_notes_path ON notes(path);
CREATE INDEX IF NOT EXISTS idx_notes_updated ON notes(updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_links_source ON links(source_id);
CREATE INDEX IF NOT EXISTS idx_links_target ON links(target_id);
CREATE INDEX IF NOT EXISTS idx_notes_tags_tag ON note_tags(tag);

-- Full-text search virtual table
CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
    title,
    content,
    tags,
    tokenize='porter unicode61'
);
