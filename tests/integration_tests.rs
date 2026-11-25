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

// ── Vault protection (ADR 0006) integration tests ───────────────────

#[test]
fn test_protection_round_trip() {
    let (vault, mut storage) = TestVault::new("protect_rt");
    // Target must exist BEFORE the linking note (links resolve at write time).
    storage.create_note("Elsewhere", "target", None, None).unwrap();
    let note = storage
        .create_note("Secret Note", "classified #business content [[Elsewhere]]", None, None)
        .unwrap();

    // 1. Enable protection.
    let mut cfg = ProfileConfig::load(&vault.profile_dir).unwrap();
    Storage::enable_protection(&vault.profile_dir, &mut cfg, "hunter2 correct").unwrap();
    assert!(cfg.encryption);
    assert!(cfg.validate().is_ok());

    // 2. Files on disk are ciphertext (magic present, plaintext absent).
    let notes_dir = vault.profile_dir.join("notes");
    let secret_path = notes_dir.join("Secret_Note.md");
    let raw = fs::read(&secret_path).unwrap();
    assert!(cerebro::crypto::looks_encrypted(&raw), "note file is plaintext after enable");
    assert!(!String::from_utf8_lossy(&raw).contains("classified"));
    // Database file is no longer a plaintext SQLite header.
    let db_header = fs::read(vault.profile_dir.join("db.sqlite")).unwrap();
    assert_ne!(&db_header[..15], b"SQLite format 3");

    // 3. The PRE-ENCRYPTION storage handle (no key) must NOT silently read
    //    ciphertext — it errors, documenting locked-state semantics.
    let stale_err = storage.read_note(&note.id).unwrap_err().to_string();
    assert!(stale_err.contains("encrypted note found"), "got: {stale_err}");

    // 4. A locked session (no key) is denied.
    let locked = Storage::open(&vault.profile_dir, &cfg);
    let err = match locked {
        Err(e) => e.to_string(),
        Ok(_) => panic!("locked open should fail"),
    };
    assert!(err.contains(cerebro::storage::VAULT_LOCKED_PREFIX), "got: {err}");

    // 5. Unlocked session via passphrase sees everything, search included.
    let kdf = cfg.protection.kdf.as_ref().unwrap();
    let key = cerebro::crypto::VaultKey::unwrap_with_passphrase(
        cfg.protection.wrapped_key_b64.as_deref().unwrap(),
        "hunter2 correct",
        kdf,
    )
    .unwrap();
    let mut unlocked = Storage::open_with_key(&vault.profile_dir, &cfg, Some(&key)).unwrap();
    let results = unlocked.search("classified", 5).unwrap();
    assert!(!results.is_empty());
    let bl = unlocked.get_backlinks("Elsewhere").unwrap();
    assert_eq!(bl.len(), 1);

    // 6. Wrong passphrase fails at open.
    let wrong = cerebro::crypto::VaultKey::unwrap_with_passphrase(
        cfg.protection.wrapped_key_b64.as_deref().unwrap(),
        "WRONG",
        kdf,
    );
    assert!(wrong.is_err());

    // 7. Writes inside the unlocked session stay encrypted on disk.
    unlocked
        .create_note("Post Enable", "written while unlocked", None, None)
        .unwrap();
    let raw2 = fs::read(notes_dir.join("Post_Enable.md")).unwrap();
    assert!(cerebro::crypto::looks_encrypted(&raw2));

    // 8. Decrypt restores plaintext + plaintext db.
    let mut cfg2 = ProfileConfig::load(&vault.profile_dir).unwrap();
    Storage::disable_protection(&vault.profile_dir, &mut cfg2, "hunter2 correct").unwrap();
    assert!(!cfg2.encryption);
    let raw3 = fs::read(&secret_path).unwrap();
    assert!(!cerebro::crypto::looks_encrypted(&raw3));
    let db_header2 = fs::read(vault.profile_dir.join("db.sqlite")).unwrap();
    assert_eq!(&db_header2[..15], b"SQLite format 3");
    let plain = Storage::open(&vault.profile_dir, &cfg2).unwrap();
    assert!(plain.read_note("Secret Note").unwrap().body.contains("classified"));
    assert!(!plain.search("classified", 5).unwrap().is_empty());
}

