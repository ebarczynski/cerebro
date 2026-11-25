//! Cerebro GUI — Tauri 2.0 desktop application.
//!
//! Exposes the Cerebro core library (storage, search, links, tags,
//! Knowledge Palace connector) as Tauri commands consumed by the
//! React/TypeScript frontend.
//!
//! # Threading
//!
//! Tauri runs a plain `fn` command on the main thread; only `async fn`
//! commands are moved off it. So every command that can take longer than a
//! few milliseconds — opening or reindexing a vault, converting documents,
//! importing an Obsidian vault, building the graph, computing auto-links —
//! is `async` and does its work on the blocking pool
//! ([`tauri::async_runtime::spawn_blocking`]), holding a *handle* to the
//! vault rather than the state lock. Quick commands keep working while a
//! long job runs: they take the handle, do their query, and let go.

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, LockResult};
use std::sync::atomic::{AtomicU64, Ordering};

use notify::{Config as NotifyConfig, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::{FileDialogBuilder, DialogExt};

use cerebro::config::app::Config;
use cerebro::config::profile::{ProfileConfig, VaultKind};
use cerebro::crypto::VaultKey;
use cerebro::storage::{Note, NoteContent, SearchResult, Storage, VAULT_LOCKED_PREFIX};

// ── Application State ──────────────────────────────────────────

/// The open vault, behind a handle that long-running work can keep on a
/// blocking thread while quick commands still get through between steps.
pub type SharedStorage = Arc<Mutex<Storage>>;

pub struct AppState {
    pub config: Mutex<Config>,
    pub storage: Mutex<Option<SharedStorage>>,
    pub profile_config: Mutex<Option<ProfileConfig>>,
    pub watcher: Mutex<Option<RecommendedWatcher>>,
    /// Path of the currently open vault directory (None until a vault is opened).
    pub open_vault_path: Mutex<Option<String>>,
    /// Unlocked master key of the current vault (ADR 0006). Held ONLY while
    /// the session is unlocked; dropped by lock_vault and on relock.
    pub vault_key: Mutex<Option<VaultKey>>,
}

const NO_VAULT: &str = "No vault open — open a vault first";

/// A poisoned lock is reported to the frontend like any other failure.
fn lock<T>(result: LockResult<T>) -> Result<T, String> {
    result.map_err(|e| e.to_string())
}

/// A handle to the open vault, for work that may take a while.
fn storage_handle(state: &AppState) -> Result<SharedStorage, String> {
    lock(state.storage.lock())?
        .clone()
        .ok_or_else(|| NO_VAULT.to_string())
}

/// Run a quick read-only command against the open vault.
fn with_storage<T>(state: &AppState, read: impl FnOnce(&Storage) -> Result<T, String>) -> Result<T, String> {
    let handle = storage_handle(state)?;
    let storage = lock(handle.lock())?;
    read(&storage)
}

/// Run a quick command that changes the open vault.
fn with_storage_mut<T>(state: &AppState, write: impl FnOnce(&mut Storage) -> Result<T, String>) -> Result<T, String> {
    let handle = storage_handle(state)?;
    let mut storage = lock(handle.lock())?;
    write(&mut storage)
}

/// Do `work` on the blocking pool instead of the main thread.
async fn off_main_thread<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| format!("Background task failed: {e}"))?
}

/// Make a freshly opened vault the current one. `key` is the unlocked master
/// key of a protected vault, `None` for a plaintext one — so switching from a
/// protected vault to another vault never leaves the old key behind.
fn install_vault(
    state: &AppState,
    storage: Storage,
    profile_config: ProfileConfig,
    key: Option<VaultKey>,
    path: String,
) -> Result<(), String> {
    *lock(state.storage.lock())? = Some(Arc::new(Mutex::new(storage)));
    *lock(state.profile_config.lock())? = Some(profile_config);
    *lock(state.vault_key.lock())? = key;
    *lock(state.open_vault_path.lock())? = Some(path);
    Ok(())
}

fn describe_open_error(error: anyhow::Error, profile_dir: &Path) -> String {
    let message = format!("{error:#}");
    if message.contains("Permission denied") || message.contains("permission denied") {
        format!(
            "Permission denied opening database at {}. Check that you own this directory and the file is not locked.",
            profile_dir.join("db.sqlite").display()
        )
    } else {
        message
    }
}

/// Open a vault directory and bring its index in line with its files.
///
/// A plain function of the directory — no app state — so it can run on a
/// blocking thread and be unit-tested.
pub fn load_vault(
    profile_dir: &Path,
    profile_config: &ProfileConfig,
) -> Result<(Storage, usize, usize), String> {
    load_vault_with_key(profile_dir, profile_config, None)
}

/// [`load_vault`] for a protected vault, opened with its unlocked master key
/// (ADR 0006). Without a key such a vault is refused — `VAULT LOCKED`.
pub fn load_vault_with_key(
    profile_dir: &Path,
    profile_config: &ProfileConfig,
    key: Option<&VaultKey>,
) -> Result<(Storage, usize, usize), String> {
    let mut storage =
        Storage::open_with_key(profile_dir, profile_config, key).map_err(|e| describe_open_error(e, profile_dir))?;
    // Reindex all notes on open.
    let stats = storage.reindex_all().map_err(|e| format!("Indexing the vault failed: {e:#}"))?;
    let note_count = storage.list_notes(None, 10_000, 0).map_err(|e| e.to_string())?.len();
    let tag_count = storage.list_tags().map_err(|e| e.to_string())?.len();
    if stats.errors > 0 {
        tracing::warn!("{} note files could not be read while indexing {}", stats.errors, profile_dir.display());
    }
    Ok((storage, note_count, tag_count))
}

/// Resolve a path that came from the frontend — ultimately from note content
/// such as `![](attachments/x.png)` — to a file *inside* the vault.
///
/// The lexical check stops `..` and absolute paths; when the file exists, a
/// symlink inside the vault pointing out of it is caught as well.
pub fn resolve_vault_path(notes_dir: &Path, user_path: &str) -> Result<PathBuf, String> {
    let user_path = user_path.trim();
    let relative = Path::new(user_path);
    if user_path.is_empty() || relative.is_absolute() {
        return Err(format!("Path must be inside the vault: {user_path}"));
    }

    let mut depth: i32 = 0;
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return Err(format!("Path escapes the vault: {user_path}"));
                }
            }
            Component::Normal(_) => depth += 1,
            _ => return Err(format!("Invalid path: {user_path}")),
        }
    }

    let full = notes_dir.join(relative);
    if full.exists() {
        let (Ok(file), Ok(root)) = (std::fs::canonicalize(&full), std::fs::canonicalize(notes_dir)) else {
            return Ok(full);
        };
        if !file.starts_with(&root) {
            return Err(format!("Path escapes the vault: {user_path}"));
        }
    }
    Ok(full)
}

