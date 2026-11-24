//! CLI interface for Cerebro.
//!
//! Provides all user-facing commands via clap derive macros.

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};

use crate::config::app::Config;
use crate::config::profile::ProfileConfig;
use crate::storage::Storage;

#[derive(Parser)]
#[command(name = "cerebro")]
#[command(about = "A fast, secure, local-first personal knowledge base")]
#[command(version)]
pub struct Cli {
    /// Profile to use
    #[arg(long, short, global = true)]
    pub profile: Option<String>,

    /// Config file path
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize a new Cerebro vault
    Init {
        /// Profile name (default: "personal")
        #[arg(long, default_value = "personal")]
        name: String,
    },

    /// Manage profiles
    Profile {
        #[command(subcommand)]
        action: ProfileCommands,
    },

    /// Note management
    #[command(subcommand)]
    Notes(NoteCommands),

    /// Search notes
    Search {
        /// Search query (FTS5 syntax)
        query: String,
        /// Maximum results
        #[arg(long, short, default_value = "10")]
        limit: i64,
    },

    /// Show backlinks for a note
    Backlinks {
        /// Note title, path, or ID
        identifier: String,
    },

    /// List all tags
    Tags {
        /// Show notes for a specific tag
        tag: Option<String>,
    },

    /// Daily note (today/yesterday/date)
    Daily {
        /// Date (today, yesterday, or YYYY-MM-DD)
        #[arg(default_value = "today")]
        date: String,
    },

    /// Quick capture to inbox
    Capture {
        /// Text to capture
        text: String,
    },

    /// Reindex all notes from filesystem
    Reindex,

    /// Import a document as a note (PDF, DOCX, PPTX, XLSX, ODS, XLS, ODT, HTML,
    /// Markdown, JSON, or a text/source file). Legacy .doc needs LibreOffice.
    Import {
        /// Path to the file to import
        file: PathBuf,
        /// Custom title (default: filename without extension)
        #[arg(long, short)]
        title: Option<String>,
        /// Tags to add
        #[arg(long, short, value_delimiter = ',')]
        tags: Vec<String>,
        /// Directory path within vault
        #[arg(long)]
        path: Option<String>,
        /// Dry run (show markdown without creating note)
        #[arg(long)]
        dry_run: bool,
    },

    /// Start MCP server (stdio)
    Mcp,

    /// Knowledge Palace semantic search
    Kp {
        #[command(subcommand)]
        action: KpCommands,
    },

    /// Sync vault with Git remote
    Sync {
        #[command(subcommand)]
        action: SyncCommands,
    },
}

#[derive(Subcommand)]
pub enum ProfileCommands {
    /// List all profiles
    List,
    /// Create a new profile
    Create {
        /// Profile name
        name: String,
        /// Display name
        #[arg(long)]
        display_name: Option<String>,
    },
    /// Switch active profile
    Switch {
        /// Profile name
        name: String,
    },
    /// Delete a profile
    Delete {
        /// Profile name
        name: String,
    },
}

#[derive(Subcommand)]
pub enum NoteCommands {
    /// Create a new note
    Create {
        /// Note title
        title: String,
        /// Directory path within vault
        #[arg(long)]
        path: Option<String>,
        /// Tags
        #[arg(long, short, value_delimiter = ',')]
        tags: Vec<String>,
        /// Content (omit to open editor)
        #[arg(long)]
        content: Option<String>,
    },
    /// List notes
    List {
        /// Filter by tag
        #[arg(long, short)]
        tag: Option<String>,
        /// Max results
        #[arg(long, default_value = "20")]
        limit: i64,
        /// Offset
        #[arg(long, default_value = "0")]
        offset: i64,
    },
    /// Show a note
    Show {
        /// Note title, path, or ID
        identifier: String,
    },
    /// Edit a note in your editor
    Edit {
        /// Note title, path, or ID
        identifier: String,
    },
    /// Delete a note
    Delete {
        /// Note title, path, or ID
        identifier: String,
    },
}

