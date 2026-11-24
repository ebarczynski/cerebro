use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::profile::ProfileConfig;
use crate::parser::{extract_links_and_metadata, split_frontmatter, ParsedNote, Wikilink};

// ── Constants ────────────────────────────────────────────────────

/// How long a write waits for another connection's write lock before giving
/// up with "database is locked". rusqlite happens to default to the same
/// 5000 ms; it is set here so the behaviour is a decision, not an accident.
pub const BUSY_TIMEOUT: Duration = Duration::from_millis(5_000);

/// Upper bound on automatic (keyword) links kept per note. Without it, N notes
/// that share a tag produce N(N-1)/2 links — 1.7 million for 5,000 notes.
pub const MAX_KEYWORD_LINKS_PER_NOTE: usize = 20;

/// Upper bound on notes loaded for whole-vault operations (graph, auto-links).
const MAX_GRAPH_NOTES: i64 = 100_000;

/// Notes are reindexed in transactions of this size, so the write lock is
/// never held for the whole pass.
const REINDEX_BATCH: usize = 500;

/// Longest stored excerpt of the line a link sits on, in characters. A
/// Markdown paragraph is often one very long line; storing all of it for
/// every link on it copied whole paragraphs into the index.
const MAX_LINK_CONTEXT_CHARS: usize = 200;

const NOTE_COLUMNS: &str =
    "id, title, path, content_hash, frontmatter, tags, word_count, created_at, updated_at, category, is_favorite";

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

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ReindexStats {
    pub created: usize,
    pub updated: usize,
    pub removed: usize,
    pub errors: usize,
}

enum Outcome {
    Created,
    Updated,
    Unchanged,
}

// ── Storage Engine ───────────────────────────────────────────────

pub struct Storage {
    db: Connection,
    profile_dir: PathBuf,
    notes_dir: PathBuf,
    profile_config: ProfileConfig,
}

impl Storage {
    /// Open (creating if needed) the index for `profile_dir`.
    ///
    /// Fails with an explicit error when the vault asks for encryption-at-rest:
    /// that feature does not exist yet, and silently writing plaintext for a
    /// vault configured as encrypted would be worse than refusing.
    pub fn open(profile_dir: &Path, profile_config: &ProfileConfig) -> Result<Self> {
        if profile_config.encryption {
            bail!(
                "vault '{}' sets encryption = true, but encryption-at-rest is not implemented yet; \
                 refusing to store this vault in plaintext (remove the setting to continue)",
                profile_config.name
            );
        }
        let notes_dir = profile_config.resolved_notes_dir(profile_dir);
        std::fs::create_dir_all(&notes_dir)
            .with_context(|| format!("creating notes directory {:?}", notes_dir))?;

        let db_path = profile_dir.join("db.sqlite");
        let db = Connection::open(&db_path)
            .with_context(|| format!("opening database at {:?}", db_path))?;
        db.busy_timeout(BUSY_TIMEOUT)
            .context("setting the SQLite busy timeout")?;
        enable_wal(&db)?;
        db.execute_batch("PRAGMA foreign_keys=ON;")
            .context("enabling foreign key enforcement")?;

        let mut storage = Storage {
            db,
            profile_dir: profile_dir.to_path_buf(),
            notes_dir,
            profile_config: profile_config.clone(),
        };
        storage.run_migrations()?;
        Ok(storage)
    }

    pub fn profile_name(&self) -> &str {
        &self.profile_config.name
    }

    /// Bring an empty index up to date with the note files in the vault.
    ///
    /// The index is empty right after it was created — `db.sqlite` was
    /// deleted, or the vault was copied without it. Without this every
    /// command would report an empty vault until someone ran `cerebro reindex`.
    /// Returns `Ok(None)` when there was nothing to do.
    pub fn rebuild_if_empty(&mut self) -> Result<Option<ReindexStats>> {
        let has_notes = self
            .db
            .query_row("SELECT 1 FROM notes LIMIT 1", [], |_| Ok(()))
            .optional()?
            .is_some();
        if has_notes || walk_md_files(&self.notes_dir)?.is_empty() {
            return Ok(None);
        }
        Ok(Some(self.reindex_all()?))
    }

    /// Directory holding this vault's `db.sqlite` and `vault.toml`.
    pub fn profile_dir(&self) -> &Path {
        &self.profile_dir
    }

