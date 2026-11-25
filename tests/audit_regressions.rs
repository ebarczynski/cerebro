//! Regression tests for the 2026-11-23 pre-release audit.
//!
//! Every test here corresponds to a defect that was reproduced against the
//! `master` binary before the fix. They use only the public library API.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cerebro::config::profile::ProfileConfig;
use cerebro::import::{import_file, ImportFormat};
use cerebro::storage::Storage;

// ── helpers ──────────────────────────────────────────────────────

/// Scratch profile directory (`<tmp>/profile`) plus a `TempDir` that must
/// outlive every `Storage` opened on it.
fn vault(tag: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::Builder::new()
        .prefix(&format!("cerebro-audit-{tag}-"))
        .tempdir()
        .expect("tempdir");
    let dir = tmp.path().join("profile");
    fs::create_dir_all(&dir).expect("mkdir");
    (tmp, dir)
}

fn open(profile_dir: &Path) -> Storage {
    let cfg = ProfileConfig::new("audit");
    cfg.save_to(profile_dir).expect("vault.toml");
    Storage::open(profile_dir, &cfg).expect("open storage")
}

fn notes_dir(profile_dir: &Path) -> PathBuf {
    profile_dir.join("notes")
}

fn file_body(profile_dir: &Path, rel_path: &str) -> String {
    fs::read_to_string(notes_dir(profile_dir).join(rel_path)).expect("note file")
}

/// `(resolved, total)` wikilinks, as the MCP graph reports them.
fn resolved_links(storage: &Storage) -> (usize, usize) {
    let (_, edges) = storage.get_graph().expect("graph");
    (edges.iter().filter(|l| l.target_id.is_some()).count(), edges.len())
}

fn query(profile_dir: &Path, sql: &str) -> Vec<Vec<String>> {
    let con = rusqlite::Connection::open(profile_dir.join("db.sqlite")).expect("db");
    let mut stmt = con.prepare(sql).expect("prepare");
    let ncols = stmt.column_count();
    stmt.query_map([], |row| {
        Ok((0..ncols)
            .map(|i| match row.get_ref(i) {
                Ok(rusqlite::types::ValueRef::Integer(v)) => v.to_string(),
                Ok(rusqlite::types::ValueRef::Real(v)) => v.to_string(),
                Ok(rusqlite::types::ValueRef::Text(v)) | Ok(rusqlite::types::ValueRef::Blob(v)) => {
                    String::from_utf8_lossy(v).into_owned()
                }
                Ok(rusqlite::types::ValueRef::Null) | Err(_) => String::new(),
            })
            .collect::<Vec<_>>())
    })
    .expect("rows")
    .map(|r| r.expect("row"))
    .collect()
}

fn one(profile_dir: &Path, sql: &str) -> String {
    query(profile_dir, sql)
        .first()
        .and_then(|row| row.first().cloned())
        .unwrap_or_default()
}

// ── create_note must never destroy an existing note ──────────────

#[test]
fn create_note_never_overwrites_an_existing_note() {
    // Repro: `notes create "Roadmap 2026"` then `notes create "Roadmap_2026"`.
    // Both sanitize to Roadmap_2026.md; the second write replaced the first
    // note's file BEFORE the database rejected the duplicate path.
    let (_tmp, dir) = vault("no-overwrite");
    let mut s = open(&dir);

    let first = s.create_note("Roadmap 2026", "ORIGINAL unique sentinel 7f3a", None, None).unwrap();
    let second = s
        .create_note("Roadmap_2026", "SECOND note sentinel 9c1d", None, None)
        .expect("a title that sanitizes to an existing filename must not fail");

    assert_ne!(first.path, second.path, "the second note needs its own file");
    assert!(file_body(&dir, &first.path).contains("7f3a"), "first note must be intact");
    assert!(file_body(&dir, &second.path).contains("9c1d"), "second note must exist");
    assert!(s.search("7f3a", 10).unwrap().iter().any(|r| r.path == first.path));
    assert!(s.search("9c1d", 10).unwrap().iter().any(|r| r.path == second.path));
}