#[test]
fn test_protection_wrong_passphrase_enable_disabled_vault_is_data_safe() {
    // Disabling protection requires the CURRENT passphrase; a wrong one
    // must leave the vault untouched.
    let (vault, mut storage) = TestVault::new("protect_wrong");
    storage.create_note("Data", "precious", None, None).unwrap();
    let mut cfg = ProfileConfig::load(&vault.profile_dir).unwrap();
    Storage::enable_protection(&vault.profile_dir, &mut cfg, "right-pass").unwrap();

    let err = Storage::disable_protection(&vault.profile_dir, &mut cfg, "wrong-pass").unwrap_err();
    assert!(err.to_string().contains("wrong passphrase"), "got: {err}");
    // Vault remains protected and readable with the right passphrase.
    assert!(cfg.encryption);
    let kdf = cfg.protection.kdf.as_ref().unwrap();
    let key = cerebro::crypto::VaultKey::unwrap_with_passphrase(
        cfg.protection.wrapped_key_b64.as_deref().unwrap(),
        "right-pass",
        kdf,
    )
    .unwrap();
    let s = Storage::open_with_key(&vault.profile_dir, &cfg, Some(&key)).unwrap();
    assert!(s.read_note("Data").unwrap().body.contains("precious"));
}

#[test]
fn test_business_vaults_default_protected_documented() {
    // ADR 0006: kind-informed defaults — the VaultKind::Project kind exists
    // and round-trips through tags (GUI contract).
    assert_eq!(cerebro::config::profile::VaultKind::from_tag("project"), cerebro::config::profile::VaultKind::Project);
    assert_eq!(cerebro::config::profile::VaultKind::Project.as_tag(), "project");
    assert_eq!(cerebro::config::profile::VaultKind::Business.as_tag(), "business");
}


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
    // Line numbers count from the top of the note FILE. Cerebro wrote three
    // lines of frontmatter above the body, so the links sit on lines 4 and 7.
    // (They used to be stored relative to the body passed to create_note,
    // while a reindex stored them relative to the file.)
    assert_eq!(bl_a[0].line_number, Some(4));
    assert_eq!(bl_b[0].line_number, Some(7));
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
    // Close our end so the server sees EOF. (This used to be `drop(stdin)` on
    // a `&mut` borrow — a no-op; the pipe was only closed later by
    // `wait_with_output`.)
    drop(child.stdin.take());

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

// ── MCP sandbox + audit (ADR 0007) integration tests ────────────────

fn spawn_mcp_session(
    config_path: &std::path::Path,
    xdg_dir: &std::path::Path,
    flags: &[&str],
    lines: &[String],
) -> Vec<String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cerebro"));
    cmd.arg("--config")
        .arg(config_path.to_str().unwrap())
        .arg("mcp");
    for f in flags {
        cmd.arg(f);
    }
    // Sandbox the child's config dir so the audit journal lands in tmp.
    cmd.env("XDG_CONFIG_HOME", xdg_dir);
    cmd.env("HOME", xdg_dir);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = cmd.spawn().unwrap();
    let stdin = child.stdin.as_mut().unwrap();
    for l in lines {
        writeln!(stdin, "{l}").unwrap();
    }
    // Close our end so the server sees EOF. (Same as above: `drop(stdin)` on
    // a `&mut` borrow is a no-op.)
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    String::from_utf8(out.stdout).unwrap().lines().map(String::from).collect()
}

fn sandbox_call(id: u64, tool: &str, args: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{tool}","arguments":{args}}}}}"#
    )
}