    /// The configuration this vault was opened with.
    pub fn profile_config(&self) -> &ProfileConfig {
        &self.profile_config
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

    fn run_migrations(&mut self) -> Result<()> {
        self.db.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .context("ensuring the schema_migrations table")?;

        const MIGRATIONS: &[(i64, &str, &str)] = &[
            (1, "initial", include_str!("../../migrations/001_initial.sql")),
            (2, "fts_triggers", include_str!("../../migrations/002_fts_triggers.sql")),
            (3, "categories_favorites", include_str!("../../migrations/003_categories_favorites.sql")),
            (4, "lookup_indexes", include_str!("../../migrations/004_lookup_indexes.sql")),
        ];

        // Fast path (the usual one): everything is applied, no lock is taken.
        if pending_migrations(&self.db, MIGRATIONS)?.is_empty() {
            return Ok(());
        }

        // Take the write lock first, so two processes opening a brand-new
        // vault at the same moment run the migrations one after the other
        // instead of both inserting the same schema_migrations row. Each
        // migration is applied whole or not at all.
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("acquiring the write lock to migrate the database")?;
        for (id, name, sql) in pending_migrations(&tx, MIGRATIONS)? {
            tx.execute_batch(sql)
                .with_context(|| format!("applying migration {id:03} ({name})"))?;
            tx.execute(
                "INSERT INTO schema_migrations (id, name) VALUES (?1, ?2)",
                params![id, name],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // ── Note CRUD ─────────────────────────────────────────────────

    /// Create a new note. Returns the note metadata.
    ///
    /// Never overwrites an existing note: a filename derived from the title
    /// that is already taken gets a numeric suffix (`Notes_2.md`); an explicit
    /// path that is taken is an error. File and index rows are written
    /// all-or-nothing — on failure the note exists neither on disk nor in the
    /// index.
    pub fn create_note(
        &mut self,
        title: &str,
        content: &str,
        path: Option<&str>,
        tags: Option<Vec<String>>,
    ) -> Result<Note> {
        let title = title.trim();
        if title.is_empty() {
            bail!("note title must not be empty");
        }
        let Self {
            db,
            notes_dir,
            ..
        } = self;
        let notes_dir: &Path = notes_dir.as_path();

        // Take the write lock before touching anything, so a busy database is
        // reported as an error instead of leaving a file the index never saw.
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let relative_path = available_note_path(&tx, notes_dir, title, path)?;

        // A soft-deleted row from an earlier version may still own this path.
        if let Some(tombstone) = find_tombstone(&tx, &relative_path)? {
            purge_note(&tx, &tombstone)?;
        }

        let parsed = extract_links_and_metadata(content);

        // Build frontmatter + content
        let mut combined_tags = tags.unwrap_or_default();
        combined_tags.extend(parsed.tags.iter().cloned());
        combined_tags.sort();
        combined_tags.dedup();

        let now = now_iso();
        let full_content = if parsed.frontmatter.is_empty() {
            let fm = build_frontmatter(title, &combined_tags);
            format!("---\n{}---\n{}", fm, content)
        } else {
            content.to_string()
        };

        let file_path = notes_dir.join(&relative_path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating directory {:?}", parent))?;
        }
        write_new_file(&file_path, &full_content)
            .with_context(|| format!("creating note file {}", relative_path))?;

        // From here on, any failure must also remove the file this call created.
        let indexed = (|| -> Result<Note> {
            let note = index_new_note(
                &tx,
                NewNote {
                    path: &relative_path,
                    title,
                    full_content: &full_content,
                    supplied: content,
                    parsed: &parsed,
                    tags: &combined_tags,
                    now: &now,
                },
            )?;
            resolve_pending_links(&tx, Some(&link_keys(title, &relative_path)))?;
            refresh_tag_counts(&tx, &combined_tags)?;
            Ok(note)
        })();
        match indexed {
            Ok(note) => {
                tx.commit()?;
                Ok(note)
            }
            Err(e) => {
                let _ = std::fs::remove_file(&file_path);
                Err(e)
            }
        }
    }

    /// Returns true when a note lives at this path — in the index, or as a
    /// file on disk that has not been indexed yet.
    pub fn note_exists(&self, path: &str) -> bool {
        let indexed = self
            .db
            .query_row(
                "SELECT 1 FROM notes WHERE path = ?1 AND deleted_at IS NULL",
                params![path],
                |_| Ok(()),
            )
            .is_ok();
        indexed || self.notes_dir.join(path).exists()
    }

    pub fn update_note(&mut self, identifier: &str, new_content: &str) -> Result<Note> {
        let Self {
            db,
            notes_dir,
            ..
        } = self;
        let notes_dir: &Path = notes_dir.as_path();
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let note = find_note(&tx, identifier)?;
        let parsed = extract_links_and_metadata(new_content);
        // The file is the source of truth: a changed `title:` in the file
        // renames the note, exactly as a reindex would.
        let title = parsed
            .explicit_title
            .clone()
            .unwrap_or_else(|| file_stem_of(&note.path));
        let combined_tags = parsed.tags.clone();
        let tags_json = serde_json::to_string(&combined_tags)?;
        let word_count = new_content.split_whitespace().count() as i64;
        let now = now_iso();
        let hash = sha256_hex(new_content);

        let file_path = notes_dir.join(&note.path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating directory {:?}", parent))?;
        }
        // The file is replaced in one step; if that fails the index is untouched.
        write_file_atomically(&file_path, new_content.as_bytes())
            .with_context(|| format!("writing note file {}", note.path))?;

        tx.execute(
            "UPDATE notes SET title = ?1, content_hash = ?2, tags = ?3, word_count = ?4, updated_at = ?5 WHERE id = ?6",
            params![title, hash, tags_json, word_count, now, note.id],
        )?;

        set_fts_row(&tx, &note.id, &title, &strip_yaml_frontmatter(new_content), &parsed.tags)?;

        tx.execute("DELETE FROM links WHERE source_id = ?1", params![note.id])?;
        insert_links(&tx, &note.id, &parsed.wikilinks, new_content)?;

        tx.execute("DELETE FROM note_tags WHERE note_id = ?1", params![note.id])?;
        set_note_tags(&tx, &note.id, &combined_tags)?;

        // The note may have been renamed: links that resolved to it by an old
        // title are opened again and re-resolved against its names.
        tx.execute(
            "UPDATE links SET target_id = NULL WHERE target_id = ?1 AND target_ref <> ?2 COLLATE NOCASE",
            params![note.id, title],
        )?;
        resolve_pending_links(&tx, Some(&link_keys(&title, &note.path)))?;
        let mut affected = note.tags.clone();
        affected.extend(combined_tags.iter().cloned());
        affected.sort();
        affected.dedup();
        // Derive counts from note_tags instead of trusting the increment
        // fast-path above (grill finding F5).
        refresh_tag_counts(&tx, &affected)?;

        tx.commit()?;

        Ok(Note {
            id: note.id,
            title,
            path: note.path,
            content_hash: hash,
            frontmatter: note.frontmatter,
            tags: combined_tags,
            word_count,
            created_at: note.created_at,
            updated_at: now,
            category: note.category,
            is_favorite: note.is_favorite,
        })
    }

    /// Delete a note: its file and every index row. If the file cannot be
    /// removed, nothing is deleted.
    pub fn delete_note(&mut self, identifier: &str) -> Result<()> {
        let Self {
            db,
            notes_dir,
            ..
        } = self;
        let notes_dir: &Path = notes_dir.as_path();
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let note = find_note(&tx, identifier)?;

        purge_note(&tx, &note.id)?;
        refresh_tag_counts(&tx, &note.tags)?;

        let file_path = notes_dir.join(&note.path);
        if file_path.exists() {
            std::fs::remove_file(&file_path)
                .with_context(|| format!("deleting note file {}", note.path))?;
        }

        tx.commit()?;
        Ok(())
    }

    /// `(title, path)` of a live note, by ID — for showing a link's other end.
    pub fn note_title(&self, note_id: &str) -> Option<(String, String)> {
        self.db
            .query_row(
                "SELECT title, path FROM notes WHERE id = ?1 AND deleted_at IS NULL",
                params![note_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok()
    }

    pub fn read_note(&self, identifier: &str) -> Result<NoteContent> {
        let note = find_note(&self.db, identifier)?;
        let file_path = self.notes_dir.join(&note.path);
        let body = std::fs::read_to_string(&file_path)
            .with_context(|| format!("reading note file {:?}", file_path))?;
        Ok(NoteContent { note, body })
    }

    pub fn list_notes(&self, tag: Option<&str>, limit: i64, offset: i64) -> Result<Vec<Note>> {
        let notes = if let Some(tag) = tag {
            self.db
                .prepare_cached(&format!(
                    "SELECT {NOTE_COLUMNS}
                     FROM notes n JOIN note_tags nt ON n.id = nt.note_id
                     WHERE n.deleted_at IS NULL AND nt.tag = ?1
                     ORDER BY n.updated_at DESC LIMIT ?2 OFFSET ?3"
                ))?
                .query_map(params![tag, limit, offset], note_from_row)?
                .filter_map(|r| r.ok())
                .collect()
        } else {
            self.db
                .prepare_cached(&format!(
                    "SELECT {NOTE_COLUMNS}
                     FROM notes WHERE deleted_at IS NULL
                     ORDER BY updated_at DESC LIMIT ?1 OFFSET ?2"
                ))?
                .query_map(params![limit, offset], note_from_row)?
                .filter_map(|r| r.ok())
                .collect()
        };
        Ok(notes)
    }

    /// Full-text search over titles, bodies and tags.
    ///
    /// `query` may use FTS5 syntax (`AND`, `OR`, `word*`, `"a phrase"`), but
    /// plain text works too: hyphenated words are searched as the phrase the
    /// indexer split them into, and a query FTS5 will not parse — a stray
    /// bracket, a dangling operator, an unbalanced quote — is retried as a
    /// search for its literal words. A broken query never masquerades as
    /// "no results", and a half-typed one still finds something.
    pub fn search(&self, query: &str, limit: i64) -> Result<Vec<SearchResult>> {
        let query = fts5_query(query.trim());
        // The statement is constant, so whatever it rejects is the query text.
        let first_error = match self.run_search(&query, limit) {
            Ok(results) => return Ok(results),
            Err(error) => error,
        };
        for literal in [literal_query(&query, true), literal_query(&query, false)] {
            if literal.is_empty() {
                continue;
            }
            if let Ok(results) = self.run_search(&literal, limit) {
                return Ok(results);
            }
        }
        Err(anyhow!(
            "'{query}' is not a valid search query ({first_error}); nor are its words searchable"
        ))
    }

    fn run_search(&self, query: &str, limit: i64) -> Result<Vec<SearchResult>> {
        let mut stmt = self.db.prepare_cached(
            "SELECT n.title, n.path, fts.rank, snippet(notes_fts, -1, '«', '»', '...', 32) as snippet, n.tags
             FROM notes_fts fts
             JOIN notes n ON n.rowid = fts.rowid
             WHERE notes_fts MATCH ?1 AND n.deleted_at IS NULL
             ORDER BY fts.rank
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![query, limit], |row| {
            Ok(SearchResult {
                title: row.get(0)?,
                path: row.get(1)?,
                rank: row.get(2)?,
                snippet: row.get::<_, String>(3).unwrap_or_default(),
                tags: row
                    .get::<_, String>(4)
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default(),
            })
        })?;
        // Errors are reported, never dropped: a dropped row used to turn a
        // rejected query into an empty result.
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ── Links ─────────────────────────────────────────────────────

    pub fn get_backlinks(&self, identifier: &str) -> Result<Vec<Link>> {
        let note = find_note(&self.db, identifier)?;
        select_links(
            &self.db,
            "SELECT source_id, target_id, target_ref, context, line_number FROM links WHERE target_id = ?1",
            params![note.id],
        )
    }

    pub fn get_forward_links(&self, identifier: &str) -> Result<Vec<Link>> {
        let note = find_note(&self.db, identifier)?;
        select_links(
            &self.db,
            "SELECT source_id, target_id, target_ref, context, line_number FROM links WHERE source_id = ?1",
            params![note.id],
        )
    }

    // ── Tags ──────────────────────────────────────────────────────

    pub fn list_tags(&self) -> Result<Vec<(String, i64)>> {
        let tags = self
            .db
            .prepare_cached("SELECT name, note_count FROM tags ORDER BY note_count DESC")?
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(tags)
    }

    pub fn notes_for_tag(&self, tag: &str) -> Result<Vec<Note>> {
        let notes = self
            .db
            .prepare_cached(&format!(
                "SELECT {NOTE_COLUMNS}
                 FROM notes n
                 JOIN note_tags nt ON n.id = nt.note_id
                 WHERE n.deleted_at IS NULL AND nt.tag = ?1
                 ORDER BY n.updated_at DESC"
            ))?
            .query_map(params![tag], note_from_row)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(notes)
    }

    // ── Reindex ───────────────────────────────────────────────────

    /// Bring the index in line with the `.md` files on disk.
    ///
    /// * Files that are new or changed are (re)indexed; unchanged files are
    ///   left alone, so their `updated_at` keeps meaning "when it changed".
    /// * Notes whose file is gone are removed together with their links,
    ///   tags and search rows.
    /// * Wikilinks are resolved once, at the end, against the complete set of
    ///   notes — so the result does not depend on the order files are walked.
    pub fn reindex_all(&mut self) -> Result<ReindexStats> {
        let Self {
            db,
            notes_dir,
            ..
        } = self;
        let notes_dir: &Path = notes_dir.as_path();
        let mut stats = ReindexStats::default();
        let md_files = walk_md_files(notes_dir)?;
        let now = now_iso();

        for chunk in md_files.chunks(REINDEX_BATCH) {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            for path in chunk {
                let relative = path
                    .strip_prefix(notes_dir)
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
                match upsert_file(&tx, &relative, &content, &now)? {
                    Outcome::Created => stats.created += 1,
                    Outcome::Updated => stats.updated += 1,
                    Outcome::Unchanged => {}
                }
            }
            tx.commit()?;
        }

        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Notes whose files no longer exist
        let live: Vec<(String, String)> = {
            let mut stmt = tx.prepare_cached("SELECT id, path FROM notes WHERE deleted_at IS NULL")?;
            let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
            rows.filter_map(|r| r.ok()).collect()
        };
        for (id, path) in &live {
            if !notes_dir.join(path).exists() {
                purge_note(&tx, id)?;
                stats.removed += 1;
            }
        }

        // Resolve every open link now that the index is complete. First the
        // bulk sweep — links whose target note is gone — which is why the
        // single-note paths do not need one.
        tx.execute(
            "UPDATE links SET target_id = NULL
             WHERE target_id IS NOT NULL
               AND target_id NOT IN (SELECT id FROM notes WHERE deleted_at IS NULL)",
            [],
        )?;
        resolve_pending_links(&tx, None)?;

        // Full recount after a bulk pass — counts are always derived from
        // note_tags (grill finding F5).
        let all_tags: Vec<String> = {
            let mut stmt = tx.prepare_cached("SELECT name FROM tags")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.filter_map(|r| r.ok()).collect()
        };
        refresh_tag_counts(&tx, &all_tags)?;
        tx.commit()?;

        Ok(stats)
    }

    /// Compute auto-links between notes based on shared tags and title keyword overlap.
    ///
    /// Returns `(source_id, target_id, confidence)` tuples where confidence is the
    /// Jaccard similarity of tag sets, boosted by title word overlap, sorted by
    /// confidence descending. At most [`MAX_KEYWORD_LINKS_PER_NOTE`] links are
    /// kept per note (its highest-confidence ones), which bounds the result —
    /// and the JSON the GUI has to swallow — to O(notes).
    pub fn compute_keyword_links(&self) -> Result<Vec<(String, String, f64)>> {
        self.compute_keyword_links_with_limit(MAX_KEYWORD_LINKS_PER_NOTE)
    }

    /// [`Self::compute_keyword_links`] with an explicit per-note limit
    /// (`usize::MAX` keeps every pair that passes the threshold).
    pub fn compute_keyword_links_with_limit(&self, max_per_note: usize) -> Result<Vec<(String, String, f64)>> {
        let notes = self.list_notes(None, MAX_GRAPH_NOTES, 0)?;
        let n = notes.len();

        // Sorted, de-duplicated tag and title-word sets per note: small enough
        // that merging beats hashing, and computed once instead of per pair.
        let sort_dedup = |mut items: Vec<String>| {
            items.sort();
            items.dedup();
            items
        };
        let tag_sets: Vec<Vec<String>> = notes.iter().map(|note| sort_dedup(note.tags.clone())).collect();
        let word_sets: Vec<Vec<String>> = notes
            .iter()
            .map(|note| {
                sort_dedup(
                    note.title
                        .split_whitespace()
                        .filter(|w| w.chars().count() > 2)
                        .map(|w| w.to_lowercase())
                        .collect(),
                )
            })
            .collect();

        // The best `max_per_note` neighbours of every note, in bounded heaps.
        let mut best: Vec<BinaryHeap<Reverse<(OrderedScore, usize)>>> = (0..n).map(|_| BinaryHeap::new()).collect();
        for i in 0..n {
            for j in (i + 1)..n {
                // Jaccard similarity on tags
                let (tag_intersection, tag_union) = overlap(&tag_sets[i], &tag_sets[j]);
                let tag_sim = ratio(tag_intersection, tag_union);

                // Title word overlap
                let (word_intersection, word_union) = overlap(&word_sets[i], &word_sets[j]);
                let title_sim = ratio(word_intersection, word_union);

                // Combined score: tags are primary, title overlap is bonus
                let score = tag_sim * 0.7 + title_sim * 0.3;

                // Minimum threshold: need at least some similarity
                if score >= 0.15 && (tag_intersection >= 1 || title_sim > 0.5) {
                    offer(&mut best[i], score, j, max_per_note);
                    offer(&mut best[j], score, i, max_per_note);
                }
            }
        }

        // A pair is kept when it is among the best of either end.
        let mut seen: HashSet<(usize, usize)> = HashSet::new();
        let mut links = Vec::new();
        for (i, heap) in best.iter().enumerate() {
            for Reverse((OrderedScore(score), j)) in heap {
                let pair = (i.min(*j), i.max(*j));
                if seen.insert(pair) {
                    links.push((notes[pair.0].id.clone(), notes[pair.1].id.clone(), *score));
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
        let notes = self.list_notes(None, MAX_GRAPH_NOTES, 0)?;
        let live_ids: HashSet<&str> = notes.iter().map(|n| n.id.as_str()).collect();

        let mut edges = select_links(
            &self.db,
            "SELECT source_id, target_id, target_ref, context, line_number FROM links",
            params![],
        )?;
        edges.retain(|l| live_ids.contains(l.source_id.as_str()));
        for link in &mut edges {
            let target_live = link.target_id.as_deref().is_some_and(|t| live_ids.contains(t));
            if !target_live {
                link.target_id = None;
            }
        }

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
}

/// The migrations whose id is not recorded in `schema_migrations`.
fn pending_migrations<'a>(
    db: &Connection,
    migrations: &'a [(i64, &'a str, &'a str)],
) -> Result<Vec<&'a (i64, &'a str, &'a str)>> {
    let applied: HashSet<i64> = {
        let mut stmt = db.prepare("SELECT id FROM schema_migrations")?;
        let ids = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        ids.filter_map(|r| r.ok()).collect()
    };
    Ok(migrations.iter().filter(|(id, _, _)| !applied.contains(id)).collect())
}

// ── Writing a new note ───────────────────────────────────────────

/// The path a new note will get.
///
/// * no path, or a directory (`inbox/`): the file name comes from the title,
///   with a numeric suffix if that name is taken;
/// * an explicit file path: must be free, and must end in `.md`.
fn available_note_path(db: &Connection, notes_dir: &Path, title: &str, path: Option<&str>) -> Result<String> {
    let requested = path.map(str::trim).filter(|p| !p.is_empty());
    let (dir, explicit_file) = match requested {
        None => (String::new(), None),
        Some(raw) => {
            let cleaned = validate_note_path(raw)?;
            let names_a_file = !cleaned.ends_with('/') && Path::new(&cleaned).extension().is_some();
            if !names_a_file {
                (cleaned.trim_end_matches('/').to_string(), None)
            } else {
                match cleaned.rfind('/') {
                    Some(i) => (cleaned[..i].to_string(), Some(cleaned[i + 1..].to_string())),
                    None => (String::new(), Some(cleaned)),
                }
            }
        }
    };

    let file_name = match explicit_file {
        Some(name) => {
            let is_md = Path::new(&name)
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("md"));
            if !is_md {
                bail!("note files must use the .md extension: {name}");
            }
            let full = join_vault_path(&dir, &name);
            if path_taken(db, notes_dir, &full) {
                bail!("a note already exists at '{full}'");
            }
            name
        }
        None => {
            let stem = sanitize_stem(title);
            let mut candidate = format!("{stem}.md");
            let mut suffix = 1;
            while path_taken(db, notes_dir, &join_vault_path(&dir, &candidate)) {
                suffix += 1;
                candidate = format!("{stem}_{suffix}.md");
                if suffix > 10_000 {
                    bail!("could not find a free file name for '{title}'");
                }
            }
            candidate
        }
    };
    Ok(join_vault_path(&dir, &file_name))
}

/// A note whose file has just been written, ready to be indexed.
struct NewNote<'a> {
    path: &'a str,
    title: &'a str,
    /// What was written to disk.
    full_content: &'a str,
    /// What the caller supplied; `full_content` is this with generated
    /// frontmatter in front of it.
    supplied: &'a str,
    parsed: &'a ParsedNote,
    tags: &'a [String],
    now: &'a str,
}

/// Insert the index rows for a brand-new note.
fn index_new_note(db: &Connection, note: NewNote<'_>) -> Result<Note> {
    let NewNote {
        path: relative_path,
        title,
        full_content,
        supplied,
        parsed,
        tags: combined_tags,
        now,
    } = note;
    let id = Uuid::now_v7().to_string();
    let tags_json = serde_json::to_string(&combined_tags)?;
    let content_hash = sha256_hex(full_content);
    let word_count = supplied.split_whitespace().count() as i64;
    let frontmatter_json = if parsed.frontmatter.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&parsed.frontmatter)?)
    };

    db.execute(
        "INSERT INTO notes (id, title, path, content_hash, frontmatter, tags, word_count, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            title,
            relative_path,
            content_hash,
            frontmatter_json,
            tags_json,
            word_count,
            now,
            now
        ],
    )?;

    // Explicit tags are searchable from the start, not only after the next
    // reindex reads them back from the generated frontmatter.
    set_fts_row(
        db,
        &id,
        title,
        &strip_yaml_frontmatter(full_content),
        combined_tags,
    )?;

    // Line numbers must refer to the file as written: when Cerebro prepended
    // frontmatter, the links sit that many lines further down.
    let extra_lines = full_content.lines().count() - supplied.lines().count();
    let file_links: Vec<Wikilink> = parsed
        .wikilinks
        .iter()
        .map(|wl| {
            let mut wl = wl.clone();
            wl.line_number += extra_lines;
            wl
        })
        .collect();
    insert_links(db, &id, &file_links, full_content)?;
    set_note_tags(db, &id, combined_tags)?;

    Ok(Note {
        id,
        title: title.to_string(),
        path: relative_path.to_string(),
        content_hash,
        frontmatter: frontmatter_json,
        tags: combined_tags.to_vec(),
        word_count,
        created_at: now.to_string(),
        updated_at: now.to_string(),
        category: None,
        is_favorite: false,
    })
}

/// Switch the database to WAL (readers never block writers, writers never
/// block readers). SQLite does not run the busy handler for a journal-mode
/// change — a second connection opening the same brand-new database while the
/// first is switching it gets "database is locked" at once — so the switch is
/// skipped when it is already done and retried briefly otherwise.
fn enable_wal(db: &Connection) -> Result<()> {
    const ATTEMPTS: u32 = 100;
    for attempt in 1..=ATTEMPTS {
        let current: String = db
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .context("reading the journal mode")?;
        if current.eq_ignore_ascii_case("wal") {
            return Ok(());
        }
        match db.execute_batch("PRAGMA journal_mode=WAL;") {
            Ok(()) => return Ok(()),
            Err(e) if is_busy(&e) && attempt < ATTEMPTS => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(e).context("setting WAL journal mode"),
        }
    }
    unreachable!("the loop returns on its last attempt")
}

fn is_busy(error: &rusqlite::Error) -> bool {
    matches!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy) | Some(rusqlite::ErrorCode::DatabaseLocked)
    )
}

// ── Note lookup ──────────────────────────────────────────────────

/// Resolve `identifier` to exactly one note: ID, exact title, exact path
/// (with or without `.md`), then a substring match — only when it is
/// UNAMBIGUOUS. Feeding a fuzzy first-row match into update/delete can hit
/// the wrong note (grill finding F8).
fn find_note(db: &Connection, identifier: &str) -> Result<Note> {
    let by = |sql: &str| -> Result<Vec<Note>> {
        let mut stmt = db.prepare_cached(&format!(
            "SELECT {NOTE_COLUMNS} FROM notes WHERE {sql} AND deleted_at IS NULL"
        ))?;
        let notes = stmt
            .query_map([identifier], note_from_row)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(notes)
    };

    // 1. Exact ID
    if let Some(note) = by("id = ?1")?.into_iter().next() {
        return Ok(note);
    }
    // 2. Exact title — only when it is unambiguous
    let titled = by("title = ?1")?;
    if let [note] = &titled[..] {
        return Ok(note.clone());
    }
    // 3. Exact path, with or without the extension
    if let Some(note) = by("path = ?1")?.into_iter().next() {
        return Ok(note);
    }
    if let Some(note) = by("path = ?1 || '.md'")?.into_iter().next() {
        return Ok(note);
    }
    // 4. Substring match — only when it is UNAMBIGUOUS (grill finding F8).
    let pattern = format!("%{}%", escape_like(identifier));
    let matches: Vec<Note> = {
        let mut stmt = db.prepare_cached(&format!(
            "SELECT {NOTE_COLUMNS} FROM notes
             WHERE (title LIKE ?1 ESCAPE '\\' OR path LIKE ?1 ESCAPE '\\')
             AND deleted_at IS NULL LIMIT 50"
        ))?;
        stmt.query_map([pattern], note_from_row)?
            .filter_map(|r| r.ok())
            .collect()
    };

    match matches.len() {
        0 => Err(anyhow!("note not found: {}", identifier)),
        1 => Ok(matches.into_iter().next().unwrap()),
        _ => {
            let mut candidates: Vec<String> = matches
                .iter()
                .map(|m| format!("'{}' ({})", m.title, m.path))
                .collect();
            candidates.truncate(5);
            Err(anyhow!(
                "ambiguous identifier '{}' matches {} notes; be specific (use ID, exact title, or exact path). Candidates: {}",
                identifier,
                matches.len(),
                candidates.join(", ")
            ))
        }
    }
}

/// Resolve a wikilink target to a note ID, Obsidian-style: exact title, then
/// exact path (with or without `.md`), then file name (case-insensitive,
/// shortest path first), then — only when `allow_substring` — a unique
/// substring match. Ambiguity resolves to nothing rather than an arbitrary
/// note (grill finding F8).
fn resolve_target(db: &Connection, target: &str, allow_substring: bool) -> Option<String> {
    // `LIMIT 2` is enough to tell "one" from "several".
    let unique_id = |sql: &str, value: &str| -> Option<String> {
        let mut stmt = db.prepare_cached(sql).ok()?;
        let mut rows = stmt.query([value]).ok()?;
        let first = rows.next().ok()??.get::<_, String>(0).ok()?;
        let exactly_one = rows.next().ok()?.is_none();
        exactly_one.then_some(first)
    };

    // 1. Exact title — unless several notes share it.
    if let Some(id) = unique_id(
        "SELECT id FROM notes WHERE title = ?1 AND deleted_at IS NULL LIMIT 2",
        target,
    ) {
        return Some(id);
    }
    // 2. Exact path, as written or without the extension.
    if let Some(id) = unique_id(
        "SELECT id FROM notes WHERE (path = ?1 OR path = ?1 || '.md') AND deleted_at IS NULL LIMIT 2",
        target,
    ) {
        return Some(id);
    }
    // 3. File name, case-insensitive; the shortest path wins.
    let by_file_name = db
        .prepare_cached(
            "SELECT id FROM notes WHERE file_stem = ?1 COLLATE NOCASE AND deleted_at IS NULL
             ORDER BY length(path), path LIMIT 1",
        )
        .ok()?
        .query_row([target], |row| row.get::<_, String>(0))
        .ok();
    if by_file_name.is_some() {
        return by_file_name;
    }
    // 4. Unique substring match (opt-in).
    if !allow_substring {
        return None;
    }
    let pattern = format!("%{}%", escape_like(target));
    unique_id(
        "SELECT id FROM notes
         WHERE (title LIKE ?1 ESCAPE '\\' OR path LIKE ?1 ESCAPE '\\')
         AND deleted_at IS NULL LIMIT 2",
        &pattern,
    )
}

/// Resolve links whose target is still open. With `only_refs`, just the links
/// pointing at those names (a note was just created or renamed); with `None`,
/// every open link (end of a reindex). Indexed lookups throughout, so a
/// single note costs the same in a vault of 50 as in one of 50,000.
fn resolve_pending_links(db: &Connection, only_refs: Option<&[String]>) -> Result<()> {
    let open_refs: Vec<String> = match only_refs {
        None => {
            let mut stmt = db.prepare_cached("SELECT DISTINCT target_ref FROM links WHERE target_id IS NULL")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.filter_map(|r| r.ok()).collect()
        }
        // One indexed seek per name. A single `IN (…)` query looks tempting,
        // but for two or more names SQLite prefers the index on `target_id`
        // and walks every unresolved link in the vault.
        Some(refs) => {
            let mut has_open = db.prepare_cached(
                "SELECT 1 FROM links WHERE target_id IS NULL AND target_ref = ?1 COLLATE NOCASE LIMIT 1",
            )?;
            let mut open = Vec::new();
            for reference in refs {
                let found: Option<i64> = has_open.query_row([reference], |row| row.get(0)).optional()?;
                if found.is_some() {
                    open.push(reference.clone());
                }
            }
            open
        }
    };

    let mut update = db.prepare_cached(
        "UPDATE links SET target_id = ?1 WHERE target_id IS NULL AND target_ref = ?2 COLLATE NOCASE",
    )?;
    let mut cache: HashMap<String, Option<String>> = HashMap::new();
    for reference in open_refs {
        let target = cache
            .entry(reference.clone())
            .or_insert_with(|| resolve_target(db, &reference, true))
            .clone();
        if let Some(id) = target {
            update.execute(params![id, reference])?;
        }
    }
    Ok(())
}

/// The names a note can be linked by: its title, its path with and without
/// the extension, and its file name.
fn link_keys(title: &str, path: &str) -> Vec<String> {
    let mut keys = vec![
        title.to_string(),
        path.to_string(),
        path.strip_suffix(".md").unwrap_or(path).to_string(),
        file_stem_of(path),
    ];
    keys.sort();
    keys.dedup();
    keys
}

/// A soft-deleted row (written by an earlier version) that still owns `path`.
fn find_tombstone(db: &Connection, path: &str) -> Result<Option<String>> {
    Ok(db
        .query_row(
            "SELECT id FROM notes WHERE path = ?1 AND deleted_at IS NOT NULL",
            params![path],
            |row| row.get::<_, String>(0),
        )
        .optional()?)
}

/// Is there already a note — indexed, or just a file on disk — at this path?
/// A soft-deleted row left by an earlier version does not count: the path is
/// freed again as soon as a note is created there.
fn path_taken(db: &Connection, notes_dir: &Path, relative: &str) -> bool {
    let indexed = db
        .query_row(
            "SELECT 1 FROM notes WHERE path = ?1 AND deleted_at IS NULL",
            params![relative],
            |_| Ok(()),
        )
        .is_ok();
    indexed || notes_dir.join(relative).exists()
}

// ── Index rows ───────────────────────────────────────────────────

/// Index one file (inside an open transaction). Unchanged files are left
/// untouched, so their timestamps survive a reindex.
fn upsert_file(db: &Connection, relative: &str, content: &str, now: &str) -> Result<Outcome> {
    let parsed = extract_links_and_metadata(content);
    // A note is named by its `title:` frontmatter, or by its file name —
    // the Obsidian convention the vault import promises.
    let title = parsed
        .explicit_title
        .clone()
        .unwrap_or_else(|| file_stem_of(relative));
    let content_hash = sha256_hex(content);
    let word_count = content.split_whitespace().count() as i64;

    let existing: Option<(String, String)> = db
        .query_row(
            "SELECT id, content_hash FROM notes WHERE path = ?1 AND deleted_at IS NULL",
            params![relative],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;

    let Some((id, old_hash)) = existing else {
        // A new file. A soft-deleted row from an earlier version may still
        // own this path (the note was deleted and later restored).
        if let Some(tombstone) = find_tombstone(db, relative)? {
            purge_note(db, &tombstone)?;
        }
        let id = Uuid::now_v7().to_string();
        let tags_json = serde_json::to_string(&parsed.tags)?;
        db.execute(
            "INSERT INTO notes (id, title, path, content_hash, tags, word_count, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, title, relative, content_hash, tags_json, word_count, now, now],
        )?;
        set_fts_row(db, &id, &title, &strip_yaml_frontmatter(content), &parsed.tags)?;
        insert_links(db, &id, &parsed.wikilinks, content)?;
        set_note_tags(db, &id, &parsed.tags)?;
        return Ok(Outcome::Created);
    };

    if old_hash == content_hash {
        return Ok(Outcome::Unchanged);
    }
    let tags_json = serde_json::to_string(&parsed.tags)?;
    db.execute(
        "UPDATE notes SET title = ?1, content_hash = ?2, tags = ?3, word_count = ?4, updated_at = ?5 WHERE id = ?6",
        params![title, content_hash, tags_json, word_count, now, id],
    )?;
    set_fts_row(db, &id, &title, &strip_yaml_frontmatter(content), &parsed.tags)?;

    db.execute("DELETE FROM links WHERE source_id = ?1", params![id])?;
    insert_links(db, &id, &parsed.wikilinks, content)?;
    db.execute("DELETE FROM note_tags WHERE note_id = ?1", params![id])?;
    set_note_tags(db, &id, &parsed.tags)?;
    Ok(Outcome::Updated)
}

/// Remove every index row of a note: search row, links in both directions,
/// tag associations, and the note row itself.
fn purge_note(db: &Connection, id: &str) -> Result<()> {
    db.execute(
        "DELETE FROM notes_fts WHERE rowid IN (SELECT rowid FROM notes WHERE id = ?1)",
        params![id],
    )?;
    db.execute("DELETE FROM links WHERE source_id = ?1 OR target_id = ?1", params![id])?;
    db.execute("DELETE FROM note_tags WHERE note_id = ?1", params![id])?;
    db.execute("DELETE FROM notes WHERE id = ?1", params![id])?;
    Ok(())
}

/// Replace the note's search row. A row with the same rowid may be left over
/// from a deleted note (rowids are reused), so it is cleared first.
fn set_fts_row(db: &Connection, id: &str, title: &str, body: &str, tags: &[String]) -> Result<()> {
    let rowid = note_rowid(db, id)?;
    db.execute("DELETE FROM notes_fts WHERE rowid = ?1", params![rowid])?;
    db.execute(
        "INSERT INTO notes_fts (rowid, title, content, tags) VALUES (?1, ?2, ?3, ?4)",
        params![rowid, title, body, tags.join(" ")],
    )?;
    Ok(())
}

fn note_rowid(db: &Connection, id: &str) -> Result<i64> {
    db.query_row("SELECT rowid FROM notes WHERE id = ?1", params![id], |row| row.get(0))
        .map_err(|_| anyhow!("note not found for rowid lookup: {}", id))
}

fn insert_links(db: &Connection, source_id: &str, wikilinks: &[Wikilink], content: &str) -> Result<()> {
    // The links table enforces UNIQUE(source_id, target_ref, line_number):
    // referencing the same target twice on one line must produce one edge,
    // not a constraint failure (grill finding F2).
    let mut seen: HashSet<(String, usize)> = HashSet::new();
    for wl in wikilinks {
        if !seen.insert((wl.target.clone(), wl.line_number)) {
            continue;
        }
        // Cheap (indexed) resolution only; anything still open is resolved
        // in bulk by `resolve_pending_links`.
        let target_id = resolve_target(db, &wl.target, false);
        let context = extract_link_context(content, wl.line_number, &wl.target);

        db.execute(
            "INSERT INTO links (source_id, target_id, target_ref, context, line_number)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![source_id, target_id, wl.target, context, wl.line_number as i64],
        )?;
    }
    Ok(())
}

fn set_note_tags(db: &Connection, note_id: &str, tags: &[String]) -> Result<()> {
    for tag in tags {
        // Ensure tag exists first (FK constraint)
        db.execute(
            "INSERT INTO tags (name, note_count) VALUES (?1, 1)
             ON CONFLICT(name) DO UPDATE SET note_count = note_count + 1",
            params![tag],
        )?;
        db.execute(
            "INSERT INTO note_tags (note_id, tag) VALUES (?1, ?2)",
            params![note_id, tag],
        )?;
    }
    Ok(())
}

/// Recompute `note_count` for the given tags from `note_tags`, counting
/// only non-deleted notes, and drop those of them that no longer have any
/// notes.
///
/// Counters must be derived, never incremented blindly: update/delete
/// paths previously left counts inflated forever (grill finding F5). Only
/// the affected tags are touched — this runs on every write.
fn refresh_tag_counts(db: &Connection, tags: &[String]) -> Result<()> {
    if tags.is_empty() {
        return Ok(());
    }
    for tag in tags {
        db.execute(
            "UPDATE tags SET note_count = (
                SELECT COUNT(*) FROM note_tags nt
                JOIN notes n ON n.id = nt.note_id
                WHERE nt.tag = ?1 AND n.deleted_at IS NULL
            ) WHERE name = ?1",
            params![tag],
        )?;
    }
    // Only these tags: a tag kept alive by rows of soft-deleted notes
    // (legacy tombstones, cleaned up by migration 004) would make the
    // delete violate the foreign key.
    let placeholders = std::iter::repeat_n("?", tags.len()).collect::<Vec<_>>().join(", ");
    let sql = format!("DELETE FROM tags WHERE note_count = 0 AND name IN ({placeholders})");
    db.execute(&sql, rusqlite::params_from_iter(tags.iter()))?;
    Ok(())
}

// ── Files ────────────────────────────────────────────────────────

/// Create `path` with `contents`; fails if anything is already there, and
/// never leaves a half-written file behind.
fn write_new_file(path: &Path, contents: &str) -> Result<()> {
    let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            bail!("a file already exists at {}", path.display())
        }
        Err(e) => return Err(e.into()),
    };
    if let Err(e) = file.write_all(contents.as_bytes()) {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(e.into());
    }
    Ok(())
}