#[test]
fn create_note_untitled_twice_keeps_both_notes() {
    // The GUI's "New note" button always uses the title "Untitled Note".
    let (_tmp, dir) = vault("untitled");
    let mut s = open(&dir);
    let a = s.create_note("Untitled Note", "first draft", None, None).unwrap();
    s.update_note(&a.id, "first draft, now with real content").unwrap();
    let b = s.create_note("Untitled Note", "# Untitled Note\n\n", None, None).unwrap();
    assert_ne!(a.path, b.path);
    let a_now = s.read_note(&a.id).unwrap();
    assert!(a_now.body.contains("now with real content"), "first note was clobbered");
}

#[test]
fn create_note_two_captures_in_the_same_minute_keep_both() {
    // `cerebro capture` derives the path from the minute; two captures in one
    // minute used to overwrite each other.
    let (_tmp, dir) = vault("capture");
    let mut s = open(&dir);
    let a = s.create_note("Capture - 2026-11-23 12:24", "first capture", Some("inbox/"), None).unwrap();
    let b = s.create_note("Capture - 2026-11-23 12:24", "second capture", Some("inbox/"), None).unwrap();
    assert_ne!(a.path, b.path);
    assert_eq!(fs::read_dir(notes_dir(&dir).join("inbox")).unwrap().count(), 2);
}

#[test]
fn create_note_with_an_explicit_existing_path_fails_and_changes_nothing() {
    let (_tmp, dir) = vault("explicit-path");
    let mut s = open(&dir);
    let a = s.create_note("Existing", "the original body", None, None).unwrap();
    let before_file = file_body(&dir, &a.path);
    let before_rows = one(&dir, "SELECT count(*) FROM notes;");

    let err = s.create_note("Existing", "OVERWRITTEN body", Some(&a.path), None).unwrap_err().to_string();
    assert!(err.contains("already exists"), "unexpected error: {err}");

    assert_eq!(file_body(&dir, &a.path), before_file, "file must not be touched");
    assert_eq!(one(&dir, "SELECT count(*) FROM notes;"), before_rows);
    assert!(s.search("overwritten", 10).unwrap().is_empty(), "no partial FTS row");
}

// ── delete / recreate / restore ──────────────────────────────────

#[test]
fn delete_then_recreate_the_same_title_succeeds() {
    // Repro: `notes delete "Meeting Notes"` then `notes create "Meeting Notes"`
    // → "UNIQUE constraint failed: notes.path", and every later `reindex` failed.
    let (_tmp, dir) = vault("recreate");
    let mut s = open(&dir);
    let first = s.create_note("Meeting Notes", "v1 body", None, None).unwrap();
    s.delete_note(&first.id).unwrap();
    assert!(!notes_dir(&dir).join(&first.path).exists(), "file removed by delete");

    let second = s.create_note("Meeting Notes", "v2 body", None, None).expect("recreate after delete");
    assert_eq!(second.path, first.path);
    assert!(s.search("v2", 10).unwrap().iter().any(|r| r.path == second.path));
    let stats = s.reindex_all().expect("reindex must recover after delete + recreate");
    assert_eq!(stats.errors, 0);
}

#[test]
fn reindex_recovers_when_a_deleted_notes_file_comes_back() {
    // e.g. restored from Trash or `git pull` after a delete.
    let (_tmp, dir) = vault("file-returns");
    let mut s = open(&dir);
    let note = s.create_note("Restored Note", "back from the dead #sometag", None, None).unwrap();
    s.delete_note(&note.id).unwrap();
    fs::write(notes_dir(&dir).join(&note.path), "---\ntitle: Restored Note\n---\nback #sometag\n").unwrap();

    s.reindex_all().expect("reindex must accept a file that reappeared at a deleted path");
    let found = s.search("back", 10).unwrap();
    assert!(found.iter().any(|r| r.path == note.path));
    assert_eq!(one(&dir, "SELECT count(*) FROM notes WHERE deleted_at IS NULL AND path = 'Restored_Note.md';"), "1");
}

