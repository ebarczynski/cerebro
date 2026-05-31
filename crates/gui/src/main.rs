//! Cerebro GUI — Tauri 2.0 desktop application.
//!
//! Exposes the Cerebro core library (storage, search, links, tags,
//! Knowledge Palace connector, Confluence publisher) as Tauri commands
//! consumed by the React/TypeScript frontend.

use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tauri::State;

use cerebro::config::app::Config;
use cerebro::config::profile::ProfileConfig;
use cerebro::storage::{Note, NoteContent, SearchResult, Storage};

// ── Application State ──────────────────────────────────────────

pub struct AppState {
    pub config: Mutex<Config>,
    pub storage: Mutex<Option<Storage>>,
    pub profile_config: Mutex<Option<ProfileConfig>>,
}

// ── DTOs for the frontend ──────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct ProfileInfo {
    pub name: String,
    pub display_name: String,
    pub is_active: bool,
    pub notes_dir: String,
    pub encryption: bool,
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
}

// ── Vault Management ───────────────────────────────────────────

#[tauri::command]
fn init_vault(state: State<AppState>, path: String, profile: Option<String>) -> Result<VaultInfo, String> {
    let profile_name = profile.unwrap_or_else(|| "personal".to_string());
    let profile_dir = PathBuf::from(&path);

    // Create profile directory structure
    std::fs::create_dir_all(&profile_dir).map_err(|e| e.to_string())?;

    // Create/load profile config
    let profile_config = ProfileConfig::load_or_default(&profile_dir, &profile_name)
        .map_err(|e| e.to_string())?;
    profile_config.save_to(&profile_dir).map_err(|e| e.to_string())?;

    // Initialize storage
    let storage = Storage::open(&profile_dir, &profile_config)
        .map_err(|e| e.to_string())?;

    let note_count = storage.list_notes(None, 10000, 0).map_err(|e| e.to_string())?.len();
    let tag_count = storage.list_tags().map_err(|e| e.to_string())?.len();

    let mut storage_guard = state.storage.lock().map_err(|e| e.to_string())?;
    *storage_guard = Some(storage);
    let mut pc_guard = state.profile_config.lock().map_err(|e| e.to_string())?;
    *pc_guard = Some(profile_config);

    Ok(VaultInfo {
        name: profile_name,
        path,
        note_count,
        tag_count,
    })
}

#[tauri::command]
fn open_vault(state: State<AppState>, path: String, profile: Option<String>) -> Result<VaultInfo, String> {
    let profile_name = profile.unwrap_or_else(|| "personal".to_string());
    let profile_dir = PathBuf::from(&path);

    if !profile_dir.exists() {
        return Err(format!("Vault not found at {}", path));
    }

    // Load profile config
    let profile_config = ProfileConfig::load(&profile_dir)
        .unwrap_or_else(|_| ProfileConfig::new(&profile_name));

    let mut storage = Storage::open(&profile_dir, &profile_config)
        .map_err(|e| e.to_string())?;

    // Reindex all notes on open
    let _stats = storage.reindex_all().map_err(|e| e.to_string())?;

    let note_count = storage.list_notes(None, 10000, 0).map_err(|e| e.to_string())?.len();
    let tag_count = storage.list_tags().map_err(|e| e.to_string())?.len();

    let mut storage_guard = state.storage.lock().map_err(|e| e.to_string())?;
    *storage_guard = Some(storage);
    let mut pc_guard = state.profile_config.lock().map_err(|e| e.to_string())?;
    *pc_guard = Some(profile_config);

    Ok(VaultInfo {
        name: profile_name,
        path,
        note_count,
        tag_count,
    })
}

// ── Profile Management ─────────────────────────────────────────

#[tauri::command]
fn list_profiles(state: State<AppState>) -> Result<Vec<ProfileInfo>, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;
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
                    name,
                });
            }
        }
    }

    Ok(profiles)
}

#[tauri::command]
fn get_active_profile(state: State<AppState>) -> Result<ProfileInfo, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;
    let name = config.active_profile.as_deref().unwrap_or(&config.default_profile);
    let profile_dir = config.profile_dir(name);
    let pc = ProfileConfig::load(&profile_dir).map_err(|e| e.to_string())?;

    Ok(ProfileInfo {
        name: name.to_string(),
        display_name: pc.display_name.clone(),
        is_active: true,
        notes_dir: pc.resolved_notes_dir(&profile_dir).to_string_lossy().to_string(),
        encryption: pc.encryption,
    })
}

// ── Notes CRUD ─────────────────────────────────────────────────

