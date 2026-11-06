# 0005 — MCP contract: 10 tools, spec-compliant protocol, KP tool gated

Date: 2026-11-06
Status: Accepted (owner verdict, grill round 1)

## Context

Audit (docs/grill/2026-11-06-design-review.md, F9/F17) found: README
documents 10 MCP tools, the server exposes 8; unknown tools return
success-shaped text ("Unknown tool: …") with no JSON-RPC error object and no
`isError` flag; the server answers the `notifications/initialized` JSON-RPC
notification (protocol violation); `initialize` hardcodes
`protocolVersion: 2024-11-05`, two revisions behind current (2025-06-18).

## Decision

1. The MCP public contract is **10 tools** and the README table stays true.
2. Implement `cerebro_graph` immediately (nodes + edges already exist in the
   `links` table).
3. Implement `cerebro_kp_search` **only after** the Knowledge Palace API
   contract is verified against KP source or a live instance. Current
   contract status: UNKNOWN (KP repo absent from this machine, :8080 not
   serving at audit time). The hand-rolled `urlencoding()` in the KP
   connector must be replaced with a real percent-encoding crate when that
   work happens.
4. Protocol compliance: unknown tools and tool failures return JSON-RPC
   errors / `isError: true` results; notifications receive no response;
   `initialize` negotiates `protocolVersion: 2025-06-18` (echoing the
   client-offered version per spec when supported).

## Consequences

- MCP clients can rely on tool listings and distinguish failures from
  successes.
- `cerebro_kp_search` remaining unimplemented while documented is a
  temporary, recorded exception — Phase B tripwire per ADR 0003.
- Any future tool addition requires a README table update in the same PR.
