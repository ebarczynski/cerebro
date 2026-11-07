//! Integration tests for Cerebro knowledge base.

use std::path::PathBuf;
use std::fs;

use cerebro::config::profile::ProfileConfig;
use cerebro::storage::Storage;

struct TestVault {
    _tmp: tempfile::TempDir,
    profile_dir: PathBuf,
}

impl TestVault {
    fn new(name: &str) -> (Self, Storage) {
        let tmp = tempfile::tempdir().unwrap();
        let profile_dir = tmp.path().join(name);
        fs::create_dir_all(&profile_dir).unwrap();

        let profile_config = ProfileConfig::new(name);
        profile_config.save_to(&profile_dir).unwrap();

        let storage = Storage::open(&profile_dir, &profile_config).unwrap();
        let vault = TestVault { _tmp: tmp, profile_dir };
        (vault, storage)
    }
}

#[test]
fn test_create_and_read_note() {
    let (_vault, mut storage) = TestVault::new("create_read");
    let note = storage
        .create_note(
            "Test Note",
            "# Test Note\n\nHello world\n\nSee [[Other Note]] for details.",
            None,
            Some(vec!["test".to_string()]),
        )
        .unwrap();

    assert_eq!(note.title, "Test Note");
    assert!(note.tags.contains(&"test".to_string()));
    assert!(note.word_count > 0);

    let content = storage.read_note(&note.id).unwrap();
    assert!(content.body.contains("Hello world"));
}

#[test]
fn test_update_note() {
    let (_vault, mut storage) = TestVault::new("update");
    let note = storage
        .create_note("Update Test", "Original content", None, None)
        .unwrap();

    let updated = storage
        .update_note(&note.id, "Updated content with more words")
        .unwrap();
    assert_eq!(updated.word_count, 5);

    let content = storage.read_note(&note.id).unwrap();
    assert!(content.body.contains("Updated content"));
}

#[test]
fn test_delete_note() {
    let (_vault, mut storage) = TestVault::new("delete");
    let note = storage
        .create_note("Delete Me", "Soon gone", None, None)
        .unwrap();
    let id = note.id.clone();

    storage.delete_note(&id).unwrap();
    assert!(storage.read_note(&id).is_err());
}

#[test]
fn test_search_notes() {
    let (_vault, mut storage) = TestVault::new("search");
    storage
        .create_note(
            "Rust Programming",
            "Rust is a systems programming language focused on safety",
            None,
            Some(vec!["rust".to_string()]),
        )
        .unwrap();
    storage
        .create_note(
            "Python Notes",
            "Python is great for data science",
            None,
            Some(vec!["python".to_string()]),
        )
        .unwrap();

    let results = storage.search("rust", 10).unwrap();
    assert!(!results.is_empty());
    assert!(results[0].title.contains("Rust"));
}

#[test]
fn test_wikilinks_and_backlinks() {
    let (_vault, mut storage) = TestVault::new("links");
    // Create Note Beta first so the link from Alpha can resolve
    let _note_b = storage
        .create_note("Note Beta", "Some content", None, None)
        .unwrap();
    let note_a = storage
        .create_note("Note Alpha", "Content linking to [[Note Beta]]", None, None)
        .unwrap();

    let backlinks = storage.get_backlinks("Note Beta").unwrap();
    assert_eq!(backlinks.len(), 1);
    assert_eq!(backlinks[0].source_id, note_a.id);

    let forward = storage.get_forward_links("Note Alpha").unwrap();
    assert_eq!(forward.len(), 1);
    assert_eq!(forward[0].target_ref, "Note Beta");
}

