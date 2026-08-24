# Tasks

## Spec review

- [ ] 1. Review and merge this OpenSpec-only PR before implementation.

## Implementation (blocked by spec review)

- [ ] 2. Add `examples/security-scan` as a workspace consumer with explicit `run_security_scan` operation and `hydra.yaml`.
- [ ] 3. Implement typed request/result models, the `SecurityScanner` adapter trait, and a deterministic fixture adapter behind one dispatch function.
- [ ] 4. Generate and commit CLI, HTTP, and MCP artifacts; add deterministic `hydra check` coverage.
- [ ] 5. Add dispatch and validation tests proving invalid inputs never reach the adapter and output excludes secret-bearing fields.
- [ ] 6. Document the adapter boundary, explicit environment-only DeepSec configuration, and the fact that Hydra never executes caller-provided commands.
- [ ] 7. Run the full Hydra gate: build, tests, clippy, fmt, and examples/notes codegen check; also run the security-scan codegen check.
