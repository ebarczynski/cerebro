# Cerebro — Personal Knowledge Base Application

> **⚠️ Status (2026-11-06):** this plan is PARTIALLY SUPERSEDED by ADRs
> 0001–0005 and the grill audit (`docs/grill/2026-11-06-design-review.md`).
> Notably: Tantivy was dropped (ADR 0002), the Confluence publisher was
> removed (ADR 0004), the product is explicitly phased (ADR 0003), and the
> MCP contract is defined in ADR 0005. Milestones below are historical
> record, not the current backlog.

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Build a fast, secure, local-first personal knowledge base application with dual modes (personal/professional), profile segregation, Knowledge Palace integration, MCP server for AI tool connectivity, and Confluence publishing — inspired by Obsidian but written in Rust.

**Codename:** **Cerebro** (Latin for "brain" — a knowledge organ)

**Architecture:** Rust core binary with embedded SQLite metadata index, plain `.md` file storage as source of truth, Tauri 2 desktop shell (optional), native MCP server, REST API, and Knowledge Palace connector for semantic/RAG search. Dual-profile vault system keeps personal and professional knowledge strictly separated.

**Tech Stack:** Rust (edition 2024), SQLite via rusqlite, Tantivy FTS, pulldown-cmark, tokio async runtime, reqwest for HTTP, Tauri 2 for GUI (phase 2), MCP protocol (stdio + HTTP SSE)

---

## Competitive Landscape & Alternatives Considered

### Why Not Use Existing Tools?

| Tool | Language | Why It Falls Short |
|------|----------|-------------------|
| **Obsidian** | TS/Electron | Proprietary sync, no native MCP, no Rust performance, not open-source core |
| **SiYuan** | Go | AGPL-3.0, not Rust/C++, block-based not pure markdown |
| **Logseq** | ClojureScript | AGPL-3.0, outliner-first (not document-first), JVM memory |
| **AFFiNE** | Rust+TS | Block-based (not pure .md files), IndexedDB storage, web-first |
| **Zettlr** | TS/Electron | GPL-3.0, no API/MCP, Electron overhead |
| **Trilium Next** | JS/Node | AGPL-3.0, single-notebook, no MCP |

### Open-Source Reusable Components

| Component | Library | Why |
|-----------|---------|-----|
| Full-text search | **Tantivy** 15.3k⭐ MIT | Lucene-inspired, FST-based, fast, Rust-native |
| Markdown parsing | **pulldown-cmark** 2.6k⭐ MIT | Has wikilinks extension, zero-copy, CommonMark |
| SQLite bindings | **rusqlite** 4.2k⭐ MIT | Battle-tested, FTS5, JSON, migrations, transactions |
| Graph traversal | **petgraph** 3k⭐ MIT/Apache | In-memory graph for backlink visualization |
| HTTP framework | **Axum** 18k⭐ MIT | Tower-based, async, typed routing |
| CLI framework | **Clap** 14k⭐ MIT/Apache | Declarative CLI with derive macros |
| MCP protocol | Custom (JSON-RPC over stdio) | Simple protocol, ~10 message types |

### Key Differentiators vs. Existing Tools

1. **Native MCP server** — No major KB app has this. Cerebro exposes full CRUD + search + graph traversal to any MCP client (Copilot, Claude, Hermes)
2. **Dual-profile vaults** — Strict personal/professional separation at the storage level
3. **Knowledge Palace connector** — Semantic/RAG search via existing pgvector infrastructure
4. **Confluence publisher** — Direct publishing from markdown to Confluence via REST API
5. **Rust performance** — Sub-millisecond search on 100k+ notes, <10ms cold start
6. **Hardened security** — Optional AES-256-GCM encryption per vault, no telemetry, no cloud dependency

---

## Architecture