#[derive(Subcommand)]
pub enum SyncCommands {
    /// Show sync status
    Status,
    /// Commit and push changes
    Push,
    /// Pull changes from remote
    Pull,
}

#[derive(Subcommand)]
pub enum KpCommands {
    /// Search via Knowledge Palace (falls back to local FTS if KP is down)
    Search {
        /// Search query
        query: String,
        /// Search mode: semantic, keyword, hybrid
        #[arg(long, default_value = "hybrid")]
        mode: String,
        /// Maximum results
        #[arg(long, short, default_value = "5")]
        limit: i64,
    },
    /// Check KP reachability
    Health,
}

// ── CLI Runner ───────────────────────────────────────────────────

pub fn run(cli: Cli) -> Result<()> {
    let config = match &cli.config {
        Some(path) => Config::load_from(path)?,
        None => Config::load_or_create()?,
    };
    // Where `profile switch` records its choice: the file this run loaded.
    let config_file = match &cli.config {
        Some(path) => path.clone(),
        None => crate::config::app::config_path()?,
    };
    // The profile every vault command acts on. A profile named on the
    // command line must exist; the default one is created on first use.
    let requested = cli.profile.as_deref();

    match cli.command {
        Commands::Init { name } => cmd_init(&config, &name),
        Commands::Profile { action } => cmd_profile(&config, action, &config_file),
        Commands::Notes(cmd) => {
            let mut storage = config.open_storage(requested)?;
            match cmd {
                NoteCommands::Create { title, path, tags, content } => {
                    cmd_note_create(&mut storage, &title, path.as_deref(), &tags, content.as_deref(), &config)
                }
                NoteCommands::List { tag, limit, offset } => {
                    cmd_note_list(&storage, tag.as_deref(), limit, offset)
                }
                NoteCommands::Show { identifier } => {
                    cmd_note_show(&storage, &identifier)
                }
                NoteCommands::Edit { identifier } => {
                    cmd_note_edit(&mut storage, &identifier, &config)
                }
                NoteCommands::Delete { identifier } => {
                    cmd_note_delete(&mut storage, &identifier)
                }
            }
        }
        Commands::Search { query, limit } => {
            let storage = config.open_storage(requested)?;
            cmd_search(&storage, &query, limit)
        }
        Commands::Backlinks { identifier } => {
            let storage = config.open_storage(requested)?;
            cmd_backlinks(&storage, &identifier)
        }
        Commands::Tags { tag } => {
            let storage = config.open_storage(requested)?;
            cmd_tags(&storage, tag.as_deref())
        }
        Commands::Daily { date } => {
            let mut storage = config.open_storage(requested)?;
            cmd_daily(&mut storage, &date, &config)
        }
        Commands::Capture { text } => {
            let mut storage = config.open_storage(requested)?;
            cmd_capture(&mut storage, &text)
        }
        Commands::Reindex => {
            let mut storage = config.open_storage(requested)?;
            cmd_reindex(&mut storage)
        }
        Commands::Import { file, title, tags, path, dry_run } => {
            let mut storage = config.open_storage(requested)?;
            cmd_import(&mut storage, &file, title.as_deref(), &tags, path.as_deref(), dry_run)
        }
        // `cerebro --profile work mcp` serves `work` unless a call names a profile.
        Commands::Mcp => crate::mcp::McpServer::run(config, cli.profile.clone()),
        Commands::Kp { action } => {
            let storage = config.open_storage(requested)?;
            cmd_kp(&config, &storage, action)
        }
        Commands::Sync { action } => {
            let target = config.profile_target(requested)?;
            cmd_sync(&target.name, &target.dir, action)
        }
    }
}

// ── Command Implementations ──────────────────────────────────────

fn cmd_init(config: &Config, name: &str) -> Result<()> {
    ProfileConfig::check_name(name)?;
    let profile_dir = config.profiles_dir.join(name);
    std::fs::create_dir_all(&profile_dir)?;

    let profile_config = ProfileConfig::new(name);
    profile_config.save(&profile_dir)?;

    // Create notes directory
    std::fs::create_dir_all(profile_dir.join(&profile_config.notes_dir))?;

    println!("✓ Initialized profile '{}' at {:?}", name, profile_dir);
    println!("  Notes directory: {:?}", profile_dir.join(&profile_config.notes_dir));
    Ok(())
}