#[test]
fn a_legacy_tombstone_row_does_not_block_its_path() {
    // Databases written by earlier versions keep soft-deleted rows.
    let (_tmp, dir) = vault("legacy-tombstone");
    let mut s = open(&dir);
    let note = s.create_note("Legacy", "old body", None, None).unwrap();
    {
        let con = rusqlite::Connection::open(dir.join("db.sqlite")).unwrap();
        con.execute("UPDATE notes SET deleted_at = '2020-01-01T00:00:00+00:00' WHERE id = ?1", [&note.id])
            .unwrap();
    }
    fs::remove_file(notes_dir(&dir).join(&note.path)).unwrap();

    let again = s.create_note("Legacy", "new body", None, None).expect("path owned by a tombstone must be reusable");
    assert_eq!(again.path, note.path);
    s.reindex_all().expect("reindex must not trip over the tombstone");
    assert_eq!(one(&dir, "SELECT count(*) FROM notes WHERE deleted_at IS NULL AND path = 'Legacy.md';"), "1");
}

// ── reindex ──────────────────────────────────────────────────────

#[test]
fn reindex_after_a_file_is_removed_outside_cerebro_succeeds() {
    // Repro: delete one .md file in Finder, then `cerebro reindex`
    // → "FOREIGN KEY constraint failed", forever (and the GUI could not open
    // the vault any more, because open_vault reindexes).
    let (_tmp, dir) = vault("file-removed");
    let mut s = open(&dir);
    let keep = s.create_note("Keep Me", "stays #shared-tag", None, None).unwrap();
    let gone = s.create_note("Orphan Note", "goes away #only-mine-tag", None, None).unwrap();
    fs::remove_file(notes_dir(&dir).join(&gone.path)).unwrap();

    let stats = s.reindex_all().expect("reindex after an external delete");
    assert_eq!(stats.removed, 1);
    s.reindex_all().expect("and it must keep working on the next run");

    assert!(s.search("stays", 10).unwrap().iter().any(|r| r.path == keep.path));
    assert!(s.search("goes away", 10).unwrap().is_empty());
    assert_eq!(one(&dir, "SELECT group_concat(name) FROM tags;"), "shared-tag");
    assert_eq!(one(
        &dir,
        "SELECT count(*) FROM note_tags t JOIN notes n ON n.id = t.note_id WHERE n.deleted_at IS NOT NULL;"
    ), "0", "no note_tags rows may dangle off a removed note");
    assert_eq!(one(
        &dir,
        "SELECT count(*) FROM notes_fts WHERE rowid NOT IN (SELECT rowid FROM notes WHERE deleted_at IS NULL);"
    ), "0", "no FTS rows may outlive their note");
}

#[test]
fn reindex_leaves_unchanged_notes_alone() {
    // Repro: a reindex rewrote `updated_at` on every untouched file and
    // reported all of them as "Updated".
    let (_tmp, dir) = vault("unchanged");
    let mut s = open(&dir);
    s.create_note("Stable One", "unchanged body", None, None).unwrap();
    s.create_note("Stable Two", "unchanged body", None, None).unwrap();
    let before = query(&dir, "SELECT title || '|' || created_at || '|' || updated_at FROM notes ORDER BY title;");
    std::thread::sleep(Duration::from_millis(30));

    let stats = s.reindex_all().unwrap();
    assert_eq!((stats.created, stats.updated, stats.removed), (0, 0, 0));
    assert_eq!(
        query(&dir, "SELECT title || '|' || created_at || '|' || updated_at FROM notes ORDER BY title;"),
        before,
        "timestamps of unchanged notes must not move"
    );

    // A real change is still picked up.
    fs::write(notes_dir(&dir).join("Stable_Two.md"), "---\ntitle: Stable Two\n---\nchanged body\n").unwrap();
    let stats = s.reindex_all().unwrap();
    assert_eq!(stats.updated, 1);
    assert!(s.search("changed", 10).unwrap().iter().any(|r| r.path == "Stable_Two.md"));
}

#[test]
fn reindex_ignores_hidden_directories() {
    // `.trash/` (Obsidian's trash), `.git/` and friends are not notes.
    let (_tmp, dir) = vault("hidden");
    let mut s = open(&dir);
    let notes = notes_dir(&dir);
    for hidden in [".trash", ".obsidian", ".git/hooks", ".smart-env/node_modules", ".attachments"] {
        fs::create_dir_all(notes.join(hidden)).unwrap();
        fs::write(notes.join(hidden).join("buried.md"), "# Buried\n").unwrap();
    }
    fs::write(notes.join(".dotfile.md"), "# Dotfile\n").unwrap();
    fs::write(notes.join("Visible.md"), "# Visible\n").unwrap();

    s.reindex_all().unwrap();
    let paths = query(&dir, "SELECT path FROM notes WHERE deleted_at IS NULL ORDER BY path;");
    assert_eq!(paths, vec![vec!["Visible.md".to_string()]]);
}

