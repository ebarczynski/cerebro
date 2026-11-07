# Cerebro Design Grill — Audit Report (2026-11-06)

Auditor: Hermes grill-with-docs session. Branch: `barczynski/grill-2026-11-06`.
Evidence classes: **MEASURED** = reproduced live against v0.4.1 debug build in a
sandbox vault; **CITED** = read from source/docs; **UNKNOWN** = could not verify.

Base: commit `a1c069f` (v0.4.1), master. Build: 0 errors, 9 warnings.
Tests: 64 passed (37s). No benches, no property tests, no fuzz (plan promised all three).

---

## S1 — Demonstrated defects (would bite a real user today)

### F1. `cerebro capture` fails on every invocation — MEASURED
`cli` passes `"inbox/"` as the note path; `Storage::create_note` joins and
writes it → `Is a directory (os error 21)`. The plan's flagship quick-capture
command has never been able to work as shipped.

### F2. Same wikilink twice on one line ⇒ note creation fails entirely — MEASURED
`[[Target A]] and again [[Target A]]` → `UNIQUE constraint failed:
links.source_id, links.target_ref, links.line_number`. The parser emits
duplicates; `update_links` inserts blindly; the error aborts the whole insert.
The note cannot be created at all.

### F3. `cerebro notes list` panics on Polish titles — MEASURED
`truncate()` slices bytes: `end byte index 37 is not a char boundary; it is
inside 'ą'` at `src/cli/mod.rs:660`. Any multibyte char landing on the cut
boundary makes the note list crash. (Polish-language vault = the owner's actual
use case.)

### F4. Path traversal via `path` argument (CLI and MCP) — MEASURED
`cerebro_create_note {path: "../../escape_test"}` wrote the file to
`~/.cerebro/profiles/escape_test` — outside the vault. The plan's security
section explicitly promises "All file paths validated against vault root".
`sanitize_path` is only applied to title-derived paths, never to user-supplied
`path`.

### F5. Tag counts are write-only fiction — MEASURED
`tags.note_count` increments on insert, never decrements. Live: 2 notes tagged
`rust` → update one to remove the tag → delete the other → `cerebro tags`
still reports `rust = 2` with zero tagged notes.

### F6. Link context is wrong for every link except the first — MEASURED
`extract_link_context(content, _position)` ignores the line number and returns
the first line containing `[[` in the whole file. Live: backlink to line 5
shows line 1's text.

### F7. Phantom links from code fences — MEASURED
`let s = "[[Phantom Target]]";` inside a Rust code fence created an unresolved
graph edge. The parser is a line regex over the raw file — code, inline code,
and frontmatter all generate links. The plan promised pulldown-cmark AST
parsing; the dependency is only used in the import module, not the link path.

### F8. Ambiguous identifier resolution feeds destructive operations — CITED (code)
`find_note` matches `id = ?1 OR title = ?1 OR path LIKE %?1%` and
`query_row` returns the first row. A fragment identifier (`"rust"`) can update
or delete an arbitrary note sharing the substring. Same fuzzy resolution is
used for wikilink target resolution. (Demonstrated only for exact-title cases
in the sandbox; the fragment case is established by code reading.)

## S2 — Documented features that do not exist (README/plan vs reality)

### F9. MCP server: 8 of the 10 documented tools exist — MEASURED
`tools/list` returns 8. README documents `cerebro_graph` and
`cerebro_kp_search`; both reply `Unknown tool: …` **as a success-shaped text
result** (no JSON-RPC error object, no `isError` flag) — clients cannot tell a
missing tool from a working one. MEASURED live.

### F10. Confluence publishing is unreachable dead code — MEASURED (grep)
`ConfluencePublisher` has zero call sites outside its module. The CLI
`publish` command without `--dry-run` prints "requires configuration" and
exits; with `--dry-run` it prints the raw Markdown labeled "HTML preview"
without invoking the converter. Even if wired, the module: uses v1
`/rest/api/content/` (plan promises v2), Bearer auth (Confluence Cloud API
tokens require Basic email:token), has no idempotent update-by-title (plan
promises it), and its code-block converter emits an unclosed CDATA macro with
the code content rendered as paragraphs. CITED: plan + Atlassian docs.

### F11. Knowledge Palace connector is GUI-only; API contract unverified — MEASURED (grep) + UNKNOWN
`KpConnector` is instantiated only in two GUI commands. No `cerebro kp` CLI
subcommand (plan quick-reference promises it), no MCP tool. The response-shape
contract (`{results: [{content, score, source, author}]}`) is UNKNOWN: KP is
not running on localhost:8080 and the KP repo is not on this machine. The
hand-rolled `urlencoding()` (4 characters) is CITED and objectively fragile
(`%`, `"`, non-ASCII break query URLs).

### F12. Encryption "AES-256-GCM ready" — dependency present, zero usage — CITED (grep)
`aes-gcm` and `argon2` compile into the binary; nothing calls them. README
lists encryption as a feature bullet.

### F13. REST API (:9090) and MCP HTTP/SSE transport — absent — CITED
Plan promises both with auth + rate limiting. `axum` + `tower-http` compile in
unused (MEASURED grep: zero usage in src).

