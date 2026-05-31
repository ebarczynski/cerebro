//! MCP (Model Context Protocol) server for Cerebro.
//!
//! Exposes note CRUD, search, graph, and Knowledge Palace tools
//! via JSON-RPC 2.0 over stdio.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::app::Config;
use crate::storage::Storage;

// ── JSON-RPC Types ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

// ── MCP Tool Definition ─────────────────────────────────────────

#[derive(Debug, Serialize)]
struct McpTool {
    name: String,
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: Value,
}

fn cerebro_tools() -> Vec<McpTool> {
    vec![
        McpTool {
            name: "cerebro_search".into(),
            description: "Search notes using full-text search. Returns matching notes with snippets.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Search query using FTS5 syntax"},
                    "limit": {"type": "integer", "description": "Max results (default 10)", "default": 10},
                    "profile": {"type": "string", "description": "Profile to search (default: active)"}
                },
                "required": ["query"]
            }),
        },
        McpTool {
            name: "cerebro_read_note".into(),
            description: "Read a note by title, path, or ID. Returns full markdown content.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "identifier": {"type": "string", "description": "Note title, path fragment, or UUID"},
                    "profile": {"type": "string", "description": "Profile (default: active)"}
                },
                "required": ["identifier"]
            }),
        },
        McpTool {
            name: "cerebro_create_note".into(),
            description: "Create a new note with markdown content. Auto-generates frontmatter and updates link graph.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string", "description": "Note title"},
                    "content": {"type": "string", "description": "Markdown body"},
                    "path": {"type": "string", "description": "Optional directory path within vault"},
                    "tags": {"type": "array", "items": {"type": "string"}, "description": "Tags to apply"},
                    "profile": {"type": "string", "description": "Profile (default: active)"}
                },
                "required": ["title", "content"]
            }),
        },
        McpTool {
            name: "cerebro_update_note".into(),
            description: "Update an existing note's content. Reconciles link graph automatically.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "identifier": {"type": "string", "description": "Note title, path, or UUID"},
                    "content": {"type": "string", "description": "New markdown body"},
                    "profile": {"type": "string", "description": "Profile (default: active)"}
                },
                "required": ["identifier", "content"]
            }),
        },
        McpTool {
            name: "cerebro_delete_note".into(),
            description: "Soft-delete a note. Recoverable within retention period.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "identifier": {"type": "string", "description": "Note title, path, or UUID"},
                    "profile": {"type": "string", "description": "Profile (default: active)"}
                },
                "required": ["identifier"]
            }),
        },
        McpTool {
            name: "cerebro_backlinks".into(),
            description: "Get all notes that link TO the specified note (backlinks).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "identifier": {"type": "string", "description": "Note title, path, or UUID"},
                    "profile": {"type": "string", "description": "Profile (default: active)"}
                },
                "required": ["identifier"]
            }),
        },
        McpTool {
            name: "cerebro_list_tags".into(),
            description: "List all tags with note counts, or notes for a specific tag.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "tag": {"type": "string", "description": "Optional: filter to specific tag"},
                    "profile": {"type": "string", "description": "Profile (default: active)"}
                }
            }),
        },
        McpTool {
            name: "cerebro_list_notes".into(),
            description: "List notes, optionally filtered by tag.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "tag": {"type": "string", "description": "Filter by tag"},
                    "limit": {"type": "integer", "default": 20},
                    "offset": {"type": "integer", "default": 0},
                    "profile": {"type": "string", "description": "Profile (default: active)"}
                }
            }),
        },
    ]
}

// ── MCP Server ───────────────────────────────────────────────────

pub struct McpServer;

impl McpServer {
    /// Run the MCP server, reading JSON-RPC from stdin and writing to stdout.
    pub fn run(config: Config) -> Result<()> {
        let stdin = io::stdin();
        let mut stdout = io::stdout();
        let reader = stdin.lock();

        for line in reader.lines() {
            let line = line?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let request: JsonRpcRequest = match serde_json::from_str(line) {
                Ok(r) => r,
                Err(e) => {
                    let resp = JsonRpcResponse {
                        jsonrpc: "2.0".into(),
                        id: None,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32700,
                            message: format!("Parse error: {}", e),
                        }),
                    };
                    writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
                    stdout.flush()?;
                    continue;
                }
            };

            let response = handle_request(&request, &config);
            writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
            stdout.flush()?;
        }

        Ok(())
    }
}