fn cmd_profile(config: &Config, action: ProfileCommands, config_file: &std::path::Path) -> Result<()> {
    match action {
        ProfileCommands::List => {
            println!("Profiles:");
            let entries = std::fs::read_dir(&config.profiles_dir)?;
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let active = config.active_profile.as_deref() == Some(&name)
                        || (config.active_profile.is_none() && name == config.default_profile);
                    let marker = if active { " ← active" } else { "" };
                    println!("  {}{}", name, marker);
                }
            }
        }
        ProfileCommands::Create { name, display_name } => {
            cmd_init(config, &name)?;
            if let Some(display_name) = display_name {
                let profile_dir = config.profiles_dir.join(&name);
                let mut profile_config = ProfileConfig::load(&profile_dir)?;
                profile_config.display_name = display_name.clone();
                profile_config.save(&profile_dir)?;
                println!("  Display name: {}", display_name);
            }
        }
        ProfileCommands::Switch { name } => {
            // Must exist: switching to a misspelled profile would otherwise
            // report success and change nothing.
            config.profile_target(Some(&name))?;
            let mut config = config.clone();
            config.active_profile = Some(name.clone());
            // The same file every later run reads (NOT the platform's
            // Application Support directory, which nothing reads).
            config.save_to(config_file)?;
            println!("✓ Switched to profile '{}'", name);
        }
        ProfileCommands::Delete { name } => {
            if name == "personal" || name == "professional" {
                return Err(anyhow!("Cannot delete default profile '{}'", name));
            }
            let target = config.profile_target(Some(&name))?;
            println!("Deleting '{}' at {:?} — this removes every note in it", name, target.dir);
            std::fs::remove_dir_all(&target.dir)?;
            println!("✓ Deleted profile '{}'", name);
        }
    }
    Ok(())
}

fn cmd_note_create(
    storage: &mut Storage,
    title: &str,
    path: Option<&str>,
    tags: &[String],
    content: Option<&str>,
    config: &Config,
) -> Result<()> {
    let body = match content {
        Some(c) => c.to_string(),
        None => {
            // Open editor with template
            let template = format!("# {}\n\n", title);
            edit_in_editor(&template, &config.editor)?
        }
    };

    let note = storage.create_note(title, &body, path, Some(tags.to_vec()))?;
    println!("✓ Created note: {} ({})", note.title, note.path);
    println!("  ID: {}", note.id);
    println!("  Words: {}", note.word_count);
    println!("  Tags: {:?}", note.tags);
    Ok(())
}

fn cmd_note_list(storage: &Storage, tag: Option<&str>, limit: i64, offset: i64) -> Result<()> {
    let notes = storage.list_notes(tag, limit, offset)?;
    if notes.is_empty() {
        println!("No notes found.");
        return Ok(());
    }

    let (title, path, words) = ("TITLE", "PATH", "WORDS");
    println!("{title:<40} {path:<30} {words:<10} UPDATED");
    println!("{}", "-".repeat(100));
    for note in &notes {
        let title = truncate(&note.title, 38);
        let path = truncate(&note.path, 28);
        println!("{:<40} {:<30} {:<10} {}", title, path, note.word_count, note.updated_at);
    }
    println!("\n{} notes shown", notes.len());
    Ok(())
}

fn cmd_note_show(storage: &Storage, identifier: &str) -> Result<()> {
    let nc = storage.read_note(identifier)?;
    println!("{}", nc.body);
    Ok(())
}

fn cmd_note_edit(storage: &mut Storage, identifier: &str, config: &Config) -> Result<()> {
    let nc = storage.read_note(identifier)?;
    let edited = edit_in_editor(&nc.body, &config.editor)?;
    if edited != nc.body {
        let note = storage.update_note(identifier, &edited)?;
        println!("✓ Updated: {} ({} words)", note.title, note.word_count);
    } else {
        println!("No changes.");
    }
    Ok(())
}

