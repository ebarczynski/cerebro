# Changelog

All notable changes to Cerebro are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); versions follow semver.

## [1.1.0] — 2026-11-25

Results of a pre-release engineering audit (2026-11-23). Every item below has
a regression test in `tests/audit_regressions.rs` or `tests/audit_cli_mcp.rs`.

### Fixed — data safety

- **Creating a note could destroy another note.** The file was written before
  the database rejected a duplicate path, so a second note whose title mapped
  to the same file name ("Roadmap 2026" / "Roadmap_2026", two captures in the
  same minute, a second "Untitled Note" from the GUI) overwrote the first
  note's file. `create_note` now never overwrites: a derived file name gets a
  numeric suffix, an explicit path that is taken is an error, and the file is
  created with `O_EXCL`.
- **Writes are atomic.** File and index rows are written under one SQLite
  transaction, write lock first; on failure neither exists.
- **Deleting a note made its path unusable.** The soft-deleted row kept the
  `UNIQUE(path)` slot: recreating a note with the same title failed, and every
  later `reindex` failed. Notes are now removed outright, and tombstones left
  by earlier versions are retired when a path is reused.
- **A deleted `db.sqlite` no longer looks like an empty vault**: the CLI and
  MCP rebuild the index from the `.md` files before the first command.
- **`cerebro reindex` failed forever after a note file was deleted by hand**
  (`FOREIGN KEY constraint failed`) — and since the GUI reindexes on open, the
  vault could no longer be opened in the GUI.
- **A reindex rewrote `updated_at` on every untouched note** and reported all
  of them as "Updated". Unchanged files are now left alone.
- **`cerebro daily <date>` recreated — and overwrote — the day's note** once
  more than 1,000 notes were newer.
- **`encryption = true` was silently ignored** and the vault stored in
  plaintext. It is now refused with an explicit error.

### Fixed — links

- **Wikilink resolution depended on the order files were walked**: after a
  cold reindex only 45 % of resolvable links were resolved (every forward
  reference stayed open until the next reindex). Links are now resolved once,
  against the complete index.
- **`[[Note]]` now resolves by file name**, the way Obsidian does (then by
  `title:`, exact path, or a unique substring) — before, a link was "ambiguous"
  as soon as another note's name contained it.
- **Searching for a hyphenated word found nothing** — `state-of-the-art`,
  `e-mail`, `2026-11-23`, `rust-lang` all made FTS5 fail, and the error was
  swallowed into "No results". Hyphenated words are now searched as the phrase
  the indexer splits them into; a query FTS5 will not parse (a stray bracket,
  an unbalanced quote, a dangling `AND`) is retried as a search for its words
  instead of failing.
- **Backlink line numbers and context were wrong for any note with
  frontmatter** (counted from below the frontmatter, looked up from the top).
  The stored context is now the link's own line, cut to a 200-character window
  around the link — it used to copy the whole line, however long.
- **A link to a note that did not exist yet resolves as soon as that note is
  created.**

### Fixed — server and CLI

- **`cerebro mcp` could corrupt its own JSON-RPC stream**: the tracing
  subscriber wrote to stdout, so `RUST_LOG=debug` (or stricter) mixed log
  lines into the protocol. Logs go to stderr.
- MCP: `ping` is answered, batches are supported, a wrong `jsonrpc` is
  `-32600`, missing required arguments are `-32602` instead of an empty
  success, and `cerebro_delete_note` no longer claims to be recoverable.
- **A `profile` value could escape the profiles directory**
  (`--profile ../../escaped`, MCP `profile: "../../x"`) and an unknown profile
  was silently created — by a search. Named profiles must exist and be plain
  names.
- **`cerebro profile switch` did nothing**: it wrote the choice to the
  platform's Application Support directory, which nothing reads. It now
  writes the config file it was loaded from.
- `profile create --display-name` is stored; `kp health` says "unreachable"
  instead of dumping a transport error; `notes edit` uses a private, deleted
  afterwards temp file; an invalid KP `mode` is an error, not a fallback.
- **Opening a brand-new vault from two processes at once** could fail with
  `UNIQUE constraint failed: schema_migrations.id` or `database is locked`.
- `cerebro sync push` with nothing to commit is no longer an error, and a
  repository starts on the configured branch.

### Fixed — importer and GUI

- **HTML import duplicated nested content** and tore bold/italic/link marks
  out of their sentences; each element is now emitted once, in place.
- DOCX paragraphs inside tables were emitted twice; text after an ODT table
  was lost; table delimiter rows were malformed (`|| --- |`); only the first
  worksheet of a spreadsheet was imported.
- GUI: opening, reindexing, document import, Obsidian import, the graph and
  auto-links run off the main thread; frontend-supplied file paths are
  confined to the vault; `open_vault` reported the previously open vault's
  kind; unused `fs`/`shell`/`clipboard` plugins are no longer compiled in or
  granted to the webview.

### Changed

- **Auto-links are capped at 20 per note.** 5,000 notes used to produce 1.7
  million links (≈220 MB of JSON handed to the webview).
- A note's title is its `title:` frontmatter, else its **file name** (the
  Obsidian convention). The first `# heading` is no longer used as a fallback.
- Hidden directories (`.trash/`, `.git/`, `.obsidian/`, dotfiles) are not
  indexed.
