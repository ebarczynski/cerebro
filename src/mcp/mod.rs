//! MCP (Model Context Protocol) server for Cerebro.
//!
//! Exposes note CRUD, search, graph, and Knowledge Palace tools
//! via JSON-RPC 2.0 over stdio.
//!
//! # Framing rules this server follows
//!
//! * **stdout carries JSON-RPC and nothing else.** Every line written to
//!   stdout is one JSON value (a response object, or an array of them for a
//!   batch). Anything a library wants to log goes to stderr — see `main.rs`.
//! * Requests may arrive one per line or as a JSON-RPC batch (an array); a
//!   batch is answered by one array, in order.
//! * Notifications (no `id`) are never answered.
//! * A request that is not valid JSON-RPC 2.0 gets `-32600 Invalid Request`
//!   (or `-32700 Parse Error` when the line is not JSON at all, with
//!   `"id": null`).
//! * `tools/call` arguments are checked against the tool's own schema before
//!   the tool runs; a bad call is `-32602 Invalid params`.

use std::io::{self, BufRead, Write};

use anyhow::Result;
use serde::Serialize;
use serde_json::{json, Value};

use crate::config::app::Config;
use crate::storage::Storage;

// ── JSON-RPC 2.0 error codes ─────────────────────────────────────

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

// ── MCP Tool Definition ─────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

fn string_arg(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn cerebro_tools() -> Vec<McpTool> {
    let profile_arg = string_arg("Profile to use. Must already exist (default: the active profile)");
    vec![
        McpTool {
            name: "cerebro_search".into(),
            description: "Search notes using full-text search. Returns matching notes with snippets.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": string_arg("Search query using FTS5 syntax"),
                    "limit": {"type": "integer", "description": "Max results (default 10)", "default": 10},
                    "profile": profile_arg
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
                    "identifier": string_arg("Note title, path, or UUID"),
                    "profile": profile_arg
                },
                "required": ["identifier"]
            }),
        },
        McpTool {
            name: "cerebro_create_note".into(),
            description: "Create a new note with markdown content. Auto-generates frontmatter and updates link graph. Never overwrites an existing note.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": string_arg("Note title"),
                    "content": string_arg("Markdown body"),
                    "path": string_arg("Optional directory path within vault"),
                    "tags": {"type": "array", "items": {"type": "string"}, "description": "Tags to apply"},
                    "profile": profile_arg
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
                    "identifier": string_arg("Note title, path, or UUID"),
                    "content": string_arg("New markdown body"),
                    "profile": profile_arg
                },
                "required": ["identifier", "content"]
            }),
        },
        McpTool {
            name: "cerebro_delete_note".into(),
            description: "Permanently delete a note: its Markdown file and all of its index entries. This cannot be undone.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "identifier": string_arg("Note title, path, or UUID"),
                    "profile": profile_arg
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
                    "identifier": string_arg("Note title, path, or UUID"),
                    "profile": profile_arg
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
                    "tag": string_arg("Optional: filter to specific tag"),
                    "profile": profile_arg
                }
            }),
        },
        McpTool {
            name: "cerebro_list_notes".into(),
            description: "List notes, optionally filtered by tag.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "tag": string_arg("Filter by tag"),
                    "limit": {"type": "integer", "default": 20},
                    "offset": {"type": "integer", "default": 0},
                    "profile": profile_arg
                }
            }),
        },
        McpTool {
            name: "cerebro_graph".into(),
            description: "Get the full knowledge graph: nodes (notes) and edges (wikilinks). Unresolved links have target=null and carry target_ref so broken links can be rendered.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "profile": profile_arg
                }
            }),
        },
        McpTool {
            name: "cerebro_kp_search".into(),
            description: "Semantic/hybrid/keyword search via Knowledge Palace (pgvector). Falls back to local FTS when KP is disabled or unreachable — the result then has source=local_fts. Contract verified against KP source (ADR 0005 addendum).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": string_arg("Search query"),
                    "mode": {"type": "string", "enum": ["semantic", "keyword", "hybrid"], "default": "hybrid", "description": "Search mode"},
                    "limit": {"type": "integer", "default": 5},
                    "profile": profile_arg
                },
                "required": ["query"]
            }),
        },
    ]
}

