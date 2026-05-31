-- Migration 001: Initial schema
-- Notes, links, tags, FTS, daily notes, sync log, KP cache

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

CREATE TABLE IF NOT EXISTS daily_notes (
    date        TEXT PRIMARY KEY,
    note_id     TEXT REFERENCES notes(id)
);

CREATE TABLE IF NOT EXISTS sync_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    operation   TEXT NOT NULL,
    note_id     TEXT NOT NULL,
    timestamp   TEXT NOT NULL DEFAULT (datetime('now')),
    synced      INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS kp_cache (
    note_id     TEXT PRIMARY KEY REFERENCES notes(id),
    kp_doc_id   TEXT,
    last_synced TEXT NOT NULL,
    sync_status TEXT NOT NULL DEFAULT 'pending'
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
