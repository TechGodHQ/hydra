# Development Instructions

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
