# Security design — open items (2026-11-07)

Status: **ROUND 2 SETTLED — design complete, awaiting GO.** All five
round-2 decisions recorded in ADR 0006/0007 addenda. One interpretation
flagged for confirmation at GO (see below).

## All decisions (round 1 + round 2)

- Threat model: disk theft + AI over-read via MCP IN; root/memory/forensics OUT.
- Encryption OPTIONAL per vault — the owner's binding constraint: it must
  never cripple Cerebro or add friction. Off = byte-identical behavior.
- Defaults: Business vaults protected by default; Personal and Project
  vaults plaintext by default, opt-in per vault (round-2 correction of the
  round-1 reversal — the owner's original intent).
- Mechanics: SQLCipher for db, per-file AES-256-GCM for notes, wrapped
  keys in vault.toml (Q6 yes).
- Relock: sleep + manual ONLY; no idle relock (Q7 correction).
- Recovery: none — passphrase is the only root credential, loud warning at
  enable (Q8 "only password"). TouchID stays an optional convenience
  wrapper, never a second way in.
- Migration: `cerebro vault encrypt|decrypt` + GUI toggle; git pushes
  ciphertext; encrypted vault loses Obsidian readability (Q9 yes).
- MCP: per-profile policies (`none|read|write`) in config.toml +
  `--scope` override; audit = ONE JSONL file, no rotation (Q10 simplified).
- Contexts: profiles are the unit; VaultKind gains Project (ADR 0008).

## Flagged interpretation (confirm at GO)

"3. only password" was read as: no recovery-code mechanism — the
passphrase is the only credential (Q8 option b). TouchID/Keychain caching
from round 1 remains an OPTIONAL convenience, not a credential. If you
meant "drop TouchID entirely," say so — that removes the macOS Keychain
work item from the queue.

## Build queue (priority order, on GO)

1. Crypto core: key hierarchy, SQLCipher migration, per-file GCM format,
   `cerebro vault encrypt|decrypt|lock|unlock` + tests (wrong passphrase,
   truncated/corrupt file, reindex-inside-session, decrypt round-trip).
2. Unlock lifecycle: session state, sleep/manual relock, macOS
   Keychain+TouchID optional path, GUI lock button + unlock prompt.
3. MCP policies + audit JSONL + locked-invisible semantics + tests.
4. VaultKind::Project + GUI tag support.
5. README security section + vault.toml docs.
