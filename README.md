# hydra

One API definition, many surfaces. Hydra projects a single, explicit
`api/operations.yaml` onto four committed surfaces — CLI (clap), HTTP
(axum), MCP (tool schemas + stdio runtime), and a fetch-based TypeScript
client — so the same operation contract powers every interface without drift.

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
        +---------------+----------------+----------------------+
        |               |                |                      |
   generated/cli.rs  generated/http.rs  generated/mcp.json  generated/ts-client/index.ts
        |               |                |                      |
   clap structs     axum routes      tool schemas          fetch SDK
        |               |                |                      |
        +---------------+----------------+----------------------+
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
- `examples/notes` — a complete four-surface project. Copy it as a template.
- `examples/ts-client` — a TypeScript-only Iris consumer fixture with
  `tsc --noEmit` coverage.

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
# Exported class name for the generated TypeScript client:
ts_client_name: "NotesClient"
```

3. Generate and commit:

```bash
cargo run -p hydra-codegen -- write   # writes generated/{cli.rs,http.rs,mcp.json,ts-client/index.ts}
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

### Fetch-based TypeScript client

The fourth surface emits `generated/ts-client/index.ts`, a zero-runtime-
dependency SDK that uses the platform `fetch` API. It exports one typed
parameter interface and result alias per unary operation, plus a configured
client class:

```yaml
- name: list_threads
  description: List conversation threads.
  method: GET
  path: /threads
  read: true
  output_type: Vec<Thread>
  surfaces: [ts_client]
  parameters:
    - name: limit
      description: Maximum number of threads.
      type: u32
      required: false
      location: query
```

Parameter `location` is the complete request mapping: path values are
`encodeURIComponent`-escaped, query values are written only when present, and
body values are serialized as JSON using their declared wire keys. JSON
parameters are projected from their explicit JSON Schema (including optional
object fields, arrays, and unions), never from parameter names. `u32` and
other numeric definitions map to TypeScript `number`; this is intentionally
documented as an IEEE-754 precision boundary for `i64`-sized values.

Construct the generated client with `{ baseUrl, token?, fetch? }`. `token`,
when supplied, becomes a bearer `Authorization` header; `fetch` is injectable
for tests and non-browser runtimes. Non-2xx responses throw `ApiError` with
the status and parsed response body. The current output model only carries
named output type strings, not domain schemas, so named result models are
generated as opaque `Record<string, unknown>` aliases for the consumer to
refine. Unary operations with `surfaces: null` include the TypeScript client;
an explicit allowlist can select `[ts_client]` or exclude it. SSE operations
must not list `ts_client` until a streaming TypeScript contract is defined.

Run the checked-in Iris consumer fixture with:

```bash
cd examples/ts-client
npm install
npm run generate
npm run check:generated
npm run typecheck
```

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

**Status: [v0.2.2](https://github.com/TechGodHQ/hydra/releases/tag/v0.2.2) is
published.** Its annotated tag resolves to
`afd8303da577898db06212d87e0fd1d4a32a4d3b`; its workspace manifests are aligned
with `0.2.2`. Hydra is **not** published to crates.io.

- **Historical tags are immutable.** Tags `v0.1.0` … `v0.2.1` were cut from
  commits whose workspace manifests still said `0.1.0`; they are reproducible
  git pins and will never be moved or rewritten. From v0.2.2 onward the
  manifest version will agree with the tag at the moment of tagging.
- **What v0.2.2 covers:** merged PRs [#6](https://github.com/TechGodHQ/hydra/pull/6)
  (DeepSec tracer example), [#7](https://github.com/TechGodHQ/hydra/pull/7)
  (layered agent context), and [#8](https://github.com/TechGodHQ/hydra/pull/8)
  (unary cursor pagination fixture) — release metadata, reference coverage,
  and documentation/context changes. No production changes under `crates/*/src`.
- **Consumers pin git tags, not crates.io.** Consumers may pin the published
  `v0.2.2` tag and regenerate:
  `hydra-core = { git = "https://github.com/TechGodHQ/hydra", tag = "v0.2.2" }`
  (same shape for `hydra-codegen`), then re-run
  `cargo run -p hydra-codegen -- write` in their own repo and commit the
  regenerated surfaces. Consumer repins remain separately authorized and
  tracked in COD-478 (Iris) and COD-479 (Rite).

## License

MIT.