/// Replace `path` with `contents` in one step, so a crash can never leave a
/// truncated note behind.
fn write_file_atomically(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(contents)?;
    tmp.persist(path)
        .map_err(|e| anyhow!("replacing {}: {e}", path.display()))?;
    Ok(())
}

/// Every note file under `dir`, in a stable order. `.trash/`, `.git/`,
/// `.obsidian/` and dotfiles are not notes.
fn walk_md_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !dir.exists() {
        return Ok(files);
    }
    for entry in walkdir::WalkDir::new(dir).sort_by_file_name() {
        let entry = entry?;
        let path = entry.path();
        // Only the part below `dir` counts: the vault itself may well live
        // under a hidden directory such as ~/.cerebro.
        let inside = path.strip_prefix(dir).unwrap_or(path);
        let hidden = inside
            .components()
            .any(|c| c.as_os_str().to_str().is_some_and(|s| s.starts_with('.') && s != "."));
        if hidden {
            continue;
        }
        let is_md = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("md"));
        if entry.file_type().is_file() && is_md {
            files.push(path.to_path_buf());
        }
    }
    Ok(files)
}

// ── Path helpers ─────────────────────────────────────────────────

fn sanitize_stem(title: &str) -> String {
    let safe: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    if safe.is_empty() {
        "_".to_string()
    } else {
        safe
    }
}