fn cmd_note_delete(storage: &mut Storage, identifier: &str) -> Result<()> {
    storage.delete_note(identifier)?;
    println!("✓ Deleted note: {}", identifier);
    Ok(())
}

fn cmd_search(storage: &Storage, query: &str, limit: i64) -> Result<()> {
    let results = storage.search(query, limit)?;
    if results.is_empty() {
        println!("No results for '{}'.", query);
        return Ok(());
    }

    for (i, r) in results.iter().enumerate() {
        println!("{}. {} [{}]", i + 1, r.title, r.path);
        println!("   Score: {:.4} | Tags: {:?}", r.rank, r.tags);
        println!("   {}", truncate(&r.snippet, 100));
        println!();
    }
    println!("{} results", results.len());
    Ok(())
}

fn cmd_backlinks(storage: &Storage, identifier: &str) -> Result<()> {
    let backlinks = storage.get_backlinks(identifier)?;
    let forward_links = storage.get_forward_links(identifier)?;

    println!("Backlinks TO '{}':", identifier);
    if backlinks.is_empty() {
        println!("  (none)");
    } else {
        for link in &backlinks {
            let source = match storage.note_title(&link.source_id) {
                Some((title, path)) => format!("{title} [{path}]"),
                None => link.source_id.clone(),
            };
            println!("  ← {source} (line {})", link.line_number.unwrap_or(0));
            if let Some(ctx) = &link.context {
                println!("    {}", truncate(ctx, 80));
            }
        }
    }

    println!("\nForward links FROM '{}':", identifier);
    if forward_links.is_empty() {
        println!("  (none)");
    } else {
        for link in &forward_links {
            // The note's title when the link resolves, and why not when it does not.
            let target = match &link.target_id {
                Some(id) => match storage.note_title(id) {
                    Some((title, path)) => format!("{title} [{path}]"),
                    None => id.clone(),
                },
                None => "(unresolved)".to_string(),
            };
            println!("  → {target} [[{}]] (line {})", link.target_ref, link.line_number.unwrap_or(0));
        }
    }

    Ok(())
}

fn cmd_tags(storage: &Storage, tag: Option<&str>) -> Result<()> {
    match tag {
        Some(t) => {
            let notes = storage.notes_for_tag(t)?;
            println!("Notes tagged '{}':", t);
            for note in &notes {
                println!("  {} [{}]", note.title, note.path);
            }
            println!("{} notes", notes.len());
        }
        None => {
            let tags = storage.list_tags()?;
            if tags.is_empty() {
                println!("No tags found.");
                return Ok(());
            }
            let (tag, count) = ("TAG", "COUNT");
            println!("{tag:<30} {count}");
            println!("{}", "-".repeat(40));
            for (name, count) in &tags {
                println!("{:<30} {}", name, count);
            }
        }
    }
    Ok(())
}

fn cmd_daily(storage: &mut Storage, date_str: &str, config: &Config) -> Result<()> {
    let date = match date_str {
        "today" => chrono::Local::now().format("%Y-%m-%d").to_string(),
        "yesterday" => {
            let yesterday = chrono::Local::now() - chrono::Duration::days(1);
            yesterday.format("%Y-%m-%d").to_string()
        }
        other => other.to_string(),
    };

    let title = format!("Daily Note - {}", date);
    let path = format!("daily/{}.md", date);

    // Look the day's note up by its path. (It used to be searched among the
    // 1000 most recently updated notes, and recreated — overwriting the
    // file — once more than a thousand notes were newer.)
    match storage.read_note(&path) {
        Ok(nc) => {
            let edited = edit_in_editor(&nc.body, &config.editor)?;
            if edited != nc.body {
                storage.update_note(&nc.note.id, &edited)?;
                println!("✓ Updated daily note for {}", date);
            } else {
                println!("Daily note for {} (no changes)", date);
            }
        }
        Err(_) => {
            let template = format!(
                "# Daily Note - {}\n\n## Tasks\n- \n\n## Notes\n\n## Ideas\n\n",
                date
            );
            let body = edit_in_editor(&template, &config.editor)?;
            let note = storage.create_note(&title, &body, Some(&path), Some(vec!["daily".to_string()]))?;
            println!("✓ Created daily note for {} ({})", date, note.path);
        }
    }
    Ok(())
}

