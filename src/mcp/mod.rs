//! MCP (Model Context Protocol) server for Cerebro.
//!
//! Exposes note CRUD, search, graph, and Knowledge Palace tools
//! via JSON-RPC 2.0 over stdio.
//!
//! Sandbox (ADR 0007): per-profile access policies (`none|read|write`),
//! CLI `--scope` override, an append-only JSONL audit journal, and
//! locked-vault invisibility — a protected vault that was not unlocked at
//! server start is denied to every tool.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::app::{Config, McpAccess};
use crate::crypto::VaultKey;
use crate::storage::Storage;

/// Tools that mutate a vault — require `write` access.
const WRITE_TOOLS: &[&str] = &["cerebro_create_note", "cerebro_update_note", "cerebro_delete_note"];

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
        McpTool {
            name: "cerebro_graph".into(),
            description: "Get the full knowledge graph: nodes (notes) and edges (wikilinks). Unresolved links have target=null and carry target_ref so broken links can be rendered.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "profile": {"type": "string", "description": "Profile (default: active)"}
                }
            }),
        },
        McpTool {
            name: "cerebro_kp_search".into(),
            description: "Semantic/hybrid/keyword search via Knowledge Palace (pgvector). Falls back to local FTS when KP is disabled or unreachable. Contract verified against KP source (ADR 0005 addendum).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Search query"},
                    "mode": {"type": "string", "enum": ["semantic", "keyword", "hybrid"], "default": "hybrid", "description": "Search mode"},
                    "limit": {"type": "integer", "default": 5},
                    "profile": {"type": "string", "description": "Profile used for the local fallback (default: active)"}
                },
                "required": ["query"]
            }),
        },
    ]
}

// ── MCP Server ───────────────────────────────────────────────────

pub struct McpServer;

/// Protocol versions this server can speak. Per the MCP version-negotiation
/// rule the server echoes a client-offered version it supports, otherwise
/// replies with its latest (ADR 0005).
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18"];
const LATEST_PROTOCOL_VERSION: &str = "2025-06-18";

/// Session-scoped sandbox state (ADR 0007): unlocked vault keys, CLI scope
/// overrides, and the audit journal path.
pub struct McpSession {
    pub keys: HashMap<String, VaultKey>,
    pub scopes: HashMap<String, McpAccess>,
    pub audit_path: PathBuf,
}

/// CLI options for starting the MCP server.
#[derive(Debug, Default, Clone)]
pub struct McpStartOpts {
    /// `--scope profile:level` overrides (highest precedence).
    pub scopes: Vec<(String, McpAccess)>,
    /// `--unlock profile` — prompt for the passphrase at server start so the
    /// session can open that protected vault. Unprompted protected vaults
    /// stay locked and invisible.
    pub unlock: Vec<String>,
}

/// Append one audit line. Audit failures are logged, never fatal: an
/// unwritable audit file must not take the server down mid-session.
fn audit_line(session: &McpSession, tool: &str, profile: &str, decision: &str, reason: &str) {
    let line = serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "tool": tool,
        "profile": profile,
        "decision": decision,
        "reason": reason,
    });
    let result = (|| -> std::io::Result<()> {
        if let Some(dir) = session.audit_path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&session.audit_path)?;
        writeln!(f, "{line}")
    })();
    if let Err(e) = result {
        tracing::warn!("audit write failed: {e}");
    }
}

/// Resolve the effective access level for a profile: CLI scope override →
/// config policy → default (active profile read, others none).
fn access_for(session: &McpSession, config: &Config, profile: &str) -> McpAccess {
    if let Some(level) = session.scopes.get(profile) {
        return *level;
    }
    if let Some(policy) = config.mcp.profiles.get(profile) {
        if let Some(level) = McpAccess::parse(&policy.allow) {
            return level;
        }
    }
    let active = config
        .active_profile
        .as_deref()
        .unwrap_or(&config.default_profile);
    if profile == active {
        McpAccess::Read
    } else {
        McpAccess::None
    }
}