fn join_vault_path(dir: &str, file_name: &str) -> String {
    if dir.is_empty() {
        file_name.to_string()
    } else {
        format!("{dir}/{file_name}")
    }
}

/// File name without directory and without `.md`: what `[[…]]` refers to.
fn file_stem_of(relative: &str) -> String {
    let name = relative.rsplit('/').next().unwrap_or(relative);
    match name.len().checked_sub(3) {
        Some(cut) if name[cut..].eq_ignore_ascii_case(".md") => name[..cut].to_string(),
        _ => name.to_string(),
    }
}

/// Validate a user-supplied note path and return it in normalized form.
///
/// The path must stay inside the vault's notes directory (no `..` traversal,
/// no absolute paths). It may name a file (`notes/a.md`) or a directory-like
/// location (`notes/`, `notes`); the caller decides what to do with each.
fn validate_note_path(raw: &str) -> Result<String> {
    if raw.is_empty() {
        bail!("note path must not be empty");
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        bail!("note path must be relative to the vault notes directory: {}", raw);
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
                    bail!("note path escapes the vault notes directory: {}", raw);
                }
            }
            Component::Normal(_) => depth += 1,
            _ => bail!("invalid path component in: {}", raw),
        }
    }

    Ok(path
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/"))
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
            .is_some_and(|v| v != 0),
    })
}