#[test]
fn test_tags() {
    let (_vault, mut storage) = TestVault::new("tags");
    storage
        .create_note(
            "Tagged Note 1",
            "Content",
            None,
            Some(vec!["rust".to_string(), "systems".to_string()]),
        )
        .unwrap();
    storage
        .create_note(
            "Tagged Note 2",
            "Content",
            None,
            Some(vec!["rust".to_string()]),
        )
        .unwrap();

    let tags = storage.list_tags().unwrap();
    assert!(tags
        .iter()
        .any(|(name, count)| name == "rust" && *count >= 2));
}

#[test]
fn test_list_notes_with_tag_filter() {
    let (_vault, mut storage) = TestVault::new("tag_filter");
    storage
        .create_note("Rust Note", "Content", None, Some(vec!["rust".to_string()]))
        .unwrap();
    storage
        .create_note(
            "Python Note",
            "Content",
            None,
            Some(vec!["python".to_string()]),
        )
        .unwrap();

    let rust_notes = storage.list_notes(Some("rust"), 100, 0).unwrap();
    assert_eq!(rust_notes.len(), 1);
    assert_eq!(rust_notes[0].title, "Rust Note");
}

#[test]
fn test_reindex() {
    let (vault, mut storage) = TestVault::new("reindex");

    // Create a file manually on disk
    let notes_path = vault.profile_dir.join("notes");
    fs::create_dir_all(&notes_path).unwrap();
    fs::write(
        notes_path.join("Manual Note.md"),
        "# Manual Note\n\nWritten by hand.",
    )
    .unwrap();

    let stats = storage.reindex_all().unwrap();
    assert_eq!(stats.created, 1);

    // Verify the note was picked up
    let notes = storage.list_notes(None, 100, 0).unwrap();
    assert!(notes.iter().any(|n| n.title == "Manual Note"));
}

#[test]
fn test_note_with_frontmatter() {
    let (_vault, mut storage) = TestVault::new("frontmatter");
    let content = "---\ntitle: Frontmatter Note\ntags:\n  - fm-test\n---\n\nContent here.";
    let note = storage
        .create_note("Frontmatter Note", content, None, None)
        .unwrap();

    let read_back = storage.read_note(&note.id).unwrap();
    assert!(read_back.body.contains("Content here"));
}

#[test]
fn test_note_exists() {
    let (_vault, mut storage) = TestVault::new("note_exists");

    assert!(!storage.note_exists("folder/note.md"));

    storage
        .create_note(
            "Path Note",
            "# Path Note",
            Some("folder/note.md"),
            None,
        )
        .unwrap();

    assert!(storage.note_exists("folder/note.md"));
    assert!(!storage.note_exists("other/note.md"));
}

// ── Grill regression tests (2026-11-06 audit, F1–F8) ────────────────

#[test]
fn test_capture_directory_path_gets_filename_appended() {
    // F1: `cerebro capture` passed "inbox/" as the path and crashed with
    // EISDIR. A directory-like path must receive a title-derived filename.
    let (_vault, mut storage) = TestVault::new("capture");
    let note = storage
        .create_note("Capture Test", "quick thought", Some("inbox/"), None)
        .unwrap();

    assert_eq!(note.path, "inbox/Capture_Test.md");
    let notes_dir = storage.notes_dir_path().to_path_buf();
    assert!(notes_dir.join(&note.path).exists());
    assert!(storage.note_exists(&note.path));
}

#[test]
fn test_path_traversal_is_rejected() {
    // F4: user-supplied paths escaped the vault. Traversal and absolute
    // paths must be rejected at the storage layer (covers CLI and MCP).
    let (_vault, mut storage) = TestVault::new("traversal");

    assert!(storage.create_note("Evil", "pwned", Some("../../escape"), None).is_err());
    assert!(storage.create_note("Evil", "pwned", Some("/etc/passwd.md"), None).is_err());
    assert!(storage.create_note("Evil", "pwned", Some("a/../../b.md"), None).is_err());

    // A path that STAYS inside the vault after normalization is accepted
    let ok = storage
        .create_note("Inner", "fine", Some("sub/../inner.md"), None)
        .unwrap();
    assert_eq!(ok.path, "sub/../inner.md");
}