#[tauri::command]
fn list_notes(state: State<AppState>, tag: Option<String>, limit: Option<i64>, offset: Option<i64>) -> Result<Vec<Note>, String> {
    let guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_ref().ok_or("No vault open — use open_vault first")?;

    storage.list_notes(tag.as_deref(), limit.unwrap_or(1000), offset.unwrap_or(0))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_note(state: State<AppState>, identifier: String) -> Result<NoteContent, String> {
    let guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_ref().ok_or("No vault open")?;

    storage.read_note(&identifier).map_err(|e| e.to_string())
}

#[tauri::command]
fn create_note(state: State<AppState>, req: CreateNoteRequest) -> Result<Note, String> {
    let mut guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_mut().ok_or("No vault open")?;

    let content = req.content.unwrap_or_default();
    let path = req.folder.as_deref();
    let tags = req.tags;

    storage.create_note(&req.title, &content, path, tags)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn update_note(state: State<AppState>, identifier: String, content: String) -> Result<Note, String> {
    let mut guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_mut().ok_or("No vault open")?;

    storage.update_note(&identifier, &content).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_note(state: State<AppState>, identifier: String) -> Result<(), String> {
    let mut guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_mut().ok_or("No vault open")?;

    storage.delete_note(&identifier).map_err(|e| e.to_string())
}

// ── Search ─────────────────────────────────────────────────────

#[tauri::command]
fn search_notes(state: State<AppState>, req: SearchRequest) -> Result<Vec<SearchResult>, String> {
    let guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_ref().ok_or("No vault open")?;

    let limit = req.limit.unwrap_or(50);
    storage.search(&req.query, limit).map_err(|e| e.to_string())
}

// ── Links & Graph ──────────────────────────────────────────────

#[tauri::command]
fn get_backlinks(state: State<AppState>, identifier: String) -> Result<Vec<serde_json::Value>, String> {
    let guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_ref().ok_or("No vault open")?;

    let links = storage.get_backlinks(&identifier).map_err(|e| e.to_string())?;
    Ok(links.into_iter().map(|l| serde_json::to_value(l).unwrap()).collect())
}

#[tauri::command]
fn get_forward_links(state: State<AppState>, identifier: String) -> Result<Vec<serde_json::Value>, String> {
    let guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_ref().ok_or("No vault open")?;

    let links = storage.get_forward_links(&identifier).map_err(|e| e.to_string())?;
    Ok(links.into_iter().map(|l| serde_json::to_value(l).unwrap()).collect())
}

#[tauri::command]
fn get_graph(state: State<AppState>) -> Result<GraphData, String> {
    let guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_ref().ok_or("No vault open")?;

    let notes = storage.list_notes(None, 100000, 0).map_err(|e| e.to_string())?;
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    for note in &notes {
        nodes.push(GraphNode {
            id: note.id.clone(),
            title: note.title.clone(),
            path: note.path.clone(),
        });

        // Get outgoing links for each note
        if let Ok(out_links) = storage.get_forward_links(&note.path) {
            for link in out_links {
                if let Some(target_id) = link.target_id {
                    edges.push(GraphEdge {
                        source: note.id.clone(),
                        target: target_id,
                    });
                }
            }
        }
    }

    Ok(GraphData { nodes, edges })
}

// ── Tags ───────────────────────────────────────────────────────

#[tauri::command]
fn list_tags(state: State<AppState>) -> Result<Vec<TagInfo>, String> {
    let guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_ref().ok_or("No vault open")?;

    let tags = storage.list_tags().map_err(|e| e.to_string())?;
    Ok(tags.into_iter().map(|(name, count)| TagInfo { name, count }).collect())
}

#[tauri::command]
fn get_notes_by_tag(state: State<AppState>, tag: String) -> Result<Vec<Note>, String> {
    let guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_ref().ok_or("No vault open")?;

    storage.notes_for_tag(&tag).map_err(|e| e.to_string())
}

// ── Daily Notes ────────────────────────────────────────────────

#[tauri::command]
fn get_daily_note(state: State<AppState>, date: Option<String>) -> Result<DailyNoteResponse, String> {
    let guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_ref().ok_or("No vault open")?;

    let date_str = date.unwrap_or_else(|| {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    });

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
}

// ── Knowledge Palace ───────────────────────────────────────────

#[tauri::command]
async fn kp_search(state: State<'_, AppState>, query: String, limit: Option<i64>) -> Result<Vec<serde_json::Value>, String> {
    let kp_config = {
        let pc_guard = state.profile_config.lock().map_err(|e| e.to_string())?;
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
    let results = connector.semantic_search(&query, limit.unwrap_or(10))
        .await
        .map_err(|e| e.to_string())?;

    Ok(results.into_iter().map(|r| serde_json::to_value(r).unwrap()).collect())
}

#[tauri::command]
async fn kp_health(state: State<'_, AppState>) -> Result<bool, String> {
    let kp_config = {
        let pc_guard = state.profile_config.lock().map_err(|e| e.to_string())?;
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

// ── Reindex ────────────────────────────────────────────────────

#[tauri::command]
fn reindex(state: State<AppState>) -> Result<serde_json::Value, String> {
    let mut guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_mut().ok_or("No vault open")?;

    let stats = storage.reindex_all().map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "created": stats.created,
        "updated": stats.updated,
        "removed": stats.removed,
        "errors": stats.errors,
    }))
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
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState {
            config: Mutex::new(config),
            storage: Mutex::new(None),
            profile_config: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            init_vault,
            open_vault,
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
            reindex,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Cerebro");
}

fn main() {
    run();
}
