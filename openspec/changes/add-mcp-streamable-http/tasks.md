# Tasks

## Spec review

- [x] 1. Review and merge this OpenSpec-only contract before any runtime,
  workspace, example, or generated-artifact implementation begins.

## Implementation (blocked by spec review)

- [x] 2. Add `hydra-mcp-http` to the workspace with the explicit configuration,
  Origin policy, manifest validation, and Axum router boundary described here.
- [x] 3. Implement current `2026-07-28` stateless POST handling for
  `server/discover`, `tools/list`, and `tools/call`, including media,
  JSON-RPC, metadata, protocol-version, method/name, and safe-error handling.
- [x] 4. Add focused runtime tests for protocol/security failures, POST-only
  and no-implicit-CORS/PNA-preflight behavior, statelessness, tool result/error
  envelopes, and unsupported `x-mcp-header` annotations.
- [x] 5. Wire the Notes reference consumer through the same MCP dispatch
  adapter as stdio; add `notes-mcp-http` and a real loopback modern-client
  acceptance test for discover → list → call.
- [x] 6. Update README and committed guidance with explicit embedding,
  localhost, Origin-policy, protocol-version, and intentionally unsupported
  capability boundaries.
- [x] 7. Run the complete Hydra gates: build, tests, strict Clippy, fmt, both
  example codegen checks, Creed diff, and `git diff --check`.

## Verification evidence

- `cargo build --locked --all-targets` passed.
- `cargo test --locked --all-targets` passed: 60 codegen tests, 3 runtime unit
  tests, 7 runtime transport tests, 1 real Notes loopback test, 14 Notes
  surface tests, and 6 security-scan tests.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`, `creed diff`, and `git diff --check` passed.
- `cargo run --locked -p hydra-codegen --bin hydra-codegen -- check` passed in
  `examples/notes`, `examples/security-scan`, and `examples/ts-client`.
  Two `write` passes in each example were byte-identical to the committed
  four-artifact snapshots and to each other; the final `check` passed.
- In `examples/ts-client`, `npm ci --ignore-scripts --no-audit --no-fund`,
  `npm run check:generated`, `npm run typecheck`, `npm run build`, and
  `npm run test:nested-schema` passed.
