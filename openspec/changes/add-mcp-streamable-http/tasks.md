# Tasks

## Spec review

- [ ] 1. Review and merge this OpenSpec-only contract before any runtime,
  workspace, example, or generated-artifact implementation begins.

## Implementation (blocked by spec review)

- [ ] 2. Add `hydra-mcp-http` to the workspace with the explicit configuration,
  Origin policy, manifest validation, and Axum router boundary described here.
- [ ] 3. Implement current `2026-07-28` stateless POST handling for
  `server/discover`, `tools/list`, and `tools/call`, including media,
  JSON-RPC, metadata, protocol-version, method/name, and safe-error handling.
- [ ] 4. Add focused runtime tests for protocol/security failures, POST-only
  and no-implicit-CORS/PNA-preflight behavior, statelessness, tool result/error
  envelopes, and unsupported `x-mcp-header` annotations.
- [ ] 5. Wire the Notes reference consumer through the same MCP dispatch
  adapter as stdio; add `notes-mcp-http` and a real loopback modern-client
  acceptance test for discover → list → call.
- [ ] 6. Update README and committed guidance with explicit embedding,
  localhost, Origin-policy, protocol-version, and intentionally unsupported
  capability boundaries.
- [ ] 7. Run the complete Hydra gates: build, tests, strict Clippy, fmt, both
  example codegen checks, Creed diff, and `git diff --check`.