#[test]
fn test_mcp_sandbox_policies_and_audit() {
    let tmp = tempfile::tempdir().unwrap();
    let xdg = tmp.path().join("xdg");
    fs::create_dir_all(&xdg).unwrap();
    let profiles_dir = tmp.path().join("profiles");

    // personal: plaintext vault (write policy); sec: PROTECTED vault (read policy).
    for name in ["personal", "sec"] {
        let profile_dir = profiles_dir.join(name);
        fs::create_dir_all(profile_dir.join("notes")).unwrap();
        let cfg = ProfileConfig::new(name);
        cfg.save_to(&profile_dir).unwrap();
        let mut storage = Storage::open(&profile_dir, &cfg).unwrap();
        storage
            .create_note(&format!("Note {name}"), "seed content", None, None)
            .unwrap();
    }
    let sec_dir = profiles_dir.join("sec");
    let mut sec_cfg = ProfileConfig::load(&sec_dir).unwrap();
    Storage::enable_protection(&sec_dir, &mut sec_cfg, "sec-pass").unwrap();

    let config_path = tmp.path().join("config.toml");
    fs::write(
        &config_path,
        format!(
            "default_profile = \"personal\"\nprofiles_dir = \"{}\"\n\n[mcp.profiles.personal]\nallow = \"write\"\n\n[mcp.profiles.sec]\nallow = \"read\"\n",
            profiles_dir.display()
        ),
    )
    .unwrap();

    // Session 1: default policies (personal=write, sec=read), sec NOT unlocked.
    let responses = spawn_mcp_session(
        &config_path,
        &xdg,
        &[],
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#.to_string(),
            sandbox_call(2, "cerebro_create_note", r#"{"title":"MCP Write","content":"written over mcp"}"#),
            sandbox_call(3, "cerebro_read_note", r#"{"identifier":"Note sec","profile":"sec"}"#),
            sandbox_call(4, "cerebro_list_notes", r#"{"profile":"nonexistent"}"#),
        ],
    );

    // write policy allows create
    assert!(responses[1].contains("MCP Write"), "got: {}", responses[1]);
    assert!(responses[1].contains("\"isError\":false"), "got: {}", responses[1]);
    // locked vault: invisible even though policy allows read
    assert!(responses[2].contains("VAULT LOCKED"), "got: {}", responses[2]);
    assert!(responses[2].contains("\"isError\":true"), "got: {}", responses[2]);
    // unknown profile → allow=none default → policy denial
    assert!(responses[3].contains("not exposed over MCP"), "got: {}", responses[3]);

    // Session 2: scope override narrows personal to none → denied.
    let responses2 = spawn_mcp_session(
        &config_path,
        &xdg,
        &["--scope", "personal:none"],
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#.to_string(),
            sandbox_call(2, "cerebro_list_notes", "{}"),
        ],
    );
    assert!(responses2[1].contains("not exposed over MCP"), "got: {}", responses2[1]);

    // Audit journal: every call recorded with its decision, never note content.
    let audit = xdg.join("cerebro/audit/mcp.jsonl");
    let audit_text = fs::read_to_string(audit).expect("audit journal must exist");
    let allow_count = audit_text.matches("\"decision\":\"allow\"").count();
    let deny_count = audit_text.matches("\"decision\":\"deny\"").count();
    let error_count = audit_text.matches("\"decision\":\"error\"").count();
    assert!(allow_count >= 1, "allow lines missing: {audit_text}");
    assert!(deny_count >= 2, "deny lines missing: {audit_text}");
    assert!(error_count >= 1, "error (locked) lines missing: {audit_text}");
    assert!(audit_text.contains("VAULT LOCKED"), "locked reason must be audited");
    assert!(!audit_text.contains("seed content"), "audit must never contain note content");
}

// ── vault.toml must never be replaced (ADR 0006: it holds the wrapped key) ──

fn cerebro_cli(home: &std::path::Path, args: &[&str]) -> (i32, String, String) {
    use std::process::{Command, Stdio};
    let out = Command::new(env!("CARGO_BIN_EXE_cerebro"))
        .args(args)
        .env("HOME", home)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.invalid")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.invalid")
        .env_remove("RUST_LOG")
        .stdin(Stdio::null())
        .output()
        .expect("run cerebro");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn init_refuses_to_run_inside_an_existing_vault() {
    // Repro: re-running the Quick-Start command on a protected vault replaced
    // vault.toml — the only copy of the wrapped master key — and the notes
    // could never be decrypted again (2026-11-25 audit, finding 1).
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    let (code, _, stderr) = cerebro_cli(home, &["init"]);
    assert_eq!(code, 0, "{}", stderr);
    let (code, _, stderr) = cerebro_cli(home, &["--profile", "personal", "notes", "create", "Keep", "--content", "precious"]);
    assert_eq!(code, 0, "{}", stderr);
    let vault_toml_before = fs::read_to_string(home.join(".cerebro/profiles/personal/vault.toml")).unwrap();

    let (code, _, stderr) = cerebro_cli(home, &["init"]);
    assert_ne!(code, 0, "a second init must not succeed");
    assert!(stderr.contains("already exists"), "{stderr}");

    let vault_toml_after = fs::read_to_string(home.join(".cerebro/profiles/personal/vault.toml")).unwrap();
    assert_eq!(vault_toml_before, vault_toml_after, "vault.toml must be untouched");
    let (code, stdout, stderr) = cerebro_cli(home, &["--profile", "personal", "notes", "list"]);
    assert_eq!(code, 0, "{}", stderr);
    assert!(stdout.contains("Keep"), "the note must survive: {stdout}");
}

#[test]
fn profile_create_refuses_the_name_of_an_existing_vault() {
    // Same defect through `profile create` (it shares cmd_init).
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let (code, _, stderr) = cerebro_cli(home, &["init"]);
    assert_eq!(code, 0, "{}", stderr);

    let (code, _, stderr) = cerebro_cli(home, &["profile", "create", "personal"]);
    assert_ne!(code, 0, "creating over an existing vault must fail");
    assert!(stderr.contains("already exists"), "{stderr}");
    assert!(home.join(".cerebro/profiles/personal/vault.toml").exists());
}

// ── migration backups (2026-11-25 audit, finding 2) ─────────────────

#[test]
fn migration_leaves_no_backup_behind_after_success() {
    // The pre-migration backup is a plaintext copy of the whole vault; after
    // a successful migration nothing plaintext may outlive it, and it must
    // live inside the vault (so `profile delete` removes it too).
    let tmp = tempfile::tempdir().unwrap();
    let profile_dir = tmp.path().join("v");
    fs::create_dir_all(&profile_dir).unwrap();
    let mut cfg = ProfileConfig::new("v");
    cfg.save_to(&profile_dir).unwrap();
    let notes = profile_dir.join("notes");
    fs::write(notes.join("a.md"), "# A\nsecret-9d2e body\n").unwrap();

    Storage::enable_protection(&profile_dir, &mut cfg, "long-enough-pass").unwrap();
    Storage::disable_protection(&profile_dir, &mut cfg, "long-enough-pass").unwrap();

    let backups = profile_dir.join("backups");
    assert!(
        !backups.exists(),
        "no backup may survive a completed migration: {:?}",
        fs::read_dir(backups).map(|d| d.count())
    );
    // The round trip itself still works.
    assert!(fs::read_to_string(notes.join("a.md")).unwrap().contains("secret-9d2e"));
}

#[test]
fn an_interrupted_encryption_is_recovered_by_decrypt() {
    // Repro (2026-11-25 audit, finding 7): the config is flipped first, note
    // files are converted one by one, and the database is rebuilt last. A
    // crash in between left a vault that no command could open — "file is
    // not a database" — even with the correct passphrase.
    let tmp = tempfile::tempdir().unwrap();
    let profile_dir = tmp.path().join("v");
    fs::create_dir_all(&profile_dir).unwrap();
    let mut cfg = ProfileConfig::new("v");
    cfg.save_to(&profile_dir).unwrap();
    let notes = profile_dir.join("notes");

    // A working plaintext vault with a database…
    {
        let mut storage = Storage::open(&profile_dir, &cfg).unwrap();
        storage.create_note("Alpha", "alpha body f3a1", None, None).unwrap();
        storage.create_note("Bravo", "bravo body 77c0", None, None).unwrap();
    }
    // Snapshot the PLAINTEXT state now, before encrypting: this is what an
    // interrupted migration has on disk in place of the rebuilt database.
    let plain = tmp.path().join("plain-snapshot");
    let _ = fs::remove_dir_all(&plain);
    copy_dir(&profile_dir, &plain);

    // …then the interrupted state: config flipped, only one of two notes
    // converted, database NOT yet rebuilt (still plaintext SQLite).
    Storage::enable_protection(&profile_dir, &mut cfg, "long-enough-pass").unwrap();
    fs::copy(plain.join("db.sqlite"), profile_dir.join("db.sqlite")).unwrap();
    for suffix in ["-wal", "-shm"] {
        let _ = fs::copy(plain.join(format!("db.sqlite{suffix}")), profile_dir.join(format!("db.sqlite{suffix}")));
    }
    // One note back to plaintext, as if the file loop stopped after Alpha.
    let bytes = fs::read(notes.join("Bravo.md")).unwrap();
    assert!(bytes.starts_with(b"CERE1"), "precondition");
    fs::write(notes.join("Bravo.md"), fs::read_to_string(plain.join("notes/Bravo.md")).unwrap()).unwrap();
    assert!(cfg.encryption);

    // The recovery: `vault decrypt` with the correct passphrase — which used
    // to die with "file is not a database" because it opened the stale index.
    Storage::disable_protection(&profile_dir, &mut cfg, "long-enough-pass").unwrap();

    let storage = Storage::open(&profile_dir, &cfg).unwrap();
    assert!(
        storage.read_note("Alpha").unwrap().body.contains("alpha body"),
        "the encrypted note must be readable after recovery"
    );
    assert!(storage.read_note("Bravo").unwrap().body.contains("bravo body 77c0"));
    // And the recovered vault's files are all plaintext again.
    for name in ["Alpha.md", "Bravo.md"] {
        let bytes = fs::read(notes.join(name)).unwrap();
        assert!(!bytes.starts_with(b"CERE1"), "{name} must be plaintext after recovery");
    }
}

/// Recursive copy for test fixtures.
fn copy_dir(src: &std::path::Path, dest: &std::path::Path) {
    fs::create_dir_all(dest).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dest.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &to);
        } else {
            fs::copy(entry.path(), &to).unwrap();
        }
    }
}