### High-Level System Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                        CEREBRO APPLICATION                       │
│                                                                  │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐   │
│  │  CLI (clap)   │  │  REST API    │  │  MCP Server (stdio)  │   │
│  │  cerebro ...  │  │  :9090/api/  │  │  cerebro mcp         │   │
│  └──────┬───────┘  └──────┬───────┘  └──────────┬───────────┘   │
│         │                  │                      │              │
│         └──────────────────┼──────────────────────┘              │
│                            │                                     │
│                   ┌────────▼─────────┐                          │
│                   │  Core Engine     │                          │
│                   │  (Rust library)  │                          │
│                   │                  │                          │
│                   │  ┌────────────┐  │                          │
│                   │  │ Vault Mgr  │  │  ← Profiles: personal   │
│                   │  │            │  │    vs professional       │
│                   │  └─────┬──────┘  │                          │
│                   │        │         │                          │
│                   │  ┌─────▼──────┐  │                          │
│                   │  │ Note Store │  │  ← CRUD, links, tags    │
│                   │  └─────┬──────┘  │                          │
│                   │        │         │                          │
│                   │  ┌─────▼──────┐  │                          │
│                   │  │ Link Graph │  │  ← petgraph backlinks   │
│                   │  └─────┬──────┘  │                          │
│                   │        │         │                          │
│                   │  ┌─────▼──────┐  │                          │
│                   │  │ Search     │  │  ← Tantivy + SQLite FTS5│
│                   │  └─────┬──────┘  │                          │
│                   │        │         │                          │
│                   │  ┌─────▼──────┐  │                          │
│                   │  │ MD Parser  │  │  ← pulldown-cmark       │
│                   │  └────────────┘  │                          │
│                   └────────┬─────────┘                          │
│                            │                                     │
│         ┌──────────────────┼──────────────────┐                │
│         │                  │                  │                 │
│  ┌──────▼──────┐  ┌───────▼───────┐  ┌──────▼──────┐         │
│  │ SQLite      │  │ .md Files     │  │ KP Connector│         │
│  │ (metadata)  │  │ (source of    │  │ (semantic   │         │
│  │ FTS5 index  │  │  truth)       │  │  search via │         │
│  │ + Tantivy   │  │               │  │  Knowledge  │         │
│  └─────────────┘  └───────────────┘  │  Palace API) │         │
│                                       └──────┬──────┘         │
└──────────────────────────────────────────────┼──────────────────┘
                                               │
                              ┌────────────────▼─────────────────┐
                              │      KNOWLEDGE PALACE            │
                              │  PostgreSQL + pgvector           │
                              │  REST API (:8080) + GraphQL      │
                              │  MCP Server (standalone)         │
                              │  Semantic/RAG/Hybrid Search      │
                              └──────────────────────────────────┘

External Integrations:
  ┌─────────────────┐  ┌──────────────────┐  ┌────────────────┐
  │ GitHub Copilot  │  │ Confluence       │  │ Git Remote     │
  │ (via MCP)       │  │ (REST publish)   │  │ (sync/backup)  │
  └─────────────────┘  └──────────────────┘  └────────────────┘
```

### Storage Architecture

```
~/.cerebro/
├── config.toml                    # Global config (profiles, keybindings, themes)
├── profiles/
│   ├── personal/
│   │   ├── vault.toml             # Vault config (name, encryption, sources)
│   │   ├── db.sqlite              # SQLite metadata + FTS5 index
│   │   ├── tantivy/               # Tantivy full-text index
│   │   └── notes/                 # Source of truth: plain .md files
│   │       ├── index.md           # Vault root / daily note
│   │       ├── projects/
│   │       │   ├── knowledge-palace.md
│   │       │   └── score-vreq.md
│   │       ├── reading/
│   │       │   └── ...
│   │       └── .cerebro/          # Per-note metadata (frontmatter cache)
│   │           └── links.jsonl    # Link graph edge list
│   │
│   └── professional/
│       ├── vault.toml
│       ├── db.sqlite
│       ├── tantivy/
│       └── notes/
│           ├── clients/
│           │   └── dxc/
│           │       ├── ai-strategy.md
│           │       └── requirements-quality.md
│           ├── meetings/
│           └── .cerebro/
│
└── secrets/
    └── vault.key                  # AES-256-GCM key (if encryption enabled)
