# 0003 — Cerebro is explicitly phased: personal KB first, integration hub second

Date: 2026-11-06
Status: Accepted (owner verdict, grill round 1)

## Context

The plan doc describes an integration hub (10 MCP tools, REST API, KP
everywhere, Confluence, git sync); shipped reality is a working personal
knowledge base GUI/CLI with partially wired integrations. The owner was asked
to pick one identity and chose both — explicitly phased.

## Decision

Cerebro develops in two explicit phases.

**Phase A — Personal knowledge base (current):** correctness and honesty of
the existing GUI/CLI/SQLite/FTS5 core; MCP correctness; removal of dead code.
No new integration surface.

**Phase B — Integration hub (gated):** deeper integrations (REST API, KP in
CLI/MCP, sync hardening) built only when Phase A is stable in daily use and
the specific integration has a demonstrated consumer.

Gate for opening Phase B items (tripwires, per item):
- REST API / MCP HTTP+SSE — tripwire: a real non-GUI consumer exists that
  stdio MCP cannot serve.
- KP in CLI/MCP (`cerebro_kp_search`) — tripwire: KP API contract verified
  against KP source or a live instance (currently UNKNOWN: repo absent from
  this machine, :8080 not serving at audit time).
- Git sync hardening (conflict handling, LWW-with-backup) — tripwire: first
  real conflict or lost note during sync.
- Encryption (AES-256-GCM + Argon2id) — owner intends to implement soon;
  `aes-gcm`/`argon2` deps are deliberately kept while all other dead deps are
  stripped. Tripwire to revisit the claim: if not implemented within the next
  release cycle, the README claim is reworded to "planned" or removed.

## Consequences

- The plan document's single-track milestone list is superseded; phase B
  items must not be started off their tripwires without an owner override.
- "Integration hub" claims in README must be reconciled to phase reality at
  each release.
