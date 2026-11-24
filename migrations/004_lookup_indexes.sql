-- Migration 004: lookup indexes and consistency clean-up (2026-11-23 audit).
--
-- Why: title lookups and link resolution did full table scans (no index on
-- notes.title), wikilink resolution had no file-name lookup at all, and
-- earlier versions left index rows behind when a note file disappeared.

-- `find_note` / `resolve_target` look notes up by title on every call.
CREATE INDEX IF NOT EXISTS idx_notes_title ON notes(title);

-- Resolving links that point at a not-yet-existing note looks them up by
-- target_ref. Case-insensitive, because `[[Plan]]` and `[[plan]]` are the
-- same link to a user.
CREATE INDEX IF NOT EXISTS idx_links_target_ref ON links(target_ref COLLATE NOCASE);

-- File name without directory and without ".md", derived from `path`, so that
-- `[[plan]]` resolves to `projects/plan.md` the way Obsidian does.
--   rtrim(path, replace(path, '/', ''))  = path up to and including the last '/'
-- so everything after it, minus the ".md" suffix, is the stem.
ALTER TABLE notes ADD COLUMN file_stem TEXT GENERATED ALWAYS AS (
    substr(path,
           length(rtrim(path, replace(path, '/', ''))) + 1,
           length(path) - length(rtrim(path, replace(path, '/', ''))) - 3)
) VIRTUAL;
CREATE INDEX IF NOT EXISTS idx_notes_file_stem ON notes(file_stem COLLATE NOCASE);

-- Index rows that outlived their note (reindex used to only soft-delete).
DELETE FROM notes_fts WHERE rowid NOT IN (SELECT rowid FROM notes WHERE deleted_at IS NULL);
DELETE FROM note_tags WHERE note_id IN (SELECT id FROM notes WHERE deleted_at IS NOT NULL);