fn select_links(db: &Connection, sql: &str, params: impl rusqlite::Params) -> Result<Vec<Link>> {
    let mut stmt = db.prepare_cached(sql)?;
    let links = stmt
        .query_map(params, |row| {
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

/// Keep `score`/`other` in `heap` when it is among the best `max_per_note`.
fn offer(
    heap: &mut BinaryHeap<Reverse<(OrderedScore, usize)>>,
    score: f64,
    other: usize,
    max_per_note: usize,
) {
    if heap.len() < max_per_note {
        heap.push(Reverse((OrderedScore(score), other)));
    } else if let Some(Reverse((lowest, _))) = heap.peek()
        && OrderedScore(score) > *lowest
    {
        heap.pop();
        heap.push(Reverse((OrderedScore(score), other)));
    }
}

/// `%`, `_` and the escape character in `text` match literally in a `LIKE`.
fn escape_like(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
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

/// The text to put in the search index: everything but the frontmatter.
fn strip_yaml_frontmatter(content: &str) -> String {
    split_frontmatter(content).body
}

/// The line a link sits on, trimmed — cut to a window around the link when
/// the line is long. Line numbers are 1-indexed from the top of the note.
fn extract_link_context(content: &str, line_number: usize, target: &str) -> Option<String> {
    // Return THAT line's text, not the first line that happens to contain a
    // link (grill finding F6).
    if line_number == 0 {
        return None;
    }
    let line = content.lines().nth(line_number - 1)?.trim();
    let length = line.chars().count();
    if length <= MAX_LINK_CONTEXT_CHARS {
        return Some(line.to_string());
    }

    // Centre a bounded window on the link itself.
    let link_start = line
        .find(&format!("[[{target}"))
        .map_or(0, |at| line[..at].chars().count());
    let window_start = link_start.saturating_sub(60);
    let window_end = (window_start + MAX_LINK_CONTEXT_CHARS).min(length);

    let mut excerpt = String::new();
    if window_start > 0 {
        excerpt.push('…');
    }
    excerpt.push_str(&line[byte_at_char(line, window_start)..byte_at_char(line, window_end)]);
    if window_end < length {
        excerpt.push('…');
    }
    Some(excerpt)
}

/// Byte offset of the `n`-th character (the length of `text` if there are fewer).
fn byte_at_char(text: &str, n: usize) -> usize {
    text.char_indices().nth(n).map_or(text.len(), |(byte, _)| byte)
}

// ── Search query helpers ────────────────────────────────────────

/// The query as FTS5 should see it.
///
/// FTS5 rejects a hyphenated word with "no such column": `e-mail`,
/// `state-of-the-art`, `2026-11-23` never matched anything. The indexer splits
/// such words into adjacent tokens, so each one becomes a phrase —
/// `state-of-the-art` → `"state of the art"` (a trailing `*` is kept). Text in
/// double quotes is syntax the user wrote and is left alone.
fn fts5_query(user_query: &str) -> String {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let chars: Vec<char> = user_query.chars().collect();
    let mut out = String::with_capacity(user_query.len() + 8);
    let mut index = 0;
    let mut in_quotes = false;

    while index < chars.len() {
        let c = chars[index];
        if c == '"' {
            in_quotes = !in_quotes;
            out.push('"');
            index += 1;
            continue;
        }
        if in_quotes || !is_word(c) {
            out.push(c);
            index += 1;
            continue;
        }

        // A word: word characters and hyphens that sit between two of them.
        let start = index;
        let mut end = index;
        let mut hyphenated = false;
        while end < chars.len()
            && (is_word(chars[end])
                || (chars[end] == '-' && end + 1 < chars.len() && is_word(chars[end + 1])))
        {
            hyphenated |= chars[end] == '-';
            end += 1;
        }
        let prefix = end < chars.len() && chars[end] == '*';

        if hyphenated {
            out.push('"');
            for &c in &chars[start..end] {
                out.push(if c == '-' { ' ' } else { c });
            }
            if prefix {
                out.push('*');
            }
            out.push('"');
        } else {
            out.extend(&chars[start..end]);
        }
        index = if hyphenated && prefix { end + 1 } else { end };
    }
    out
}

/// The query as a search for its literal words, for when FTS5 rejects the
/// syntax (a stray bracket, an unbalanced quote — the kind of thing typed
/// halfway through). Stray quotes are dropped. With `keep_operators`, the
/// words `AND` / `OR` / `NOT` stay operators where they sit between two
/// words and are dropped elsewhere (a dangling `AND` is a half-typed query,
/// not a search for the word "and"); without, every word is searched for.
fn literal_query(query: &str, keep_operators: bool) -> String {
    let is_operator = |word: &str| matches!(word, "AND" | "OR" | "NOT");
    let words: Vec<String> = query
        .split_whitespace()
        .map(|word| word.replace('"', ""))
        .filter(|word| !word.is_empty())
        .collect();

    let mut out: Vec<String> = Vec::new();
    for (index, word) in words.iter().enumerate() {
        let operator = is_operator(word);
        if operator && !keep_operators {
            out.push(format!("\"{word}\""));
            continue;
        }
        if operator {
            // Keep only between two operands.
            let between = index > 0
                && !is_operator(&words[index - 1])
                && words[index + 1..].iter().any(|next| !is_operator(next));
            if between {
                out.push(word.clone());
            }
            continue;
        }
        out.push(format!("\"{word}\""));
    }
    out.join(" ")
}

// ── Auto-link math ───────────────────────────────────────────────

/// `(shared, combined)` of two sorted, de-duplicated sets.
fn overlap(a: &[String], b: &[String]) -> (usize, usize) {
    let (mut i, mut j, mut shared) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                shared += 1;
                i += 1;
                j += 1;
            }
        }
    }
    (shared, a.len() + b.len() - shared)
}

fn ratio(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

/// A confidence that can go into a `BinaryHeap` (f64 is not `Ord`).
#[derive(Debug, Clone, Copy)]
struct OrderedScore(f64);

impl PartialEq for OrderedScore {
    fn eq(&self, other: &Self) -> bool {
        self.0.total_cmp(&other.0).is_eq()
    }
}

impl Eq for OrderedScore {}

impl PartialOrd for OrderedScore {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedScore {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault_for_test(tag: &str) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::Builder::new().prefix(&format!("cerebro-{tag}-")).tempdir().unwrap();
        let dir = tmp.path().join("profile");
        std::fs::create_dir_all(&dir).unwrap();
        (tmp, dir)
    }

    #[test]
    fn file_stem_matches_the_sql_generated_column() {
        for (path, stem) in [
            ("c.md", "c"),
            ("projects/plan.md", "plan"),
            ("deep/nest/v1.2.final.md", "v1.2.final"),
            ("Zażółć gęślą.md", "Zażółć gęślą"),
            ("Dir/PLAN.MD", "PLAN"),
            ("a/b/notes", "notes"),
        ] {
            assert_eq!(file_stem_of(path), stem, "{path}");
        }
    }

    #[test]
    fn like_wildcards_in_identifiers_are_literal() {
        assert_eq!(escape_like("100%_done"), r"100\%\_done");
        assert_eq!(escape_like("plain"), "plain");
    }

    #[test]
    fn vault_paths_are_joined_like_the_index_stores_them() {
        assert_eq!(join_vault_path("", "a.md"), "a.md");
        assert_eq!(join_vault_path("inbox", "a.md"), "inbox/a.md");
    }

    #[test]
    fn overlap_counts_shared_and_combined_elements() {
        let set = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let a = set(&["a", "b", "c"]);
        let b = set(&["b", "c", "d", "e"]);
        assert_eq!(overlap(&a, &b), (2, 5));
        assert_eq!(overlap(&a, &a), (3, 3));
        assert_eq!(overlap(&a, &[]), (0, 3));
        assert_eq!(ratio(0, 0), 0.0);
        assert_eq!(ratio(2, 5), 0.4);
    }

    #[test]
    fn link_keys_cover_every_way_of_naming_a_note() {
        let keys = link_keys("Plan", "projects/plan.md");
        assert_eq!(keys, vec!["Plan", "plan", "projects/plan", "projects/plan.md"]);
        assert_eq!(link_keys("plan", "plan.md"), vec!["plan", "plan.md"]);
    }

    #[test]
    fn link_context_is_the_link_line_and_stays_short() {
        let short = "Intro\nSee [[Target]] here.\n";
        assert_eq!(extract_link_context(short, 2, "Target").as_deref(), Some("See [[Target]] here."));
        assert_eq!(extract_link_context(short, 0, "Target"), None);
        assert_eq!(extract_link_context(short, 99, "Target"), None);

        // One paragraph on one long line: a window around the link, never the paragraph.
        let long = format!("{} then [[Target]] {}", "Zażółć gęślą ".repeat(40), "end ".repeat(400));
        let context = extract_link_context(&long, 1, "Target").unwrap();
        assert!(context.chars().count() <= MAX_LINK_CONTEXT_CHARS + 2, "{} chars", context.chars().count());
        assert!(context.contains("[[Target]]"), "{context}");
        assert!(context.starts_with('…') && context.ends_with('…'), "{context}");

        // The link near the start: no leading ellipsis.
        let head = format!("[[Target]] {}", "word ".repeat(400));
        let context = extract_link_context(&head, 1, "Target").unwrap();
        assert!(context.starts_with("[[Target]]") && context.ends_with('…'));
    }

    #[test]
    fn hyphenated_words_become_phrases_and_syntax_is_kept() {
        assert_eq!(fts5_query("state-of-the-art"), "\"state of the art\"");
        assert_eq!(fts5_query("e-mail AND 2026-11-23"), "\"e mail\" AND \"2026 11 23\"");
        assert_eq!(fts5_query("rust-lang*"), "\"rust lang*\"");
        assert_eq!(fts5_query("vault-which"), "\"vault which\"");
        // A column filter keeps working: tags:rust-lang → tags:"rust lang"
        assert_eq!(fts5_query("tags:rust-lang"), "tags:\"rust lang\"");
        // Not a hyphenated word
        assert_eq!(fts5_query("minus -5 and a - b"), "minus -5 and a - b");
        assert_eq!(fts5_query("trailing-"), "trailing-");
        // Quoted text and operators are the user's syntax
        assert_eq!(fts5_query("\"e-mail\" OR x-ray"), "\"e-mail\" OR \"x ray\"");
        assert_eq!(fts5_query("kuber*"), "kuber*");
        assert_eq!(literal_query("foo (bar AND \"baz", true), "\"foo\" \"(bar\" AND \"baz\"");
        // A dangling operator is dropped, not searched for
        assert_eq!(literal_query("state AND", true), "\"state\"");
        assert_eq!(literal_query("AND state OR", true), "\"state\"");
        assert_eq!(literal_query("a AND AND b", true), "\"a\" AND \"b\"");
        assert_eq!(literal_query("state AND", false), "\"state\" \"AND\"");
        assert_eq!(literal_query("\"", true), "");
    }

    #[test]
    fn single_note_writes_never_scan_the_index() {
        // Creating a note in a 10,000-note vault took ~15 ms while these
        // statements were full-table scans.
        let (_tmp, dir) = vault_for_test("plans");
        let storage = Storage::open(&dir, &ProfileConfig::new("plan")).unwrap();
        for sql in [
            "SELECT id FROM notes WHERE id = 'x' AND deleted_at IS NULL",
            "SELECT id FROM notes WHERE title = 'x' AND deleted_at IS NULL",
            "SELECT id FROM notes WHERE (path = 'x' OR path = 'x' || '.md') AND deleted_at IS NULL",
            "SELECT id FROM notes WHERE file_stem = 'x' COLLATE NOCASE AND deleted_at IS NULL",
            "SELECT 1 FROM links WHERE target_id IS NULL AND target_ref = 'x' COLLATE NOCASE LIMIT 1",
            "UPDATE links SET target_id = 'y' WHERE target_id IS NULL AND target_ref = 'x' COLLATE NOCASE",
            "UPDATE links SET target_id = NULL WHERE target_id = 'x'",
            "SELECT name FROM tags WHERE name = 'x'",
            "SELECT id FROM notes WHERE path = 'x'",
        ] {
            let mut stmt = storage.db.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let plan: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(3))
                .unwrap()
                .map(|row| row.unwrap())
                .collect();
            let detail = plan.join(" | ");
            assert!(detail.contains("USING") && !detail.contains("SCAN"), "{sql}\n  -> {detail}");
        }
    }

    #[test]
    fn paths_cannot_leave_the_vault() {
        assert!(validate_note_path("../../escape").is_err());
        assert!(validate_note_path("a/../../b.md").is_err());
        assert!(validate_note_path("/etc/passwd.md").is_err());
        assert!(validate_note_path("").is_err());
        assert_eq!(validate_note_path("./sub/../inner.md").unwrap(), "sub/../inner.md");
        assert_eq!(validate_note_path("Inbox/").unwrap(), "Inbox");
    }
}
