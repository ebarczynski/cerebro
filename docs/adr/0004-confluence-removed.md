# 0004 — Confluence publisher removed until a real workspace exists

Date: 2026-11-06
Status: Accepted (owner verdict, grill round 1)

## Context

The Confluence publisher (`src/publish/`) is unreachable dead code: zero call
sites outside its module, the CLI `publish` command never invokes it (prints
"requires configuration", or raw Markdown mislabeled "HTML preview" in
dry-run). Even if wired, it carries three contract bugs: Confluence Cloud v1
endpoint style (`/rest/api/content/`) where the plan promises v2, Bearer auth
where Cloud API tokens require Basic email:token, and a code-block converter
that emits an unclosed CDATA macro with code content rendered as paragraphs.
The plan also promises idempotent update-by-title, which is not implemented.

## Decision

Delete `src/publish/`, the CLI `publish` command, and the Confluence config
section. Reintroduce only when the owner has a real Confluence workspace to
publish to, and then build it against the v2 API with Basic auth, a real
AST-based converter, and idempotent create-or-update by title.

Tripwire for reintroduction: owner names an actual space/page they publish to
regularly.

## Consequences

- README's Confluence feature bullet is removed at the same time.
- `reqwest` stays (still used by the KP connector).
- The grill audit's F10 findings are closed as "removed by decision", not
  "fixed".
