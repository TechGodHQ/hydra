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

### CLI-only boolean presentation flags

An operation can explicitly declare boolean presentation choices that belong
only to its generated CLI argument struct. They are not parameters: Hydra does
not add them to HTTP input, MCP input, or `GeneratedCommand::parameters_json()`.
The consumer reads the generated boolean and decides how to render its own
output.

```yaml
- name: list_records
  description: List records.
  method: GET
  path: /records
  read: true
  output_type: Vec<Record>
  parameters:
    - name: cursor
      description: Opaque page cursor.
      type: string
      required: false
      location: query
  cli_output_flags:
    - flag: include-cursor
      field: include_cursor
      description: Include the checkpoint beside each displayed record.
```

This emits `pub include_cursor: bool` with an explicit
`#[arg(long = "include-cursor", action = clap::ArgAction::SetTrue)]` attribute:
the field is `false` when omitted and `true` for `--include-cursor`; it does
not accept a value. `flag` is kebab-case without `--`, `field` is a Rust-safe
snake_case field, and both must be unique across parameter fields, effective
long flags, CLI companions, and clap's reserved `--help`. Output flags require
a CLI surface and are intentionally boolean-only in v1—no aliases, defaults,
types, inferred semantics, or runtime actions.

**Rust API compatibility:** adding `cli_output_flags` is source-breaking for
Rust callers that construct `Operation` with a struct literal. Existing YAML
definitions remain compatible through `#[serde(default)]`, but direct literals
must add `cli_output_flags: vec![]`. Account for that public API break when
selecting the release version; do not hide it in a consumer-only dependency
repin.

### Declared typed HTTP errors

An HTTP-generating operation can declare the fixed error responses that its
existing runtime binding may return. Hydra owns the wire representation; the
consumer still owns classification, authentication, and the decision to return
an error rather than continue its operation or open an SSE stream.

```yaml
- name: subscribe_events
  description: Stream normalized events.
  method: GET
  path: /events
  read: true
  output_type: Event
  parameters: []
  delivery: sse
  surfaces: [http]
  http_error_responses:
    - name: invalid_replay_cursor
      status: 400
      fields:
        - name: error
          type: string
          required: true
          const: invalid_replay_cursor
    - name: replay_cursor_expired
      status: 409
      fields:
        - name: error
          type: string
          required: true
          const: replay_cursor_expired
        - name: oldest_cursor
          type: string
          required: false
```

The generated HTTP artifact exposes a public
`subscribe_events_http_errors` module with typed constructors such as
`invalid_replay_cursor()` and
`replay_cursor_expired(oldest_cursor: Option<String>)`. Each returned public
response type implements Axum `IntoResponse`, emits the declared status and an
`application/json` object, and keeps its status/body state private. Constants
are declaration-owned and never constructor arguments; optional dynamic fields
are omitted rather than serialized as `null` when `None`.

V1 is intentionally closed: error names and field names are Rust-safe
`snake_case`; field names are the exact flat JSON keys; `type: string` is the
only supported field type; statuses must be 400 through 599; and `const` is a
string allowed only on required fields. Hydra rejects unknown keys, duplicate
names, invalid statuses, generated-symbol collisions, and declarations on
operations without HTTP generation before code generation. The declaration
does not alter request inputs, CLI arguments, MCP schemas, unary route
dispatch, or the existing SSE binding hook.

**Rust API compatibility:** adding `http_error_responses` is source-breaking
for Rust callers that construct `Operation` with a struct literal. Existing
YAML definitions remain compatible through `#[serde(default)]`, but direct
literals must add `http_error_responses: vec![]`. Account for that public API
break when selecting a future release version; do not hide it in a consumer
dependency repin.

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