- Tool results from MCP are compact JSON; `cerebro_graph` responses are about
  a quarter smaller.
- Writing a note is one durable transaction (file and index rows together),
  which costs about 1 ms per note instead of a few tenths.
- `cerebro backlinks` / `forward links` show the other note's title instead
  of its UUID; `![[Note#Heading]]` embeds that section rather than the whole
  note; hidden directories are not indexed; files with an upper-case `.MD`
  extension are indexed too (the Obsidian importer accepts them, the indexer
  did not, so they were lost on a rebuild).
- Removed eight unused dependencies (`pulldown-cmark`, `aes-gcm`, `argon2`,
  `notify`, `open`, `assert_cmd`, `predicates`, `criterion`) and three unused
  Tauri plugins.
- `cargo clippy --workspace --all-targets -- -D warnings` passes and is run in
  CI.

### Known issues, not fixed in this pass

- Tags passed at creation (`--tags`, the MCP `tags` argument) are written
  into the note only when the content has no frontmatter of its own;
  otherwise they live in the index and disappear at the next reindex.
- The MCP server handles one request at a time; a `cerebro_kp_search` call
  waits up to 10 s on Knowledge Palace before anything else is answered.
- Auto-linking compares every pair of notes (about 2.5 s for 10,000 notes,
  off the GUI's main thread).

### Development notes

- Build the GUI through the tauri CLI (`cargo tauri build`, or `tauri build`
  from `crates/gui` — see the README). A plain `cargo build -p cerebro-gui`
  compiles fine but links a binary that expects the Vite dev server and shows
  a white window: the frontend bundle is only embedded when the tauri crate's
  `custom-protocol` feature is on, which the tauri CLI enables.

## [0.4.1] — 2026-11-06

### Changed
- **New app icon** — minimal brain-circuit mark (organic fold on the left
  hemisphere, circuit trace on the right) in the app's indigo gradient on a
  dark squircle. Replaces the plain circle everywhere: macOS icns, Windows
  ico, the full PNG set, the in-app top bar and welcome screen (new `Logo`
  component), and the webview favicon. Design source:
  `crates/gui/icons/icon.svg`.

## [0.4.0] — 2026-11-05

### Added
- **Obsidian vault import** — import a whole Obsidian vault from the GUI:
  toolbar gem button or `Ctrl+Shift+O`. Walks the vault (skipping `.obsidian`,
  `.trash`, hidden dirs), imports every `.md` note preserving folder structure
  under an optional destination folder, merges frontmatter + inline tags, and
  skips notes that already exist so re-imports never duplicate.
- **Keyboard shortcuts help** — `?` toolbar button (or press `?` outside text
  fields, or `Ctrl+/`) opens a cheat-sheet modal listing all shortcuts.
- `Storage::note_exists()` — path-existence check used by vault import.

### Changed
- **Editor readability** — brighter editor text, line-height 1.75, horizontal
  padding, default font size 15px; markdown preview at 15px / 1.75; sidebar
  search, find & replace bar and vault inputs got taller fields, brighter
  text and focus rings.
- **Toolbar spacing** — wider gaps and larger hit targets on toolbar and
  sidebar quick-action icons.

### Fixed
- **macOS DMG "damaged" on install** — the app bundle is now ad-hoc signed
  (`bundle.macOS.signingIdentity: "-"`). Since macOS Sequoia, Gatekeeper
  reports downloaded *unsigned* apps as "damaged and can't be opened"; the
  ad-hoc signature restores the normal right-click → Open flow. CI now
  verifies the signature before publishing artifacts.

## [0.3.1] — previous release

- CI fixes: release workflow action ref, tauri-cli discovery, npm/rust
  version skew; deb packaging verified locally.
- In-app vault modals for macOS (WKWebView has no `window.prompt`), vault
  name shown in the status bar.

## [0.3.0] — knowledge graph era

- Obsidian-style knowledge graph: degree-scaled nodes, hover neighborhood
  highlighting, wheel/drag zoom + pan.
- Recently-used vaults list with private/business tags, switchable from the
  status bar.
- Build+test CI (ubuntu x86_64, macos arm64) and tag-driven release workflow
  with artifacts.

## [0.2.x]

- Wikilink autocomplete, live preview modes (edit/split/preview), outgoing
  links panel, file watcher for external edits, note templates with
  `{{title}}`/`{{date}}`/`{{time}}`, transclusions (`![[Note]]`), Mermaid
  diagram rendering.
- Search & replace (`Ctrl+H`), live search results, persistent last-vault.
- Categories/favorites, keyword-based graph links, editor font size controls
  (`Ctrl+=`/`Ctrl−`/`Ctrl+0`).
- Image embedding + paste/drop, Excalidraw diagram rendering, syntax
  highlighting.
- Document import: PDF, PPTX, DOCX, DOC, ODS, XLSX, XLS, ODT, HTML →
  Markdown, via toolbar and drag-and-drop.

[0.4.1]: https://github.com/ebarczynski/cerebro/releases/tag/v0.4.1
[0.4.0]: https://github.com/ebarczynski/cerebro/releases/tag/v0.4.0
[0.3.1]: https://github.com/ebarczynski/cerebro/releases/tag/v0.3.1
