# Add stateless MCP Streamable HTTP transport

## Why

Hydra currently provides an MCP runtime only over child-process stdio. That
transport cannot serve a Rust MCP server embedded in a host process, such as an
Anki add-on. Hydra needs a reusable HTTP transport that preserves the existing
single generated tool definition and single consumer dispatch boundary rather
than asking each embedded project to invent its own MCP route.

## What changes

- Add a reusable Axum-based `hydra-mcp-http` runtime alongside
  `hydra-mcp-stdio`.
- Implement the current MCP `2026-07-28` Streamable HTTP **server** profile:
  stateless POST requests, per-request metadata, origin validation, and no
  protocol-level session or GET stream.
- Serve `server/discover`, `tools/list`, and `tools/call` from the same
  generated `mcp.json` tool objects and caller-owned dispatch closure used by
  the stdio surface.
- Add a real local client-to-server acceptance path using the Notes reference
  consumer, plus protocol, security, and manifest-validation coverage.
- Document the explicit host embedding, origin-policy boundary, and initial
  no-CORS/PNA behavior.

## What does not change

- `api/operations.yaml`, code generation, tool names, input schemas, location
  metadata, and consumer business dispatch do not gain transport-specific
  inference or parallel definitions.
- This change does not add an MCP client library, authentication policy,
  subscription/SSE delivery, multi-round-trip requests, legacy
  `initialize`/HTTP+SSE compatibility, sessions, GET/DELETE endpoint support,
  or AnkiMCP-specific code.
- `x-mcp-header` annotations are not silently accepted: this first runtime
  rejects a manifest containing one until Hydra can generate and validate that
  transport-specific contract safely.

## Impact

- New OpenSpec change: `add-mcp-streamable-http`.
- Planned implementation areas: workspace manifests, a new
  `crates/hydra-mcp-http/` runtime, Notes reference wiring and an MCP-over-HTTP
  binary/test, README guidance, and focused runtime integration tests.
- This PR contains specification artifacts only. A separately tracked
  implementation slice stays blocked until this contract is reviewed and
  merged onto `main`.
