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

## Commands

Run before handing work back:

```bash
cargo build --all-targets
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
cd examples/notes && cargo run -p hydra-codegen --bin hydra-codegen -- check
```

After changing the generator or any `operations.yaml`, regenerate and commit:

```bash
cd examples/notes && cargo run -p hydra-codegen --bin hydra-codegen -- write
```

Note: `cargo fmt` formats include!'d generated files — if fmt touches
`examples/notes/generated/`, re-run `write` and commit the regenerated
versions instead.

## Style

- All public items need doc comments.
- Tests cover real behavior (generation output, validation rejections,
  live surface equivalence), not compilation only.
- Keep hydra-core free of generation concerns; keep hydra-codegen free of
  runtime concerns.

## Git / PR Rules

- Shiv's global git identity. Runner commits may add
  `Co-authored-by: Archon <archon@purelymail.com>`.
- No auto-merge; human review required.
- Conventional commits: `feat:`, `fix:`, `refactor:`, `docs:`, `chore:`.