/// Open the target profile's storage under the sandbox rules. Protected
/// vaults need a session key (from `--unlock` at start) — otherwise they are
/// invisible (ADR 0006/0007: locked means invisible, never a prompt).
fn open_sandboxed(
    session: &McpSession,
    config: &Config,
    profile: Option<&str>,
) -> std::result::Result<Storage, String> {
    let profile_name = profile
        .or(config.active_profile.as_deref())
        .unwrap_or(&config.default_profile)
        .to_string();
    let profile_dir = config.profiles_dir.join(&profile_name);
    let profile_config =
        crate::config::profile::ProfileConfig::load_or_default(&profile_dir, &profile_name)
            .map_err(|e| format!("vault config error: {e}"))?;

    if profile_config.encryption {
        let key = session.keys.get(&profile_name).ok_or_else(|| {
            format!(
                "VAULT LOCKED: protected vault '{}' was not unlocked at server start \
                 (--unlock {}); its contents are invisible to this session",
                profile_name, profile_name
            )
        })?;
        Storage::open_with_key(&profile_dir, &profile_config, Some(key))
            .map_err(|e| format!("vault open error: {e}"))
    } else {
        Storage::open(&profile_dir, &profile_config).map_err(|e| format!("vault open error: {e}"))
    }
}

impl McpServer {
    /// Run the MCP server, reading JSON-RPC from stdin and writing to stdout.
    pub fn run(config: Config, opts: McpStartOpts) -> Result<()> {
        // Build the session: unlock keys first (prompting before the JSON-RPC
        // loop starts — never mid-session).
        let mut keys = HashMap::new();
        for name in &opts.unlock {
            let profile_dir = config.profiles_dir.join(name);
            let cfg = crate::config::profile::ProfileConfig::load_or_default(&profile_dir, name)?;
            if !cfg.encryption {
                continue; // nothing to unlock
            }
            let kdf = match cfg.protection.kdf.as_ref() {
                Some(k) => k,
                None => {
                    eprintln!("skip unlock '{name}': vault config missing KDF parameters");
                    continue;
                }
            };
            let wrapped = match cfg.protection.wrapped_key_b64.as_deref() {
                Some(w) => w,
                None => {
                    eprintln!("skip unlock '{name}': vault config missing wrapped key");
                    continue;
                }
            };
            let passphrase = rpassword::prompt_password(format!(
                "Unlock passphrase for vault '{name}' (MCP session): "
            ))?;
            match VaultKey::unwrap_with_passphrase(wrapped, &passphrase, kdf) {
                Ok(key) => {
                    keys.insert(name.clone(), key);
                }
                Err(_) => eprintln!("wrong passphrase for vault '{name}' — it stays locked"),
            }
        }

        let mut scopes = HashMap::new();
        for (name, level) in &opts.scopes {
            scopes.insert(name.clone(), *level);
        }

        let audit_path = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("cerebro")
            .join("audit")
            .join("mcp.jsonl");
        let session = McpSession { keys, scopes, audit_path };

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

            // JSON-RPC 2.0: notifications (requests without an id, including
            // `notifications/initialized`) must NOT be answered. Replying to
            // them is a protocol violation that strict clients misroute
            // (grill finding F17).
            if request.id.is_none() || request.method.starts_with("notifications/") {
                continue;
            }

            let response = handle_request(&request, &config, &session);
            if let Some(resp) = response {
                writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
                stdout.flush()?;
            }
        }

        Ok(())
    }
}