#[test]
fn reindex_accepts_upper_case_md_extensions_like_the_obsidian_importer() {
    // The GUI's Obsidian import accepts `UPPER.MD`; the indexer used to walk
    // `.md` only, so such notes were never re-indexed and were lost when the
    // index was rebuilt.
    let (_tmp, dir) = vault("upper-ext");
    let mut s = open(&dir);
    let notes = notes_dir(&dir);
    fs::write(notes.join("UPPER.MD"), "# Upper\n").unwrap();
    fs::write(notes.join("lower.md"), "# Lower\n").unwrap();

    assert_eq!(s.reindex_all().unwrap().created, 2);
    fs::write(notes.join("UPPER.MD"), "# Upper\n\nchanged after import\n").unwrap();
    let stats = s.reindex_all().unwrap();
    assert_eq!(stats.updated, 1, "a change to an imported .MD note must be picked up");
    assert!(s.search("after import", 5).unwrap().iter().any(|r| r.path == "UPPER.MD"));
}

#[test]
fn reindex_rejects_non_utf8_files_without_aborting() {
    let (_tmp, dir) = vault("binary");
    let mut s = open(&dir);
    fs::write(notes_dir(&dir).join("bytes.md"), [0xff, 0xfe, 0x00, 0x81]).unwrap();
    fs::write(notes_dir(&dir).join("Fine.md"), "# Fine\n").unwrap();
    let stats = s.reindex_all().unwrap();
    assert_eq!(stats.errors, 1);
    assert_eq!(stats.created, 1);
}

// ── link resolution ──────────────────────────────────────────────

#[test]
fn cold_reindex_resolves_links_whatever_order_files_are_walked_in() {
    // Repro: only 45 % of resolvable wikilinks were resolved after a cold
    // reindex, because each note's links were resolved before later notes
    // existed in the index.
    let (_tmp, dir) = vault("order");
    let notes = notes_dir(&dir);
    fs::create_dir_all(&notes).unwrap();
    fs::write(notes.join("Aardvark.md"), "---\ntitle: Aardvark\n---\nsee [[Zebra]]\n").unwrap();
    fs::write(notes.join("Zebra.md"), "---\ntitle: Zebra\n---\nsee [[Aardvark]]\n").unwrap();
    fs::write(notes.join("Dangling.md"), "---\ntitle: Dangling\n---\nsee [[Never Made]]\n").unwrap();
    let mut s = open(&dir);

    s.reindex_all().unwrap();
    assert_eq!(resolved_links(&s), (2, 3), "both real links must resolve on the first pass");

    s.reindex_all().unwrap();
    assert_eq!(resolved_links(&s), (2, 3), "and stay resolved");
}

#[test]
fn wikilinks_resolve_by_file_name_like_obsidian() {
    // Obsidian resolves `[[plan]]` to the file `plan.md`, whatever the note's
    // heading or `title:` says. Cerebro only matched titles, then fell back to
    // a substring search — ambiguous as soon as another note contains "plan".
    let (_tmp, dir) = vault("basename");
    let notes = notes_dir(&dir);
    fs::create_dir_all(notes.join("projects")).unwrap();
    fs::write(notes.join("projects/plan.md"), "---\ntitle: Plan for Q4\n---\nBody.\n").unwrap();
    fs::write(notes.join("planning.md"), "---\ntitle: Planning board\n---\nBody.\n").unwrap();
    fs::write(notes.join("Inbox.md"), "See [[plan]] and [[planning]] for details.\n").unwrap();
    let mut s = open(&dir);

    s.reindex_all().unwrap();
    assert_eq!(resolved_links(&s), (2, 2));
    s.reindex_all().unwrap();
    assert_eq!(resolved_links(&s), (2, 2), "resolution must not depend on how often we reindex");
}

