# Open Items — updated after grill round 1 (2026-11-06)

Status: **GO RECEIVED 2026-11-07 — Phase A queue EXECUTED; Phase B KP leg
EXECUTED** on branch `barczynski/grill-2026-11-06`. 84 tests green; defect
battery + KP fallback re-verified live. KP contract verified against KP
source @ `82775ae` → `cerebro_kp_search` shipped (ADR 0005 addendum);
end-to-end against a live KP instance still pending (KP not running at
implementation time). Remaining Phase B tripwires per ADR 0003: REST/SSE
(needs a consumer), sync hardening (needs a real conflict), encryption
(owner implementation). Owner merges the branch.

## Decided (see ADRs)

- Product identity: **both, explicitly phased** — ADR 0003.
- Confluence: **removed** until a real workspace exists — ADR 0004.
- MCP: **10-tool contract, protocol fixes, KP tool gated on verified
  contract** — ADR 0005.
- S1 defects: **fix all 8, each with a regression test** (build queue item 1).
- Honesty pass: strip `axum`/`tower-http`; **keep** `aes-gcm`/`argon2`
  (encryption implementation planned by owner); drop 3 dead tables
  (`sync_log`/`kp_cache`/`daily_notes`); fix `tantivy/` ghost in sync
  .gitignore. README "Encryption-ready" claim: acceptable only while the
  implementation actually lands next cycle (tripwire per ADR 0003).

## Still open

1. **Phase boundary pinning** — Phase A "done" shipped with the grill
   branch; Phase B items follow their tripwires (ADR 0003).
2. **KP live end-to-end** — contract verified against KP source and
   `cerebro_kp_search` shipped; run one search against a live KP instance
   (:8080) to close the loop.
3. **Deferred with tripwires** (recorded in ADR 0003): REST/SSE transport,
   sync conflict handling, encryption claim rewording, O(n²) keyword links
   (tripwire: any vault >5k notes).

## Build queue (priority order, on GO — Phase A)

1. S1 defect fixes (8 items, regression tests) — protects existing users.
2. Honesty pass (strip axum/tower-http, keep crypto deps, dead schema,
   .gitignore ghost) — binary size + truthful docs.
3. MCP fixes + `cerebro_graph` (`cerebro_kp_search` waits on KP contract).
4. README/plan doc reconciliation to shipped reality (Confluence bullet
   removed, 10-tool table true, encryption claim honest).
5. Everything else waits for Phase B tripwires per ADR 0003.