// ── Responses ────────────────────────────────────────────────────

fn ok(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn err(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
}

/// The `id` to echo back: whatever the client sent if it is a valid id,
/// otherwise null (JSON-RPC 2.0 §4.1).
fn echo_id(value: &Value) -> Value {
    match value.get("id") {
        Some(id @ (Value::String(_) | Value::Number(_))) | Some(id @ Value::Null) => id.clone(),
        _ => Value::Null,
    }
}

fn text_result(text: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

/// Serialize a tool result compactly: it is parsed by the client, never read
/// by a human, and the graph alone runs to megabytes on a large vault.
fn compact(value: &impl Serialize) -> String {
    serde_json::to_string(value).expect("tool results are plain data and always serialize")
}

// ── MCP Server ───────────────────────────────────────────────────

pub struct McpServer;

/// Protocol versions this server can speak. Per the MCP version-negotiation
/// rule the server echoes a client-offered version it supports, otherwise
/// replies with its latest (ADR 0005).
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18"];
const LATEST_PROTOCOL_VERSION: &str = "2025-06-18";

impl McpServer {
    /// Run the MCP server, reading JSON-RPC from stdin and writing to stdout.
    ///
    /// `default_profile` is used for tool calls that do not name a profile
    /// (`cerebro --profile work mcp`); without it the active or default
    /// profile is used.
    pub fn run(config: Config, default_profile: Option<String>) -> Result<()> {
        let stdin = io::stdin();
        let mut stdout = io::stdout();
        let reader = stdin.lock();

        for line in reader.lines() {
            let line = line?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let (is_batch, responses) = handle_line(line, &config, default_profile.as_deref());
            if responses.is_empty() {
                // only notifications
                continue;
            }
            let payload = if is_batch {
                Value::Array(responses)
            } else {
                responses.into_iter().next().expect("non-empty")
            };
            writeln!(stdout, "{payload}")?;
            stdout.flush()?;
        }

        Ok(())
    }
}

/// Handle one input line. Returns `(was a batch, responses in order)`.
fn handle_line(line: &str, config: &Config, default_profile: Option<&str>) -> (bool, Vec<Value>) {
    let value: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(e) => {
            return (
                false,
                vec![err(Value::Null, PARSE_ERROR, format!("Parse error: {e}"))],
            )
        }
    };

    match value {
        Value::Array(items) => {
            if items.is_empty() {
                // JSON-RPC 2.0: an empty array is a single Invalid Request.
                return (true, vec![err(Value::Null, INVALID_REQUEST, "Invalid Request: empty batch")]);
            }
            let responses = items.iter().filter_map(|item| handle_value(item, config, default_profile)).collect();
            (true, responses)
        }
        item => (false, handle_value(&item, config, default_profile).into_iter().collect()),
    }
}

/// Handle one request object. `None` for a notification (never answered).
fn handle_value(value: &Value, config: &Config, default_profile: Option<&str>) -> Option<Value> {
    let invalid = |message: &str| Some(err(echo_id(value), INVALID_REQUEST, message.to_string()));

    let object = value.as_object()?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return invalid("Invalid Request: \"jsonrpc\" must be \"2.0\"");
    }
    let method = match object.get("method").and_then(Value::as_str) {
        Some(method) => method,
        None => return invalid("Invalid Request: \"method\" must be a string"),
    };
    // MCP notifications (`notifications/initialized`, …) are never answered,
    // whatever id they carry; a JSON-RPC notification has no `id` at all.
    if method.starts_with("notifications/") {
        return None;
    }
    let id = object.get("id")?;

    let params = object.get("params").cloned().unwrap_or(Value::Null);
    Some(handle_method(method, id, &params, config, default_profile))
}

