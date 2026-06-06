//! Cerebro GUI — Tauri 2.0 desktop application.
//!
//! Exposes the Cerebro core library (storage, search, links, tags,
//! Knowledge Palace connector, Confluence publisher) as Tauri commands
//! consumed by the React/TypeScript frontend.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use notify::{RecommendedWatcher, RecursiveMode, Watcher, Config as NotifyConfig, EventKind};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::{FileDialogBuilder, DialogExt};

use cerebro::config::app::Config;
use cerebro::config::profile::ProfileConfig;
use cerebro::storage::{Note, NoteContent, SearchResult, Storage};

// ── Application State ──────────────────────────────────────────

pub struct AppState {
    pub config: Mutex<Config>,
    pub storage: Mutex<Option<Storage>>,
    pub profile_config: Mutex<Option<ProfileConfig>>,
    pub watcher: Mutex<Option<RecommendedWatcher>>,
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

#[derive(Debug, Serialize, Deserialize)]
pub struct TemplateInfo {
    pub name: String,
    pub path: String,
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
        .map_err(|e| {
            let err_msg = e.to_string();
            if err_msg.contains("Permission denied") || err_msg.contains("permission denied") {
                format!(
                    "Permission denied opening database at {}. Check that you own this directory and the file is not locked.",
                    profile_dir.join("db.sqlite").display()
                )
            } else {
                err_msg
            }
        })?;

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
fn home_dir() -> Result<String, String> {
    dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .ok_or_else(|| "Could not determine home directory".to_string())
}

#[tauri::command]
fn create_vault(state: State<AppState>, name: String, path: String) -> Result<VaultInfo, String> {
    let profile_dir = PathBuf::from(&path);

    // Create directory structure
    std::fs::create_dir_all(&profile_dir).map_err(|e| {
        format!("Failed to create vault directory at {}: {}", path, e)
    })?;

    // Initialize a ProfileConfig with the given name
    let profile_config = ProfileConfig::new(&name);
    profile_config.save_to(&profile_dir).map_err(|e| {
        format!("Failed to save profile config: {}", e)
    })?;

    // Open storage
    let storage = Storage::open(&profile_dir, &profile_config)
        .map_err(|e| e.to_string())?;

    let note_count = storage.list_notes(None, 10000, 0).map_err(|e| e.to_string())?.len();
    let tag_count = storage.list_tags().map_err(|e| e.to_string())?.len();

    // Store in AppState
    let mut storage_guard = state.storage.lock().map_err(|e| e.to_string())?;
    *storage_guard = Some(storage);
    let mut pc_guard = state.profile_config.lock().map_err(|e| e.to_string())?;
    *pc_guard = Some(profile_config);

    Ok(VaultInfo {
        name,
        path,
        note_count,
        tag_count,
    })
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

// ── Keyword Links (Knowledge Graph) ──────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct KeywordLink {
    pub source_id: String,
    pub target_id: String,
    pub confidence: f64,
}

#[tauri::command]
fn get_keyword_links(state: State<AppState>) -> Result<Vec<KeywordLink>, String> {
    let guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_ref().ok_or("No vault open")?;

    let links = storage.compute_keyword_links().map_err(|e| e.to_string())?;
    Ok(links.into_iter().map(|(source, target, confidence)| KeywordLink {
        source_id: source,
        target_id: target,
        confidence,
    }).collect())
}

// ── Category & Favorites ─────────────────────────────────────

#[tauri::command]
fn set_note_category(state: State<AppState>, note_id: String, category: Option<String>) -> Result<(), String> {
    let guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_ref().ok_or("No vault open")?;

    storage.set_category(&note_id, category.as_deref()).map_err(|e| e.to_string())
}

#[tauri::command]
fn toggle_note_favorite(state: State<AppState>, note_id: String, favorite: bool) -> Result<(), String> {
    let guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_ref().ok_or("No vault open")?;

    storage.set_favorite(&note_id, favorite).map_err(|e| e.to_string())
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

// ── Templates ──────────────────────────────────────────────────

#[tauri::command]
fn list_templates(state: State<AppState>) -> Result<Vec<TemplateInfo>, String> {
    let guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_ref().ok_or("No vault open")?;

    let templates_dir = storage.notes_dir_path().join("templates");

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
}

#[tauri::command]
fn get_template(state: State<AppState>, name: String) -> Result<String, String> {
    let guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_ref().ok_or("No vault open")?;

    let template_path = storage.notes_dir_path().join("templates").join(format!("{name}.md"));
    std::fs::read_to_string(&template_path).map_err(|e| {
        format!("Failed to read template '{}': {}", name, e)
    })
}

#[tauri::command]
fn create_template(state: State<AppState>, name: String, content: String) -> Result<(), String> {
    let guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_ref().ok_or("No vault open")?;

    let templates_dir = storage.notes_dir_path().join("templates");
    std::fs::create_dir_all(&templates_dir).map_err(|e| e.to_string())?;

    let template_path = templates_dir.join(format!("{name}.md"));
    std::fs::write(&template_path, &content).map_err(|e| {
        format!("Failed to write template '{}': {}", name, e)
    })
}

#[tauri::command]
fn delete_template(state: State<AppState>, name: String) -> Result<(), String> {
    let guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_ref().ok_or("No vault open")?;

    let template_path = storage.notes_dir_path().join("templates").join(format!("{name}.md"));
    std::fs::remove_file(&template_path).map_err(|e| {
        format!("Failed to delete template '{}': {}", name, e)
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
    let (content, notes_dir) = {
        let guard = state.storage.lock().map_err(|e| e.to_string())?;
        let storage = guard.as_ref().ok_or("No vault open")?;

        let template_path = storage.notes_dir_path()
            .join("templates")
            .join(format!("{template_name}.md"));
        let raw = std::fs::read_to_string(&template_path).map_err(|e| {
            format!("Failed to read template '{}': {}", template_name, e)
        })?;
        (raw, storage.notes_dir_path().to_path_buf())
    }; // guard dropped here

    let now = chrono::Local::now();
    let date_str = now.format("%Y-%m-%d").to_string();
    let time_str = now.format("%H:%M").to_string();

    let content = content
        .replace("{{title}}", &title)
        .replace("{{date}}", &date_str)
        .replace("{{time}}", &time_str);

    let mut guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_mut().ok_or("No vault open")?;

    // Temporarily we need to ensure we can use storage — but notes_dir was read above.
    // The storage lock was released and re-acquired; the content is ready.
    drop(notes_dir);
    storage
        .create_note(&title, &content, folder.as_deref(), tags)
        .map_err(|e| e.to_string())
}

// ── Entry Point ────────────────────────────────────────────────

// ── Image / Attachment Commands ────────────────────────────────

#[tauri::command]
fn save_image(state: State<AppState>, file_name: String, data: String) -> Result<String, String> {
    use base64::Engine;
    let guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_ref().ok_or("No vault open")?;

    let attachments_dir = storage.attachments_dir();
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
    let guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_ref().ok_or("No vault open")?;

    let file_path = storage.notes_dir_path().join(&path);
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

#[tauri::command]
fn import_document(state: State<AppState>, req: ImportDocumentRequest) -> Result<ImportDocumentResponse, String> {
    use cerebro::import::import_file;

    let path = std::path::Path::new(&req.file_path);
    let result = import_file(path).map_err(|e| format!("Import failed: {}", e))?;

    let title = req.title.unwrap_or(result.title);
    let mut tags = result.tags;
    if let Some(extra) = req.tags {
        tags.extend(extra);
    }

    let mut guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_mut().ok_or("No vault open — open a vault first")?;

    let note = storage
        .create_note(&title, &result.markdown, req.folder.as_deref(), Some(tags))
        .map_err(|e| e.to_string())?;

    Ok(ImportDocumentResponse {
        title: note.title.clone(),
        format: result.format.name().to_string(),
        tags: note.tags.clone(),
        note_id: note.id,
        note_path: note.path,
    })
}

/// Import multiple files at once (batch import for drag-and-drop).
#[tauri::command]
fn import_documents(state: State<AppState>, file_paths: Vec<String>, folder: Option<String>) -> Result<Vec<ImportDocumentResponse>, String> {
    use cerebro::import::import_file;

    let mut results = Vec::new();

    let mut guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_mut().ok_or("No vault open — open a vault first")?;

    for file_path in &file_paths {
        let path = std::path::Path::new(file_path);
        match import_file(path) {
            Ok(result) => {
                let tags = result.tags;
                let note = storage
                    .create_note(&result.title, &result.markdown, folder.as_deref(), Some(tags))
                    .map_err(|e| e.to_string())?;

                results.push(ImportDocumentResponse {
                    title: note.title.clone(),
                    format: result.format.name().to_string(),
                    tags: note.tags.clone(),
                    note_id: note.id,
                    note_path: note.path,
                });
            }
            Err(e) => {
                return Err(format!("Failed to import '{}': {}", file_path, e));
            }
        }
    }

    Ok(results)
}

// ── Transclusion Resolution ──────────────────────────────────

#[tauri::command]
fn resolve_transclusions(state: State<AppState>, content: String) -> Result<String, String> {
    use cerebro::parser::wikilinks::extract_transclusions;

    let transclusions = extract_transclusions(&content);
    if transclusions.is_empty() {
        return Ok(content);
    }

    let guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_ref().ok_or("No vault open")?;

    let mut result = content;
    for tc in transclusions {
        // Try to find the note by title
        if let Ok(nc) = storage.read_note(&tc.target) {
            let replacement = nc.body.clone();
            // Handle with heading: ![[Target#Heading]]
            if let Some(ref heading) = tc.heading {
                let full_ref = format!("![[{}#{}]]", tc.target, heading);
                result = result.replace(&full_ref, &replacement);
            }
            // Handle plain: ![[Target]]
            let plain_ref = format!("![[{}]]", tc.target);
            result = result.replace(&plain_ref, &replacement);
        }
    }
    Ok(result)
}

// ── Text Attachment Commands ──────────────────────────────────

#[tauri::command]
fn read_attachment_text(state: State<AppState>, path: String) -> Result<String, String> {
    let guard = state.storage.lock().map_err(|e| e.to_string())?;
    let storage = guard.as_ref().ok_or("No vault open")?;
    let file_path = storage.notes_dir_path().join(&path);
    let content = std::fs::read_to_string(&file_path).map_err(|e| {
        format!("Failed to read {}: {}", file_path.display(), e)
    })?;
    Ok(content)
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
    let mut guard = state.watcher.lock().map_err(|e| e.to_string())?;
    *guard = Some(watcher);

    Ok(())
}

#[tauri::command]
fn stop_watcher(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.watcher.lock().map_err(|e| e.to_string())?;
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
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState {
            config: Mutex::new(config),
            storage: Mutex::new(None),
            profile_config: Mutex::new(None),
            watcher: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            home_dir,
            create_vault,
            pick_directory,
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
            set_note_category,
            toggle_note_favorite,
            get_keyword_links,
            reindex,
            save_image,
            read_image_as_base64,
            read_attachment_text,
            import_document,
            import_documents,
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
