# 0006 — Vault at-rest encryption and unlock

Date: 2026-11-07
Status: Accepted (security grill round 1; implementation params pending round 2)

## Threat model (in scope / out of scope)

IN: laptop theft and disk imaging; AI clients over-reading via MCP,
including exfiltration through prompt injection.
OUT (explicitly, documented honestly): root compromise of a running
machine, memory scraping while a vault is unlocked, forensic memory
analysis. An unlocked session with the key in memory is accessible by
construction; relocking shrinks, never eliminates, that window.

## Decision

1. **Protection is per-vault.** When protection is enabled for a vault,
   EVERYTHING in it is encrypted at rest: all `.md` note files AND the
   database (SQLCipher via rusqlite `bundled-sqlcipher`). No mixed
   plaintext/ciphertext vaults; no kind-based defaults — enabling
   protection on any vault (Private, Business, Project) encrypts that
   vault completely.
2. **Key hierarchy:** a random per-vault 256-bit master key wraps all
   content keys. The master key is wrapped twice independently:
   - passphrase wrapper: Argon2id(passphrase, salt) → AES-256-GCM wrap
     (argon2 + aes-gcm deps already in the tree)
   - macOS optional: the master key cached in the user's Keychain behind
     TouchID (biometry-gated item), one biometric prompt per app session.
     Linux has no biometric path; passphrase only.
3. **Unlock lifecycle:** per-app-session. Unprotected vaults behave as
   today. Keys live in memory only while the session is unlocked;
   auto-relock after an idle timeout (default value pending round 2),
   manual lock command, and relock-on-sleep are required behaviors.
   MCP clients never see a locked vault's contents — locked means
   invisible to every MCP tool (see ADR 0007).
4. **Plaintext vaults remain fully supported.** Encryption is opt-in per
   vault; no forced migration.

## Consequences

- Encrypted vaults lose human readability and Obsidian compatibility for
  their notes while at rest; git sync keeps working (ciphertext is
  content-addressable by git, diffs of ciphertext are meaningless).
- `db.sqlite`, WAL, and SHM are all inside the SQLCipher envelope; the
  derived-index-rebuild promise (ADR 0001) still holds — reindex operates
  inside the unlocked session.
- A forgotten passphrase with no cached Keychain copy means permanent
  loss of that vault's contents. The enable-time flow must state this and
  the recovery stance (pending round 2).
- **Known limitation (documented, deferred):** note ciphertext does not
  bind the file path as AEAD associated data. An attacker with disk access
  can relocate a ciphertext file to another note's path and it will still
  decrypt — content stays authenticated and unreadable, but its placement
  is not cryptographically bound. Path-AAD is a candidate for a format
  version bump; deliberately deferred per the simplicity budget.
- **Crash safety:** migrations take a full pre-flight backup (db + WAL +
  notes) under `~/.cerebro/backups/`. A crash mid-migration leaves the
  config flag flipped and files partially converted; reads tolerate the
  mixed state (plaintext-in-protected-vault passthrough), and the backup
  is the recovery path.

---

## Addendum — round 2 (2026-11-07): owner corrections + convenience guardrail

The owner corrected this ADR in round 2. Two changes are **dated
corrections** of the round-1 text; the rest settles the pending params.

**Correction 1 — kind-informed defaults replace "all vaults when enabled".**
Encryption is OPTIONAL per vault and stays optional: Business vaults get
protection enabled by default (recommended at creation); **Personal and
Project vaults are plaintext by default** and encrypt only when the owner
explicitly enables it. The round-1 "no kind-based defaults" sentence is
superseded by this one.

**Correction 2 — no idle relock.** The session does NOT relock after idle
time. Relock happens on system sleep and via manual lock
(`cerebro lock` / GUI button) only. Simplicity beats hardening here.

**Overriding design principle (owner's words, binding):** encryption is
optional and must never cripple Cerebro or make daily work inconvenient.
When protection is off, behavior is byte-for-byte identical to today; when
on, the entire friction budget is: passphrase at session unlock, relock on
sleep/manual. Anything beyond that budget needs a new owner decision.

**Settled mechanics:**
- SQLCipher (`bundled-sqlcipher`) for db.sqlite — `PRAGMA key` from the
  vault master key.
- Per-file AES-256-GCM for notes: each `.md` becomes ciphertext with its
  own nonce and a small header (magic, version, salt info).
- The master key's wrapped copies (passphrase wrap + optional macOS
  Keychain handle), salt, and Argon2id parameters live in `vault.toml`.
- The passphrase is the ONLY root credential. There is NO recovery code
  and NO recovery path; enable-time flow warns loudly that a forgotten
  passphrase means permanent loss. TouchID stays an optional convenience
  wrapper around the cached key (round-1 decision) — it is convenience,
  never a second way in, and never a recovery mechanism.
- Enable/migrate: `cerebro vault encrypt <name>` (re-encrypts notes,
  rebuilds the database as SQLCipher, keeps a pre-flight backup directory)
  and `cerebro vault decrypt`; GUI toggle equivalent. Git sync pushes
  ciphertext as-is.