fn handle_method(
    method: &str,
    id: &Value,
    params: &Value,
    config: &Config,
    default_profile: Option<&str>,
) -> Value {
    match method {
        "initialize" => {
            let client_version = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
            let negotiated = if SUPPORTED_PROTOCOL_VERSIONS.contains(&client_version) {
                client_version
            } else {
                LATEST_PROTOCOL_VERSION
            };
            ok(
                id,
                json!({
                    "protocolVersion": negotiated,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "cerebro", "version": env!("CARGO_PKG_VERSION") }
                }),
            )
        }

        // MCP: every participant must answer `ping` with an empty result.
        "ping" => ok(id, json!({})),

        "tools/list" => ok(id, json!({ "tools": cerebro_tools() })),

        "tools/call" => handle_tool_call(id, params, config, default_profile),

        _ => err(id.clone(), METHOD_NOT_FOUND, format!("Method not found: {method}")),
    }
}

fn handle_tool_call(id: &Value, params: &Value, config: &Config, default_profile: Option<&str>) -> Value {
    let invalid = |message: String| err(id.clone(), INVALID_PARAMS, message);

    let name = match params.get("name").and_then(Value::as_str) {
        Some(name) => name,
        None => return invalid("Invalid params: \"name\" must name a tool".to_string()),
    };
    // Unknown tools are a PROTOCOL-level error (-32602), not a success-shaped
    // text result: clients must be able to tell a missing tool from a failed
    // call (grill finding F9/F17).
    let Some(tool) = cerebro_tools().into_iter().find(|tool| tool.name == name) else {
        return invalid(format!("Unknown tool: {name}"));
    };

    let arguments = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(arguments @ Value::Object(_)) => arguments.clone(),
        Some(_) => return invalid(format!("Invalid params: \"arguments\" of {name} must be an object")),
    };
    if let Err(message) = validate_arguments(&tool, &arguments) {
        return invalid(message);
    }

    let profile = arguments.get("profile").and_then(Value::as_str).or(default_profile);
    let outcome = execute_tool(&tool.name, &arguments, config, profile);

    // Tool execution failures are reported as a result with isError=true per
    // the MCP tools specification.
    match outcome {
        Ok(text) => ok(id, text_result(text, false)),
        Err(message) => ok(id, text_result(message, true)),
    }
}

/// Check `arguments` against the tool's `inputSchema`: required fields are
/// present, and present fields have the declared JSON type.
fn validate_arguments(tool: &McpTool, arguments: &Value) -> std::result::Result<(), String> {
    let missing = tool
        .input_schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find(|field| arguments.get(field).is_none_or(Value::is_null));
    if let Some(field) = missing {
        return Err(format!("Invalid arguments: '{}' requires '{}'", tool.name, field));
    }

    let Some(properties) = tool.input_schema.get("properties").and_then(Value::as_object) else {
        return Ok(());
    };
    if let Some(values) = arguments.as_object() {
        for (field, value) in values {
            let declared = properties.get(field).and_then(|property| property.get("type")).and_then(Value::as_str);
            if let Some(declared) = declared
                && !has_json_type(value, declared)
            {
                return Err(format!("Invalid arguments: '{field}' of '{}' must be of type {declared}", tool.name));
            }
        }
    }
    Ok(())
}

fn has_json_type(value: &Value, declared: &str) -> bool {
    match declared {
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "null" => value.is_null(),
        _ => true,
    }
}

fn open_storage(config: &Config, profile: Option<&str>) -> Result<Storage> {
    config.open_storage(profile)
}

