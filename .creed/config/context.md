# Hydra Project Context

Hydra is TechGodHQ's shared projection layer: one explicit API definition
(`api/operations.yaml`) generating CLI (clap), HTTP (axum), and MCP (tool
schemas + stdio runtime) surfaces from a single source of truth. Extracted
from iris's `iris-codegen` after the server-less spike (see iris
`docs/spikes/server-less.md`) rejected inference-based macro layers.

## Non-negotiable design rules

- **No name-based inference.** Method, path, parameter locations, surface
  allowlists are declared in YAML; the router, schemas, and docs cannot
  disagree because none of them guess.
- **Validation is the product.** `hydra-core::validate` rejects ambiguous or
  contradictory definitions at generation time, never runtime.
- **Generated code is committed** and deterministic. `hydra check` gates CI.
- **One dispatch function per project.** All three surfaces funnel through
  the same operation implementation; business logic never duplicates per
  surface.
- **Rust-first, boring code.** String-building codegen, no proc macros.
  Errors must point at real code, never at attributes.

## Crates

- `crates/hydra-core/`: definition model + validation (no I/O).
- `crates/hydra-codegen/`: generator + `hydra` CLI (`write` / `check`);
  per-project knobs in `hydra.yaml`.
- `crates/hydra-mcp-stdio/`: reusable MCP stdio JSON-RPC server runtime.
- `examples/notes/`: complete three-surface reference project; copy as
  template.