```

### Database Schema (SQLite per profile)

```sql
-- Core notes table
CREATE TABLE notes (
    id          TEXT PRIMARY KEY,          -- UUID v7 (time-sortable)
    title       TEXT NOT NULL,
    path        TEXT NOT NULL UNIQUE,      -- Relative path from vault root
    content_hash TEXT NOT NULL,            -- SHA-256 of file content
    frontmatter TEXT,                      -- JSON-extracted YAML frontmatter
    tags        TEXT,                      -- JSON array of tags
    word_count  INTEGER,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
    deleted_at  TEXT                       -- Soft delete
);

-- Bidirectional link graph
CREATE TABLE links (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    source_id   TEXT NOT NULL REFERENCES notes(id),
    target_id   TEXT REFERENCES notes(id), -- NULL for unresolved links
    target_ref  TEXT NOT NULL,             -- Raw wikilink target (e.g., "[[Some Note]]")
    context     TEXT,                      -- Surrounding paragraph text
    line_number INTEGER,
    UNIQUE(source_id, target_ref, line_number)
);

-- Tags (normalized, for faceted search)
CREATE TABLE tags (
    name        TEXT PRIMARY KEY,
    note_count  INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE note_tags (
    note_id     TEXT REFERENCES notes(id),
    tag         TEXT REFERENCES tags(name),
    PRIMARY KEY (note_id, tag)
);

-- Full-text search virtual table
CREATE VIRTUAL TABLE notes_fts USING fts5(
    title,
    content,                            -- Markdown body (stripped of YAML)
    tags,
    content='notes',
    content_rowid='rowid',
    tokenize='porter unicode61'         -- Stemming + unicode support
);

-- Daily notes / journal entries
CREATE TABLE daily_notes (
    date        TEXT PRIMARY KEY,        -- YYYY-MM-DD
    note_id     TEXT REFERENCES notes(id)
);

-- Sync tracking (for Git-based sync)
CREATE TABLE sync_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    operation   TEXT NOT NULL,           -- 'create', 'update', 'delete'
    note_id     TEXT NOT NULL,
    timestamp   TEXT NOT NULL DEFAULT (datetime('now')),
    synced      BOOLEAN NOT NULL DEFAULT 0
);

-- Knowledge Palace integration cache
CREATE TABLE kp_cache (
    note_id     TEXT PRIMARY KEY REFERENCES notes(id),
    kp_doc_id   TEXT,                    -- Knowledge Palace document UUID
    last_synced TEXT NOT NULL,
    sync_status TEXT NOT NULL DEFAULT 'pending'  -- pending, synced, error
);

-- Indexes
CREATE INDEX idx_notes_path ON notes(path);
CREATE INDEX idx_notes_updated ON notes(updated_at DESC);
CREATE INDEX idx_links_source ON links(source_id);
CREATE INDEX idx_links_target ON links(target_id);
CREATE INDEX idx_notes_tags_tag ON note_tags(tag);
```

---

## Dual-Profile System Design

### Profile Isolation

Each profile is a **completely independent vault** with its own:
- SQLite database (separate file)
- Note directory tree (separate root)
- Tantivy search index
- Encryption key (optional, per-profile)
- Git remote (optional, per-profile)
- Knowledge Palace sync scope

### Mode Switching

```bash
# CLI — explicit profile selection
cerebro --profile personal search "brake controller"
cerebro --profile professional notes list

# Or set default profile
cerebro config set default-profile professional

# MCP — profile passed as tool parameter
# Tool: cerebro_search(query="AI strategy", profile="professional")
```

### Profile Configuration (`vault.toml`)

```toml
[profile]
name = "professional"
display_name = "DXC Engineering"
icon = "briefcase"               # For GUI phase

[storage]
notes_dir = "notes"              # Relative to profile dir
database = "db.sqlite"
encryption = false               # true = AES-256-GCM

[search]
default_mode = "hybrid"          # "keyword" | "semantic" | "hybrid"
max_results = 50

[knowledge_palace]
enabled = true
endpoint = "http://localhost:8080"
sync_on_startup = false          # Don't block startup for KP sync
sync_scope = "professional"      # Only sync notes tagged with this scope

[sync]
method = "git"                   # "git" | "none"
remote = "git@github.com:ebarczynski/cerebro-pro.git"
branch = "main"
auto_commit = true
commit_interval_minutes = 5

