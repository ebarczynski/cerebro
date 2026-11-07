# Security design — open items (2026-11-07)

Status: **AWAITING OWNER INPUT — round 2 timed out unanswered.** Round-1
verdicts are settled (ADRs 0006–0008). The five round-2 questions below are
proposals, NOT decisions; no implementation may start from this file alone.
Any of them answered in chat counts as the decision.

## Round-1 verdicts (recorded)

- Threat model: disk theft + AI over-read via MCP IN; root/memory/forensics OUT.
- Encryption: per-vault enablement, but ALL vaults fully encrypted when
  enabled (owner reversed the Business-default proposal — no kind-based
  defaults).
- Unlock: Argon2id passphrase always; macOS optional Keychain+TouchID cache.
- MCP: per-profile policies + CLI scope override + audit journal; locked
  vault = invisible to MCP.
- Contexts: profiles are the isolation unit; VaultKind gains Project.

## Pending round-2 decisions (proposals attached)

1. **Crypto mechanics** — proposal: SQLCipher for db.sqlite
   (`bundled-sqlcipher`), per-file AES-256-GCM for .md (own nonce + header
   per file, git-friendly), wrapped master-key copies + salt + Argon2id
   params stored in vault.toml.
2. **Relock lifecycle** — proposal: 15 min idle relock (configurable),
   relock on system sleep, manual `cerebro lock` + GUI button.
3. **Forgotten passphrase** — proposal: loud unrecoverable warning + optional
   one-time 32-char recovery code with its own Argon2id wrap of the master
   key.
4. **Enable/migration flow** — proposal: `cerebro vault encrypt|decrypt
   <name>` + GUI toggle; git sync pushes ciphertext; encrypted vault loses
   Obsidian readability, documented.
5. **MCP policy + audit schema** — proposal: `[mcp.profiles.<name>]`
   allow = none|read|write (+ optional tool deny-list) in config.toml;
   `cerebro mcp --scope <profile>:<level>` session override; audit =
   append-only JSONL `~/.cerebro/audit/mcp-YYYYMM.jsonl` (ts, tool,
   profile, decision, reason — never note content), rotated monthly.

## Build queue (draft, on round-2 completion)

1. Crypto core: key hierarchy, SQLCipher migration, per-file GCM, vault
   encrypt/decrypt commands + tests (including wrong-passphrase and
   truncation/corruption cases).
2. Unlock lifecycle: session state, relock timers, Keychain/TouchID path
   (macOS), GUI lock UI.
3. MCP policies + audit journal + locked-invisible semantics + tests.
4. VaultKind::Project + GUI tag support.
5. README/docs security section.