fn cmd_capture(storage: &mut Storage, text: &str) -> Result<()> {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    let title = format!("Capture - {}", now);
    let content = format!("{}\n\nCaptured: {}\n", text, now);
    let note = storage.create_note(&title, &content, Some("inbox/"), Some(vec!["inbox".to_string()]))?;
    println!("✓ Captured to: {} ({})", note.title, note.path);
    Ok(())
}

fn cmd_reindex(storage: &mut Storage) -> Result<()> {
    println!("Reindexing vault '{}'...", storage.profile_name());
    let stats = storage.reindex_all()?;
    println!("✓ Reindex complete:");
    println!("  Created: {}", stats.created);
    println!("  Updated: {}", stats.updated);
    println!("  Removed: {}", stats.removed);
    if stats.errors > 0 {
        println!("  Errors: {}", stats.errors);
    }
    Ok(())
}

fn cmd_import(
    storage: &mut Storage,
    file: &PathBuf,
    title: Option<&str>,
    tags: &[String],
    path: Option<&str>,
    dry_run: bool,
) -> Result<()> {
    println!("Importing {:?}...", file);

    let result = crate::import::import_file(file)?;

    let final_title = title.unwrap_or(&result.title);
    let mut all_tags = result.tags;
    all_tags.extend(tags.iter().cloned());

    if dry_run {
        println!("--- DRY RUN ---");
        println!("Title: {}", final_title);
        println!("Format: {}", result.format.name());
        println!("Tags: {:?}", all_tags);
        if let Some(p) = path {
            println!("Path: {}", p);
        }
        println!("\n--- Markdown Preview ---\n");
        println!("{}", result.markdown);
        return Ok(());
    }

    let note = storage.create_note(
        final_title,
        &result.markdown,
        path,
        Some(all_tags),
    )?;

    println!("✓ Imported {} as note: {}", result.format.name(), note.title);
    println!("  ID: {}", note.id);
    println!("  Path: {}", note.path);
    println!("  Words: {}", note.word_count);
    println!("  Tags: {:?}", note.tags);
    Ok(())
}

fn cmd_kp(config: &Config, storage: &Storage, action: KpCommands) -> Result<()> {
    let kp_config = crate::kp::KpConfig {
        enabled: config.knowledge_palace.enabled,
        endpoint: if config.knowledge_palace.endpoint.is_empty() {
            "http://localhost:8080".to_string()
        } else {
            config.knowledge_palace.endpoint.clone()
        },
    };
    let connector = crate::kp::KpConnector::new(kp_config);

    // KP calls are async; the CLI loop is sync — same throwaway-runtime
    // pattern as the MCP server.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    match action {
        KpCommands::Health => {
            let endpoint = &config.knowledge_palace.endpoint;
            match rt.block_on(connector.health_check()) {
                Ok(true) => println!("✓ Knowledge Palace healthy at {endpoint}"),
                Ok(false) if !config.knowledge_palace.enabled => {
                    println!("✗ Knowledge Palace integration is disabled in config.toml")
                }
                Ok(false) => println!("✗ Knowledge Palace did not report healthy at {endpoint}"),
                Err(e) => println!("✗ Knowledge Palace unreachable at {endpoint} ({e})"),
            }
        }
        KpCommands::Search { query, mode, limit } => {
            let hits = match rt.block_on(connector.search_with_mode(&query, &mode, limit)) {
                Ok(h) => h,
                Err(e) => {
                    println!("⚠ KP unavailable ({}); falling back to local FTS", e);
                    let local = storage.search(&query, limit)?;
                    for (i, r) in local.iter().enumerate() {
                        println!("{}. {} [{}] (score: {:.4})", i + 1, r.title, r.path, r.rank);
                        println!("   {}", truncate(&r.snippet, 100));
                    }
                    println!("{} local results", local.len());
                    return Ok(());
                }
            };
            if hits.is_empty() {
                println!("No KP results for '{}'.", query);
                return Ok(());
            }
            for (i, h) in hits.iter().enumerate() {
                println!(
                    "{}. {} (score: {:.3}, source: {})",
                    i + 1,
                    h.title.as_deref().unwrap_or("(untitled)"),
                    h.score,
                    h.source.as_deref().unwrap_or("unknown")
                );
                println!("   {}", truncate(&h.content, 120));
                if !h.tags.is_empty() {
                    println!("   Tags: {:?}", h.tags);
                }
            }
            println!("{} KP results (mode: {})", hits.len(), mode);
        }
    }
    Ok(())
}

