# Changelog

All notable changes to Cerebro are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); versions follow semver.

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
