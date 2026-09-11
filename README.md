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
# Only needed if the definition has raw_request operations:
# http_raw_dispatch_fn: "crate::execute_operation_raw_http"
```

3. Generate and commit:

```bash
cargo run -p hydra-codegen -- write   # writes generated/{cli.rs,http.rs,mcp.json}
cargo run -p hydra-codegen -- check   # CI guard: fails if artifacts are stale
```

4. `include!` the generated files, implement one dispatch function, and wire
   your binaries. See `examples/notes/src/lib.rs`.

### JSON batch operations

Hydra v0.2.0 can project a declared `json` body parameter to HTTP, MCP, and
CLI. Declare the JSON Schema explicitly; Hydra embeds it in the MCP input
schema and generates the HTTP route from the same operation. For a batch that
needs a shell-friendly CLI representation, declare the representation rather
than inferring one:

```yaml
- name: ingest_batch
  description: Apply an ordered, replayable source batch.
  method: POST
  path: /ingest/batches
  read: false
  output_type: IngestReceipt
  parameters:
    - name: replay_key
      description: Stable idempotency key.
      type: string
      required: true
      location: body
    - name: events
      description: Ordered source events.
      type: json
      required: true
      location: body
      schema:
        type: array
        minItems: 1
        items: { type: object }
      cli:
        flag: event
        multiple: true
```

HTTP and MCP callers pass `events` as the declared JSON array. The generated
CLI accepts repeated `--event '<json object>'` flags; the consumer's single
dispatch function parses that explicit CLI representation before typed
validation and persistence. Hydra does not own source-specific event models,
batch hashing, idempotency, or transactions. `examples/notes` contains a
tested `ingest_batch` reference operation and is the pattern Iris should use.

### Security-scanner consumer boundary

`examples/security-scan` is a deliberately small, fixture-backed consumer
reference. Its explicit `run_security_scan` operation projects to CLI, HTTP,
and MCP, while the consumer—not Hydra—owns a typed `SecurityScanner` trait and
the single dispatch function. The checked-in fixture only accepts
`fixture:demo-repo` and the optional `baseline` profile. It never treats input
as a command, filesystem path, or URL.

The fixture needs no configuration. A future live adapter must read only
`DEEPSEC_ENDPOINT` (an absolute HTTPS URL) and `DEEPSEC_TOKEN` (a non-empty
credential) from its environment. Neither belongs in source, generated output,
logs, requests, or public errors. Consumer adapters must map private failures
to the fixed public `invalid_request`, `scanner_unavailable`, or `scan_failed`
error codes without serializing vendor details, credentials, headers, endpoints,
or raw scanner output.

## Raw-request (webhook) operations

Operations that must see the exact wire representation — signature-verified
webhooks, for example — opt in with `raw_request: true`. The generated HTTP
handler receives the raw body bytes and a header map instead of typed
extraction, and dispatches to `http_raw_dispatch_fn`:

```yaml
- name: receive_webhook
  description: Receive a signed webhook payload.
  method: POST
  path: /hooks/github
  read: false
  output_type: Value
  parameters: []
  surfaces: [http]
  raw_request: true
```

Rules: `http` must be the only listed surface, the operation is unary (no
SSE), and body-location parameters are rejected — the raw bytes replace
JSON body extraction. The header map lowercases names, drops non-UTF-8
values, and collapses repeated headers to the last value. Definitions
without raw operations generate byte-identical output to before the flag
existed.

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

## Releases & versioning

**Status: v0.2.2 is proposed, not published.** The workspace manifests have been
aligned to `0.2.2` in preparation; no tag, GitHub release, or crates.io
publication exists yet. A separate task will publish it.

- **Historical tags are immutable.** Tags `v0.1.0` … `v0.2.1` were cut from
  commits whose workspace manifests still said `0.1.0`; they are reproducible
  git pins and will never be moved or rewritten. From v0.2.2 onward the
  manifest version will agree with the tag at the moment of tagging.
- **What v0.2.2 covers:** merged PRs [#6](https://github.com/TechGodHQ/hydra/pull/6)
  (DeepSec tracer example), [#7](https://github.com/TechGodHQ/hydra/pull/7)
  (layered agent context), and [#8](https://github.com/TechGodHQ/hydra/pull/8)
  (unary cursor pagination fixture) — release metadata, reference coverage,
  and documentation/context changes. No production changes under `crates/*/src`.
- **Consumers pin git tags, not crates.io.** hydra is not published to
  crates.io. Consumers pin a tag and regenerate:
  `hydra-core = { git = "https://github.com/TechGodHQ/hydra", tag = "v0.2.2" }`
  (same shape for `hydra-codegen`), then re-run
  `cargo run -p hydra-codegen -- write` in their own repo and commit the
  regenerated surfaces. Tag-pin examples become valid **only after the
  v0.2.2 tag is actually published** — until then pin `v0.2.1`.

## License

MIT.
