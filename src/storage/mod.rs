use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::profile::ProfileConfig;
use crate::parser::{extract_links_and_metadata, ParsedNote};

// ── Types ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub path: String,
    pub content_hash: String,
    pub frontmatter: Option<String>,
    pub tags: Vec<String>,
    pub word_count: i64,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub is_favorite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteContent {
    pub note: Note,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub path: String,
    pub rank: f64,
    pub snippet: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub source_id: String,
    pub target_id: Option<String>,
    pub target_ref: String,
    pub context: Option<String>,
    pub line_number: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReindexStats {
    pub created: usize,
    pub updated: usize,
    pub removed: usize,
    pub errors: usize,
}

// ── Storage Engine ───────────────────────────────────────────────

pub struct Storage {
    db: Connection,
    notes_dir: PathBuf,
    profile_name: String,
}

impl Storage {
    pub fn open(profile_dir: &Path, profile_config: &ProfileConfig) -> Result<Self> {
        let notes_dir = profile_config.resolved_notes_dir(profile_dir);
        std::fs::create_dir_all(&notes_dir)?;

        let db_path = profile_dir.join("db.sqlite");
        let db = Connection::open(&db_path)
            .with_context(|| format!("opening database at {:?}", db_path))?;

        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

        let profile_name = profile_config.name.clone();
        let mut storage = Storage { db, notes_dir, profile_name };
        storage.run_migrations()?;
        Ok(storage)
    }

    pub fn profile_name(&self) -> &str {
        &self.profile_name
    }

    /// Returns the path to the notes directory.
    pub fn notes_dir_path(&self) -> &Path {
        &self.notes_dir
    }

    /// Returns (and creates if needed) the attachments directory under notes_dir.
    pub fn attachments_dir(&self) -> PathBuf {
        let dir = self.notes_dir.join("attachments");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    // ── Migrations ────────────────────────────────────────────────

    fn run_migrations(&self) -> Result<()> {
        self.db.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at TEXT NOT NULL DEFAULT (datetime('now'))
            )"
        )?;

        let applied: Vec<i64> = {
            let mut stmt = self.db.prepare("SELECT id FROM schema_migrations")?;
            stmt.query_map([], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect()
        };

        let migrations: Vec<(i64, &str, &str)> = vec![
            (
                1,
                "initial",
                include_str!("../../migrations/001_initial.sql"),
            ),
            (
                2,
                "fts_triggers",
                include_str!("../../migrations/002_fts_triggers.sql"),
            ),
            (
                3,
                "categories_favorites",
                include_str!("../../migrations/003_categories_favorites.sql"),
            ),
        ];

        for (id, name, sql) in &migrations {
            if !applied.contains(id) {
                self.db.execute_batch(sql)?;
                self.db.execute(
                    "INSERT INTO schema_migrations (id, name) VALUES (?1, ?2)",
                    params![id, name],
                )?;
            }
        }

        Ok(())
    }

    // ── Note CRUD ─────────────────────────────────────────────────

    /// Create a new note. Returns the note metadata.
    pub fn create_note(
        &mut self,
        title: &str,
        content: &str,
        path: Option<&str>,
        tags: Option<Vec<String>>,
    ) -> Result<Note> {
        let id = Uuid::now_v7().to_string();
        let relative_path = match path {
            Some(p) => self.validate_note_path(p, title)?,
            None => self.sanitize_path(title),
        };
        let file_path = self.notes_dir.join(&relative_path);

        // Ensure parent directory exists
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let parsed = extract_links_and_metadata(content);

        // Build frontmatter + content
        let combined_tags = {
            let mut t = tags.unwrap_or_default();
            t.extend(parsed.tags);
            t.sort();
            t.dedup();
            t
        };

        // Write the .md file
        let now = now_iso();

        // Write the .md file
        let full_content = if parsed.frontmatter.is_empty() {
            let fm = build_frontmatter(title, &combined_tags);
            format!("---\n{}---\n{}", fm, content)
        } else {
            content.to_string()
        };

        std::fs::write(&file_path, &full_content)
            .with_context(|| format!("writing note file {:?}", file_path))?;

        // Insert into database
        let frontmatter_json = if parsed.frontmatter.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&parsed.frontmatter)?)
        };
        let tags_json = serde_json::to_string(&combined_tags)?;
        let word_count = content.split_whitespace().count() as i64;

        self.db.execute(
            "INSERT INTO notes (id, title, path, content_hash, frontmatter, tags, word_count, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![id, title, relative_path, sha256_hex(&full_content), frontmatter_json, tags_json, word_count, now, now],
        )?;

        // Update FTS
        self.db.execute(
            "INSERT INTO notes_fts (rowid, title, content, tags) VALUES (?1, ?2, ?3, ?4)",
            params![
                self.note_rowid(&id)?,
                title,
                strip_yaml_frontmatter(content),
                combined_tags.join(" ")
            ],
        )?;

        // Update link graph
        self.update_links(&id, &parsed.wikilinks, content)?;
        self.update_tag_counts(&id, &combined_tags)?;

        Ok(Note {
            id,
            title: title.to_string(),
            path: relative_path,
            content_hash: sha256_hex(&full_content),
            frontmatter: frontmatter_json,
            tags: combined_tags,
            word_count,
            created_at: now.clone(),
            updated_at: now,
            category: None,
            is_favorite: false,
        })
    }

    /// Returns true when a note with this path already exists in the vault.
    pub fn note_exists(&self, path: &str) -> bool {
        self.db
            .query_row(
                "SELECT 1 FROM notes WHERE path = ?1 AND deleted_at IS NULL",
                params![path],
                |_| Ok(()),
            )
            .is_ok()
    }

    pub fn update_note(&mut self, identifier: &str, new_content: &str) -> Result<Note> {
        let note = self.find_note(identifier)?;
        let parsed = extract_links_and_metadata(new_content);
        let combined_tags = parsed.tags.clone();
        let tags_json = serde_json::to_string(&combined_tags)?;
        let word_count = new_content.split_whitespace().count() as i64;
        let now = now_iso();

        // Write file
        let file_path = self.notes_dir.join(&note.path);
        std::fs::write(&file_path, new_content)?;

        // Update DB
        self.db.execute(
            "UPDATE notes SET title = ?1, content_hash = ?2, tags = ?3, word_count = ?4, updated_at = ?5 WHERE id = ?6",
            params![note.title, sha256_hex(new_content), tags_json, word_count, now, note.id],
        )?;

        // Update FTS
        let rowid = self.note_rowid(&note.id)?;
        self.db.execute(
            "UPDATE notes_fts SET title = ?1, content = ?2, tags = ?3 WHERE rowid = ?4",
            params![
                note.title,
                strip_yaml_frontmatter(new_content),
                combined_tags.join(" "),
                rowid
            ],
        )?;

        // Rebuild links for this note
        self.db
            .execute("DELETE FROM links WHERE source_id = ?1", params![note.id])?;
        self.update_links(&note.id, &parsed.wikilinks, new_content)?;

        // Update tags
        self.db
            .execute("DELETE FROM note_tags WHERE note_id = ?1", params![note.id])?;
        self.update_tag_counts(&note.id, &combined_tags)?;
        // Derive counts from note_tags instead of trusting the increment
        // fast-path above (grill finding F5).
        let mut affected = note.tags.clone();
        affected.extend(combined_tags.iter().cloned());
        affected.sort();
        affected.dedup();
        self.refresh_tag_counts(&affected)?;

        Ok(Note {
            id: note.id,
            title: note.title,
            path: note.path,
            content_hash: sha256_hex(new_content),
            frontmatter: note.frontmatter,
            tags: combined_tags,
            word_count,
            created_at: note.created_at,
            updated_at: now,
            category: note.category,
            is_favorite: note.is_favorite,
        })
    }

    pub fn delete_note(&mut self, identifier: &str) -> Result<()> {
        let note = self.find_note(identifier)?;
        let now = now_iso();

        // Soft delete in DB
        self.db.execute(
            "UPDATE notes SET deleted_at = ?1 WHERE id = ?2",
            params![now, note.id],
        )?;

        // Delete file
        let file_path = self.notes_dir.join(&note.path);
        if file_path.exists() {
            std::fs::remove_file(&file_path)?;
        }

        // Remove from FTS
        let rowid = self.note_rowid(&note.id)?;
        self.db
            .execute("DELETE FROM notes_fts WHERE rowid = ?1", params![rowid])?;

        // Remove links
        self.db.execute(
            "DELETE FROM links WHERE source_id = ?1 OR target_id = ?1",
            params![note.id],
        )?;

        // Remove tag associations and recompute affected tag counts
        // (grill finding F5: counts previously stayed inflated forever)
        self.db.execute(
            "DELETE FROM note_tags WHERE note_id = ?1",
            params![note.id],
        )?;
        self.refresh_tag_counts(&note.tags)?;

        Ok(())
    }

    pub fn read_note(&self, identifier: &str) -> Result<NoteContent> {
        let note = self.find_note(identifier)?;
        let file_path = self.notes_dir.join(&note.path);
        let body = std::fs::read_to_string(&file_path)
            .with_context(|| format!("reading note file {:?}", file_path))?;
        Ok(NoteContent { note, body })
    }

    pub fn list_notes(&self, tag: Option<&str>, limit: i64, offset: i64) -> Result<Vec<Note>> {
        let notes = if let Some(tag) = tag {
            self.db.prepare(
                "SELECT n.id, n.title, n.path, n.content_hash, n.frontmatter, n.tags, n.word_count, n.created_at, n.updated_at, n.category, n.is_favorite
                 FROM notes n JOIN note_tags nt ON n.id = nt.note_id
                 WHERE n.deleted_at IS NULL AND nt.tag = ?1
                 ORDER BY n.updated_at DESC LIMIT ?2 OFFSET ?3"
            )?
            .query_map(params![tag, limit, offset], note_from_row)?
            .filter_map(|r| r.ok())
            .collect()
        } else {
            self.db.prepare(
                "SELECT id, title, path, content_hash, frontmatter, tags, word_count, created_at, updated_at, category, is_favorite
                 FROM notes WHERE deleted_at IS NULL
                 ORDER BY updated_at DESC LIMIT ?1 OFFSET ?2"
            )?
            .query_map(params![limit, offset], note_from_row)?
            .filter_map(|r| r.ok())
            .collect()
        };
        Ok(notes)
    }

    pub fn search(&self, query: &str, limit: i64) -> Result<Vec<SearchResult>> {
        let results = self.db.prepare(
            "SELECT n.title, n.path, fts.rank, snippet(notes_fts, -1, '«', '»', '...', 32) as snippet, n.tags
             FROM notes_fts fts
             JOIN notes n ON n.rowid = fts.rowid
             WHERE notes_fts MATCH ?1 AND n.deleted_at IS NULL
             ORDER BY fts.rank
             LIMIT ?2"
        )?
        .query_map(params![query, limit], |row| {
            Ok(SearchResult {
                title: row.get(0)?,
                path: row.get(1)?,
                rank: row.get(2)?,
                snippet: row.get::<_, String>(3).unwrap_or_default(),
                tags: row.get::<_, String>(4)
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default(),
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
        Ok(results)
    }

    // ── Links ─────────────────────────────────────────────────────

    pub fn get_backlinks(&self, identifier: &str) -> Result<Vec<Link>> {
        let note = self.find_note(identifier)?;
        let links = self
            .db
            .prepare(
                "SELECT source_id, target_id, target_ref, context, line_number
             FROM links WHERE target_id = ?1",
            )?
            .query_map(params![note.id], |row| {
                Ok(Link {
                    source_id: row.get(0)?,
                    target_id: row.get(1)?,
                    target_ref: row.get(2)?,
                    context: row.get(3)?,
                    line_number: row.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(links)
    }

    pub fn get_forward_links(&self, identifier: &str) -> Result<Vec<Link>> {
        let note = self.find_note(identifier)?;
        let links = self
            .db
            .prepare(
                "SELECT source_id, target_id, target_ref, context, line_number
             FROM links WHERE source_id = ?1",
            )?
            .query_map(params![note.id], |row| {
                Ok(Link {
                    source_id: row.get(0)?,
                    target_id: row.get(1)?,
                    target_ref: row.get(2)?,
                    context: row.get(3)?,
                    line_number: row.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(links)
    }

    // ── Tags ──────────────────────────────────────────────────────

    pub fn list_tags(&self) -> Result<Vec<(String, i64)>> {
        let tags = self
            .db
            .prepare("SELECT name, note_count FROM tags ORDER BY note_count DESC")?
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(tags)
    }

    pub fn notes_for_tag(&self, tag: &str) -> Result<Vec<Note>> {
        let notes = self.db.prepare(
            "SELECT n.id, n.title, n.path, n.content_hash, n.frontmatter, n.tags, n.word_count, n.created_at, n.updated_at, n.category, n.is_favorite
             FROM notes n
             JOIN note_tags nt ON n.id = nt.note_id
             WHERE n.deleted_at IS NULL AND nt.tag = ?1
             ORDER BY n.updated_at DESC"
        )?
        .query_map(params![tag], note_from_row)?
        .filter_map(|r| r.ok())
        .collect();
        Ok(notes)
    }

    // ── Reindex ───────────────────────────────────────────────────

    pub fn reindex_all(&mut self) -> Result<ReindexStats> {
        let mut stats = ReindexStats { created: 0, updated: 0, removed: 0, errors: 0 };

        // Walk notes directory
        let md_files = walk_md_files(&self.notes_dir)?;

        for path in &md_files {
            let relative = path
                .strip_prefix(&self.notes_dir)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();

            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => {
                    stats.errors += 1;
                    continue;
                }
            };

            let parsed = extract_links_and_metadata(&content);

            let title = parsed
                .title
                .clone()
                .unwrap_or_else(|| relative.trim_end_matches(".md").to_string());

            let content_hash = sha256_hex(&content);
            let word_count = content.split_whitespace().count() as i64;
            let now = now_iso();

            // Upsert the note
            let existing_id: Option<String> = self
                .db
                .query_row(
                    "SELECT id FROM notes WHERE path = ?1 AND deleted_at IS NULL",
                    params![relative],
                    |row| row.get(0),
                )
                .ok();

            if let Some(id) = existing_id {
                // Update existing
                let tags_json = serde_json::to_string(&parsed.tags)?;
                self.db.execute(
                    "UPDATE notes SET title = ?1, content_hash = ?2, tags = ?3, word_count = ?4, updated_at = ?5 WHERE id = ?6",
                    params![title, content_hash, tags_json, word_count, now, id],
                )?;

                let rowid = self.note_rowid(&id)?;
                self.db.execute(
                    "UPDATE notes_fts SET title = ?1, content = ?2, tags = ?3 WHERE rowid = ?4",
                    params![
                        title,
                        strip_yaml_frontmatter(&content),
                        parsed.tags.join(" "),
                        rowid
                    ],
                )?;

                self.db
                    .execute("DELETE FROM links WHERE source_id = ?1", params![id])?;
                self.update_links(&id, &parsed.wikilinks, &content)?;
                self.db
                    .execute("DELETE FROM note_tags WHERE note_id = ?1", params![id])?;
                self.update_tag_counts(&id, &parsed.tags)?;

                stats.updated += 1;
            } else {
                // Create new
                let id = Uuid::now_v7().to_string();
                let tags_json = serde_json::to_string(&parsed.tags)?;
                self.db.execute(
                    "INSERT INTO notes (id, title, path, content_hash, tags, word_count, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![id, title, relative, content_hash, tags_json, word_count, now, now],
                )?;

                let rowid = self.note_rowid(&id)?;
                self.db.execute(
                    "INSERT INTO notes_fts (rowid, title, content, tags) VALUES (?1, ?2, ?3, ?4)",
                    params![
                        rowid,
                        title,
                        strip_yaml_frontmatter(&content),
                        parsed.tags.join(" ")
                    ],
                )?;

                self.update_links(&id, &parsed.wikilinks, &content)?;
                self.update_tag_counts(&id, &parsed.tags)?;

                stats.created += 1;
            }
        }

        // Remove notes whose files no longer exist
        let all_notes: Vec<(String, String)> = self
            .db
            .prepare("SELECT id, path FROM notes WHERE deleted_at IS NULL")?
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();

        for (id, path) in all_notes {
            if !self.notes_dir.join(&path).exists() {
                self.db.execute(
                    "UPDATE notes SET deleted_at = ?1 WHERE id = ?2",
                    params![now_iso(), id],
                )?;
                stats.removed += 1;
            }
        }

        // Full recount after a bulk pass — counts are always derived from
        // note_tags (grill finding F5).
        let all_tags: Vec<String> = self
            .db
            .prepare("SELECT name FROM tags")?
            .query_map([], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        self.refresh_tag_counts(&all_tags)?;

        Ok(stats)
    }

    /// Compute auto-links between notes based on shared tags and title keyword overlap.
    ///
    /// Returns `(source_id, target_id, confidence)` tuples where confidence is the
    /// Jaccard similarity of tag sets, boosted by title word overlap.
    pub fn compute_keyword_links(&self) -> Result<Vec<(String, String, f64)>> {
        use std::collections::HashSet;

        let notes = self.list_notes(None, 100000, 0)?;
        let mut links = Vec::new();

        // Pre-compute tag sets and title keyword sets
        let note_data: Vec<(&Note, HashSet<String>, Vec<String>)> = notes
            .iter()
            .map(|n| {
                let tag_set: HashSet<String> = n.tags.iter().cloned().collect();
                let title_words: Vec<String> = n
                    .title
                    .split_whitespace()
                    .filter(|w| w.len() > 2)
                    .map(|w| w.to_lowercase())
                    .collect();
                (n, tag_set, title_words)
            })
            .collect();

        for i in 0..note_data.len() {
            for j in (i + 1)..note_data.len() {
                let (note_a, tags_a, words_a) = &note_data[i];
                let (note_b, tags_b, words_b) = &note_data[j];

                // Jaccard similarity on tags
                let tag_intersection = tags_a.intersection(tags_b).count() as f64;
                let tag_union = tags_a.union(tags_b).count() as f64;
                let tag_sim = if tag_union > 0.0 {
                    tag_intersection / tag_union
                } else {
                    0.0
                };

                // Title word overlap
                let words_a_set: HashSet<&str> = words_a.iter().map(|s| s.as_str()).collect();
                let words_b_set: HashSet<&str> = words_b.iter().map(|s| s.as_str()).collect();
                let word_intersection = words_a_set.intersection(&words_b_set).count() as f64;
                let word_union = words_a_set.union(&words_b_set).count() as f64;
                let title_sim = if word_union > 0.0 {
                    word_intersection / word_union
                } else {
                    0.0
                };

                // Combined score: tags are primary, title overlap is bonus
                let score = tag_sim * 0.7 + title_sim * 0.3;

                // Minimum threshold: need at least some similarity
                if score >= 0.15 && (tag_intersection >= 1.0 || title_sim > 0.5) {
                    links.push((note_a.id.clone(), note_b.id.clone(), score));
                }
            }
        }

        // Sort by confidence descending
        links.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        Ok(links)
    }

    // ── Graph ─────────────────────────────────────────────────────

    /// Full knowledge graph (nodes + edges) for the current profile.
    /// Nodes are non-deleted notes; edges keep unresolved targets as
    /// `target_id: None` with the raw `target_ref` so clients can render
    /// broken links (feeds the `cerebro_graph` MCP tool, ADR 0005).
    pub fn get_graph(&self) -> Result<(Vec<Note>, Vec<Link>)> {
        let notes = self.list_notes(None, 100_000, 0)?;
        let live_ids: std::collections::HashSet<&str> =
            notes.iter().map(|n| n.id.as_str()).collect();

        let mut edges: Vec<Link> = self
            .db
            .prepare(
                "SELECT source_id, target_id, target_ref, context, line_number
                 FROM links",
            )?
            .query_map([], |row| {
                Ok(Link {
                    source_id: row.get(0)?,
                    target_id: row.get(1)?,
                    target_ref: row.get(2)?,
                    context: row.get(3)?,
                    line_number: row.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .filter(|l| live_ids.contains(l.source_id.as_str()))
            .map(|l| {
                let target_live = l
                    .target_id
                    .as_deref()
                    .map(|t| live_ids.contains(t))
                    .unwrap_or(false);
                if target_live {
                    l
                } else {
                    Link {
                        target_id: None,
                        ..l
                    }
                }
            })
            .collect();

        edges.sort_by(|a, b| a.source_id.cmp(&b.source_id));
        Ok((notes, edges))
    }

    // ── Category & Favorites ──────────────────────────────────────

    pub fn set_category(&self, note_id: &str, category: Option<&str>) -> Result<()> {
        self.db.execute(
            "UPDATE notes SET category = ?1 WHERE id = ?2",
            params![category, note_id],
        )?;
        Ok(())
    }

    pub fn set_favorite(&self, note_id: &str, favorite: bool) -> Result<()> {
        self.db.execute(
            "UPDATE notes SET is_favorite = ?1 WHERE id = ?2",
            params![favorite as i64, note_id],
        )?;
        Ok(())
    }

    // ── Internal helpers ──────────────────────────────────────────

    fn find_note(&self, identifier: &str) -> Result<Note> {
        let columns = "id, title, path, content_hash, frontmatter, tags, word_count, created_at, updated_at, category, is_favorite";

        // 1. Exact ID
        if let Ok(note) = self.db.query_row(
            &format!("SELECT {columns} FROM notes WHERE id = ?1 AND deleted_at IS NULL"),
            params![identifier],
            note_from_row,
        ) {
            return Ok(note);
        }
        // 2. Exact title
        if let Ok(note) = self.db.query_row(
            &format!("SELECT {columns} FROM notes WHERE title = ?1 AND deleted_at IS NULL"),
            params![identifier],
            note_from_row,
        ) {
            return Ok(note);
        }
        // 3. Exact path
        if let Ok(note) = self.db.query_row(
            &format!("SELECT {columns} FROM notes WHERE path = ?1 AND deleted_at IS NULL"),
            params![identifier],
            note_from_row,
        ) {
            return Ok(note);
        }
        // 4. Substring match — only when it is UNAMBIGUOUS. Feeding a fuzzy
        // first-row match into update/delete can hit the wrong note
        // (grill finding F8).
        let pattern = format!("%{}%", identifier);
        let matches: Vec<Note> = self
            .db
            .prepare(&format!(
                "SELECT {columns} FROM notes WHERE (title LIKE ?1 OR path LIKE ?1)
                 AND deleted_at IS NULL LIMIT 50"
            ))?
            .query_map(params![pattern], note_from_row)?
            .filter_map(|r| r.ok())
            .collect();

        match matches.len() {
            0 => Err(anyhow!("note not found: {}", identifier)),
            1 => Ok(matches.into_iter().next().unwrap()),
            n => {
                let mut candidates: Vec<String> = matches
                    .iter()
                    .map(|m| format!("'{}' ({})", m.title, m.path))
                    .collect();
                candidates.truncate(5);
                Err(anyhow!(
                    "ambiguous identifier '{}' matches {} notes; be specific (use ID, exact title, or exact path). Candidates: {}",
                    identifier,
                    n,
                    candidates.join(", ")
                ))
            }
        }
    }

    fn note_rowid(&self, id: &str) -> Result<i64> {
        self.db
            .query_row(
                "SELECT rowid FROM notes WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .map_err(|_| anyhow!("note not found for rowid lookup: {}", id))
    }

    fn update_links(
        &self,
        source_id: &str,
        wikilinks: &[crate::parser::Wikilink],
        content: &str,
    ) -> Result<()> {
        // The links table enforces UNIQUE(source_id, target_ref, line_number):
        // referencing the same target twice on one line must produce one edge,
        // not a constraint failure (grill finding F2).
        let mut seen: std::collections::HashSet<(String, usize)> = std::collections::HashSet::new();
        for wl in wikilinks {
            if !seen.insert((wl.target.clone(), wl.line_number)) {
                continue;
            }
            // Try to resolve the target note
            let target_id = self.resolve_target(&wl.target);
            let context = extract_link_context(content, wl.line_number);

            self.db.execute(
                "INSERT INTO links (source_id, target_id, target_ref, context, line_number)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![source_id, target_id, wl.target, context, wl.line_number as i64],
            )?;
        }
        Ok(())
    }

    /// Resolve a wikilink target to a note ID. Exact title, then exact path,
    /// then a UNIQUE substring match; ambiguity resolves to nothing rather
    /// than an arbitrary note (grill finding F8).
    fn resolve_target(&self, target: &str) -> Option<String> {
        if let Ok(id) = self
            .db
            .query_row(
                "SELECT id FROM notes WHERE title = ?1 AND deleted_at IS NULL",
                params![target],
                |row| row.get(0),
            )
        {
            return Some(id);
        }
        if let Ok(id) = self
            .db
            .query_row(
                "SELECT id FROM notes WHERE path = ?1 AND deleted_at IS NULL",
                params![target],
                |row| row.get(0),
            )
        {
            return Some(id);
        }
        let pattern = format!("%{}%", target);
        let matches: Vec<String> = self
            .db
            .prepare(
                "SELECT id FROM notes WHERE (title LIKE ?1 OR path LIKE ?1)
                 AND deleted_at IS NULL LIMIT 2",
            )
            .ok()?
            .query_map(params![pattern], |row| row.get(0))
            .ok()?
            .filter_map(|r| r.ok())
            .collect();
        if matches.len() == 1 {
            Some(matches.into_iter().next().unwrap())
        } else {
            None
        }
    }

    fn update_tag_counts(&self, note_id: &str, tags: &[String]) -> Result<()> {
        for tag in tags {
            // Ensure tag exists first (FK constraint)
            self.db.execute(
                "INSERT INTO tags (name, note_count) VALUES (?1, 1)
                 ON CONFLICT(name) DO UPDATE SET note_count = note_count + 1",
                params![tag],
            )?;
            self.db.execute(
                "INSERT INTO note_tags (note_id, tag) VALUES (?1, ?2)",
                params![note_id, tag],
            )?;
        }
        Ok(())
    }

    /// Recompute `note_count` for the given tags from `note_tags`, counting
    /// only non-deleted notes, and drop tags that no longer have any notes.
    ///
    /// Counters must be derived, never incremented blindly: update/delete
    /// paths previously left counts inflated forever (grill finding F5).
    fn refresh_tag_counts(&self, tags: &[String]) -> Result<()> {
        for tag in tags {
            self.db.execute(
                "UPDATE tags SET note_count = (
                    SELECT COUNT(*) FROM note_tags nt
                    JOIN notes n ON n.id = nt.note_id
                    WHERE nt.tag = ?1 AND n.deleted_at IS NULL
                ) WHERE name = ?1",
                params![tag],
            )?;
        }
        self.db.execute("DELETE FROM tags WHERE note_count = 0", [])?;
        Ok(())
    }

    fn sanitize_path(&self, title: &str) -> String {
        let safe: String = title
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        format!("{}.md", safe)
    }

    /// Normalize and validate a user-supplied note path.
    ///
    /// The resolved path must stay inside the vault's notes directory (no
    /// `..` traversal, no absolute paths) and must name a `.md` file:
    /// directory-like inputs (trailing `/`, no extension) get a filename
    /// derived from the title appended.
    fn validate_note_path(&self, raw: &str, title: &str) -> Result<String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok(self.sanitize_path(title));
        }
        let path = Path::new(raw);
        if path.is_absolute() {
            return Err(anyhow!(
                "note path must be relative to the vault notes directory: {}",
                raw
            ));
        }

        // Containment: after cancelling `.`/`..` components, depth must
        // never go above the notes root.
        let mut depth: i32 = 0;
        for comp in path.components() {
            match comp {
                Component::CurDir => {}
                Component::ParentDir => {
                    depth -= 1;
                    if depth < 0 {
                        return Err(anyhow!(
                            "note path escapes the vault notes directory: {}",
                            raw
                        ));
                    }
                }
                Component::Normal(_) => depth += 1,
                _ => {
                    return Err(anyhow!("invalid path component in: {}", raw));
                }
            }
        }

        let cleaned: String = path
            .components()
            .filter(|c| !matches!(c, Component::CurDir))
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("/");

        // Directory-like or extension-less input -> append filename from title
        if cleaned.ends_with('/') || Path::new(&cleaned).extension().is_none() {
            let dir = cleaned.trim_end_matches('/');
            let file = self.sanitize_path(title);
            if dir.is_empty() {
                return Ok(file);
            }
            return Ok(format!("{}/{}", dir, file));
        }

        Ok(cleaned)
    }
}

// ── Helper functions ─────────────────────────────────────────────

fn note_from_row(row: &rusqlite::Row) -> rusqlite::Result<Note> {
    Ok(Note {
        id: row.get("id")?,
        title: row.get("title")?,
        path: row.get("path")?,
        content_hash: row.get("content_hash")?,
        word_count: row.get("word_count")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        frontmatter: row.get("frontmatter")?,
        tags: row
            .get::<_, String>("tags")
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
        category: row.get::<_, Option<String>>("category").ok().flatten(),
        is_favorite: row
            .get::<_, Option<i64>>("is_favorite")
            .ok()
            .flatten()
            .map(|v| v != 0)
            .unwrap_or(false),
    })
}

fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

fn build_frontmatter(title: &str, tags: &[String]) -> String {
    let mut fm = format!("title: {}\n", title);
    if !tags.is_empty() {
        fm.push_str("tags:\n");
        for tag in tags {
            fm.push_str(&format!("  - {}\n", tag));
        }
    }
    fm
}

fn strip_yaml_frontmatter(content: &str) -> String {
    if content.starts_with("---") {
        if let Some(end) = content[3..].find("---") {
            return content[3 + end + 3..].trim().to_string();
        }
    }
    content.to_string()
}

fn extract_link_context(content: &str, line_number: usize) -> Option<String> {
    // 1-indexed line number from the wikilink parser; return THAT line's text,
    // not the first line that happens to contain a link (grill finding F6).
    if line_number == 0 {
        return None;
    }
    content
        .lines()
        .nth(line_number - 1)
        .map(|s| s.trim().to_string())
}

fn walk_md_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !dir.exists() {
        return Ok(files);
    }
    for entry in walkdir::WalkDir::new(dir) {
        let entry = entry?;
        if entry.file_type().is_file() {
            let path = entry.path().to_path_buf();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                files.push(path);
            }
        }
    }
    Ok(files)
}