fn execute_tool(name: &str, args: &Value, config: &Config, profile: Option<&str>) -> std::result::Result<String, String> {
    let mut storage = match open_storage(config, profile) {
        Ok(storage) => storage,
        Err(e) => return Err(format!("Error opening storage: {e:#}")),
    };

    let text_arg = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("");
    let int_arg = |key: &str, default: i64| args.get(key).and_then(Value::as_i64).unwrap_or(default);

    match name {
        "cerebro_search" => match storage.search(text_arg("query"), int_arg("limit", 10)) {
            Ok(results) => Ok(compact(&results)),
            Err(e) => Err(format!("Search error: {e}")),
        },

        "cerebro_read_note" => match storage.read_note(text_arg("identifier")) {
            Ok(nc) => Ok(nc.body),
            Err(e) => Err(format!("{e:#}")),
        },

        "cerebro_create_note" => {
            let tags: Vec<String> = args
                .get("tags")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            match storage.create_note(text_arg("title"), text_arg("content"), args.get("path").and_then(Value::as_str), Some(tags)) {
                Ok(note) => Ok(compact(&note)),
                Err(e) => Err(format!("Create error: {e:#}")),
            }
        }

        "cerebro_update_note" => match storage.update_note(text_arg("identifier"), text_arg("content")) {
            Ok(note) => Ok(compact(&note)),
            Err(e) => Err(format!("Update error: {e:#}")),
        },

        "cerebro_delete_note" => match storage.delete_note(text_arg("identifier")) {
            Ok(()) => Ok(format!("Note '{}' deleted", text_arg("identifier"))),
            Err(e) => Err(format!("Delete error: {e:#}")),
        },

        "cerebro_backlinks" => match storage.get_backlinks(text_arg("identifier")) {
            Ok(links) => Ok(compact(&links)),
            Err(e) => Err(format!("Backlinks error: {e:#}")),
        },

        "cerebro_list_tags" => match storage.list_tags() {
            Ok(tags) => Ok(tags.iter().map(|(name, count)| format!("{name} ({count})")).collect::<Vec<_>>().join("\n")),
            Err(e) => Err(format!("Tags error: {e}")),
        },

        "cerebro_list_notes" => match storage.list_notes(
            args.get("tag").and_then(Value::as_str),
            int_arg("limit", 20),
            int_arg("offset", 0),
        ) {
            Ok(notes) => Ok(notes.iter().map(|n| format!("{} [{}] {}", n.title, n.path, n.updated_at)).collect::<Vec<_>>().join("\n")),
            Err(e) => Err(format!("List error: {e}")),
        },

        "cerebro_graph" => match storage.get_graph() {
            Ok((notes, edges)) => {
                let nodes: Vec<Value> = notes
                    .iter()
                    .map(|n| json!({ "id": n.id, "title": n.title, "path": n.path, "tags": n.tags }))
                    .collect();
                let edge_list: Vec<Value> = edges
                    .iter()
                    .map(|l| json!({ "source": l.source_id, "target": l.target_id, "target_ref": l.target_ref }))
                    .collect();
                Ok(compact(&json!({
                    "node_count": nodes.len(),
                    "edge_count": edge_list.len(),
                    "nodes": nodes,
                    "edges": edge_list,
                })))
            }
            Err(e) => Err(format!("Graph error: {e}")),
        },

        "cerebro_kp_search" => kp_search(&mut storage, args, config),

        _ => Err(format!("Unknown tool: {name}")),
    }
}

/// `cerebro_kp_search`: ask Knowledge Palace; if it is disabled or cannot be
/// reached, serve local full-text results — explicitly labelled as such.
fn kp_search(storage: &mut Storage, args: &Value, config: &Config) -> std::result::Result<String, String> {
    let query = args.get("query").and_then(Value::as_str).unwrap_or("");
    let mode = args.get("mode").and_then(Value::as_str).unwrap_or("hybrid");
    let limit = args.get("limit").and_then(Value::as_i64).unwrap_or(5);

    // A mode KP does not have is the caller's mistake, not an outage: report
    // it instead of quietly answering with a different kind of search.
    if !crate::kp::KP_MODES.contains(&mode) {
        return Err(format!(
            "invalid KP mode '{mode}'; expected one of: {}",
            crate::kp::KP_MODES.join(", ")
        ));
    }

    // KP calls are async; the MCP loop is sync — drive them on a
    // throwaway current-thread runtime (one call at a time).
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Runtime error: {e}"))?;

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
        Ok(hits) => Ok(compact(&json!({
            "source": "knowledge_palace",
            "mode": mode,
            "query": query,
            "total": hits.len(),
            "hits": hits,
        }))),
        Err(kp_err) => {
            // Graceful degradation: KP disabled/unreachable → local FTS with
            // an explicit note, still a success result.
            match storage.search(query, limit) {
                Ok(local) => Ok(compact(&json!({
                    "source": "local_fts",
                    "note": format!("Knowledge Palace unavailable ({kp_err}); served local FTS results"),
                    "mode": "keyword",
                    "query": query,
                    "total": local.len(),
                    "hits": local,
                }))),
                Err(local_err) => Err(format!("KP error: {kp_err}; local fallback error: {local_err}")),
            }
        }
    }
}
