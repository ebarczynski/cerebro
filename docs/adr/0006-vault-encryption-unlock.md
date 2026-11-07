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
