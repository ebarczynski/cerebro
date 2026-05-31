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