[confluence]
enabled = true
base_url = "https://your-org.atlassian.net"
space_key = "ENG"
default_parent_page = "12345"    # Parent page ID for published notes
publish_tag = "confluence"       # Only notes with this tag get published

[export]
formats = ["markdown", "html", "pdf"]
```

---

## MCP Server Design

### Exposed Tools

```json
[
  {
    "name": "cerebro_search",
    "description": "Search notes across profiles. Supports keyword, semantic (via Knowledge Palace), and hybrid modes.",
    "inputSchema": {
      "query": "string (required)",
      "profile": "string (default: active profile)",
      "mode": "enum[keyword, semantic, hybrid]",
      "limit": "integer (default: 10)",
      "tags": "string[] (filter by tags)"
    }
  },
  {
    "name": "cerebro_read_note",
    "description": "Read a note by title or path. Returns full markdown content with frontmatter.",
    "inputSchema": {
      "identifier": "string (required) — title, path fragment, or UUID",
      "profile": "string"
    }
  },
  {
    "name": "cerebro_create_note",
    "description": "Create a new note. Auto-generates frontmatter, updates link graph.",
    "inputSchema": {
      "title": "string (required)",
      "content": "string (required) — markdown body",
      "path": "string (optional) — directory path within vault",
      "tags": "string[]",
      "profile": "string"
    }
  },
  {
    "name": "cerebro_update_note",
    "description": "Update an existing note. Handles link graph reconciliation.",
    "inputSchema": {
      "identifier": "string (required)",
      "content": "string (new markdown body)",
      "profile": "string"
    }
  },
  {
    "name": "cerebro_delete_note",
    "description": "Soft-delete a note. Recoverable within retention period.",
    "inputSchema": {
      "identifier": "string (required)",
      "profile": "string"
    }
  },
  {
    "name": "cerebro_backlinks",
    "description": "Get all notes that link TO the specified note (backlinks).",
    "inputSchema": {
      "identifier": "string (required)",
      "profile": "string"
    }
  },
  {
    "name": "cerebro_graph",
    "description": "Get the link graph around a note (N hops). Returns nodes + edges for visualization.",
    "inputSchema": {
      "center": "string (optional) — starting note, omit for full graph",
      "depth": "integer (default: 2) — hop depth",
      "profile": "string"
    }
  },
  {
    "name": "cerebro_tags",
    "description": "List all tags with note counts, or notes for a specific tag.",
    "inputSchema": {
      "tag": "string (optional) — filter to specific tag",
      "profile": "string"
    }
  },
  {
    "name": "cerebro_publish_confluence",
    "description": "Publish a note to Confluence. Converts markdown to Confluence storage format.",
    "inputSchema": {
      "identifier": "string (required)",
      "space_key": "string (optional)",
      "parent_page_id": "string (optional)",
      "profile": "string"
    }
  },
  {
    "name": "cerebro_kp_search",
    "description": "Semantic/RAG search via Knowledge Palace. Falls back to local if KP unavailable.",
    "inputSchema": {
      "query": "string (required)",
      "mode": "enum[semantic, hybrid, ask]",
      "limit": "integer (default: 5)",
      "profile": "string"
    }
  }
]
```

### MCP Configuration (Hermes / Copilot)

```yaml
# Hermes config.yaml
mcp_servers:
  cerebro:
    command: cerebro
    args: ["mcp", "--profile", "professional"]
    # Or for dual-profile access:
    # args: ["mcp", "--all-profiles"]
```

---

## Confluence Publishing Pipeline

```
Note (.md) → pulldown-cmark (AST)
           → Custom renderer (Confluence storage format)
           → REST API PUT /rest/api/content/
           → Return page URL

Features:
  - Wiki links → Confluence internal links (if target exists)
  - Code blocks → Confluence code macro
  - Images → Upload as attachment + embed
  - Tables → Confluence table format
  - Tags → Confluence labels
  - Frontmatter → page metadata (title, parent, space override)
  - Idempotent: updates existing pages by title match
