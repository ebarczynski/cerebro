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