#[test]
fn creating_a_note_resolves_links_that_were_waiting_for_it() {
    let (_tmp, dir) = vault("forward");
    let mut s = open(&dir);
    let hub = s.create_note("Hub", "see [[Later Note]] when it exists", None, None).unwrap();
    assert_eq!(resolved_links(&s), (0, 1), "unresolved until the target exists");

    let later = s.create_note("Later Note", "here I am", None, None).unwrap();
    let back = s.get_backlinks(&later.id).unwrap();
    assert_eq!(back.len(), 1, "the link from Hub must resolve as soon as the target is created");
    assert_eq!(back[0].source_id, hub.id);
}

#[test]
fn link_line_numbers_and_context_count_from_the_top_of_the_file() {
    // Repro: a link on file line 10 was stored as "line 3, context: `tags:`"
    // because line numbers were computed after the frontmatter was cut off.
    let (_tmp, dir) = vault("line-numbers");
    let mut s = open(&dir);
    let target = s.create_note("Target B", "target body", None, None).unwrap();
    let source = "---\ntitle: Source A\ntags:\n  - one\n  - two\n---\n\nIntro paragraph.\n\nReal link: [[Target B]]\n";
    let note = s.create_note("Source A", source, None, None).unwrap();

    let links = s.get_forward_links(&note.id).unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].line_number, Some(10), "the link sits on line 10 of the file (6 lines of frontmatter above it)");
    assert_eq!(links[0].context.as_deref(), Some("Real link: [[Target B]]"));
    assert_eq!(links[0].target_id.as_deref(), Some(target.id.as_str()));
}

// ── auto (keyword) links ─────────────────────────────────────────

#[test]
fn keyword_links_are_bounded_per_note() {
    // N notes that all share a tag used to yield N(N-1)/2 links: 1.7 million
    // for 5,000 notes, all shipped to the GUI as a ~220 MB JSON payload.
    const N: usize = 60;
    const MAX_PER_NOTE: usize = 20;
    let (_tmp, dir) = vault("auto-links");
    let mut s = open(&dir);
    for i in 0..N {
        s.create_note(
            &format!("Shared Topic Common Words {i:03}"),
            "body",
            None,
            Some(vec!["common".to_string()]),
        )
        .unwrap();
    }
    let links = s.compute_keyword_links().unwrap();
    assert!(
        links.len() <= MAX_PER_NOTE * N,
        "{} auto-links for {N} notes is unbounded (cap is {MAX_PER_NOTE} per note)",
        links.len()
    );
    // Highest confidence first, every confidence inside (0, 1].
    assert!(links.windows(2).all(|w| w[0].2 >= w[1].2));
    assert!(links.iter().all(|l| l.2 > 0.0 && l.2 <= 1.0));
}

// ── SQLite configuration / concurrency ───────────────────────────

#[test]
fn a_write_waits_for_a_lock_held_by_another_connection() {
    // Proves a non-zero busy timeout behaviourally: another connection holds
    // the write lock for 400 ms; the write must wait and succeed.
    let (_tmp, dir) = vault("busy");
    let mut s = open(&dir);
    let holder = rusqlite::Connection::open(dir.join("db.sqlite")).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE; INSERT INTO schema_migrations(id, name) VALUES(77, 'hold');").unwrap();

    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        let t0 = std::time::Instant::now();
        let outcome = s.create_note("Under Lock", "written while another connection held the lock", None, None)
            .map(|n| n.path);
        tx.send((t0.elapsed(), outcome.map(|_| ()).map_err(|e| e.to_string()))).unwrap();
        // keep `s` alive until the write is done
        drop(s);
    });
    std::thread::sleep(Duration::from_millis(400));
    holder.execute_batch("ROLLBACK;").unwrap();
    drop(holder);

    let (waited, outcome) = rx.recv().unwrap();
    handle.join().unwrap();
    assert!(waited >= Duration::from_millis(350), "write did not wait for the lock: {waited:?}");
    outcome.unwrap_or_else(|e| panic!("write failed although the lock was released in time: {e}"));
}