fn handle_request(
    request: &JsonRpcRequest,
    config: &Config,
    session: &McpSession,
) -> Option<JsonRpcResponse> {
    match request.method.as_str() {
        "initialize" => {
            let client_version = request
                .params
                .as_ref()
                .and_then(|p| p.get("protocolVersion"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let negotiated = if SUPPORTED_PROTOCOL_VERSIONS.contains(&client_version) {
                client_version
            } else {
                LATEST_PROTOCOL_VERSION
            };
            Some(JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: request.id.clone(),
                result: Some(json!({
                    "protocolVersion": negotiated,
                    "capabilities": {
                        "tools": { "listChanged": false }
                    },
                    "serverInfo": {
                        "name": "cerebro",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                })),
                error: None,
            })
        }

        "tools/list" => Some(JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: request.id.clone(),
            result: Some(json!({
                "tools": cerebro_tools()
            })),
            error: None,
        }),

        "tools/call" => {
            let params = request.params.clone().unwrap_or_default();
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_default();

            // Unknown tools are a PROTOCOL-level error (-32602), not a
            // success-shaped text result: clients must be able to tell a
            // missing tool from a failed call (grill finding F9/F17).
            if !cerebro_tools().iter().any(|t| t.name == tool_name) {
                return Some(JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id: request.id.clone(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32602,
                        message: format!("Unknown tool: {}", tool_name),
                    }),
                });
            }

            // Sandbox: resolve the effective profile + access level, enforce,
            // audit. Locked/hidden vaults and policy denials are first-class
            // isError results so clients can react (ADR 0007).
            let target_profile = args
                .get("profile")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    config
                        .active_profile
                        .clone()
                        .unwrap_or_else(|| config.default_profile.clone())
                });
            let access = access_for(session, config, &target_profile);
            let tool_in_deny_list = config
                .mcp
                .profiles
                .get(&target_profile)
                .map(|p| p.deny_tools.iter().any(|t| t == tool_name))
                .unwrap_or(false);

            if tool_in_deny_list {
                audit_line(session, tool_name, &target_profile, "deny", "tool in profile deny-list");
                return Some(JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id: request.id.clone(),
                    result: Some(json!({
                        "content": [{"type": "text", "text":
                            format!("Access denied: tool '{tool_name}' is denied for profile '{target_profile}' by policy.")}],
                        "isError": true
                    })),
                    error: None,
                });
            }

            let is_write = WRITE_TOOLS.contains(&tool_name);
            if access == McpAccess::None {
                audit_line(session, tool_name, &target_profile, "deny", "policy: profile not exposed (allow=none)");
                return Some(JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id: request.id.clone(),
                    result: Some(json!({
                        "content": [{"type": "text", "text":
                            format!("Access denied: profile '{target_profile}' is not exposed over MCP (allow=none).")}],
                        "isError": true
                    })),
                    error: None,
                });
            }
            if is_write && access != McpAccess::Write {
                audit_line(session, tool_name, &target_profile, "deny", "policy: read-only access");
                return Some(JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id: request.id.clone(),
                    result: Some(json!({
                        "content": [{"type": "text", "text":
                            format!("Access denied: profile '{target_profile}' is read-only over MCP.")}],
                        "isError": true
                    })),
                    error: None,
                });
            }

            // Tool execution failures are reported as a result with
            // isError=true per the MCP tools specification.
            let result = execute_tool(tool_name, &args, config, session);
            let (text, is_error) = match result {
                Ok(text) => {
                    audit_line(session, tool_name, &target_profile, "allow", "policy");
                    (text, false)
                }
                Err(e) => {
                    audit_line(session, tool_name, &target_profile, "error", &e);
                    (e, true)
                }
            };

            Some(JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: request.id.clone(),
                result: Some(json!({
                    "content": [{
                        "type": "text",
                        "text": text
                    }],
                    "isError": is_error
                })),
                error: None,
            })
        }

        _ => Some(JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: request.id.clone(),
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: format!("Method not found: {}", request.method),
            }),
        }),
    }
}

