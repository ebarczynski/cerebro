# Open Items — pending owner verdicts (2026-11-06)

Status: **AWAITING OWNER INPUT.** Round 1 of the grill timed out unanswered —
nothing below is approved; nothing may be executed from this file alone.
Any of these questions answered in chat counts as the decision.

## Pending decisions (proposed answers included)

1. **Product identity** — proposal: GUI-first personal Obsidian replacement;
   integrations (MCP extras, KP in CLI/MCP, Confluence, REST/SSE) gated on
   demonstrated usage (ADR to be written on acceptance).
2. **Confluence** — proposal: delete `src/publish/` + CLI stub now;
   reintroduce when a real Confluence workspace exists. (Dead code today:
   zero call sites; v1 endpoint + Bearer auth + broken CDATA even if wired.)
3. **MCP** — proposal: implement `cerebro_graph` (data exists) and
   `cerebro_kp_search` (only after KP contract verified against KP source),
   add `isError`/JSON-RPC errors for unknown tools, stop answering
   `notifications/initialized`, bump `protocolVersion` to 2025-06-18;
   README's 10-tool table becomes true.
4. **S1 defect pile** — proposal: fix all 8 demonstrated defects in one
   regression-tested pass on this branch: capture EISDIR, dup-wikilink
   UNIQUE crash, Polish truncate panic, path traversal, tag-count drift,
   wrong link context, phantom code-fence links, fuzzy destructive
   identifiers (exact-or-unique resolution).
5. **Honesty pass** — proposal: ratify ADR 0002 (FTS5-only), strip
   aes-gcm/argon2/axum/tower-http, drop "Encryption-ready" claim (or
   implement encryption — separate decision), drop dead tables
   sync_log/kp_cache/daily_notes, fix `tantivy/` ghost in sync .gitignore.

## Deferred-with-tripwire (proposed)

- **Encryption (AES-256-GCM + Argon2id)** — tripwire: first real
  sensitive/professional note lands in a vault → implement or drop the claim.
- **O(n²) keyword links** — tripwire: any vault >5k notes makes graph view
  sluggish → shard/index or cap pair scan.
- **Sync conflict handling** — tripwire: first observed conflict or lost note
  during sync → implement LWW-with-backup per plan.

## Build queue (priority order, on GO)

1. S1 defect fixes (8 items, regression tests) — protects existing users.
2. Honesty pass (deps, claims, dead schema) — binary size + truthful docs.
3. MCP fixes + `cerebro_graph` (+ `cerebro_kp_search` pending KP contract).
4. README/plan doc reconciliation to shipped reality.
5. Everything else waits for usage evidence per decision 1.