#[test]
fn opening_a_brand_new_vault_from_two_threads_at_once_succeeds() {
    // Repro: "Error opening storage: UNIQUE constraint failed: schema_migrations.id"
    // when two processes ran the migrations on a fresh database simultaneously.
    for round in 0..8 {
        let (tmp, dir) = vault(&format!("race-{round}"));
        let a = dir.clone();
        let b = dir.clone();
        let ta = std::thread::spawn(move || Storage::open(&a, &ProfileConfig::new("audit")).map(|_| ()));
        let tb = std::thread::spawn(move || Storage::open(&b, &ProfileConfig::new("audit")).map(|_| ()));
        ta.join().unwrap().expect("thread A could not open the fresh vault");
        tb.join().unwrap().expect("thread B could not open the fresh vault");
        drop(tmp);
    }
}

// ── import ───────────────────────────────────────────────────────

fn import_bytes(_tag: &str, ext: &str, bytes: &[u8]) -> String {
    let tmp = tempfile::Builder::new().prefix("cerebro-audit-import-").tempdir().unwrap();
    let path = tmp.path().join(format!("document.{ext}"));
    fs::write(&path, bytes).unwrap();
    import_file(&path).expect("import").markdown
}

#[test]
fn html_import_emits_each_text_once_and_keeps_inline_marks() {
    // Repro: a `div > p > strong` article was emitted twice, with the bold and
    // italic runs torn out of their sentence.
    let html = br#"<html><head><title>Page Title</title></head><body>
<div class="article"><h1>Article Heading</h1>
<p>This is <strong>bold</strong> and <em>italic</em>.</p>
<p>Read <a href="https://example.com/docs">the docs</a> now.</p>
<ul><li>Item one</li><li>Item two</li></ul></div></body></html>"#;
    let md = import_bytes("html", "html", html);

    for once in ["Article Heading", "bold", "italic", "Item one", "Item two", "the docs"] {
        assert_eq!(md.matches(once).count(), 1, "{once:?} must appear exactly once in:\n{md}");
    }
    assert!(md.contains("This is **bold** and *italic*."), "inline marks must stay in place:\n{md}");
    assert!(md.contains("Read [the docs](https://example.com/docs) now."), "links must stay in place:\n{md}");
    assert!(md.contains("# Article Heading"), "\n{md}");
    assert!(md.contains("- Item one\n- Item two"), "\n{md}");
    assert!(!md.contains("  "), "no doubled spaces:\n{md}");
}

#[test]
fn html_import_handles_nested_blocks_and_tables() {
    let html = br#"<body><main><section><div><p>Deep paragraph.</p></div><table><tr><th>Name</th><th>Age</th></tr><tr><td>Alice</td><td>30</td></tr></table></section></main></body>"#;
    let md = import_bytes("html2", "html", html);
    assert_eq!(md.matches("Deep paragraph.").count(), 1, "{md}");
    assert!(md.contains("| Name | Age |\n| --- | --- |\n| Alice | 30 |"), "{md}");
}

const XLSX_PARTS: &[(&str, &str)] = &[
    ("[Content_Types].xml", "<?xml version=\"1.0\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/><Override PartName=\"/xl/worksheets/sheet2.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/></Types>"),
    ("_rels/.rels", "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>"),
    ("xl/workbook.xml", "<?xml version=\"1.0\"?><workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets><sheet name=\"FirstSheet\" sheetId=\"1\" r:id=\"rId1\"/><sheet name=\"SecondSheet\" sheetId=\"2\" r:id=\"rId2\"/></sheets></workbook>"),
    ("xl/_rels/workbook.xml.rels", "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet2.xml\"/></Relationships>"),
    ("xl/worksheets/sheet1.xml", "<?xml version=\"1.0\"?><worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData><row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Item</t></is></c><c r=\"B1\" t=\"inlineStr\"><is><t>Price</t></is></c></row><row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Bolt</t></is></c><c r=\"B2\"><v>0.25</v></c></row></sheetData></worksheet>"),
    ("xl/worksheets/sheet2.xml", "<?xml version=\"1.0\"?><worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData><row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>SECOND_SHEET_DATA</t></is></c></row></sheetData></worksheet>"),
];

