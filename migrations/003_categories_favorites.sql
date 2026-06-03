-- Migration 003: Add category and favorites support
ALTER TABLE notes ADD COLUMN category TEXT;
ALTER TABLE notes ADD COLUMN is_favorite INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_notes_category ON notes(category);
CREATE INDEX IF NOT EXISTS idx_notes_favorite ON notes(is_favorite) WHERE is_favorite = 1;
