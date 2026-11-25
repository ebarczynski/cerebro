# 0007 — MCP sandboxing and access audit

Date: 2026-11-07
Status: Accepted (security grill round 1; policy schema details pending round 2)

## Context

Today every MCP tool call can address any profile via its `profile`
parameter, and there is no record of what AI clients read or wrote. With
encryption (ADR 0006) and multi-context vaults (ADR 0008), MCP becomes a
delegation boundary: AI clients are useful precisely because they are
semi-trusted.

## Decision

1. **Declared policies per profile.** `config.toml` declares an MCP policy
   for each profile: access level `none | read | write` and the visibility
   scope (which content within the profile is exposed). The
   `cerebro mcp` start command may override via scope flags; absent
   policy, the default is **active profile, read-only**.
2. **Locked means invisible.** When a vault is locked (ADR 0006), MCP
   tools neither return its content nor leak its existence beyond what a
   policy statement already reveals. No fallback-to-plaintext, no partial
   metadata.
3. **Audit journal.** Every MCP tool call is logged: timestamp, client
   identification as available over stdio, tool name, profile addressed,
   access decision (allow/deny + reason). The journal is local-only and
   its location/format is specified in round 2.
4. **Transport unchanged:** stdio JSON-RPC remains the only transport;
   HTTP/SSE stays behind its ADR 0003 tripwire. Network-exposed MCP would
   require revisiting this ADR.

## Consequences

- `cerebro_read_note`-style tools can no longer silently cross profile
  boundaries; cross-profile reads require an explicit policy statement,
  which is auditable.
- Denials are first-class results (isError with a clear reason), so AI
  clients can react instead of guessing.
- The audit journal is itself sensitive (it reveals content patterns) and
  lives under the user's config dir with user-only permissions.

---

## Addendum — round 2 (2026-11-07): audit simplified

The owner questioned the monthly rotation: "why monthly audit? keep it
simple." Accepted — the audit journal is the SIMPLEST useful form:

- **One** append-only JSONL file: `~/.cerebro/audit/mcp.jsonl`. No
  rotation, no retention machinery, no size management. If it grows too
  big, the owner deletes it — that's the retention policy.
- Fields per line: timestamp, tool name, profile addressed, decision
  (allow/deny), reason. Never note content.
- Policy schema: `[mcp.profiles.<name>]` in config.toml with
  `allow = "none" | "read" | "write"` (optional tool deny-list);
  `cerebro mcp --scope <profile>:<level>` overrides for the session;
  default remains active-profile read-only.
- Same simplicity principle as ADR 0006's addendum: the sandbox must not
  add friction for the common single-vault, local-client case — with no
  policy declared, behavior is exactly today's, plus one audit line per
  call.


---

## Amendment — 2026-11-25 (post-merge audit): implementation as shipped

- **The default policy breaks v0.6.0 behaviour.** Round-2's "with no policy
  declared, behavior is exactly today's" is superseded: with no policy, the
  active profile and the `--profile` a server was started for are
  read-only, every other profile is `none`. On v0.6.0 a client could read
  any profile by name and write to the active one. Users who need writes
  add `[mcp.profiles.<name>] allow = "write"` or pass `--scope`; the
  CHANGELOG calls this out as breaking.
- **The journal is at `~/.cerebro/audit/mcp.jsonl`** — the code originally
  wrote to the platform config directory (Application Support on macOS),
  and with default permissions; both fixed. Directory 0700, file 0600.
- **The journal fails open.** If it cannot be written, the call still runs
  and only a stderr warning notes the loss. Chosen deliberately over
  refusing every call while the journal is unwritable; this ADR and the
  code comment now say so.
- **Tool-error reasons for protected vaults are withheld** in the journal:
  they carry caller-supplied identifiers ("note not found: <title>"),
  which must not land in a file outside the vault. The client still gets
  the full error. "Never note content" therefore extends to identifiers.
- **"Locked means invisible" is about content.** A denied call does tell
  the client that the profile exists and is protected, and names the
  `--unlock` flag — the shape of the denial leaks, the content does not.
- **`--unlock` uses the keychain cache first** (no terminal needed);
  without a cache entry and without a controlling terminal the vault is
  skipped with guidance on stderr instead of failing the server start.
- Calls that fail argument validation are audited as denials, like every
  other call.