fn open_storage(config: &Config, profile: Option<&str>) -> Result<Storage> {
    let profile_name = profile
        .or(config.active_profile.as_deref())
        .or(Some(&config.default_profile))
        .unwrap_or(&config.default_profile);

    let profile_dir = config.profiles_dir.join(profile_name);
    let profile_config = crate::config::profile::ProfileConfig::load_or_default(&profile_dir, profile_name)?;
    Storage::open(&profile_dir, &profile_config)
}

fn handle_request(request: &JsonRpcRequest, config: &Config) -> JsonRpcResponse {
    match request.method.as_str() {
        "initialize" => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: request.id.clone(),
            result: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": { "listChanged": false }
                },
                "serverInfo": {
                    "name": "cerebro",
                    "version": env!("CARGO_PKG_VERSION")
                }
            })),
            error: None,
        },

        "notifications/initialized" => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: request.id.clone(),
            result: Some(json!({})),
            error: None,
        },

        "tools/list" => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: request.id.clone(),
            result: Some(json!({
                "tools": cerebro_tools()
            })),
            error: None,
        },

        "tools/call" => {
            let params = request.params.clone().unwrap_or_default();
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_default();

            let result = execute_tool(tool_name, &args, config);

            JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: request.id.clone(),
                result: Some(json!({
                    "content": [{
                        "type": "text",
                        "text": result
                    }]
                })),
                error: None,
            }
        }

        _ => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: request.id.clone(),
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: format!("Method not found: {}", request.method),
            }),
        },
    }
}

fn execute_tool(name: &str, args: &Value, config: &Config) -> String {
    let profile = args.get("profile").and_then(|v| v.as_str());

    let mut storage = match open_storage(config, profile) {
        Ok(s) => s,
        Err(e) => return format!("Error opening storage: {}", e),
    };

    match name {
        "cerebro_search" => {
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(10);

            match storage.search(query, limit) {
                Ok(results) => serde_json::to_string_pretty(&results).unwrap_or_else(|e| e.to_string()),
                Err(e) => format!("Search error: {}", e),
            }
        }

        "cerebro_read_note" => {
            let identifier = args.get("identifier").and_then(|v| v.as_str()).unwrap_or("");
            match storage.read_note(identifier) {
                Ok(nc) => nc.body,
                Err(e) => format!("Note not found: {}", e),
            }
        }

        "cerebro_create_note" => {
            let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let path = args.get("path").and_then(|v| v.as_str());
            let tags: Vec<String> = args.get("tags")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();

            match storage.create_note(title, content, path, Some(tags)) {
                Ok(note) => serde_json::to_string_pretty(&note).unwrap_or_else(|e| e.to_string()),
                Err(e) => format!("Create error: {}", e),
            }
        }

        "cerebro_update_note" => {
            let identifier = args.get("identifier").and_then(|v| v.as_str()).unwrap_or("");
            let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");

            match storage.update_note(identifier, content) {
                Ok(note) => serde_json::to_string_pretty(&note).unwrap_or_else(|e| e.to_string()),
                Err(e) => format!("Update error: {}", e),
            }
        }

        "cerebro_delete_note" => {
            let identifier = args.get("identifier").and_then(|v| v.as_str()).unwrap_or("");
            match storage.delete_note(identifier) {
                Ok(()) => format!("Note '{}' deleted", identifier),
                Err(e) => format!("Delete error: {}", e),
            }
        }

        "cerebro_backlinks" => {
            let identifier = args.get("identifier").and_then(|v| v.as_str()).unwrap_or("");
            match storage.get_backlinks(identifier) {
                Ok(links) => serde_json::to_string_pretty(&links).unwrap_or_else(|e| e.to_string()),
                Err(e) => format!("Backlinks error: {}", e),
            }
        }

        "cerebro_list_tags" => {
            match storage.list_tags() {
                Ok(tags) => {
                    let tag_strs: Vec<String> = tags.iter()
                        .map(|(name, count)| format!("{} ({})", name, count))
                        .collect();
                    tag_strs.join("\n")
                }
                Err(e) => format!("Tags error: {}", e),
            }
        }

        "cerebro_list_notes" => {
            let tag = args.get("tag").and_then(|v| v.as_str());
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20);
            let offset = args.get("offset").and_then(|v| v.as_i64()).unwrap_or(0);

            match storage.list_notes(tag, limit, offset) {
                Ok(notes) => {
                    let note_strs: Vec<String> = notes.iter()
                        .map(|n| format!("{} [{}] {}", n.title, n.path, n.updated_at))
                        .collect();
                    note_strs.join("\n")
                }
                Err(e) => format!("List error: {}", e),
            }
        }

        _ => format!("Unknown tool: {}", name),
    }
}