fn cmd_sync(profile_name: &str, profile_dir: &std::path::Path, action: SyncCommands) -> Result<()> {
    let profile_config = ProfileConfig::load_or_default(profile_dir, profile_name)?;
    let sync = crate::sync::GitSync::new(
        profile_dir.join(&profile_config.notes_dir),
        crate::sync::SyncConfig {
            method: match profile_config.sync.method {
                crate::config::profile::SyncMethod::Git => "git".to_string(),
                crate::config::profile::SyncMethod::None => "none".to_string(),
            },
            remote: if profile_config.sync.remote.is_empty() { None } else { Some(profile_config.sync.remote.clone()) },
            branch: Some(profile_config.sync.branch.clone()),
            auto_commit: false,
            commit_interval_minutes: 5,
        },
    );

    match action {
        SyncCommands::Status => {
            println!("{}", sync.status()?);
        }
        SyncCommands::Push => {
            sync.init()?;
            let msg = format!("cerebro sync {}", chrono::Local::now().format("%Y-%m-%d %H:%M"));
            println!("{}", sync.commit(&msg)?);
            println!("{}", sync.push()?);
        }
        SyncCommands::Pull => {
            sync.init()?;
            println!("{}", sync.pull()?);
        }
    }
    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────

fn edit_in_editor(content: &str, editor: &str) -> Result<String> {
    // A private copy of the note that disappears afterwards. (One fixed
    // path in the temp directory used to be shared by every edit, and the
    // note's text stayed there.)
    let mut draft = tempfile::Builder::new()
        .prefix("cerebro-")
        .suffix(".md")
        .tempfile()?;
    std::io::Write::write_all(&mut draft, content.as_bytes())?;

    // "code -w" is a program and an argument, not a program called "code -w".
    let mut words = editor.split_whitespace();
    let program = words.next().ok_or_else(|| anyhow!("editor command is empty"))?;
    let status = std::process::Command::new(program)
        .args(words)
        .arg(draft.path())
        .status()?;

    if !status.success() {
        return Err(anyhow!("Editor exited with error"));
    }

    Ok(std::fs::read_to_string(draft.path())?)
}

fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.len() <= max {
        return s.to_string();
    }
    // Cut on a CHAR boundary, never mid-multibyte-character — byte slicing
    // at an arbitrary offset panicked on Polish/Cyrillic titles
    // (grill finding F3).
    let mut cut = max.saturating_sub(1);
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &s[..cut])
}

#[cfg(test)]
mod truncate_tests {
    use super::truncate;

    #[test]
    fn ascii_truncated_with_ellipsis() {
        assert_eq!(truncate("hello world this is long", 10), "hello wor…");
    }

    #[test]
    fn short_string_untouched() {
        assert_eq!(truncate("short", 10), "short");
    }

    #[test]
    fn polish_title_does_not_panic() {
        // Grill finding F3: byte 37 previously landed inside 'ą'
        let title = format!("{}ąąą longer title", "a".repeat(36));
        let out = truncate(&title, 38);
        assert!(out.ends_with('…'));
        assert!(out.len() < title.len());
    }

    #[test]
    fn emoji_cut_is_char_safe() {
        let s = "🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀";
        let out = truncate(s, 10);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn zero_max_is_empty() {
        assert_eq!(truncate("anything", 0), "");
    }
}