/// The part of `body` that belongs to `heading`: from that heading up to the
/// next heading of the same or a higher level. `None` when there is no such
/// heading.
pub fn section_under_heading(body: &str, heading: &str) -> Option<String> {
    let level_of = |line: &str| -> Option<usize> {
        let hashes = line.chars().take_while(|c| *c == '#').count();
        ((1..=6).contains(&hashes) && line[hashes..].starts_with(' ')).then_some(hashes)
    };

    let mut lines = body.lines().skip_while(|line| {
        level_of(line).is_none_or(|level| line[level..].trim() != heading.trim())
    });
    let start = lines.next()?;
    let level = level_of(start)?;
    let mut section = vec![start.to_string()];
    for line in lines {
        if level_of(line).is_some_and(|next| next <= level) {
            break;
        }
        section.push(line.to_string());
    }
    // Without the blank line(s) that separate the section from the next heading.
    Some(section.join("\n").trim_end().to_string())
}

// ── DTOs for the frontend ──────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct ProfileInfo {
    pub name: String,
    pub display_name: String,
    pub is_active: bool,
    pub notes_dir: String,
    pub encryption: bool,
    /// "private" | "business" — ownership tag from vault.toml.
    pub kind: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateNoteRequest {
    pub title: String,
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
    pub folder: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TagInfo {
    pub name: String,
    pub count: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub title: String,
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DailyNoteResponse {
    pub path: String,
    pub content: String,
    pub exists: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VaultInfo {
    pub name: String,
    pub path: String,
    pub note_count: usize,
    pub tag_count: usize,
    /// "private" | "business" — ownership tag from vault.toml.
    pub kind: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TemplateInfo {
    pub name: String,
    pub path: String,
}

// ── Vault Management ───────────────────────────────────────────

#[tauri::command]
async fn init_vault(
    state: State<'_, AppState>,
    path: String,
    profile: Option<String>,
) -> Result<VaultInfo, String> {
    let profile_name = profile.unwrap_or_else(|| "personal".to_string());
    let profile_dir = PathBuf::from(&path);
    let open_name = profile_name.clone();

    let (storage, profile_config, note_count, tag_count) = off_main_thread(move || {
        std::fs::create_dir_all(&profile_dir).map_err(|e| e.to_string())?;

        // Create/load profile config
        let profile_config = ProfileConfig::load_or_default(&profile_dir, &open_name)
            .map_err(|e| e.to_string())?;
        profile_config.save_to(&profile_dir).map_err(|e| e.to_string())?;

        // Initialize storage
        let storage = Storage::open(&profile_dir, &profile_config).map_err(|e| describe_open_error(e, &profile_dir))?;
        let note_count = storage.list_notes(None, 10_000, 0).map_err(|e| e.to_string())?.len();
        let tag_count = storage.list_tags().map_err(|e| e.to_string())?.len();
        Ok((storage, profile_config, note_count, tag_count))
    })
    .await?;

    // The kind of the vault being opened — not of whichever was open before.
    let info = VaultInfo {
        name: profile_name,
        path,
        note_count,
        tag_count,
        kind: profile_config.kind.as_tag().to_string(),
    };
    install_vault(&state, storage, profile_config, None, info.path.clone())?;
    Ok(info)
}

/// The master key of a vault the user is opening (ADR 0006): from the
/// passphrase that was typed, else from the OS keychain convenience cache —
/// never a prompt from the backend.
///
/// `Ok(None)` for a plaintext vault. A protected vault that still needs its
/// passphrase is an error starting with `VAULT LOCKED`; the frontend answers
/// it with the unlock dialog.
pub fn unlock_key(profile_config: &ProfileConfig, passphrase: Option<&str>) -> Result<Option<VaultKey>, String> {
    if !profile_config.encryption {
        return Ok(None);
    }
    let unlocked = match passphrase {
        Some(passphrase) => VaultKey::unwrap_from_config(profile_config, passphrase)
            .map_err(|e| format!("wrong passphrase: {e}")),
        None => {
            let from_keychain = profile_config
                .protection
                .keychain_service
                .as_ref()
                .and_then(|service| VaultKey::from_keychain(service).ok().flatten());
            match from_keychain {
                Some(key) => Ok(key),
                None => Err(format!(
                    "{}: vault '{}' is locked — passphrase required",
                    VAULT_LOCKED_PREFIX, profile_config.name
                )),
            }
        }
    };
    unlocked.map(Some)
}

#[tauri::command]
async fn open_vault(
    state: State<'_, AppState>,
    path: String,
    profile: Option<String>,
    passphrase: Option<String>,
) -> Result<VaultInfo, String> {
    let profile_name = profile.unwrap_or_else(|| "personal".to_string());
    let profile_dir = PathBuf::from(&path);

    if !profile_dir.exists() {
        return Err(format!("Vault not found at {path}"));
    }

    // Reindexing a large vault takes seconds: it happens on a blocking
    // thread, and the vault that was open stays usable until it is done.
    let open_name = profile_name.clone();
    let (storage, profile_config, key, note_count, tag_count) =
        off_main_thread(move || {
            let profile_config =
                ProfileConfig::load(&profile_dir).unwrap_or_else(|_| ProfileConfig::new(&open_name));
            let key = unlock_key(&profile_config, passphrase.as_deref())?;
            let (storage, note_count, tag_count) =
                load_vault_with_key(&profile_dir, &profile_config, key.as_ref())?;
            Ok((storage, profile_config, key, note_count, tag_count))
        })
        .await?;

    // The kind of the vault being opened — not of whichever was open before.
    let info = VaultInfo {
        name: profile_name,
        path: path.clone(),
        note_count,
        tag_count,
        kind: profile_config.kind.as_tag().to_string(),
    };
    install_vault(&state, storage, profile_config, key, path)?;
    Ok(info)
}

/// Unlock flow: identical to open_vault with a passphrase — exists so the
/// frontend lock/unlock UI has an explicit command.
#[tauri::command]
async fn unlock_vault(
    state: State<'_, AppState>,
    path: String,
    profile: Option<String>,
    passphrase: String,
) -> Result<VaultInfo, String> {
    open_vault(state, path, profile, Some(passphrase)).await
}

/// Relock: drop the storage handle and the in-memory key. The vault becomes
/// locked for every subsequent operation until unlocked again.
#[tauri::command]
fn lock_vault(state: State<AppState>) -> Result<(), String> {
    let mut storage_guard = lock(state.storage.lock())?;
    *storage_guard = None;
    let mut key_guard = lock(state.vault_key.lock())?;
    *key_guard = None;
    Ok(())
}

/// True when the current vault is protected (frontend shows the lock UI).
#[tauri::command]
fn vault_is_protected(state: State<AppState>) -> Result<bool, String> {
    let pc_guard = lock(state.profile_config.lock())?;
    Ok(pc_guard.as_ref().map(|pc| pc.encryption).unwrap_or(false))
}

// ── Profile Management ─────────────────────────────────────────

#[tauri::command]
fn home_dir() -> Result<String, String> {
    dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .ok_or_else(|| "Could not determine home directory".to_string())
}

#[tauri::command]
async fn create_vault(
    state: State<'_, AppState>,
    name: String,
    path: String,
    kind: Option<String>,
) -> Result<VaultInfo, String> {
    let profile_dir = PathBuf::from(&path);
    let vault_kind = kind.as_deref().map(VaultKind::from_tag);
    let new_name = name.clone();

    // Initialize a ProfileConfig with the given name and ownership kind
    let (storage, profile_config, note_count, tag_count) = off_main_thread(move || {
        std::fs::create_dir_all(&profile_dir)
            .map_err(|e| format!("Failed to create vault directory at {}: {}", profile_dir.display(), e))?;

        let mut profile_config = ProfileConfig::new(&new_name);
        if let Some(kind) = vault_kind {
            profile_config.kind = kind;
        }
        profile_config
            .save_to(&profile_dir)
            .map_err(|e| format!("Failed to save profile config: {e}"))?;

        let storage = Storage::open(&profile_dir, &profile_config).map_err(|e| describe_open_error(e, &profile_dir))?;
        let note_count = storage.list_notes(None, 10_000, 0).map_err(|e| e.to_string())?.len();
        let tag_count = storage.list_tags().map_err(|e| e.to_string())?.len();
        Ok((storage, profile_config, note_count, tag_count))
    })
    .await?;

    let kind_tag = profile_config.kind.as_tag().to_string();
    install_vault(&state, storage, profile_config, None, path.clone())?;

    Ok(VaultInfo {
        name,
        path,
        note_count,
        tag_count,
        kind: kind_tag,
    })
}

/// Change the ownership tag ("private" | "business") of a vault.
///
/// Persists to `<vault>/vault.toml`. If this is the currently open vault,
/// the in-memory config is updated too, so the UI can re-tag immediately.
#[tauri::command]
fn set_vault_kind(state: State<AppState>, path: String, kind: String) -> Result<(), String> {
    let new_kind = VaultKind::from_tag(&kind);
    let profile_dir = PathBuf::from(&path);

    let mut profile_config = ProfileConfig::load(&profile_dir)
        .map_err(|e| format!("Failed to load vault config at {}: {e}", path))?;
    profile_config.kind = new_kind;
    profile_config
        .save_to(&profile_dir)
        .map_err(|e| format!("Failed to save vault config at {}: {e}", path))?;

    // Keep in-memory state consistent if this vault is open right now.
    let open_path = lock(state.open_vault_path.lock())?.clone();
    if open_path.as_deref() == Some(path.as_str()) {
        let mut pc_guard = lock(state.profile_config.lock())?;
        if let Some(open_pc) = pc_guard.as_mut() {
            open_pc.kind = new_kind;
        }
    }
    Ok(())
}

#[tauri::command]
async fn pick_directory(app: AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Option<PathBuf>>();

    FileDialogBuilder::new(app.dialog().to_owned())
        .set_title("Select Directory")
        .pick_folder(move |path: Option<tauri_plugin_dialog::FilePath>| {
            let resolved = path.and_then(|fp| fp.into_path().ok());
            let _ = tx.send(resolved);
        });

    rx.await
        .map(|opt| opt.map(|p| p.to_string_lossy().to_string()))
        .map_err(|e| format!("Dialog error: {}", e))
}

#[tauri::command]
fn list_profiles(state: State<AppState>) -> Result<Vec<ProfileInfo>, String> {
    let config = lock(state.config.lock())?;
    let profiles_dir = &config.profiles_dir;

    if !profiles_dir.exists() {
        return Ok(vec![]);
    }

    let mut profiles = vec![];
    let entries = std::fs::read_dir(profiles_dir).map_err(|e| e.to_string())?;

    for entry in entries.filter_map(|e| e.ok()) {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            let name = entry.file_name().to_string_lossy().to_string();
            let profile_dir = entry.path();
            let pc = ProfileConfig::load(&profile_dir).ok();

            if let Some(pc) = pc {
                let active = config.active_profile.as_deref() == Some(&name)
                    || config.default_profile == name;
                profiles.push(ProfileInfo {
                    display_name: pc.display_name.clone(),
                    is_active: active,
                    notes_dir: pc.resolved_notes_dir(&profile_dir).to_string_lossy().to_string(),
                    encryption: pc.encryption,
                    kind: pc.kind.as_tag().to_string(),
                    name,
                });
            }
        }
    }

    Ok(profiles)
}

#[tauri::command]
fn get_active_profile(state: State<AppState>) -> Result<ProfileInfo, String> {
    let config = lock(state.config.lock())?;
    let name = config.active_profile.as_deref().unwrap_or(&config.default_profile);
    let profile_dir = config.profile_dir(name);
    let pc = ProfileConfig::load(&profile_dir).map_err(|e| e.to_string())?;

    Ok(ProfileInfo {
        name: name.to_string(),
        display_name: pc.display_name.clone(),
        is_active: true,
        notes_dir: pc.resolved_notes_dir(&profile_dir).to_string_lossy().to_string(),
        encryption: pc.encryption,
        kind: pc.kind.as_tag().to_string(),
    })
}

// ── Notes CRUD ─────────────────────────────────────────────────

#[tauri::command]
fn list_notes(
    state: State<AppState>,
    tag: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<Note>, String> {
    with_storage(&state, |storage| {
        storage
            .list_notes(tag.as_deref(), limit.unwrap_or(1000), offset.unwrap_or(0))
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn get_note(state: State<AppState>, identifier: String) -> Result<NoteContent, String> {
    with_storage(&state, |storage| storage.read_note(&identifier).map_err(|e| e.to_string()))
}

#[tauri::command]
fn create_note(state: State<AppState>, req: CreateNoteRequest) -> Result<Note, String> {
    with_storage_mut(&state, |storage| {
        let content = req.content.unwrap_or_default();
        storage
            .create_note(&req.title, &content, req.folder.as_deref(), req.tags)
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn update_note(state: State<AppState>, identifier: String, content: String) -> Result<Note, String> {
    with_storage_mut(&state, |storage| {
        storage.update_note(&identifier, &content).map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn delete_note(state: State<AppState>, identifier: String) -> Result<(), String> {
    with_storage_mut(&state, |storage| storage.delete_note(&identifier).map_err(|e| e.to_string()))
}

// ── Search ─────────────────────────────────────────────────────

#[tauri::command]
fn search_notes(state: State<AppState>, req: SearchRequest) -> Result<Vec<SearchResult>, String> {
    with_storage(&state, |storage| {
        storage.search(&req.query, req.limit.unwrap_or(50)).map_err(|e| e.to_string())
    })
}

// ── Links & Graph ──────────────────────────────────────────────

#[tauri::command]
fn get_backlinks(state: State<AppState>, identifier: String) -> Result<Vec<serde_json::Value>, String> {
    with_storage(&state, |storage| {
        let links = storage.get_backlinks(&identifier).map_err(|e| e.to_string())?;
        Ok(links.into_iter().map(|l| serde_json::to_value(l).unwrap()).collect())
    })
}

#[tauri::command]
fn get_forward_links(state: State<AppState>, identifier: String) -> Result<Vec<serde_json::Value>, String> {
    with_storage(&state, |storage| {
        let links = storage.get_forward_links(&identifier).map_err(|e| e.to_string())?;
        Ok(links.into_iter().map(|l| serde_json::to_value(l).unwrap()).collect())
    })
}

/// Nodes and resolved wikilink edges for the graph view.
pub fn build_graph(storage: &Storage) -> Result<GraphData, String> {
    // One query for the whole graph. (It used to be one query per note, each
    // of which scanned the notes table: O(notes²) on the main thread.)
    let (notes, links) = storage.get_graph().map_err(|e| e.to_string())?;
    Ok(GraphData {
        nodes: notes
            .iter()
            .map(|note| GraphNode {
                id: note.id.clone(),
                title: note.title.clone(),
                path: note.path.clone(),
            })
            .collect(),
        edges: links
            .iter()
            .filter_map(|link| {
                link.target_id.clone().map(|target| GraphEdge {
                    source: link.source_id.clone(),
                    target,
                })
            })
            .collect(),
    })
}

#[tauri::command]
async fn get_graph(state: State<'_, AppState>) -> Result<GraphData, String> {
    let handle = storage_handle(&state)?;
    off_main_thread(move || {
        let storage = lock(handle.lock())?;
        build_graph(&storage)
    })
    .await
}

// ── Tags ───────────────────────────────────────────────────────

#[tauri::command]
fn list_tags(state: State<AppState>) -> Result<Vec<TagInfo>, String> {
    with_storage(&state, |storage| {
        let tags = storage.list_tags().map_err(|e| e.to_string())?;
        Ok(tags.into_iter().map(|(name, count)| TagInfo { name, count }).collect())
    })
}

#[tauri::command]
fn get_notes_by_tag(state: State<AppState>, tag: String) -> Result<Vec<Note>, String> {
    with_storage(&state, |storage| storage.notes_for_tag(&tag).map_err(|e| e.to_string()))
}

// ── Daily Notes ────────────────────────────────────────────────

#[tauri::command]
fn get_daily_note(state: State<AppState>, date: Option<String>) -> Result<DailyNoteResponse, String> {
    with_storage(&state, |storage| {
        let date_str = date.unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());
        let identifier = format!("Daily/{}", date_str);

        match storage.read_note(&identifier) {
            Ok(nc) => Ok(DailyNoteResponse {
                path: nc.note.path,
                content: nc.body,
                exists: true,
            }),
            Err(_) => Ok(DailyNoteResponse {
                path: format!("Daily/{}.md", date_str),
                content: format!("# Daily Note — {}\n\n## Tasks\n\n## Notes\n\n", date_str),
                exists: false,
            }),
        }
    })
}

// ── Knowledge Palace ───────────────────────────────────────────

#[tauri::command]
async fn kp_search(
    state: State<'_, AppState>,
    query: String,
    limit: Option<i64>,
) -> Result<Vec<serde_json::Value>, String> {
    let kp_config = {
        let pc_guard = lock(state.profile_config.lock())?;
        let pc = pc_guard.as_ref().ok_or("No profile loaded")?;
        let kp = &pc.knowledge_palace;
        if !kp.enabled {
            return Err("Knowledge Palace integration is disabled for this profile".to_string());
        }
        cerebro::kp::KpConfig {
            enabled: true,
            endpoint: if kp.endpoint.is_empty() {
                "http://localhost:8080".to_string()
            } else {
                kp.endpoint.clone()
            },
        }
    }; // guard dropped here, before .await

    let connector = cerebro::kp::KpConnector::new(kp_config);
    let results = connector
        .semantic_search(&query, limit.unwrap_or(10))
        .await
        .map_err(|e| e.to_string())?;

    Ok(results.into_iter().map(|r| serde_json::to_value(r).unwrap()).collect())
}

#[tauri::command]
async fn kp_health(state: State<'_, AppState>) -> Result<bool, String> {
    let kp_config = {
        let pc_guard = lock(state.profile_config.lock())?;
        let pc = pc_guard.as_ref().ok_or("No profile loaded")?;
        let kp = &pc.knowledge_palace;
        cerebro::kp::KpConfig {
            enabled: kp.enabled,
            endpoint: if kp.endpoint.is_empty() {
                "http://localhost:8080".to_string()
            } else {
                kp.endpoint.clone()
            },
        }
    }; // guard dropped here, before .await

    let connector = cerebro::kp::KpConnector::new(kp_config);
    connector.health_check().await.map_err(|e| e.to_string())
}

// ── Keyword Links (Knowledge Graph) ──────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct KeywordLink {
    pub source_id: String,
    pub target_id: String,
    pub confidence: f64,
}

#[tauri::command]
async fn get_keyword_links(state: State<'_, AppState>) -> Result<Vec<KeywordLink>, String> {
    let handle = storage_handle(&state)?;
    // Pairwise over all notes: seconds on a large vault, so not on the main thread.
    off_main_thread(move || {
        let storage = lock(handle.lock())?;
        let links = storage.compute_keyword_links().map_err(|e| e.to_string())?;
        Ok(links
            .into_iter()
            .map(|(source_id, target_id, confidence)| KeywordLink {
                source_id,
                target_id,
                confidence,
            })
            .collect())
    })
    .await
}

// ── Category & Favorites ─────────────────────────────────────

#[tauri::command]
fn set_note_category(state: State<AppState>, note_id: String, category: Option<String>) -> Result<(), String> {
    with_storage(&state, |storage| {
        storage.set_category(&note_id, category.as_deref()).map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn toggle_note_favorite(state: State<AppState>, note_id: String, favorite: bool) -> Result<(), String> {
    with_storage(&state, |storage| {
        storage.set_favorite(&note_id, favorite).map_err(|e| e.to_string())
    })
}

// ── Reindex ────────────────────────────────────────────────────

#[tauri::command]
async fn reindex(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let handle = storage_handle(&state)?;
    off_main_thread(move || {
        let mut storage = lock(handle.lock())?;
        let stats = storage.reindex_all().map_err(|e| e.to_string())?;
        Ok(serde_json::json!({
            "created": stats.created,
            "updated": stats.updated,
            "removed": stats.removed,
            "errors": stats.errors,
        }))
    })
    .await
}

// ── Templates ──────────────────────────────────────────────────

/// Where templates live in the vault.
fn templates_dir(storage: &Storage) -> PathBuf {
    storage.notes_dir_path().join("templates")
}

#[tauri::command]
fn list_templates(state: State<AppState>) -> Result<Vec<TemplateInfo>, String> {
    with_storage(&state, |storage| {
        let templates_dir = templates_dir(storage);

        if !templates_dir.exists() {
            return Ok(vec![]);
        }

        let mut templates = Vec::new();
        let entries = std::fs::read_dir(&templates_dir).map_err(|e| e.to_string())?;

        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                let relative = path
                    .strip_prefix(&templates_dir)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();
                templates.push(TemplateInfo {
                    name,
                    path: relative,
                });
            }
        }

        Ok(templates)
    })
}

/// `templates/<name>.md`, checked to stay inside the vault.
fn template_path(storage: &Storage, name: &str) -> Result<PathBuf, String> {
    let notes_dir = storage.notes_dir_path();
    let relative = resolve_vault_path(notes_dir, &format!("templates/{name}.md"))?;
    Ok(relative)
}

#[tauri::command]
fn get_template(state: State<AppState>, name: String) -> Result<String, String> {
    with_storage(&state, |storage| {
        let template_path = template_path(storage, &name)?;
        std::fs::read_to_string(&template_path)
            .map_err(|e| format!("Failed to read template '{}': {}", name, e))
    })
}

#[tauri::command]
fn create_template(state: State<AppState>, name: String, content: String) -> Result<(), String> {
    with_storage(&state, |storage| {
        let path = template_path(storage, &name)?;
        std::fs::create_dir_all(templates_dir(storage)).map_err(|e| e.to_string())?;
        std::fs::write(&path, &content).map_err(|e| format!("Failed to write template '{}': {}", name, e))
    })
}

#[tauri::command]
fn delete_template(state: State<AppState>, name: String) -> Result<(), String> {
    with_storage(&state, |storage| {
        let path = template_path(storage, &name)?;
        std::fs::remove_file(&path).map_err(|e| format!("Failed to delete template '{}': {}", name, e))
    })
}

#[tauri::command]
fn create_note_from_template(
    state: State<AppState>,
    title: String,
    template_name: String,
    folder: Option<String>,
    tags: Option<Vec<String>>,
) -> Result<Note, String> {
    let content = with_storage(&state, |storage| {
        let template_path = template_path(storage, &template_name)?;
        std::fs::read_to_string(&template_path)
            .map_err(|e| format!("Failed to read template '{}': {}", template_name, e))
    })?;

    let now = chrono::Local::now();
    let date_str = now.format("%Y-%m-%d").to_string();
    let time_str = now.format("%H:%M").to_string();

    let content = content
        .replace("{{title}}", &title)
        .replace("{{date}}", &date_str)
        .replace("{{time}}", &time_str);

    with_storage_mut(&state, |storage| {
        storage
            .create_note(&title, &content, folder.as_deref(), tags)
            .map_err(|e| e.to_string())
    })
}

// ── Image / Attachment Commands ────────────────────────────────

#[tauri::command]
fn save_image(state: State<AppState>, file_name: String, data: String) -> Result<String, String> {
    use base64::Engine;
    let attachments_dir = with_storage(&state, |storage| Ok(storage.attachments_dir()))?;
    std::fs::create_dir_all(&attachments_dir).map_err(|e| e.to_string())?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let safe_name = file_name.replace(|c: char| !c.is_alphanumeric() && c != '.' && c != '-' && c != '_', "_");
    let unique_name = format!("{}_{}", timestamp, safe_name);
    let file_path = attachments_dir.join(&unique_name);

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&data)
        .map_err(|e| format!("Invalid base64: {}", e))?;

    std::fs::write(&file_path, decoded).map_err(|e| e.to_string())?;

    Ok(format!("attachments/{}", unique_name))
}

#[tauri::command]
fn read_image_as_base64(state: State<AppState>, path: String) -> Result<String, String> {
    use base64::Engine;
    let file_path = with_storage(&state, |storage| resolve_vault_path(storage.notes_dir_path(), &path))?;
    let data = std::fs::read(&file_path).map_err(|e| e.to_string())?;

    Ok(base64::engine::general_purpose::STANDARD.encode(&data))
}

// ── Document Import ─────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct ImportDocumentRequest {
    pub file_path: String,
    pub title: Option<String>,
    pub tags: Option<Vec<String>>,
    pub folder: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImportDocumentResponse {
    pub title: String,
    pub format: String,
    pub tags: Vec<String>,
    pub note_id: String,
    pub note_path: String,
}

/// A converted document, ready to be stored.
struct Converted {
    title: String,
    markdown: String,
    format: String,
    tags: Vec<String>,
}

fn convert(file_path: &str) -> Result<Converted, String> {
    use cerebro::import::import_file;
    let result = import_file(Path::new(file_path)).map_err(|e| format!("{e:#}"))?;
    Ok(Converted {
        title: result.title,
        markdown: result.markdown,
        format: result.format.name().to_string(),
        tags: result.tags,
    })
}

fn store_converted(
    storage: &mut Storage,
    document: Converted,
    folder: Option<&str>,
) -> Result<ImportDocumentResponse, String> {
    let note = storage
        .create_note(&document.title, &document.markdown, folder, Some(document.tags.clone()))
        .map_err(|e| e.to_string())?;

    Ok(ImportDocumentResponse {
        title: note.title.clone(),
        format: document.format,
        tags: note.tags.clone(),
        note_id: note.id,
        note_path: note.path,
    })
}

#[tauri::command]
async fn import_document(
    state: State<'_, AppState>,
    req: ImportDocumentRequest,
) -> Result<ImportDocumentResponse, String> {
    let handle = storage_handle(&state)?;
    off_main_thread(move || {
        // Converting (a PDF, a PowerPoint, …) can take a while and does not
        // need the vault: no lock is held while it runs.
        let mut document = convert(&req.file_path).map_err(|e| format!("Import failed: {e}"))?;
        if let Some(title) = req.title {
            document.title = title;
        }
        if let Some(extra) = req.tags {
            document.tags.extend(extra);
        }
        let mut storage = lock(handle.lock())?;
        store_converted(&mut storage, document, req.folder.as_deref())
    })
    .await
}

#[tauri::command]
async fn import_documents(
    state: State<'_, AppState>,
    file_paths: Vec<String>,
    folder: Option<String>,
) -> Result<Vec<ImportDocumentResponse>, String> {
    let handle = storage_handle(&state)?;
    off_main_thread(move || {
        // Convert everything first, without holding the vault …
        let mut documents = Vec::with_capacity(file_paths.len());
        for file_path in &file_paths {
            match convert(file_path) {
                Ok(document) => documents.push(document),
                Err(e) => return Err(format!("Failed to import '{}': {}", file_path, e)),
            }
        }
        // … then store, under one short lock.
        let mut storage = lock(handle.lock())?;
        documents
            .into_iter()
            .map(|document| store_converted(&mut storage, document, folder.as_deref()))
            .collect()
    })
    .await
}

// ── Obsidian Vault Import ────────────────────────────────────────

/// Directories inside an Obsidian vault that are never imported.
const OBSIDIAN_SKIP_DIRS: &[&str] = &[".obsidian", ".trash", ".git", ".smart-env", "node_modules"];
/// Hard cap so a monstrous vault can't wedge the app indefinitely.
const MAX_OBSIDIAN_IMPORT_FILES: usize = 5000;

/// Recursively collect `.md` files, skipping hidden dirs and Obsidian metadata.
fn collect_markdown_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 16 || out.len() >= MAX_OBSIDIAN_IMPORT_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if name.starts_with('.') || OBSIDIAN_SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            collect_markdown_files(&path, out, depth + 1);
        } else if name.to_ascii_lowercase().ends_with(".md") && out.len() < MAX_OBSIDIAN_IMPORT_FILES {
            out.push(path);
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImportVaultResponse {
    pub imported: usize,
    pub skipped: usize,
    pub failed: usize,
    pub total_found: usize,
    /// First few error messages (capped to keep the payload sane).
    pub errors: Vec<String>,
}

/// Walk an Obsidian-style vault and import every Markdown note into `storage`.
///
/// The vault lock is taken per note, so the rest of the app keeps working
/// while a large import runs. Emits an `import-progress` event every 10 files
/// when `app` is given.
fn run_vault_import(
    storage: &SharedStorage,
    app: Option<&AppHandle>,
    root: &Path,
    folder: Option<&str>,
) -> Result<ImportVaultResponse, String> {
    let mut files = Vec::new();
    collect_markdown_files(root, &mut files, 0);
    let total = files.len();

    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for (i, path) in files.iter().enumerate() {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        // Destination path inside the Cerebro vault (optional folder prefix).
        let dest = match folder.map(str::trim).filter(|f| !f.is_empty()) {
            Some(prefix) => format!("{}/{}", prefix.trim_matches('/'), rel),
            None => rel.clone(),
        };

        // Obsidian convention: note title = filename without extension.
        let title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string();

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                failed += 1;
                if errors.len() < 10 {
                    errors.push(format!("{}: {}", rel, e));
                }
                continue;
            }
        };

        // Reading the file is the slow part and needs no lock; only the
        // index update does.
        let outcome = {
            let mut storage = lock(storage.lock())?;
            if storage.note_exists(&dest) {
                None
            } else {
                let tags = vec!["import/obsidian".to_string(), "imported".to_string()];
                Some(storage.create_note(&title, &content, Some(&dest), Some(tags)))
            }
        };
        match outcome {
            None => skipped += 1,
            Some(Ok(_)) => imported += 1,
            Some(Err(e)) => {
                failed += 1;
                if errors.len() < 10 {
                    errors.push(format!("{}: {e:#}", rel));
                }
            }
        }

        // Progress event every 10 files (and on the last one) for the UI.
        if let Some(handle) = app
            && ((i + 1) % 10 == 0 || i + 1 == total)
        {
            let _ = handle.emit(
                "import-progress",
                serde_json::json!({ "done": i + 1, "total": total }),
            );
        }
    }

    Ok(ImportVaultResponse {
        imported,
        skipped,
        failed,
        total_found: total,
        errors,
    })
}

/// Import an Obsidian vault (a directory of Markdown notes) into the open vault.
///
/// Walks the directory recursively, skipping `.obsidian`/`.trash`/hidden dirs,
/// and imports every `.md` file preserving its relative folder structure under
/// the optional `folder` prefix. Existing notes at the same path are skipped.
/// Obsidian frontmatter tags are merged automatically by the parser; note
/// titles follow the Obsidian convention (filename without extension).
#[tauri::command]
async fn import_obsidian_vault(
    app: AppHandle,
    state: State<'_, AppState>,
    vault_path: String,
    folder: Option<String>,
) -> Result<ImportVaultResponse, String> {
    let root = PathBuf::from(&vault_path);
    if !root.exists() {
        return Err(format!("Directory not found: {}", vault_path));
    }
    if !root.is_dir() {
        return Err(format!("Not a directory: {}", vault_path));
    }

    // Up to thousands of files: the whole import runs on a blocking thread.
    let handle = storage_handle(&state)?;
    off_main_thread(move || run_vault_import(&handle, Some(&app), &root, folder.as_deref())).await
}

// ── Transclusion Resolution ──────────────────────────────────

#[tauri::command]
fn resolve_transclusions(state: State<AppState>, content: String) -> Result<String, String> {
    use cerebro::parser::wikilinks::extract_transclusions;

    let transclusions = extract_transclusions(&content);
    if transclusions.is_empty() {
        return Ok(content);
    }

    with_storage(&state, |storage| {
        let mut result = content;
        for tc in transclusions {
            if let Ok(nc) = storage.read_note(&tc.target) {
                // `![[Target#Heading]]` embeds that section, not the whole note.
                let replacement = match &tc.heading {
                    Some(heading) => section_under_heading(&nc.body, heading).unwrap_or_else(|| nc.body.clone()),
                    None => nc.body.clone(),
                };
                if let Some(heading) = &tc.heading {
                    result = result.replace(&format!("![[{}#{}]]", tc.target, heading), &replacement);
                }
                result = result.replace(&format!("![[{}]]", tc.target), &replacement);
            }
        }
        Ok(result)
    })
}

// ── Text Attachment Commands ──────────────────────────────────

#[tauri::command]
fn read_attachment_text(state: State<AppState>, path: String) -> Result<String, String> {
    let file_path = with_storage(&state, |storage| resolve_vault_path(storage.notes_dir_path(), &path))?;
    std::fs::read_to_string(&file_path)
        .map_err(|e| format!("Failed to read {}: {}", file_path.display(), e))
}

// ── File Watcher Commands ──────────────────────────────────────

/// Global debounce state: stores the timestamp (ms) of the last emitted event.
static LAST_EMIT_MS: AtomicU64 = AtomicU64::new(0);

/// Payload emitted to the frontend via the `vault-changed` event.
#[derive(Debug, Clone, Serialize)]
struct VaultChangeEvent {
    kind: String,
    path: String,
}

#[tauri::command]
fn start_watcher(app: AppHandle, state: State<'_, AppState>, notes_dir: String) -> Result<(), String> {
    let watch_path = PathBuf::from(&notes_dir);

    if !watch_path.exists() {
        return Err(format!("Notes directory does not exist: {}", notes_dir));
    }

    let app_handle = app.clone();
    let watch_path_clone = watch_path.clone();

    // Build the watcher with a callback that emits Tauri events.
    let mut watcher = RecommendedWatcher::new(
        move |res: Result<notify::Event, notify::Error>| {
            let event = match res {
                Ok(e) => e,
                Err(err) => {
                    tracing::warn!("File watcher error: {err:?}");
                    return;
                }
            };

            // Determine the kind string from the event.
            let kind_str = match event.kind {
                EventKind::Create(_) => "create",
                EventKind::Modify(_) => "update",
                EventKind::Remove(_) => "remove",
                _ => return, // ignore access/other events
            };

            // Process each affected path.
            for path in &event.paths {
                // Only care about .md files.
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }

                // Skip hidden files / directories (components starting with '.').
                if path.components().any(|c| {
                    c.as_os_str()
                        .to_str()
                        .is_some_and(|s| s.starts_with('.') && s != ".")
                }) {
                    continue;
                }

                // Skip directories.
                if path.is_dir() {
                    continue;
                }

                // Compute relative path from the notes directory root.
                let relative = path
                    .strip_prefix(&watch_path_clone)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();

                // Debounce: ignore events within 100ms of the last emit.
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;

                let last = LAST_EMIT_MS.load(Ordering::Relaxed);
                if now_ms.saturating_sub(last) < 100 {
                    continue;
                }
                LAST_EMIT_MS.store(now_ms, Ordering::Relaxed);

                let payload = VaultChangeEvent {
                    kind: kind_str.to_string(),
                    path: relative,
                };

                if let Err(e) = app_handle.emit("vault-changed", &payload) {
                    tracing::warn!("Failed to emit vault-changed event: {e}");
                }
            }
        },
        NotifyConfig::default(),
    )
    .map_err(|e| format!("Failed to create file watcher: {e}"))?;

    // Start watching recursively.
    watcher
        .watch(&watch_path, RecursiveMode::Recursive)
        .map_err(|e| format!("Failed to start watching {notes_dir}: {e}"))?;

    tracing::info!("File watcher started for: {notes_dir}");

    // Store the watcher in AppState (replaces any previous one).
    let mut guard = lock(state.watcher.lock())?;
    *guard = Some(watcher);

    Ok(())
}

#[tauri::command]
fn stop_watcher(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = lock(state.watcher.lock())?;
    if guard.is_some() {
        tracing::info!("File watcher stopped");
    }
    *guard = None;
    Ok(())
}

// ── Entry Point ────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config = Config::load().unwrap_or_else(|_| {
        let c = Config::new().expect("failed to create default config");
        let _ = c.save();
        c
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            config: Mutex::new(config),
            storage: Mutex::new(None),
            profile_config: Mutex::new(None),
            watcher: Mutex::new(None),
            open_vault_path: Mutex::new(None),
            vault_key: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            home_dir,
            create_vault,
            pick_directory,
            init_vault,
            open_vault,
            unlock_vault,
            lock_vault,
            vault_is_protected,
            set_vault_kind,
            list_profiles,
            get_active_profile,
            list_notes,
            get_note,
            create_note,
            update_note,
            delete_note,
            search_notes,
            get_backlinks,
            get_forward_links,
            get_graph,
            list_tags,
            get_notes_by_tag,
            get_daily_note,
            kp_search,
            kp_health,
            set_note_category,
            toggle_note_favorite,
            get_keyword_links,
            reindex,
            save_image,
            read_image_as_base64,
            read_attachment_text,
            import_document,
            import_documents,
            import_obsidian_vault,
            start_watcher,
            stop_watcher,
            list_templates,
            get_template,
            create_template,
            delete_template,
            create_note_from_template,
            resolve_transclusions,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Cerebro");
}

fn main() {
    run();
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(tag: &str) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let profile_dir = tmp.path().join("vault");
        std::fs::create_dir_all(&profile_dir).expect("mkdir");
        let pc = ProfileConfig::new(tag);
        pc.save_to(&profile_dir).expect("vault.toml");
        (tmp, profile_dir)
    }

    /// Build a fake Obsidian vault on disk:
    ///   vault/
    ///     note.md
    ///     projects/plan.md
    ///     .obsidian/config.json      (skipped)
    ///     .trash/old.md              (skipped)
    ///     attachments/img.txt        (skipped — not .md)
    #[test]
    fn test_collect_markdown_files_skips_obsidian_metadata() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        std::fs::create_dir_all(root.join("projects")).unwrap();
        std::fs::create_dir_all(root.join(".obsidian")).unwrap();
        std::fs::create_dir_all(root.join(".trash")).unwrap();
        std::fs::create_dir_all(root.join("attachments")).unwrap();

        std::fs::write(root.join("note.md"), "# Root").unwrap();
        std::fs::write(root.join("projects/plan.md"), "# Plan").unwrap();
        std::fs::write(root.join(".obsidian/config.json"), "{}").unwrap();
        std::fs::write(root.join(".trash/old.md"), "# Old").unwrap();
        std::fs::write(root.join("attachments/img.txt"), "data").unwrap();

        let mut files = Vec::new();
        collect_markdown_files(root, &mut files, 0);

        let mut names: Vec<String> = files
            .iter()
            .map(|p| p.strip_prefix(root).unwrap().to_string_lossy().to_string())
            .collect();
        names.sort();

        assert_eq!(names, vec!["note.md".to_string(), "projects/plan.md".to_string()]);
    }

    #[test]
    fn test_collect_markdown_files_missing_dir_is_empty() {
        let mut files = Vec::new();
        collect_markdown_files(Path::new("/nonexistent/vault/xyz"), &mut files, 0);
        assert!(files.is_empty());
    }

    #[test]
    fn test_collect_markdown_files_is_case_insensitive_on_ext() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("UPPER.MD"), "# Upper").unwrap();

        let mut files = Vec::new();
        collect_markdown_files(tmp.path(), &mut files, 0);
        assert_eq!(files.len(), 1);
    }

    /// Full import pipeline: fake Obsidian vault → Storage, verifying folder
    /// structure, folder prefix, frontmatter tag merge, and skip-on-reimport.
    #[test]
    fn test_run_vault_import_end_to_end() {
        // Fake Obsidian vault
        let vault_tmp = tempfile::tempdir().expect("tempdir");
        let root = vault_tmp.path();
        std::fs::create_dir_all(root.join("projects")).unwrap();
        std::fs::create_dir_all(root.join(".obsidian")).unwrap();
        std::fs::write(
            root.join("Inbox.md"),
            "# Inbox\n\nSee [[plan]] for details.\n\n#inline-tag",
        )
        .unwrap();
        std::fs::write(
            root.join("projects/plan.md"),
            "---\ntags: [work, rust]\n---\n# Plan\n\nBody.",
        )
        .unwrap();
        std::fs::write(root.join(".obsidian/app.json"), "{}").unwrap();

        // Cerebro vault storage
        let target_tmp = tempfile::tempdir().expect("tempdir");
        let profile_dir = target_tmp.path().join("vault");
        std::fs::create_dir_all(&profile_dir).unwrap();
        let pc = ProfileConfig::new("vault");
        pc.save_to(&profile_dir).unwrap();
        let storage = Arc::new(Mutex::new(Storage::open(&profile_dir, &pc).unwrap()));

        // First import — 2 notes, metadata dir skipped
        let resp = run_vault_import(&storage, None, root, Some("Obsidian")).unwrap();
        assert_eq!(resp.total_found, 2, "should find exactly the 2 .md files");
        assert_eq!(resp.imported, 2);
        assert_eq!(resp.skipped, 0);
        assert_eq!(resp.failed, 0);
        assert!(resp.errors.is_empty());

        // Folder structure preserved under the prefix
        {
            let storage = storage.lock().unwrap();
            assert!(storage.note_exists("Obsidian/Inbox.md"));
            assert!(storage.note_exists("Obsidian/projects/plan.md"));

            // Frontmatter tags merged, import tags applied, title = filename stem
            let plan = storage.read_note("Obsidian/projects/plan.md").unwrap();
            assert_eq!(plan.note.title, "plan");
            assert!(plan.note.tags.contains(&"work".to_string()));
            assert!(plan.note.tags.contains(&"rust".to_string()));
            assert!(plan.note.tags.contains(&"import/obsidian".to_string()));

            // Inline tags picked up too
            let inbox = storage.read_note("Obsidian/Inbox.md").unwrap();
            assert!(inbox.note.tags.contains(&"inline-tag".to_string()));
        }

        // Second import — all notes skipped (already exist at destination)
        let resp2 = run_vault_import(&storage, None, root, Some("Obsidian")).unwrap();
        assert_eq!(resp2.imported, 0);
        assert_eq!(resp2.skipped, 2);

        // Re-import without the prefix lands at fresh paths (no clash)
        let resp3 = run_vault_import(&storage, None, root, None).unwrap();
        assert_eq!(resp3.imported, 2);
        assert!(storage.lock().unwrap().note_exists("Inbox.md"));
    }

    // ── 2026-11-23 audit ────────────────────────────────────────

    #[test]
    fn paths_from_the_frontend_cannot_leave_the_vault() {
        // A note can contain `![](attachments/../../../.ssh/id_rsa)`; the
        // preview hands that path to `read_image_as_base64`.
        let tmp = tempfile::tempdir().unwrap();
        let notes = tmp.path().join("notes");
        std::fs::create_dir_all(notes.join("attachments")).unwrap();
        std::fs::write(notes.join("attachments/a.png"), "png").unwrap();
        std::fs::write(tmp.path().join("secret.txt"), "s3cret").unwrap();

        for bad in [
            "../secret.txt",
            "attachments/../../../secret.txt",
            "attachments/../../notes",
            "/etc/passwd",
            "",
        ] {
            assert!(resolve_vault_path(&notes, bad).is_err(), "{bad:?} must be refused");
        }
        let ok = resolve_vault_path(&notes, "attachments/a.png").unwrap();
        assert!(ok.is_file());

        // A symlink inside the vault that points out of it is refused too.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(tmp.path().join("secret.txt"), notes.join("attachments/link.png")).unwrap();
            assert!(resolve_vault_path(&notes, "attachments/link.png").is_err());
        }
    }

    #[test]
    fn opening_a_vault_reports_that_vaults_kind() {
        // `open_vault` used to report the kind of the vault that had been
        // open *before* — "private" on the first open, whatever the vault.
        let (_tmp, profile_dir) = vault("kind");
        let mut cfg = ProfileConfig::new("kind");
        cfg.kind = VaultKind::Business;
        cfg.save_to(&profile_dir).unwrap();
        std::fs::create_dir_all(profile_dir.join("notes")).unwrap();
        std::fs::write(profile_dir.join("notes/a.md"), "# A\n").unwrap();

        let (mut storage, note_count, tag_count) = load_vault(&profile_dir, &cfg).unwrap();
        assert_eq!((note_count, tag_count), (1, 0));
        assert_eq!(cfg.kind.as_tag(), "business");
        storage.reindex_all().unwrap();
        assert_eq!(storage.list_notes(None, 10, 0).unwrap().len(), 1);
    }

    #[test]
    fn opening_a_vault_survives_a_note_file_removed_outside_the_app() {
        // The GUI reindexes on every open; that used to fail forever with
        // "FOREIGN KEY constraint failed" once a file had been deleted by hand.
        let (_tmp, profile_dir) = vault("removed");
        let cfg = ProfileConfig::new("removed");
        let notes = profile_dir.join("notes");
        std::fs::create_dir_all(&notes).unwrap();
        std::fs::write(notes.join("keep.md"), "# Keep\n").unwrap();
        std::fs::write(notes.join("gone.md"), "# Gone #only-mine\n").unwrap();
        load_vault(&profile_dir, &cfg).unwrap();
        std::fs::remove_file(notes.join("gone.md")).unwrap();

        let (storage, note_count, _) = load_vault(&profile_dir, &cfg).expect("second open must succeed");
        assert_eq!(note_count, 1);
        assert!(storage.search("gone", 5).unwrap().is_empty());
    }

    #[test]
    fn graph_has_every_note_once_and_only_resolved_edges() {
        let (_tmp, profile_dir) = vault("graph");
        let mut storage = Storage::open(&profile_dir, &ProfileConfig::new("graph")).unwrap();
        let a = storage.create_note("A", "see [[B]] and [[Missing]]", None, None).unwrap();
        let b = storage.create_note("B", "back to [[A]]", None, None).unwrap();

        let graph = build_graph(&storage).unwrap();
        let mut titles: Vec<&str> = graph.nodes.iter().map(|n| n.title.as_str()).collect();
        titles.sort();
        assert_eq!(titles, vec!["A", "B"]);

        let mut edges: Vec<(String, String)> = graph
            .edges
            .iter()
            .map(|e| (e.source.clone(), e.target.clone()))
            .collect();
        edges.sort();
        assert_eq!(edges, vec![(a.id.clone(), b.id.clone()), (b.id.clone(), a.id.clone())]);
    }

    #[test]
    fn transclusion_with_a_heading_embeds_that_section() {
        let body = "---\ntitle: X\n---\n\n# Title\n\nintro\n\n## Setup\n\nstep one\nstep two\n\n### Notes\n\nnested\n\n## Next\n\nlater\n";
        assert_eq!(section_under_heading(body, "Missing"), None);
        assert_eq!(section_under_heading(body, "Setup").unwrap(), "## Setup\n\nstep one\nstep two\n\n### Notes\n\nnested");
        assert_eq!(section_under_heading(body, "Title").unwrap().trim_end(), "# Title\n\nintro\n\n## Setup\n\nstep one\nstep two\n\n### Notes\n\nnested\n\n## Next\n\nlater");
        // "Set" is not the heading "Setup"
        assert_eq!(section_under_heading(body, "Set"), None);
    }

    #[test]
    fn commands_that_can_take_long_run_off_the_main_thread() {
        // Tauri runs a plain `fn` command on the main thread. This is the
        // list of commands that must never block it; keep it in step with the
        // commands above.
        let source = include_str!("main.rs");
        let mut checked = 0;
        for command in [
            "init_vault",
            "open_vault",
            "create_vault",
            "reindex",
            "get_graph",
            "get_keyword_links",
            "import_document",
            "import_documents",
            "import_obsidian_vault",
        ] {
            let declared = source
                .match_indices(&format!("fn {command}("))
                .any(|(at, _)| source[..at].trim_end().ends_with("async"));
            assert!(declared, "`{command}` must be an `async fn` so Tauri does not run it on the main thread");
            checked += 1;
        }
        assert_eq!(checked, 9);

        // …and they are registered exactly once each.
        let registered = source
            .split("tauri::generate_handler![")
            .nth(1)
            .and_then(|s| s.split(']').next())
            .expect("generate_handler! list");
        let names: Vec<&str> = registered.split(',').map(str::trim).filter(|c| !c.is_empty()).collect();
        assert_eq!(names.len(), 43, "the README states the number of Tauri commands");
        for command in ["init_vault", "open_vault", "create_vault", "reindex", "get_graph", "get_keyword_links"] {
            assert_eq!(names.iter().filter(|n| **n == command).count(), 1, "{command}");
        }
    }

    #[test]
    fn the_webview_is_granted_only_what_it_uses() {
        // The frontend imports `@tauri-apps/plugin-dialog` and nothing else.
        let capability = include_str!("../capabilities/default.json");
        let value: serde_json::Value = serde_json::from_str(capability).unwrap();
        let granted: Vec<&str> = value["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap())
            .collect();
        for unused in ["fs:", "shell:", "clipboard-manager:"] {
            assert!(
                !granted.iter().any(|p| p.starts_with(unused)),
                "{unused}* is granted but never used by the frontend: {granted:?}"
            );
        }
        assert!(granted.contains(&"dialog:default"), "the vault picker needs the dialog plugin");
    }
}