```

---

## Implementation Plan

### Phase 1: Core Engine (CLI-only, ~4 weeks)

**Milestone:** Fully functional CLI tool with CRUD, search, link graph, dual profiles.

#### Task 1: Project Scaffolding
- `cargo init --name cerebro`
- Cargo.toml with dependencies: clap, tokio, rusqlite, pulldown-cmark, tantivy, serde, toml, uuid, sha2, walkdir, petgraph, anyhow, tracing
- Directory structure: `src/{main,cli,core,storage,search,links,mcp,config,kp,publish}/`
- CI: GitHub Actions (Linux + macOS, clippy, fmt, test)

#### Task 2: Configuration System
- TOML-based config: `config.toml` (global) + `vault.toml` (per-profile)
- Profile management: create, list, switch, delete profiles
- Config validation with helpful error messages

#### Task 3: Storage Layer (SQLite + Files)
- Database migrations (SQL files embedded via include_str!)
- Note CRUD: create, read, update, delete (soft-delete)
- File watcher: inotify (Linux) / FSEvents (macOS) for external changes
- Frontmatter extraction (YAML between `---` delimiters)
- Content hashing for change detection

#### Task 4: Markdown Parser & Link Graph
- pulldown-cmark with wikilinks extension (`[[note name]]`)
- Bidirectional link extraction → `links` table
- Link reconciliation on note update (add/remove links)
- Unresolved link tracking (broken links dashboard)
- Tag extraction from `#tag` syntax and frontmatter

#### Task 5: Full-Text Search
- SQLite FTS5 for basic keyword search
- Tantivy index for advanced search (faceted, field boosts)
- Hybrid search: FTS5 for exact matches, Tantivy for relevance
- Search result ranking: BM25 + recency boost

#### Task 6: CLI Interface
- `cerebro init` — Initialize new vault/profile
- `cerebro notes create <title>` — Create note (opens $EDITOR)
- `cerebro notes list [--tag X] [--profile P]`
- `cerebro notes show <identifier>` — Display note
- `cerebro notes edit <identifier>` — Open in $EDITOR
- `cerebro notes delete <identifier>`
- `cerebro search <query> [--mode M] [--limit N]`
- `cerebro backlinks <identifier>`
- `cerebro graph [--center ID] [--depth N]`
- `cerebro tags [tag]`
- `cerebro profile list|create|switch|delete`

#### Task 7: Daily Notes
- `cerebro daily [today|yesterday|YYYY-MM-DD]`
- Auto-create from template if doesn't exist
- Template system with date/time variables

### Phase 2: Knowledge Palace Integration (~1 week)

#### Task 8: KP Connector
- HTTP client to Knowledge Palace REST API
- Semantic search proxy: `cerebro search --mode semantic` → KP `/api/v1/search`
- RAG ask: `cerebro ask "question"` → KP MCP `ask` tool
- Sync: bidirectional sync of tagged notes → KP documents
- KP cache table for sync status tracking
- Graceful degradation when KP is unavailable

### Phase 3: MCP Server (~1 week)

#### Task 9: MCP Protocol Implementation
- JSON-RPC 2.0 over stdio (for CLI/MCP clients)
- Tool registration for all 10 tools
- Request validation with typed schemas
- Profile context per-request (no cross-profile leakage)

#### Task 10: MCP HTTP Transport (SSE)
- HTTP SSE transport for remote MCP clients
- Authentication: API key or token-based
- Rate limiting

### Phase 4: Confluence Publishing (~1 week)

#### Task 11: Confluence Publisher
- Markdown → Confluence storage format converter
- REST API client (Atlassian Confluence v2 API)
- Image upload + embedding
- Page creation / update (idempotent by title)
- Label sync (tags → Confluence labels)
- Dry-run mode for preview

### Phase 5: Sync & Security (~1 week)

#### Task 12: Git-Based Sync
- `cerebro sync push/pull/status`
- Auto-commit with configurable interval
- Conflict detection (content hash comparison)
- Merge strategy: last-write-wins with backup

#### Task 13: Vault Encryption
- AES-256-GCM encryption for `.md` files at rest
- Key derivation from master password (Argon2id)
- Transparent encrypt/decrypt in storage layer
- Per-profile encryption settings

### Phase 6: GUI (Tauri 2) — Future Phase

