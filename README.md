# hydra

One API definition, many surfaces. Hydra projects a single, explicit
`api/operations.yaml` onto three committed surfaces — CLI (clap), HTTP
(axum), and MCP (tool schemas + stdio runtime) — so the same Rust operation
implementation powers every interface without drift.

Extracted from [iris](https://github.com/TechGodHQ/iris)'s `iris-codegen`,
generalized for reuse across TechGodHQ Rust projects. Born from a spike that
evaluated (and rejected) macro-based inference layers; see iris's
`docs/spikes/server-less.md` for the rationale. Hydra's rule: **no name-based
inference — everything is declared**.

```
               api/operations.yaml          (single source of truth)
                        |
              cargo run -p hydra-codegen -- write
                        |
        +---------------+----------------+
        |               |                |
   generated/cli.rs  generated/http.rs  generated/mcp.json
        |               |                |
   clap structs     axum routes      tool schemas
        |               |                |
        +---------------+----------------+
                        |
            your operation dispatch (one function)
```

## Crates

- `hydra-core` — the API definition model (operations, parameters,
  locations, surface allowlists) and validation. No generation, no I/O.
- `hydra-codegen` — the generator + `hydra` CLI (`write` / `check`).
  Per-project knobs live in `hydra.yaml`.
- `hydra-mcp-stdio` — a minimal reusable MCP stdio server (JSON-RPC 2.0,
  newline-delimited) you hand your generated tool schemas and one dispatch
  closure.
- `examples/notes` — a complete three-surface project. Copy it as a template.

## Usage

1. Describe operations in `api/operations.yaml`:

```yaml
operations:
  - name: get_note
    description: Get a single note by ID.
    method: GET
    path: /notes/{note_id}
    read: true
    output_type: Note
    parameters:
      - name: note_id
        description: Note ID to fetch.
        type: string
        required: true
        location: path
```

2. Add a `hydra.yaml` pointing generated handlers at your dispatch and state:

```yaml
http_dispatch_fn: "crate::execute_operation_http"
http_state_type: "crate::AppState"
```

3. Generate and commit:

```bash
cargo run -p hydra-codegen -- write   # writes generated/{cli.rs,http.rs,mcp.json}
cargo run -p hydra-codegen -- check   # CI guard: fails if artifacts are stale
```

4. `include!` the generated files, implement one dispatch function, and wire
   your binaries. See `examples/notes/src/lib.rs`.

## Design rules

- **No inference.** Method, path, parameter locations, and surface
  allowlists are declared. The generated router, schemas, and docs cannot
  disagree because none of them guess.
- **Validation is the product.** `hydra-core` rejects path placeholders
  without parameters, duplicate names, read/POST mismatches, empty or
  duplicate surface lists, and reserved identifiers at generation time —
  not at runtime.
- **Generated code is committed** and deterministic; `check` gates CI.
- **One dispatch function** per project routes every surface to the same
  operation implementation. Business logic never duplicates per surface.
- **Selective projection** via `surfaces: [http, mcp]` allowlists, with CLI
  command renames (`cli_command:`) when the public name should differ.

## License

MIT.
