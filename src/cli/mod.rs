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

    /// Start MCP server (stdio)
    Mcp,

    /// Publish a note to Confluence
    Publish {
        /// Note title, path, or ID
        identifier: String,
        /// Dry run (show HTML without publishing)
        #[arg(long)]
        dry_run: bool,
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

// ── CLI Runner ───────────────────────────────────────────────────

pub fn run(cli: Cli) -> Result<()> {
    let config = match &cli.config {
        Some(path) => Config::load_from(path)?,
        None => Config::load_or_create()?,
    };

    let profile_name = cli.profile.as_deref()
        .or(config.active_profile.as_deref())
        .unwrap_or(&config.default_profile);

    match cli.command {
        Commands::Init { name } => cmd_init(&config, &name),
        Commands::Profile { action } => cmd_profile(&config, action),
        Commands::Notes(cmd) => {
            let mut storage = open_storage(&config, profile_name)?;
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
            let storage = open_storage(&config, profile_name)?;
            cmd_search(&storage, &query, limit)
        }
        Commands::Backlinks { identifier } => {
            let storage = open_storage(&config, profile_name)?;
            cmd_backlinks(&storage, &identifier)
        }
        Commands::Tags { tag } => {
            let storage = open_storage(&config, profile_name)?;
            cmd_tags(&storage, tag.as_deref())
        }
        Commands::Daily { date } => {
            let mut storage = open_storage(&config, profile_name)?;
            cmd_daily(&mut storage, &date, &config)
        }
        Commands::Capture { text } => {
            let mut storage = open_storage(&config, profile_name)?;
            cmd_capture(&mut storage, &text)
        }
        Commands::Reindex => {
            let mut storage = open_storage(&config, profile_name)?;
            cmd_reindex(&mut storage)
        }
        Commands::Mcp => {
            crate::mcp::McpServer::run(config)
        }
        Commands::Publish { identifier, dry_run } => {
            let storage = open_storage(&config, profile_name)?;
            cmd_publish(&storage, &identifier, dry_run)
        }
        Commands::Sync { action } => {
            cmd_sync(&config, profile_name, action)
        }
    }
}

// ── Command Implementations ──────────────────────────────────────

fn cmd_init(config: &Config, name: &str) -> Result<()> {
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

fn cmd_profile(config: &Config, action: ProfileCommands) -> Result<()> {
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
            if let Some(dn) = display_name {
                println!("  Display name: {}", dn);
            }
        }
        ProfileCommands::Switch { name } => {
            let profile_dir = config.profiles_dir.join(&name);
            if !profile_dir.exists() {
                return Err(anyhow!("Profile '{}' does not exist", name));
            }
            let mut config = config.clone();
            config.active_profile = Some(name.clone());
            let config_path = dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("cerebro")
                .join("config.toml");
            config.save_to(&config_path)?;
            println!("✓ Switched to profile '{}'", name);
        }
        ProfileCommands::Delete { name } => {
            if name == "personal" || name == "professional" {
                return Err(anyhow!("Cannot delete default profile '{}'", name));
            }
            let profile_dir = config.profiles_dir.join(&name);
            std::fs::remove_dir_all(&profile_dir)?;
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

    println!("{:<40} {:<30} {:<10} {}", "TITLE", "PATH", "WORDS", "UPDATED");
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
            println!("  ← {} (line {})", link.source_id, link.line_number.unwrap_or(0));
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
            let target = link.target_id.as_deref().unwrap_or("(unresolved)");
            println!("  → {} [[{}]] (line {})", target, link.target_ref, link.line_number.unwrap_or(0));
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
            println!("{:<30} {}", "TAG", "COUNT");
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
    let existing = storage.list_notes(None, 1000, 0)?
        .into_iter()
        .find(|n| n.title == title);

    match existing {
        Some(note) => {
            let nc = storage.read_note(&note.id)?;
            let edited = edit_in_editor(&nc.body, &config.editor)?;
            if edited != nc.body {
                storage.update_note(&note.id, &edited)?;
                println!("✓ Updated daily note for {}", date);
            } else {
                println!("Daily note for {} (no changes)", date);
            }
        }
        None => {
            let template = format!(
                "# Daily Note - {}\n\n## Tasks\n- \n\n## Notes\n\n## Ideas\n\n",
                date
            );
            let body = edit_in_editor(&template, &config.editor)?;
            let note = storage.create_note(&title, &body, Some(&format!("daily/{}.md", date)), Some(vec!["daily".to_string()]))?;
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

fn cmd_publish(storage: &Storage, identifier: &str, dry_run: bool) -> Result<()> {
    let nc = storage.read_note(identifier)?;
    println!("Publishing '{}' to Confluence...", nc.note.title);
    if dry_run {
        println!("DRY RUN - HTML preview:");
        println!("{}", nc.body);
    } else {
        println!("Note: Confluence publishing requires configuration. Run with --dry-run first.");
    }
    Ok(())
}

fn cmd_sync(config: &Config, profile_name: &str, action: SyncCommands) -> Result<()> {
    let profile_dir = config.profiles_dir.join(profile_name);
    let profile_config = ProfileConfig::load_or_default(&profile_dir, profile_name)?;
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

fn open_storage(config: &Config, profile_name: &str) -> Result<Storage> {
    let profile_dir = config.profiles_dir.join(profile_name);
    let profile_config = ProfileConfig::load_or_default(&profile_dir, profile_name)?;
    Storage::open(&profile_dir, &profile_config)
}

fn edit_in_editor(content: &str, editor: &str) -> Result<String> {
    let temp_dir = std::env::temp_dir();
    let temp_path = temp_dir.join("cerebro_edit.md");
    std::fs::write(&temp_path, content)?;

    let status = std::process::Command::new(editor)
        .arg(&temp_path)
        .status()?;

    if !status.success() {
        return Err(anyhow!("Editor exited with error"));
    }

    Ok(std::fs::read_to_string(&temp_path)?)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}