#### Task 14: Tauri Shell
- Svelte/SolidJS frontend (lightweight)
- Editor: CodeMirror 6 with markdown + wikilink plugins
- Graph visualization: force-directed layout (d3-force or custom)
- File tree sidebar, tag pane, search bar, backlinks pane
- Profile switcher in toolbar

---

## Testing Strategy

| Level | Tool | Coverage Target |
|-------|------|-----------------|
| Unit tests | `#[test]` + `cargo test` | 80%+ core engine |
| Integration tests | `tests/` directory | All CLI commands, MCP tools |
| Property tests | `proptest` | Link graph invariants, search ranking |
| Fuzz testing | `cargo-fuzz` | Markdown parser edge cases |
| Benchmark | `criterion` | Search <1ms, note load <5ms, startup <10ms |

---

## Security Considerations

- **No telemetry, no analytics, no phone-home**
- **No network requests unless explicitly configured** (KP, Confluence, Git sync)
- **SQL injection**: All queries parameterized (rusqlite prepared statements)
- **Path traversal**: All file paths validated against vault root
- **Encryption**: AES-256-GCM with Argon2id key derivation (optional per-profile)
- **MCP authentication**: API key required for HTTP transport
- **Secrets**: Stored in OS keychain (keyring crate) or encrypted config
- **Supply chain**: `cargo audit` in CI, pinned dependencies in `Cargo.lock`

---

## Performance Targets

| Operation | Target | Hardware |
|-----------|--------|----------|
| Cold start | <10ms | Any |
| Search (100k notes) | <1ms | HDD, <0.1ms SSD |
| Note create | <5ms | Any |
| Link graph rebuild (1k notes) | <100ms | Any |
| Full vault reindex (10k notes) | <5s | SSD |
| Memory footprint (idle) | <20MB | Any |
| Binary size | <5MB (stripped) | — |

---

## Dependencies (Cargo.toml)

```toml
[dependencies]
# CLI
clap = { version = "4", features = ["derive"] }

# Async
tokio = { version = "1", features = ["full"] }

# Database
rusqlite = { version = "0.33", features = ["bundled", "functions"] }

# Search
tantivy = "0.22"

# Markdown
pulldown-cmark = { version = "0.12", features = ["simd"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"

# Crypto & Security
sha2 = "0.10"
aes-gcm = "0.10"
argon2 = "0.5"

# Graph
petgraph = "0.7"

# HTTP (KP connector, Confluence)
reqwest = { version = "0.12", features = ["json"] }
axum = "0.8"
tower = "0.5"
tower-http = { version = "0.6", features = ["cors", "sse"] }

# MCP Protocol
serde_jsonrpc = "0.2"  # or custom lightweight JSON-RPC

# Utilities
uuid = { version = "1", features = ["v7"] }
walkdir = "2"
notify = "7"           # File watching
chrono = { version = "0.4", features = ["serde"] }
tracing = "0.1"
tracing-subscriber = "0.3"
anyhow = "1"
thiserror = "2"
dirs = "6"             # Cross-platform config directories
open = "5"             # Open in default editor/browser

[dev-dependencies]
tempfile = "3"
proptest = "1"
criterion = "0.5"
assert_cmd = "2"
predicates = "3"
```

---

## CLI Quick Reference (Target UX)

```bash
# Initialize
cerebro init                           # Create ~/.cerebro with default profile
cerebro profile create professional    # Add professional vault

# Daily workflow
cerebro daily                          # Open/create today's note
cerebro capture "Met with DXC team..." # Quick capture to inbox

# Notes
cerebro notes create "AI Strategy for Automotive"
cerebro notes edit "AI Strategy"
cerebro notes list --tag dxc
cerebro search "brake controller ASIL"
cerebro backlinks "AI Strategy"

# Knowledge Palace
cerebro kp search "requirement quality patterns" --mode hybrid
cerebro kp sync --profile professional

# Confluence
cerebro publish "AI Strategy" --space ENG
cerebro publish --tag confluence --dry-run

# Sync
cerebro sync status
cerebro sync push

# MCP server (for Copilot/Hermes)
cerebro mcp                            # Start stdio MCP server
cerebro mcp --transport http --port 9090  # HTTP SSE transport
```
