# Cerebro 🧠

> A fast, secure, local-first personal knowledge base with dual-profile vaults, bidirectional links, and MCP integration.

**Cerebro** (Latin for "brain") is a Rust-based knowledge management tool inspired by [Obsidian](https://obsidian.md). It stores notes as plain Markdown files, maintains a SQLite index with full-text search, tracks bidirectional wikilinks, and exposes everything via an MCP server for AI tool integration.

## Features

- **Dual-profile vaults** — separate personal and professional knowledge bases
- **Markdown-native** — notes are `.md` files you own; no lock-in
- **Bidirectional links** — `[[wikilinks]]` with automatic backlink tracking
- **Full-text search** — SQLite FTS5 with Porter stemmer + Unicode support
- **Tag management** — frontmatter tags + inline `#tags`, unified
- **MCP server** — expose your knowledge base to GitHub Copilot, Hermes, and other MCP clients
- **Knowledge Palace connector** — semantic/RAG search via pgvector
- **Confluence publishing** — push notes to Atlassian Confluence
- **Git sync** — automatic version control for your vault
- **Daily notes** — quick journaling with date-indexed notes
- **Quick capture** — instant inbox notes from CLI

## Architecture

```
┌──────────────────────────────────────────────┐
│                   CLI (clap)                  │
├──────────┬───────────┬───────────┬───────────┤
│  Parser  │  Storage  │  Search   │   Links   │
│(pulldown │ (SQLite + │  (FTS5)   │(wikilinks │
│  -cmark) │  FTS5)    │           │ + graph)  │
├──────────┴───────────┴───────────┴───────────┤
│            MCP Server (JSON-RPC)              │
├───────────────────┬──────────────────────────┤
│  KP Connector     │  Confluence Publisher     │
│  (HTTP/pgvector)  │  (REST API)               │
└───────────────────┴──────────────────────────┘
```

## Quick Start

```bash
# Build
cargo build --release

# Initialize your first vault
cerebro init

# Create a note
cerebro notes create "My First Note" --content "Hello, world!"

# Create with tags
cerebro notes create "Rust Tips" --tags "rust,programming" --content "# Rust Tips\n\nUse `cargo clippy`!"

# Search
cerebro search "rust safety"

# Show backlinks
cerebro backlinks "Rust Tips"

# List tags
cerebro tags

# Daily note
cerebro daily today

# Quick capture
cerebro capture "Remember to check the new API docs"

# Reindex vault from filesystem
cerebro reindex

# Start MCP server (stdio)
cerebro mcp
```

## Profile Management

```bash
# List profiles
cerebro profile list

# Create a new profile
cerebro profile create work --display-name "Work Vault"

# Switch profile
cerebro profile switch work

# Use a specific profile for one command
cerebro --profile work notes list
```

## Configuration

Global config at `~/.cerebro/config.toml`:

```toml
default_profile = "personal"
editor = "vim"

[profiles_dir]
# Default: ~/.cerebro/profiles/

[knowledge_palace]
enabled = true
endpoint = "http://localhost:8080"

[confluence]
enabled = false
base_url = "https://yourorg.atlassian.net/wiki"
space_key = "ENG"
```

Per-profile vault config at `~/.cerebro/profiles/<name>/vault.toml`:

```toml
name = "personal"
display_name = "Personal Vault"
notes_dir = "notes"
encryption = false

[sync]
method = "git"
remote = "git@github.com:user/notes.git"
branch = "main"

[knowledge_palace]
enabled = true
endpoint = "http://localhost:8080"
sync_scope = "all"
```

## MCP Server

Cerebro exposes an MCP (Model Context Protocol) server with these tools:

| Tool | Description |
|------|-------------|
| `cerebro_search` | Full-text search across all notes |
| `cerebro_read_note` | Read a specific note by title or ID |
| `cerebro_create_note` | Create a new note |
| `cerebro_update_note` | Update an existing note |
| `cerebro_delete_note` | Delete a note |
| `cerebro_list_notes` | List notes with optional tag filter |
| `cerebro_backlinks` | Get backlinks for a note |
| `cerebro_tags` | List all tags |
| `cerebro_daily` | Get/create daily note |
| `cerebro_kp_search` | Semantic search via Knowledge Palace |

### Configure in Copilot / VS Code

Add to your MCP settings:

```json
{
  "mcpServers": {
    "cerebro": {
      "command": "cerebro",
      "args": ["mcp"]
    }
  }
}
```

## Tech Stack

| Component | Technology |
|-----------|-----------|
| Language | Rust 2024 edition |
| CLI | clap 4 (derive) |
| Markdown | pulldown-cmark |
| Database | SQLite 3 + FTS5 (via rusqlite) |
| Search | SQLite FTS5 (Porter stemmer) |
| Link Graph | Custom (SQLite-backed) |
| HTTP Client | reqwest |
| Serialization | serde + serde_json + toml |
| MCP | JSON-RPC 2.0 over stdio |
| UUID | uuid v7 (time-ordered) |

## Project Structure

```
cerebro/
├── src/
│   ├── cli/          # CLI commands (clap)
│   ├── config/       # TOML configuration
│   ├── parser/       # Markdown parsing, frontmatter, wikilinks, tags
│   ├── storage/      # SQLite storage engine
│   ├── search/       # Full-text search
│   ├── links/        # Link graph (placeholder for petgraph)
│   ├── mcp/          # MCP server (JSON-RPC)
│   ├── kp/           # Knowledge Palace connector
│   ├── publish/      # Confluence publisher
│   └── sync/         # Git sync
├── migrations/       # SQLite migrations
├── tests/            # Integration tests
└── docs/             # Architecture & plans
```

## Running Tests

```bash
# All tests (unit + integration + doctests)
cargo test

# Just integration tests
cargo test --test integration_tests

# Release build
cargo build --release
```

## License

MIT
