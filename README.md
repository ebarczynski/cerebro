# 🧠 Cerebro

A fast, secure, local-first personal knowledge base with dual-profile vaults, 
MCP integration for AI coding tools, Knowledge Palace connector, and Confluence publishing.

Built with **Rust 1.96.0** (edition 2024), **Tauri 2.0**, **React 19**, 
**CodeMirror 6**, and **TailwindCSS v4**.

## Features

- **Dual-profile vaults** — separate personal and professional knowledge domains
- **Markdown-native** — `.md` files are the source of truth, with YAML frontmatter
- **Obsidian vault import** — import an entire Obsidian vault in one step:
  folder structure, YAML frontmatter tags, inline `#tags` and `[[wikilinks]]`
  are all preserved (`.obsidian` config is skipped, duplicates detected)
- **Document import** — drag & drop PDF, DOCX, PPTX, XLSX/ODS, HTML, Markdown,
  JSON or source files; each converts to a Markdown note
- **Wikilinks** — `[[Note Name]]` with `#heading` and `|alias` support
- **Bidirectional links** — automatic backlink and forward-link tracking
- **Full-text search** — SQLite FTS5 with Porter stemmer
- **Tag management** — unified frontmatter `tags:` + inline `#tags`
- **Daily notes** — `/Daily/YYYY-MM-DD.md` auto-generated
- **Knowledge graph** — interactive D3.js force-directed graph visualization
- **Keyboard shortcuts help** — press `?` (or the **?** toolbar button) anytime
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
│   ├── src/main.rs               # 40 Tauri commands wired to core lib
│   ├── frontend/                 # React 19 + TypeScript frontend
│   │   ├── src/
│   │   │   ├── App.tsx           # Main app with graph view, preview, welcome
│   │   │   └── components/       # Sidebar, Editor, Search, StatusBar,
│   │   │                         # ImportDropZone, ImportObsidianModal, ShortcutsModal
│   │   └── index.css             # TailwindCSS v4 dark theme
│   └── tauri.conf.json           # Tauri configuration
├── migrations/                   # SQLite schema migrations
└── tests/                        # Integration tests (68 total incl. unit tests)
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

**macOS:** requires macOS 26 (Tahoe) or newer — grab the signed DMG from
[Releases](https://github.com/ebarczynski/cerebro/releases/latest).

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

### Importing from Obsidian

Bring an existing Obsidian vault into Cerebro without losing anything:

1. Open a vault in the GUI (create a fresh one if you like)
2. Click the **💠 gem** button in the toolbar — or press `Ctrl+Shift+O` (`⌘⇧O` on macOS)
3. Pick your Obsidian vault folder, choose a destination folder (default: `Obsidian/`), and hit **Import Vault**

What's preserved:

| Obsidian artifact | Cerebro result |
|---|---|
| Folder structure | Mirrored under the destination folder |
| Note names | Titles (Obsidian convention: filename without extension) |
| YAML frontmatter `tags:` | Merged into the note's tags |
| Inline `#tags` | Merged into the note's tags |
| `[[wikilinks]]` | Kept as-is and resolved against imported titles |
| `.obsidian/`, `.trash/`, hidden dirs | Skipped |

Notes that already exist at the destination path are skipped, so re-running an
import never duplicates anything.

### Keyboard shortcuts

Press `?` anywhere outside a text field (or click the **?** toolbar button) for
the always-available cheat sheet. Highlights:

| Keys | Action |
|---|---|
| `Ctrl+K` | Search notes |
| `Ctrl+N` | New note |
| `Ctrl+S` | Save note |
| `Ctrl+E` | Cycle view: edit → split → preview |
| `Ctrl+P` | Toggle preview |
| `Ctrl+G` | Knowledge graph |
| `Ctrl+Shift+O` | Import Obsidian vault |
| `Ctrl+F` / `Ctrl+H` | Find / find & replace in note |
| `Ctrl+=` / `Ctrl−` / `Ctrl+0` | Editor font size up / down / reset |

(Use `⌘` instead of `Ctrl` on macOS.)

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