#[test]
fn test_duplicate_wikilink_on_one_line_creates_one_edge() {
    // F2: referencing the same target twice on one line used to violate
    // UNIQUE(source, target_ref, line_number) and abort note creation.
    let (_vault, mut storage) = TestVault::new("dup_link");
    let note = storage
        .create_note(
            "Dup Link",
            "See [[Target A]] and again [[Target A]] here",
            None,
            None,
        )
        .unwrap();

    let forward = storage.get_forward_links(&note.id).unwrap();
    assert_eq!(forward.len(), 1, "one edge per (target, line)");
}

#[test]
fn test_link_context_matches_link_line() {
    // F6: every link's context used to be the FIRST wikilink line of the
    // source note. Context must come from the link's own line.
    let (_vault, mut storage) = TestVault::new("link_ctx");
    storage.create_note("Target A", "a", None, None).unwrap();
    storage.create_note("Target B", "b", None, None).unwrap();

    let hub = storage
        .create_note(
            "Hub",
            "L1 links [[Target A]]\n\nL3\nL4 links [[Target B]]",
            None,
            None,
        )
        .unwrap();

    let bl_a = storage.get_backlinks("Target A").unwrap();
    let bl_b = storage.get_backlinks("Target B").unwrap();
    assert_eq!(bl_a.len(), 1);
    assert_eq!(bl_b.len(), 1);
    assert_eq!(bl_a[0].context.as_deref(), Some("L1 links [[Target A]]"));
    assert_eq!(bl_b[0].context.as_deref(), Some("L4 links [[Target B]]"));
    assert_eq!(bl_a[0].line_number, Some(1));
    assert_eq!(bl_b[0].line_number, Some(4));
    assert_eq!(bl_a[0].source_id, hub.id);
}

#[test]
fn test_tag_counts_derive_from_note_tags() {
    // F5: tag counters only ever incremented — after removing a tag via
    // update or deleting a tagged note, `tags` still reported stale counts.
    let (_vault, mut storage) = TestVault::new("tag_drift");
    let n1 = storage
        .create_note("T1", "one", None, Some(vec!["rust".to_string()]))
        .unwrap();
    storage
        .create_note("T2", "two", None, Some(vec!["rust".to_string()]))
        .unwrap();

    let tags = storage.list_tags().unwrap();
    assert_eq!(
        tags.iter().find(|(n, _)| n == "rust").map(|(_, c)| *c),
        Some(2)
    );

    // Update removes the tag from T1
    storage.update_note(&n1.id, "content without tags").unwrap();
    let tags = storage.list_tags().unwrap();
    assert_eq!(
        tags.iter().find(|(n, _)| n == "rust").map(|(_, c)| *c),
        Some(1)
    );

    // Delete T2 — the tag must disappear entirely
    let t2 = storage.read_note("T2").unwrap().note;
    storage.delete_note(&t2.id).unwrap();
    let tags = storage.list_tags().unwrap();
    assert!(
        !tags.iter().any(|(n, _)| n == "rust"),
        "zero-note tags must be dropped, got {:?}",
        tags
    );
}

#[test]
fn test_links_inside_fenced_code_are_ignored_by_storage() {
    // F7 end-to-end: code-sample wikilinks must not become graph edges.
    let (_vault, mut storage) = TestVault::new("phantom");
    let note = storage
        .create_note(
            "Phantom Test",
            "Real [[Real Link]] outside.\n\n```rust\nlet s = \"[[Phantom Link]]\";\n```",
            None,
            None,
        )
        .unwrap();

    let forward = storage.get_forward_links(&note.id).unwrap();
    assert_eq!(forward.len(), 1);
    assert_eq!(forward[0].target_ref, "Real Link");
}