### F14. Git sync: plan promises conflict detection + LWW-with-backup; ships raw git shelling — CITED
No conflict handling at all (`git pull` surfaces raw git errors). Config field
`auto_commit` is hardwired `false` at the only call site (CITED);
`commit_interval_minutes` is dead. `commit()` fails on a clean vault (git
exits non-zero with nothing staged) — MEASURED-adjacent (code path, not
probed live). `.gitignore` template references nonexistent `tantivy/` —
MEASURED (file read).

### F15. Dead schema — CITED
`sync_log`, `kp_cache`, `daily_notes` tables are created by migration 001 and
never written by any code path. (`cmd_daily` finds daily notes by scanning
`list_notes`.)

### F16. Plan's testing strategy unmet — CITED
No `proptest`, no `cargo-fuzz`, criterion in dev-deps but no `benches/`.
64 tests pass but none cover: capture, MCP tool shape, publish, sync, KP,
traversal — exactly the seams where S1/S2 live.

## S3 — Protocol / robustness notes

### F17. MCP JSON-RPC violations — MEASURED
Server answers the `notifications/initialized` notification with a response
(JSON-RPC 2.0 forbids responding to notifications; MEASURED live).
`initialize` hardcodes `protocolVersion: 2024-11-05` — two revisions behind
current spec 2025-06-18 (CITED: modelcontextprotocol.io; legal per version
negotiation, but disables newer features). Tool errors return text without
`isError: true` (CITED code).

### F18. `update_note` metadata drift — CITED (code)
Title is immutable through update (renames impossible via MCP/CLI edit);
frontmatter column keeps the old value when content is rewritten (file gets
the new bytes verbatim). MCP tool description says delete is "Recoverable
within retention period" — there is no retention, no recovery: the file is
hard-removed (CITED code).

### F19. No transactions around multi-step writes — CITED (code)
create/update/delete issue 3–6 statements without a transaction; a crash
mid-way leaves DB/FTS/links disagreeing. `delete_note` failure between DB
soft-delete and file removal permanently blocks reindex re-import
(`path` UNIQUE).

### F20. Fixed temp file for editing — CITED
`edit_in_editor` uses `/tmp/cerebro_edit.md` (no unique suffix, note contents
left in shared tmp). Concurrent sessions clobber each other.

### F21. `compute_keyword_links` is O(n²) — ESTIMATE
1k notes ≈ 500k pair comparisons (fine); 10k notes ≈ 50M (sluggish); plan
targets 100k notes (unusable). GUI calls it per graph render (CITED).

---

## What's solid

- **Architecture identity works as advertised**: files-as-truth + derived
  SQLite index + real reindex path; dual profiles are genuinely isolated
  (separate dirs/DBs, per-request profile param in MCP).
- **Migration discipline**: `schema_migrations` + `include_str!`, applied-once
  semantics, WAL + foreign_keys ON on open.
- **FTS5 managed in application layer** (no triggers) — the correct pattern;
  porter stemming + snippets + BM25 rank for free.
- **Wikilink parser is careful**: alias/heading splitting, transclusion
  exclusion, 1-indexed line numbers, good unit tests.
- **Import surface is a real differentiator**: full Obsidian vault import +
  PDF/DOCX/PPTX/XLSX/HTML conversion (1000+ line module) with dry-run.
- **Shipping discipline**: 2-platform CI, tag-driven releases with bundles,
  Dependabot-clean, 40 GUI commands wired, keyboard-driven dark UI.
- **Default-deny config**: all external integrations disabled until enabled.

## Scorecard (owner's call: concede / partially concede / defend)

| Finding | Class | Proposal |
|---|---|---|
| F1 capture broken | MEASURED | fix (join+sanitize, regression test) |
| F2 dup wikilink crash | MEASURED | fix (dedupe links before insert) |
| F3 Polish panic | MEASURED | fix (char-boundary-safe truncate) |
| F4 traversal | MEASURED | fix (validate path inside notes_dir) |
| F5 tag counts | MEASURED | fix (recount from note_tags) |
| F6 link context | MEASURED | fix (use line_number) |
| F7 phantom links | MEASURED | fix (strip fenced code before link pass) |
| F8 fuzzy destructive ops | CITED | fix (exact-or-unique resolution) |
| F9 MCP 8≠10 + error shape | MEASURED | decide: implement or re-document; add isError |
| F10 Confluence dead | MEASURED | decide: delete or finish properly |
| F11 KP GUI-only, UNKNOWN contract | UNKNOWN | decide: verify against KP source, wire MCP, or descope |
| F12 encryption claim | CITED | decide: implement or drop claim + deps |
| F13 REST/SSE absent | CITED | decide: descope or build |
| F14 sync minimal | CITED | decide: accept (files are git-syncable by hand) or finish |
| F15 dead schema | CITED | fix (drop tables or wire them) |
| F16 test strategy | CITED | fix with F1–F9 regression tests |
| F17 MCP protocol notes | MEASURED | fix (no-reply to notifications, current protocolVersion) |
| F18 update semantics | CITED | fix (title re-extract, frontmatter refresh, honest delete wording) |
| F19 no transactions | CITED | fix (wrap write paths) |
| F20 fixed tmp file | CITED | fix (unique tempfile) |
| F21 O(n²) links | ESTIMATE | defer with tripwire (vault >5k notes) |