#[test]
fn a_keyed_open_of_a_plaintext_database_names_the_recovery_path() {
    // The old error was "file is not a database: Error code 26", which gave
    // the user nothing to do (2026-11-25 audit, finding 7).
    let tmp = tempfile::tempdir().unwrap();
    let profile_dir = tmp.path().join("v");
    fs::create_dir_all(&profile_dir).unwrap();
    let mut cfg = ProfileConfig::new("v");
    cfg.save_to(&profile_dir).unwrap();
    fs::write(profile_dir.join("notes/a.md"), "# A\nbody\n").unwrap();

    // Plaintext database on disk, config says protected, key at hand —
    // exactly the interrupted-migration state.
    let mut storage = Storage::open(&profile_dir, &cfg).unwrap();
    storage.create_note("A", "body", None, None).unwrap();
    drop(storage);
    cfg.encryption = true;
    cfg.protection = cerebro::config::profile::ProtectionSection {
        kdf: Some(cerebro::crypto::KdfParams::generate()),
        wrapped_key_b64: Some("AAAA".to_string()),
        keychain_service: None,
    };

    let err = match Storage::open_with_key(&profile_dir, &cfg, Some(&cerebro::crypto::VaultKey::generate())) {
        Ok(_) => panic!("a keyed open against a plaintext database must fail"),
        Err(e) => e,
    };
    let message = format!("{err:#}");
    assert!(message.contains("did not finish"), "{message}");
    assert!(message.contains("vault decrypt"), "{message}");
    assert!(message.contains("delete db.sqlite"), "{message}");
}

