# 0008 — Contexts are profiles; Project is the third kind

Date: 2026-11-07
Status: Accepted (security grill round 1)

## Context

The user works across personal, business, and project knowledge. ADR 0001
already makes the vault/profile the isolation unit (separate directories,
databases, and now keys per ADR 0006); `VaultKind` distinguishes
`Private | Business`.

## Decision

1. **Profiles remain the isolation unit.** Business/personal/project
   separation is achieved by having one vault per context — NOT by a
   category field on notes inside a shared vault. Storage-level isolation
   (directories, SQLCipher databases, independent keys, independent MCP
   policies) is the boundary we can actually defend.
2. **`VaultKind` gains `Project`.** Any number of project vaults may
   exist (`project-clients`, `project-vision2map`, …), each with its own
   encryption and MCP policy.
3. **MCP policies bind per profile** (ADR 0007); a context is sandboxed
   by giving its profile a policy, not by content filtering.

## Consequences

- No cross-context queries by design; the audit journal records any
  attempt (ADR 0007).
- Creating a new context is `cerebro profile create <name> --kind project`
  — cheap, already-supported flow.
- GUI vault lists and ownership tags gain the Project kind (UI work item).