fn two_sheet_xlsx() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, body) in XLSX_PARTS {
        zip.start_file(*name, options).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn spreadsheet_import_includes_every_sheet_with_well_formed_tables() {
    // Repro: only the first worksheet was imported, and the delimiter row was
    // rendered as `|| --- | --- |`.
    let md = import_bytes("xlsx", "xlsx", &two_sheet_xlsx());
    assert!(md.contains("## Sheet: FirstSheet"), "{md}");
    assert!(md.contains("## Sheet: SecondSheet"), "the second worksheet was dropped:\n{md}");
    assert!(md.contains("SECOND_SHEET_DATA"), "{md}");
    assert!(md.contains("| Item | Price |\n| --- | --- |\n| Bolt | 0.25 |"), "{md}");
}

#[test]
fn supported_extensions_lists_every_accepted_extension() {
    // The "Unsupported file format" error prints `supported_extensions()`,
    // which used to omit 22 extensions the importer does accept.
    let accepted: Vec<&str> = [
        "pdf", "pptx", "docx", "doc", "ods", "xlsx", "xls", "odt", "html", "htm", "md", "markdown",
        "json", "jsonl", "txt", "text", "csv", "tsv", "yaml", "yml", "toml", "ini", "cfg", "xml",
        "css", "scss", "less", "log", "rst", "asciidoc", "adoc", "js", "ts", "jsx", "tsx", "py",
        "rb", "rs", "go", "java", "c", "cpp", "h", "hpp", "sh", "bash", "zsh", "fish", "sql",
        "graphql", "dockerfile", "makefile",
    ]
    .into_iter()
    .filter(|ext| ImportFormat::from_extension(ext).is_some())
    .collect();
    let listed = ImportFormat::supported_extensions();
    let missing: Vec<&str> = accepted.iter().copied().filter(|e| !listed.contains(e)).collect();
    assert!(missing.is_empty(), "accepted but not listed: {missing:?}");
    for ext in listed {
        assert!(ImportFormat::from_extension(ext).is_some(), "{ext} is listed but not accepted");
    }
}

// ── search ──────────────────────────────────────────────────────

#[test]
fn search_finds_hyphenated_words_and_survives_half_typed_queries() {
    // FTS5 rejects `a-b` ("no such column: b") and the error used to be
    // dropped, so "state-of-the-art", "e-mail" or "2026-11-23" returned
    // nothing at all — with exit code 0.
    let (_tmp, dir) = vault("hyphen-search");
    let mut s = open(&dir);
    let note = s
        .create_note(
            "Stack",
            "We prefer a state-of-the-art e-mail stack (see rust-lang), decided 2026-11-23.\n\nsentinel vault-which",
            None,
            Some(vec!["rust-lang".to_string()]),
        )
        .unwrap();
    s.create_note("Other", "the state of another art form, mailed yesterday", None, None).unwrap();

    for query in [
        "state-of-the-art",
        "e-mail",
        "2026-11-23",
        "rust-lang*",
        "vault-which",
        "STATE-OF-THE-ART",
        "e-mail AND rust-lang",
        "\"e-mail\"",
        "tags:rust-lang",
    ] {
        let hits = s.search(query, 10).unwrap_or_else(|e| panic!("search {query:?} failed: {e:#}"));
        assert!(hits.iter().any(|h| h.path == note.path), "search {query:?} must find the note; got {:?}", hits.iter().map(|h| h.path.as_str()).collect::<Vec<_>>());
    }

    // The hyphenated phrase does not match words that merely appear in the same note.
    let hits = s.search("state-of-the-art", 10).unwrap();
    assert!(hits.iter().all(|h| h.path == note.path), "{:?}", hits.iter().map(|h| h.path.as_str()).collect::<Vec<_>>());

    // FTS5 syntax still means what it says.
    assert!(s.search("\"state of the art\"", 10).unwrap().iter().all(|h| h.path == note.path));
    assert!(s.search("e-mail OR yesterday", 10).unwrap().iter().any(|h| h.path != note.path));
    assert!(s.search("e-mail NOT rust*", 10).unwrap().is_empty());

    // Half-typed queries (a typeahead sends these) are not errors and not "nothing".
    for partial in ["state (", "\"e-mail", "state AND", "art)"] {
        let hits = s.search(partial, 10).unwrap_or_else(|e| panic!("search {partial:?} failed: {e:#}"));
        assert!(!hits.is_empty(), "search {partial:?} should still find the note by its words");
    }
}
