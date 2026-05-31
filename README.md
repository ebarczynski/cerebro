# 🧠 Cerebro

A fast, secure, local-first personal knowledge base with dual-profile vaults, 
MCP integration for AI coding tools, Knowledge Palace connector, and Confluence publishing.

Built with **Rust 1.96.0** (edition 2024), **Tauri 2.0**, **React 19**, 
**CodeMirror 6**, and **TailwindCSS v4**.

## Features

- **Dual-profile vaults** — separate personal and professional knowledge domains
- **Markdown-native** — `.md` files are the source of truth, with YAML frontmatter
- **Wikilinks** — `[[Note Name]]` with `#heading` and `|alias` support
- **Bidirectional links** — automatic backlink and forward-link tracking
- **Full-text search** — SQLite FTS5 with Porter stemmer
- **Tag management** — unified frontmatter `tags:` + inline `#tags`
- **Daily notes** — `/Daily/YYYY-MM-DD.md` auto-generated
- **Knowledge graph** — interactive D3.js force-directed graph visualization
- **Knowledge Palace connector** — semantic/RAG search via pgvector at `localhost:8080`
- **Confluence publisher** — markdown → Confluence Storage Format → REST API
- **Git sync** — automatic version control for vaults
- **MCP server** — 10 tools via JSON-RPC over stdio for Copilot, Hermes, and other MCP clients
- **Tauri 2.0 GUI** — native desktop app (Linux + macOS), dark theme, split-pane layout
- **CLI** — full command-line interface for scripting and automation
- **Encryption-ready** — AES-256-GCM profile encryption support

## Architecture

```
cerebro/
├── src/                          # Core library (cerebro crate)
│   ├── config/                   # TOML config, profile management
│   ├── storage/                  # SQLite CRUD, FTS5, migrations
│   ├── parser/                   # Markdown parsing, frontmatter, wikilinks
│   ├── links/                    # petgraph bidirectional link graph
│   ├── search/                   # Full-text search wrapper
│   ├── mcp/                      # MCP server (JSON-RPC 2.0 over stdio)
│   ├── kp/                       # Knowledge Palace HTTP connector
│   ├── publish/                  # Confluence REST publisher
│   ├── sync/                     # Git-based sync
│   └── cli/                      # clap CLI interface
├── crates/gui/                   # Tauri 2.0 GUI application
│   ├── src/main.rs               # 19 Tauri commands wired to core lib
│   ├── frontend/                 # React 19 + TypeScript frontend
│   │   ├── src/
│   │   │   ├── App.tsx           # Main app with graph view, preview, welcome
│   │   │   └── components/       # Sidebar, Editor, Search, StatusBar
│   │   └── index.css             # TailwindCSS v4 dark theme
│   └── tauri.conf.json           # Tauri configuration
├── migrations/                   # SQLite schema migrations
└── tests/                        # Integration tests (9 tests)
```

## Quick Start

### CLI

```bash
# Build release
cargo build --release

# Initialize a vault
cerebro init

# Create a note
cerebro notes create "My First Note" -c "# Hello World\n\nThis is my first note."

# Search
cerebro search "hello world"

# Show backlinks
cerebro backlinks "My First Note"

# Start MCP server (for Copilot/Hermes)
cerebro mcp
```

### GUI

```bash
# Prerequisites (Linux)
sudo apt-get install -y pkg-config libwebkit2gtk-4.1-dev libgtk-3-dev \
  libayatana-appindicator3-dev librsvg2-dev libsoup-3.0-dev \
  libjavascriptcoregtk-4.1-dev

# Install frontend deps
cd crates/gui/frontend && npm install && cd ../../..

# Run in dev mode
cargo tauri dev

# Build release bundle
cargo tauri build
```

### MCP Tools (for AI coding tools)

The MCP server exposes 10 tools over stdio:

| Tool | Description |
|------|-------------|
| `cerebro_search` | Full-text search across all notes |
| `cerebro_read_note` | Read a note by title or path |
| `cerebro_create_note` | Create a new note with optional tags |
| `cerebro_update_note` | Update an existing note's content |
| `cerebro_delete_note` | Delete a note |
| `cerebro_list_notes` | List notes, optionally filtered by tag |
| `cerebro_list_tags` | List all tags with counts |
| `cerebro_backlinks` | Get notes linking to a given note |
| `cerebro_graph` | Get the full knowledge graph |
| `cerebro_kp_search` | Semantic search via Knowledge Palace |

## Configuration

Cerebro stores config at `~/.cerebro/config.toml`:

```toml
[app]
default_profile = "personal"

[profiles.personal]
display_name = "Personal"
notes_dir = "notes"
encryption = false

[profiles.personal.knowledge_palace]
enabled = true
endpoint = "http://localhost:8080"

[profiles.professional]
display_name = "Professional"
notes_dir = "notes"
encryption = false
```

## Development

```bash
# Run all tests
cargo test

# Run with clippy
cargo clippy -- -D warnings

# Build core library only
cargo build --release -p cerebro

# Build GUI only
cargo build --release -p cerebro-gui

# Frontend development
cd crates/gui/frontend
npm run dev
```

## Tech Stack

| Component | Technology |
|-----------|------------|
| Core library | Rust 1.96.0, edition 2024 |
| Storage | SQLite + FTS5 |
| Parser | pulldown-cmark |
| Link graph | petgraph |
| GUI framework | Tauri 2.0 |
| Frontend | React 19 + TypeScript |
| Editor | CodeMirror 6 |
| Styling | TailwindCSS v4 |
| Graph viz | D3.js force-directed |
| CLI | clap 4 |
| Search | rusqlite FTS5 |
| HTTP client | reqwest (KP connector) |
| MCP | JSON-RPC 2.0 over stdio |
| Build | cargo + Vite |

## License

MIT
