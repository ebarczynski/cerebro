//! Binary-level regression tests for the 2026-11-23 pre-release audit.
//!
//! These drive the real `cerebro` executable (CLI and `cerebro mcp`). Every
//! run gets its own `HOME`, so the developer's real `~/.cerebro` and
//! `~/Library/Application Support` are never touched, and git is pointed at an
//! empty configuration so the outcome does not depend on the machine.

use std::fs;
use std::io::Write as _;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cerebro::config::profile::ProfileConfig;
use cerebro::storage::Storage;

// ── helpers ──────────────────────────────────────────────────────

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn new_home(tag: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::Builder::new().prefix(&format!("cerebro-audit-{tag}-")).tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    fs::create_dir_all(home.join(".cerebro")).expect("mkdir");
    (tmp, home)
}

/// Run the real binary against an isolated home.
fn cerebro(home: &Path, args: &[&str]) -> Out {
    cerebro_env(home, args, &[])
}

fn cerebro_env(home: &Path, args: &[&str], extra_env: &[(&str, &str)]) -> Out {
    let out = Command::new(env!("CARGO_BIN_EXE_cerebro"))
        .args(args)
        .env("HOME", home)
        // Never read or depend on the developer's git configuration.
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "audit")
        .env("GIT_AUTHOR_EMAIL", "audit@example.invalid")
        .env("GIT_COMMITTER_NAME", "audit")
        .env("GIT_COMMITTER_EMAIL", "audit@example.invalid")
        .env_remove("RUST_LOG")
        .envs(extra_env.iter().copied())
        .output()
        .expect("run cerebro");
    Out {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn init(home: &Path) {
    let out = cerebro(home, &["init", "--name", "personal"]);
    assert_eq!(out.code, 0, "init failed: {}", out.stderr);
}

fn profiles_dir(home: &Path) -> PathBuf {
    home.join(".cerebro/profiles")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn readme() -> String {
    fs::read_to_string(repo_root().join("README.md")).expect("README.md")
}

fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    drop(listener); // nothing listens here any more → connection refused
    port
}

/// Feed JSON-RPC lines to `cerebro mcp`; returns every stdout line.
fn mcp(home: &Path, requests: &[serde_json::Value], extra_env: &[(&str, &str)]) -> (Vec<String>, String, i32) {
    mcp_raw(home, &requests.iter().map(|r| r.to_string()).collect::<Vec<_>>().join("\n"), extra_env)
}

fn mcp_raw(home: &Path, payload: &str, extra_env: &[(&str, &str)]) -> (Vec<String>, String, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cerebro"))
        .args(["mcp"])
        .env("HOME", home)
        .env_remove("RUST_LOG")
        .envs(extra_env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cerebro mcp");
    child.stdin.as_mut().unwrap().write_all((payload.to_string() + "\n").as_bytes()).unwrap();
    let out = child.wait_with_output().expect("mcp exit");
    (
        String::from_utf8_lossy(&out.stdout).lines().map(str::to_owned).collect(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn rpc(id: u64, method: &str, params: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

fn tool_call(id: u64, name: &str, arguments: serde_json::Value) -> serde_json::Value {
    rpc(id, "tools/call", serde_json::json!({ "name": name, "arguments": arguments }))
}

fn parsed(lines: &[String]) -> Vec<serde_json::Value> {
    lines.iter().map(|l| serde_json::from_str(l).expect("stdout line is JSON")).collect()
}

/// The single response for a one-request session.
fn one(home: &Path, request: &serde_json::Value) -> serde_json::Value {
    let (lines, stderr, code) = mcp(home, std::slice::from_ref(request), &[]);
    assert_eq!(code, 0, "mcp exited {code}: {stderr}");
    assert_eq!(lines.len(), 1, "expected exactly one response line, got {lines:?}");
    serde_json::from_str(&lines[0]).expect("JSON")
}

fn tool_names(home: &Path) -> Vec<(String, String)> {
    let resp = one(home, &rpc(1, "tools/list", serde_json::Value::Null));
    resp["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| (t["name"].as_str().unwrap().to_owned(), t["description"].as_str().unwrap().to_owned()))
        .collect()
}

// ── profile handling ─────────────────────────────────────────────

#[test]
fn profile_switch_is_honoured_by_the_next_command() {
    // Repro: `profile switch work` printed "✓ Switched to profile 'work'" but
    // wrote the choice to ~/Library/Application Support/cerebro/config.toml,
    // a file nothing ever reads; the next command still used `personal`.
    let (_tmp, home) = new_home("switch");
    init(&home);
    assert_eq!(cerebro(&home, &["profile", "create", "work"]).code, 0);
    assert_eq!(cerebro(&home, &["--profile", "work", "notes", "create", "Work Only", "--content", "workmarker"]).code, 0);

    let sw = cerebro(&home, &["profile", "switch", "work"]);
    assert_eq!(sw.code, 0, "{}", sw.stderr);

    let bare = cerebro(&home, &["search", "workmarker"]);
    assert_eq!(bare.code, 0, "{}", bare.stderr);
    assert!(bare.stdout.contains("Work Only"), "after `switch work` a bare search must see the work vault:\n{}", bare.stdout);

    assert_eq!(cerebro(&home, &["notes", "create", "After Switch", "--content", "x"]).code, 0);
    assert!(profiles_dir(&home).join("work/notes/After_Switch.md").exists(), "note must land in the switched-to profile");
    assert!(!profiles_dir(&home).join("personal/notes/After_Switch.md").exists(), "…and not in personal");

    let list = cerebro(&home, &["profile", "list"]);
    assert!(list.stdout.lines().any(|l| l.contains("work") && l.contains("active")), "{}", list.stdout);
    assert!(!list.stdout.lines().any(|l| l.contains("personal") && l.contains("active")), "{}", list.stdout);
}

#[test]
fn profile_create_stores_the_display_name() {
    // Repro: `--display-name` was printed but never written to vault.toml.
    let (_tmp, home) = new_home("display-name");
    init(&home);
    let out = cerebro(&home, &["profile", "create", "work", "--display-name", "Work Vault"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let toml = fs::read_to_string(profiles_dir(&home).join("work/vault.toml")).unwrap();
    assert!(toml.contains("display_name = \"Work Vault\""), "{toml}");
}

#[test]
fn an_explicit_profile_must_exist_and_be_a_safe_name() {
    // Repro: `--profile persnoal` (a typo) exited 0 and silently created an
    // empty vault; `--profile ../../escaped` created db.sqlite / notes /
    // vault.toml outside ~/.cerebro/profiles.
    let (_tmp, home) = new_home("explicit-profile");
    init(&home);

    let typo = cerebro(&home, &["--profile", "persnoal", "search", "anything"]);
    assert_ne!(typo.code, 0, "a misspelled profile must not look like an empty vault:\n{}", typo.stdout);
    assert!(typo.stderr.contains("persnoal"), "{}", typo.stderr);
    assert!(!profiles_dir(&home).join("persnoal").exists(), "a typo must not create a profile");

    let escape = cerebro(&home, &["--profile", "../../escaped", "notes", "create", "X", "--content", "y"]);
    assert_ne!(escape.code, 0);
    assert!(!home.join("escaped").exists(), "profile name escaped the profiles directory");
    assert!(!home.join("escaped").exists());

    // The default profile keeps its first-run convenience.
    let fresh = cerebro(&home, &["--profile", "personal", "notes", "create", "Fine", "--content", "z"]);
    assert_eq!(fresh.code, 0, "{}", fresh.stderr);
}

#[test]
fn encryption_true_is_refused_instead_of_storing_plaintext() {
    // Repro: `encryption = true` was parsed, ignored, and the note was written
    // in plaintext with exit code 0 and nothing on stderr.
    let (_tmp, home) = new_home("encryption");
    init(&home);
    let vault_toml = profiles_dir(&home).join("personal/vault.toml");
    let text = fs::read_to_string(&vault_toml).unwrap().replace("encryption = false", "encryption = true");
    fs::write(&vault_toml, text).unwrap();

    let out = cerebro(&home, &["notes", "create", "Secret Note", "--content", "TOPSECRET-PHRASE-77e1"]);
    assert_ne!(out.code, 0, "a vault that asks for encryption must not be written as plaintext");
    assert!(out.stderr.to_lowercase().contains("encryption"), "the error must say why:\n{}", out.stderr);
    assert!(!out.stderr.trim().is_empty(), "never fail silently");
    assert!(!profiles_dir(&home).join("personal/notes/Secret_Note.md").exists(), "no plaintext file may be created");
}

// ── README must describe the real thing ──────────────────────────

#[test]
fn readme_config_examples_match_the_real_schemas() {
    // Repro: the README showed `[app]` and `[profiles.<name>]` tables in
    // config.toml; neither exists, so following the README configured nothing.
    let text = readme();
    let section = text
        .split("## Configuration")
        .nth(1)
        .and_then(|s| s.split("\n## ").next())
        .expect("README has a Configuration section");
    let blocks: Vec<&str> = section.split("```toml\n").skip(1).filter_map(|b| b.split("```").next()).collect();
    assert!(blocks.len() >= 2, "expected a config.toml example and a vault.toml example");

    let config_keys: Vec<String> = blocks[0]
        .parse::<toml::Value>()
        .expect("config.toml example is valid TOML")
        .as_table()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    let real_keys = [
        "default_profile", "editor", "profiles_dir", "active_profile", "knowledge_palace",
    ];
    for key in &config_keys {
        assert!(real_keys.contains(&key.as_str()), "README documents a config.toml key that does not exist: {key}");
    }
    assert!(config_keys.contains(&"default_profile".to_string()), "config.toml example must show default_profile at top level");
    assert!(!config_keys.iter().any(|k| k.starts_with("profiles")), "profiles are not configured in config.toml");

    let vault_keys: Vec<String> = blocks[1]
        .parse::<toml::Value>()
        .expect("vault.toml example is valid TOML")
        .as_table()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    let real_vault_keys = [
        "name", "display_name", "notes_dir", "encryption", "kind", "knowledge_palace", "sync",
    ];
    for key in &vault_keys {
        assert!(real_vault_keys.contains(&key.as_str()), "README documents a vault.toml key that does not exist: {key}");
    }
    let _: ProfileConfig = toml::from_str(blocks[1]).expect("vault.toml example deserializes into ProfileConfig");
}

#[test]
fn readme_tool_and_command_counts_match_the_code() {
    let (_tmp, home) = new_home("readme-counts");
    init(&home);
    let text = readme();

    // MCP tools
    let tools = tool_names(&home);
    let stated: usize = text
        .split("exposes ")
        .nth(1)
        .and_then(|s| s.split(" tools").next())
        .and_then(|s| s.trim().parse().ok())
        .expect("README states how many MCP tools it exposes");
    assert_eq!(stated, tools.len(), "README says {stated} tools, the server lists {}", tools.len());
    for (name, description) in &tools {
        assert!(text.contains(&format!("`{name}`")), "tool {name} is missing from the README");
        assert!(!description.contains("Recoverable"), "{name} promises recovery that does not exist");
    }

    // Tauri commands
    let gui = fs::read_to_string(repo_root().join("crates/gui/src/main.rs")).unwrap();
    let registered = gui
        .split("tauri::generate_handler![")
        .nth(1)
        .and_then(|s| s.split(']').next())
        .expect("generate_handler! list");
    let commands = registered.split(',').map(str::trim).filter(|c| !c.is_empty()).count();
    let stated_cmds: usize = text
        .split("main.rs")
        .nth(1)
        .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).find(|n| !n.is_empty()))
        .and_then(|s| s.parse().ok())
        .expect("README states the number of Tauri commands next to main.rs");
    assert_eq!(stated_cmds, commands, "README says {stated_cmds} Tauri commands, main.rs registers {commands}");
}

// ── git sync ─────────────────────────────────────────────────────

fn sync_setup(home: &Path, branch: &str) {
    init(home);
    let remote = home.join("remote.git");
    assert!(Command::new("git").args(["init", "-q", "--bare"]).arg(remote.as_os_str()).status().unwrap().success());
    let vault_toml = profiles_dir(home).join("personal/vault.toml");
    let text = fs::read_to_string(&vault_toml)
        .unwrap()
        .replace("method = \"none\"", "method = \"git\"")
        .replace("remote = \"\"", &format!("remote = \"file://{}\"", remote.display()))
        .replace("branch = \"main\"", &format!("branch = \"{branch}\""));
    fs::write(&vault_toml, text).unwrap();
}

#[test]
fn sync_push_with_nothing_to_commit_still_pushes() {
    // Repro: a second `sync push` failed with "git commit -m ... failed"
    // because `git commit` exits non-zero when there is nothing to commit.
    let (_tmp, home) = new_home("sync");
    sync_setup(&home, "main");
    assert_eq!(cerebro(&home, &["notes", "create", "Synced", "--content", "one"]).code, 0);
    let first = cerebro(&home, &["sync", "push"]);
    assert_eq!(first.code, 0, "{}", first.stderr);

    let again = cerebro(&home, &["sync", "push"]);
    assert_eq!(again.code, 0, "push with nothing new to commit must succeed:\n{}", again.stderr);
}

#[test]
fn sync_uses_the_configured_branch_from_the_start() {
    // Repro: `git init` created whatever `init.defaultBranch` says; `push`
    // then targeted the configured branch and failed with "src refspec …".
    let (_tmp, home) = new_home("sync-branch");
    sync_setup(&home, "trunk");
    assert_eq!(cerebro(&home, &["notes", "create", "On Trunk", "--content", "body"]).code, 0);
    assert_eq!(cerebro(&home, &["sync", "push"]).code, 0);
    let notes = profiles_dir(&home).join("personal/notes");
    let head = Command::new("git").args(["-C"]).arg(&notes).args(["symbolic-ref", "--short", "HEAD"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), "trunk");
    let pushed = Command::new("git").args(["-C"]).arg(home.join("remote.git")).args(["rev-parse", "--verify", "refs/heads/trunk"]).output().unwrap();
    assert!(pushed.status.success(), "configured branch was never pushed");
}

// ── daily note / editor ──────────────────────────────────────────

#[test]
fn daily_note_is_found_even_with_more_than_a_thousand_newer_notes() {
    // Repro: `cerebro daily <date>` looked for the note only among the 1000
    // most recently updated notes; beyond that it recreated the file.
    let (_tmp, home) = new_home("daily");
    init(&home);
    let profile_dir = profiles_dir(&home).join("personal");
    {
        let cfg = ProfileConfig::new("personal");
        let mut s = Storage::open(&profile_dir, &cfg).unwrap();
        s.create_note("Daily Note - 2020-01-01", "DAILYMARKER original text", Some("daily/2020-01-01.md"), Some(vec!["daily".into()]))
            .unwrap();
        for i in 0..1_100 {
            s.create_note(&format!("Newer Note {i:04}"), "newer", None, None).unwrap();
        }
    }
    // A no-op "editor": exits 0 and leaves the file as it is.
    fs::write(
        home.join(".cerebro/config.toml"),
        format!("default_profile = \"personal\"\neditor = \"/usr/bin/true\"\nprofiles_dir = \"{}\"\n", profiles_dir(&home).display()),
    )
    .unwrap();

    let out = cerebro(&home, &["daily", "2020-01-01"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stdout.contains("no changes"), "{}", out.stdout);
    let body = fs::read_to_string(profile_dir.join("notes/daily/2020-01-01.md")).unwrap();
    assert!(body.contains("DAILYMARKER original text"), "the daily note was overwritten:\n{body}");
}

#[test]
fn editing_a_note_uses_a_private_temporary_file() {
    // Repro: every `notes edit` / `notes create` used the one fixed path
    // $TMPDIR/cerebro_edit.md and left the note's text behind there.
    let (_tmp, home) = new_home("editor");
    init(&home);
    let seen = home.join("editor-arg.txt");
    let script = home.join("fake-editor.sh");
    fs::write(&script, format!("#!/bin/sh\nprintf '%s' \"$1\" > {}\n", seen.display())).unwrap();
    fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let profile_dir = profiles_dir(&home).join("personal");
    fs::write(
        home.join(".cerebro/config.toml"),
        format!("default_profile = \"personal\"\neditor = \"{}\"\nprofiles_dir = \"{}\"\n", script.display(), profile_dir.display()),
    )
    .unwrap();
    assert_eq!(cerebro(&home, &["notes", "create", "Editable", "--content", "hello"]).code, 0);

    let out = cerebro(&home, &["notes", "edit", "Editable"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let temp_path = PathBuf::from(fs::read_to_string(&seen).unwrap().trim());
    assert_ne!(temp_path.file_name().and_then(|n| n.to_str()), Some("cerebro_edit.md"), "shared temp path is still in use");
    assert!(!temp_path.exists(), "the editor's temporary copy must be deleted afterwards");
}

// ── MCP ──────────────────────────────────────────────────────────

fn kp_enabled_config(home: &Path, port: u16) {
    init(home);
    fs::write(
        home.join(".cerebro/config.toml"),
        format!(
            "default_profile = \"personal\"\neditor = \"vi\"\nprofiles_dir = \"{}\"\n\n[knowledge_palace]\nenabled = true\nendpoint = \"http://127.0.0.1:{port}\"\nsync_scope = \"all\"\n",
            profiles_dir(home).display()
        ),
    )
    .unwrap();
}

#[test]
fn mcp_stdout_is_pure_json_even_with_tracing_at_trace_level() {
    // Repro: the tracing subscriber writes to STDOUT by default, so with
    // RUST_LOG=debug (and Knowledge Palace enabled) reqwest/hyper log lines
    // landed between the JSON-RPC frames.
    let (_tmp, home) = new_home("mcp-stdout");
    kp_enabled_config(&home, closed_port());
    assert_eq!(cerebro(&home, &["notes", "create", "Note", "--content", "kubernetes"]).code, 0);

    let requests = vec![
        rpc(1, "initialize", serde_json::json!({ "protocolVersion": "2025-06-18" })),
        tool_call(2, "cerebro_kp_search", serde_json::json!({ "query": "kubernetes", "mode": "semantic" })),
        tool_call(3, "cerebro_list_notes", serde_json::json!({})),
    ];
    for level in ["trace", "debug", "info", "warn"] {
        let (lines, stderr, code) = mcp(&home, &requests, &[("RUST_LOG", level)]);
        assert_eq!(code, 0);
        assert_eq!(lines.len(), requests.len(), "RUST_LOG={level}: one response line per request, got {lines:?}");
        for line in &lines {
            let value: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("RUST_LOG={level}: non-JSON output on stdout corrupts the protocol: {e}: {line:?}"));
            assert_eq!(value["jsonrpc"], "2.0");
        }
        let responses = parsed(&lines);
        assert!(responses[1]["result"]["content"][0]["text"].as_str().unwrap().contains("local_fts"), "fallback still works");
        // Trace output is fine on stderr — that is where logs belong.
        let _ = stderr;
    }
}

#[test]
fn mcp_answers_ping() {
    let (_tmp, home) = new_home("mcp-ping");
    init(&home);
    let resp = one(&home, &rpc(7, "ping", serde_json::Value::Null));
    assert!(resp.get("error").is_none(), "ping must not be an error: {resp}");
    assert!(resp["result"].is_object(), "ping must answer with an empty result: {resp}");
    assert_eq!(resp["id"], 7);
}

#[test]
fn mcp_answers_a_batch_with_a_batch() {
    let (_tmp, home) = new_home("mcp-batch");
    init(&home);
    let payload = format!(
        "[{}, {}]",
        rpc(1, "tools/list", serde_json::Value::Null),
        rpc(2, "tools/list", serde_json::Value::Null)
    );
    let (lines, stderr, code) = mcp_raw(&home, &payload, &[]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(lines.len(), 1, "a batch is answered by one line: {lines:?}");
    let batch: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    let items = batch.as_array().expect("batch response is a JSON array");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["id"], 1);
    assert_eq!(items[1]["id"], 2);
    assert!(items.iter().all(|r| r["result"]["tools"].is_array()));
}

#[test]
fn mcp_rejects_a_wrong_jsonrpc_version() {
    let (_tmp, home) = new_home("mcp-jsonrpc");
    init(&home);
    let req = serde_json::json!({ "jsonrpc": "1.0", "id": 1, "method": "tools/list" });
    let resp = one(&home, &req);
    assert_eq!(resp["error"]["code"], -32600, "must be an Invalid Request error: {resp}");
    assert_eq!(resp["id"], 1);
}

#[test]
fn mcp_parse_error_carries_a_null_id() {
    let (_tmp, home) = new_home("mcp-parse");
    init(&home);
    let (lines, _, _) = mcp_raw(&home, "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":", &[]);
    let resp: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(resp["error"]["code"], -32700);
    assert!(resp.get("id").is_some_and(|id| id.is_null()), "JSON-RPC requires \"id\": null on parse errors: {resp}");
}

#[test]
fn mcp_rejects_a_call_that_misses_a_required_argument() {
    // Repro: `cerebro_search` with `{}` answered "[]" with isError=false.
    let (_tmp, home) = new_home("mcp-required");
    init(&home);
    let resp = one(&home, &tool_call(1, "cerebro_search", serde_json::json!({})));
    assert_eq!(resp["error"]["code"], -32602, "missing required `query` must be Invalid params: {resp}");
    let resp = one(&home, &tool_call(2, "cerebro_create_note", serde_json::json!({ "title": "no content" })));
    assert_eq!(resp["error"]["code"], -32602, "missing required `content`: {resp}");
}

#[test]
fn mcp_profile_argument_cannot_escape_the_profiles_dir() {
    // Repro: `profile: "../../mcp-escaped"` made the server create db.sqlite,
    // notes/ and vault.toml outside ~/.cerebro/profiles; any unknown profile
    // name was silently created — even by a read-only search.
    let (_tmp, home) = new_home("mcp-profile");
    init(&home);

    let resp = one(
        &home,
        &tool_call(1, "cerebro_create_note", serde_json::json!({ "title": "Pwned", "content": "x", "profile": "../../mcp-escaped" })),
    );
    assert_eq!(resp["result"]["isError"], true, "{resp}");
    assert!(!home.join("mcp-escaped").exists(), "files were created outside the profiles directory");

    let resp = one(&home, &tool_call(2, "cerebro_search", serde_json::json!({ "query": "x", "profile": "does-not-exist" })));
    assert_eq!(resp["result"]["isError"], true, "an unknown profile must be an error, not an empty vault: {resp}");
    assert!(!profiles_dir(&home).join("does-not-exist").exists(), "a read created a profile");

    // The default profile keeps working without a `profile` argument.
    let resp = one(&home, &tool_call(3, "cerebro_search", serde_json::json!({ "query": "x" })));
    assert_eq!(resp["result"]["isError"], false, "{resp}");
}

#[test]
fn mcp_invalid_kp_mode_is_an_error_not_a_fallback() {
    // Repro: mode "fuzzy" answered with local keyword results labelled
    // "Knowledge Palace unavailable (invalid KP mode …)" and isError=false.
    let (_tmp, home) = new_home("mcp-kp-mode");
    kp_enabled_config(&home, closed_port());
    let resp = one(&home, &tool_call(1, "cerebro_kp_search", serde_json::json!({ "query": "x", "mode": "fuzzy" })));
    assert_eq!(resp["result"]["isError"], true, "a caller error is not an outage: {resp}");
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("invalid KP mode"), "{text}");
}

#[test]
fn mcp_kp_search_labels_the_local_fallback() {
    let (_tmp, home) = new_home("mcp-kp-fallback");
    kp_enabled_config(&home, closed_port());
    assert_eq!(cerebro(&home, &["notes", "create", "Local Hit", "--content", "kubernetes local text"]).code, 0);
    let resp = one(&home, &tool_call(1, "cerebro_kp_search", serde_json::json!({ "query": "kubernetes" })));
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let body: serde_json::Value = serde_json::from_str(text).expect("fallback payload is JSON");
    assert_eq!(body["source"], "local_fts");
    assert_eq!(body["mode"], "keyword");
    assert!(body["note"].as_str().unwrap().contains("Knowledge Palace unavailable"));
    assert_eq!(body["total"], 1);
}

#[test]
fn mcp_honours_the_profile_option() {
    let (_tmp, home) = new_home("mcp-profile-option");
    init(&home);
    assert_eq!(cerebro(&home, &["profile", "create", "work"]).code, 0);
    assert_eq!(cerebro(&home, &["--profile", "work", "notes", "create", "Only In Work", "--content", "workmarker"]).code, 0);

    let mut child = Command::new(env!("CARGO_BIN_EXE_cerebro"))
        .args(["--profile", "work", "mcp"])
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cerebro mcp");
    let request = tool_call(1, "cerebro_search", serde_json::json!({ "query": "workmarker" }));
    child.stdin.as_mut().unwrap().write_all((request.to_string() + "\n").as_bytes()).unwrap();
    let out = child.wait_with_output().expect("mcp exit");
    let resp: serde_json::Value = serde_json::from_slice(&out.stdout).expect("one JSON response line");
    let hits: Vec<serde_json::Value> = serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(hits.len(), 1, "the server must search the profile given on its command line: {resp}");
    assert_eq!(hits[0]["title"], "Only In Work");
}

#[test]
fn deleting_the_database_is_recovered_by_the_next_command() {
    // `db.sqlite` is an index; the `.md` files are the source of truth. After
    // it is deleted, a search used to answer "No results" (exit 0) until
    // someone ran `cerebro reindex` by hand.
    let (_tmp, home) = new_home("db-deleted");
    init(&home);
    assert_eq!(cerebro(&home, &["notes", "create", "Alpha Note", "--content", "alpha body #project"]).code, 0);
    assert_eq!(cerebro(&home, &["notes", "create", "Beta Note", "--content", "beta links to [[Alpha Note]]"]).code, 0);
    let profile = profiles_dir(&home).join("personal");
    for leftover in ["db.sqlite", "db.sqlite-wal", "db.sqlite-shm"] {
        let _ = fs::remove_file(profile.join(leftover));
    }

    let search = cerebro(&home, &["search", "alpha"]);
    assert_eq!(search.code, 0, "{}", search.stderr);
    assert!(search.stdout.contains("Alpha Note"), "the index must be rebuilt from the files:\n{}", search.stdout);

    let list = cerebro(&home, &["notes", "list"]);
    let listed = list.stdout.lines().filter(|line| line.contains("_Note.md")).count();
    assert_eq!(listed, 2, "{}", list.stdout);

    // Links, tags and search rows came back with it.
    let backlinks = cerebro(&home, &["backlinks", "Alpha Note"]);
    assert!(backlinks.stdout.contains("Beta Note"), "{}", backlinks.stdout);
    assert!(cerebro(&home, &["tags"]).stdout.contains("project"));
    // And an MCP client no longer sees an empty vault.
    let resp = one(&home, &tool_call(1, "cerebro_graph", serde_json::json!({})));
    assert!(resp["result"]["content"][0]["text"].as_str().unwrap().contains("\"node_count\":2"), "{resp}");
}