fn execute_tool(
    name: &str,
    args: &Value,
    config: &Config,
    session: &McpSession,
) -> std::result::Result<String, String> {
    let profile = args.get("profile").and_then(|v| v.as_str());

    let mut storage = match open_sandboxed(session, config, profile) {
        Ok(s) => s,
        Err(e) => return Err(e),
    };

    match name {
        "cerebro_search" => {
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(10);

            match storage.search(query, limit) {
                Ok(results) => serde_json::to_string_pretty(&results)
                    .map_err(|e| format!("Serialization error: {}", e)),
                Err(e) => Err(format!("Search error: {}", e)),
            }
        }

        "cerebro_read_note" => {
            let identifier = args.get("identifier").and_then(|v| v.as_str()).unwrap_or("");
            match storage.read_note(identifier) {
                Ok(nc) => Ok(nc.body),
                Err(e) => Err(format!("Note not found: {}", e)),
            }
        }

        "cerebro_create_note" => {
            let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let path = args.get("path").and_then(|v| v.as_str());
            let tags: Vec<String> = args
                .get("tags")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();

            match storage.create_note(title, content, path, Some(tags)) {
                Ok(note) => serde_json::to_string_pretty(&note)
                    .map_err(|e| format!("Serialization error: {}", e)),
                Err(e) => Err(format!("Create error: {}", e)),
            }
        }

        "cerebro_update_note" => {
            let identifier = args.get("identifier").and_then(|v| v.as_str()).unwrap_or("");
            let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");

            match storage.update_note(identifier, content) {
                Ok(note) => serde_json::to_string_pretty(&note)
                    .map_err(|e| format!("Serialization error: {}", e)),
                Err(e) => Err(format!("Update error: {}", e)),
            }
        }

        "cerebro_delete_note" => {
            let identifier = args.get("identifier").and_then(|v| v.as_str()).unwrap_or("");
            match storage.delete_note(identifier) {
                Ok(()) => Ok(format!("Note '{}' deleted", identifier)),
                Err(e) => Err(format!("Delete error: {}", e)),
            }
        }

        "cerebro_backlinks" => {
            let identifier = args.get("identifier").and_then(|v| v.as_str()).unwrap_or("");
            match storage.get_backlinks(identifier) {
                Ok(links) => serde_json::to_string_pretty(&links)
                    .map_err(|e| format!("Serialization error: {}", e)),
                Err(e) => Err(format!("Backlinks error: {}", e)),
            }
        }

        "cerebro_list_tags" => match storage.list_tags() {
            Ok(tags) => {
                let tag_strs: Vec<String> = tags
                    .iter()
                    .map(|(name, count)| format!("{} ({})", name, count))
                    .collect();
                Ok(tag_strs.join("\n"))
            }
            Err(e) => Err(format!("Tags error: {}", e)),
        },

        "cerebro_list_notes" => {
            let tag = args.get("tag").and_then(|v| v.as_str());
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20);
            let offset = args.get("offset").and_then(|v| v.as_i64()).unwrap_or(0);

            match storage.list_notes(tag, limit, offset) {
                Ok(notes) => {
                    let note_strs: Vec<String> = notes
                        .iter()
                        .map(|n| format!("{} [{}] {}", n.title, n.path, n.updated_at))
                        .collect();
                    Ok(note_strs.join("\n"))
                }
                Err(e) => Err(format!("List error: {}", e)),
            }
        }

        "cerebro_graph" => match storage.get_graph() {
            Ok((notes, edges)) => {
                let nodes: Vec<Value> = notes
                    .iter()
                    .map(|n| {
                        json!({
                            "id": n.id,
                            "title": n.title,
                            "path": n.path,
                            "tags": n.tags,
                        })
                    })
                    .collect();
                let edge_list: Vec<Value> = edges
                    .iter()
                    .map(|l| {
                        json!({
                            "source": l.source_id,
                            "target": l.target_id,
                            "target_ref": l.target_ref,
                        })
                    })
                    .collect();
                serde_json::to_string_pretty(&json!({
                    "node_count": nodes.len(),
                    "edge_count": edge_list.len(),
                    "nodes": nodes,
                    "edges": edge_list,
                }))
                .map_err(|e| format!("Serialization error: {}", e))
            }
            Err(e) => Err(format!("Graph error: {}", e)),
        },

        "cerebro_kp_search" => {
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("hybrid");
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(5);

            // KP calls are async; the MCP loop is sync — drive them on a
            // throwaway current-thread runtime (one call at a time).
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| format!("Runtime error: {}", e))?;

            let kp_config = crate::kp::KpConfig {
                enabled: config.knowledge_palace.enabled,
                endpoint: if config.knowledge_palace.endpoint.is_empty() {
                    "http://localhost:8080".to_string()
                } else {
                    config.knowledge_palace.endpoint.clone()
                },
            };
            let connector = crate::kp::KpConnector::new(kp_config);

            let kp_result = rt.block_on(connector.search_with_mode(query, mode, limit));
            match kp_result {
                Ok(hits) => {
                    let payload = serde_json::json!({
                        "source": "knowledge_palace",
                        "mode": mode,
                        "query": query,
                        "total": hits.len(),
                        "hits": hits,
                    });
                    serde_json::to_string_pretty(&payload)
                        .map_err(|e| format!("Serialization error: {}", e))
                }
                Err(kp_err) => {
                    // Graceful degradation per the plan: KP disabled/unreachable
                    // → local FTS with an explicit note, still a success result.
                    match storage.search(query, limit) {
                        Ok(local) => {
                            let payload = serde_json::json!({
                                "source": "local_fts",
                                "note": format!("Knowledge Palace unavailable ({kp_err}); served local FTS results"),
                                "mode": "keyword",
                                "query": query,
                                "total": local.len(),
                                "hits": local,
                            });
                            serde_json::to_string_pretty(&payload)
                                .map_err(|e| format!("Serialization error: {}", e))
                        }
                        Err(local_err) => Err(format!(
                            "KP error: {kp_err}; local fallback error: {local_err}"
                        )),
                    }
                }
            }
        }

        _ => Err(format!("Unknown tool: {}", name)),
    }
}