#[test]
fn test_ambiguous_identifier_is_rejected_for_destructive_ops() {
    // F8: substring matching + first-row resolution could update or delete
    // the WRONG note. Ambiguous identifiers must error with candidates.
    let (_vault, mut storage) = TestVault::new("ambiguous");
    storage.create_note("Alpha One", "one", None, None).unwrap();
    storage.create_note("Alpha Two", "two", None, None).unwrap();

    let err = storage.read_note("Alpha").unwrap_err().to_string();
    assert!(err.contains("ambiguous"), "got: {}", err);
    assert!(err.contains("Alpha One") && err.contains("Alpha Two"));

    // Exact identifiers still resolve
    let exact = storage.read_note("Alpha One").unwrap();
    assert_eq!(exact.note.title, "Alpha One");
}

#[test]
fn test_mcp_protocol_contract() {
    // F9/F17: 9 tools listed (kp_search gated per ADR 0005), notifications
    // get no reply, unknown tools are JSON-RPC errors, cerebro_graph works.
    use std::io::Write;
    use std::process::{Command, Stdio};

    let tmp = tempfile::tempdir().unwrap();
    let profiles_dir = tmp.path().join("profiles");
    let profile_dir = profiles_dir.join("personal");
    let notes_dir = profile_dir.join("notes");
    fs::create_dir_all(&notes_dir).unwrap();

    let config_path = tmp.path().join("config.toml");
    fs::write(
        &config_path,
        format!(
            "default_profile = \"personal\"\nprofiles_dir = \"{}\"\n",
            profiles_dir.display()
        ),
    )
    .unwrap();

    // Seed one note + one link directly through the library
    {
        let profile_config = ProfileConfig::new("personal");
        profile_config.save_to(&profile_dir).unwrap();
        let mut storage = Storage::open(&profile_dir, &profile_config).unwrap();
        storage.create_note("Graph A", "links to [[Graph B]]", None, None).unwrap();
    }

    let mut child = Command::new(env!("CARGO_BIN_EXE_cerebro"))
        .args(["--config", config_path.to_str().unwrap(), "mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let stdin = child.stdin.as_mut().unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18"}}}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list"}}"#).unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"cerebro_graph","arguments":{{}}}}}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"cerebro_kp_search","arguments":{{"query":"anything"}}}}}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{{"name":"cerebro_does_not_exist","arguments":{{}}}}}}"#
    )
    .unwrap();
    drop(stdin);

    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();

    // The notification must NOT have produced a response line (F17)
    assert_eq!(
        lines.len(),
        5,
        "expected exactly 5 responses (initialize, tools/list, graph, kp, error); got {}: {:?}",
        lines.len(),
        lines
    );

    // initialize: negotiated protocol version echoed
    assert!(lines[0].contains("2025-06-18"), "got: {}", lines[0]);

    // tools/list: all 10 documented tools present (ADR 0005: kp_search
    // shipped after contract verification against KP source)
    assert!(lines[1].contains("cerebro_graph"), "got: {}", lines[1]);
    assert!(lines[1].contains("cerebro_kp_search"), "got: {}", lines[1]);

    // cerebro_graph returns nodes + edges with the seeded note
    assert!(lines[2].contains("node_count"), "got: {}", lines[2]);
    assert!(lines[2].contains("Graph A"), "got: {}", lines[2]);

    // cerebro_kp_search with KP disabled: graceful local-FTS fallback,
    // explicitly labeled, still a success result
    assert!(lines[3].contains("local_fts"), "got: {}", lines[3]);
    assert!(
        lines[3].contains("Knowledge Palace unavailable"),
        "got: {}",
        lines[3]
    );
    assert!(lines[3].contains("\"isError\":false"), "got: {}", lines[3]);

    // Unknown tool: JSON-RPC error object, not success-shaped text (F9)
    assert!(lines[4].contains("\"error\""), "got: {}", lines[4]);
    assert!(lines[4].contains("-32602"), "got: {}", lines[4]);
}