#[test]
fn a_stray_plaintext_note_in_a_protected_vault_is_reencrypted_by_reindex() {
    // (2026-11-25 audit, finding 5): a plaintext .md inside a protected
    // vault was read as-is and stayed plaintext forever while `vault status`
    // reported ENABLED.
    let tmp = tempfile::tempdir().unwrap();
    let profile_dir = tmp.path().join("v");
    fs::create_dir_all(&profile_dir).unwrap();
    let mut cfg = ProfileConfig::new("v");
    cfg.save_to(&profile_dir).unwrap();
    let notes = profile_dir.join("notes");
    fs::write(notes.join("a.md"), "# A\nprotected body\n").unwrap();

    Storage::enable_protection(&profile_dir, &mut cfg, "long-enough-pass").unwrap();
    assert!(fs::read(notes.join("a.md")).unwrap().starts_with(b"CERE1"));

    // Drop a plaintext file into the protected vault from outside.
    fs::write(notes.join("stray.md"), "# Stray\nleaked-4f88 body\n").unwrap();
    assert!(fs::read(notes.join("stray.md")).unwrap().starts_with(b"# Stray"));

    let key = cerebro::crypto::VaultKey::unwrap_from_config(&cfg, "long-enough-pass").unwrap();
    let mut storage = Storage::open_with_key(&profile_dir, &cfg, Some(&key)).unwrap();
    let stats = storage.reindex_all().unwrap();
    assert_eq!(stats.created, 1);

    // The stray file is ciphertext now, and its content is still readable.
    assert!(
        fs::read(notes.join("stray.md")).unwrap().starts_with(b"CERE1"),
        "the stray plaintext file must be encrypted by the reindex"
    );
    assert!(storage.read_note("Stray").unwrap().body.contains("leaked-4f88"));
}

#[test]
fn weak_passphrases_are_refused_at_encryption_time() {
    // (2026-11-25 audit, finding 6): "", "1", "password" and whitespace were
    // all accepted and the vault was reported protected.
    let tmp = tempfile::tempdir().unwrap();
    let profile_dir = tmp.path().join("v");
    fs::create_dir_all(&profile_dir).unwrap();
    let mut cfg = ProfileConfig::new("v");
    cfg.save_to(&profile_dir).unwrap();
    fs::write(profile_dir.join("notes/a.md"), "# A\nbody\n").unwrap();

    for weak in ["", "   ", "1", "short"] {
        let mut try_cfg = cfg.clone();
        let err = Storage::enable_protection(&profile_dir, &mut try_cfg, weak).unwrap_err();
        assert!(format!("{err:#}").contains("at least 8 characters"), "passphrase {weak:?}: {err}");
        assert!(!try_cfg.encryption, "a refused passphrase must leave the vault plaintext");
    }

    // Exactly 8 characters is accepted, and the vault round-trips.
    Storage::enable_protection(&profile_dir, &mut cfg, "12345678").unwrap();
    Storage::disable_protection(&profile_dir, &mut cfg, "12345678").unwrap();
}
