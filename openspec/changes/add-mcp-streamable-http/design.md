# Design: stateless MCP Streamable HTTP transport

## Context

Hydra's reusable MCP runtime currently speaks newline-delimited JSON-RPC on
stdio at protocol version `2024-11-05`. It accepts generated tool definitions
and one consumer-owned dispatch closure; consumer code maps declared argument
locations into its existing operation input. That boundary is correct and must
remain shared by stdio and HTTP.

The current MCP specification revision is
[`2026-07-28` Streamable HTTP](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http): one POST endpoint, no
protocol-level sessions, no GET stream, and request metadata mirrored in
headers. `server/discover` is mandatory in this revision. The transport can
return one JSON object for each unary request, so Hydra does not need SSE to
provide the initial tool-only surface.

## Decision

Add a small `hydra-mcp-http` crate. It owns only HTTP/JSON-RPC transport,
metadata validation, and conversion to/from MCP envelopes. It receives generated
`mcp.json` tools and a single async `(tool_name, arguments) -> Result<Value,
String>` closure; it never knows an operation name's semantics or maps generated
locations itself.

The public construction API will be deliberately explicit:

```rust
pub const PROTOCOL_VERSION: &str = "2026-07-28";

pub fn router<F, Fut>(
    config: ServerConfig,
    dispatch: F,
) -> Result<axum::Router, ConfigurationError>
where
    F: Fn(String, serde_json::Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<serde_json::Value, String>> + Send + 'static;
```

`ServerConfig` carries the server name/version, generated manifest value, and
an explicit origin policy. The runtime does not choose an endpoint path; a
consumer nests the returned router at a declared path such as `/mcp`. This
avoids host, path, or operation-name inference and allows a desktop host to own
its listener and lifecycle.

### Origin policy

`OriginPolicy` is secure by default:

- absent `Origin` is allowed for ordinary non-browser MCP clients;
- `RejectPresented` is the default and rejects every present Origin;
- `AllowExact` accepts only a caller-provided finite list of exact origins;
- wildcard, suffix, reflected-origin, and `AllowAny` modes are not provided.

The runtime checks Origin before decoding or dispatching the body and returns
HTTP 403 for an invalid presented Origin. A consumer exposing a browser-facing
endpoint must configure its literal allowed origin(s); the runtime never guesses
from a host name, request path, or client identity.

This first runtime is not a CORS or Private Network Access (PNA) service. It
does not emit `Access-Control-Allow-*` or
`Access-Control-Allow-Private-Network` response headers, and its POST-only
router does not service `OPTIONS` preflights. `AllowExact` is an inbound Origin
validator, not browser authorization. A consumer that deliberately makes the
endpoint browser-accessible must add a separately reviewed host-owned CORS/PNA
layer that preserves an explicit finite Origin policy; that integration is out
of scope for this transport slice.

### Manifest construction validation

The runtime accepts the same bare tools array or `{ "tools": [...] }` wrapper
as the stdio runtime. Construction validates that tools are an array of objects
with unique string names and object `inputSchema` values. It preserves the
validated tool objects and their deterministic generated order verbatim for
`tools/list`.

The first HTTP runtime rejects construction if any tool input schema contains
`x-mcp-header`. The current MCP transport requires servers to validate declared
`Mcp-Param-*` mirrors, while Hydra has no generated model/validation for those
annotations yet. Failing closed prevents a server from advertising a schema it
cannot safely validate; a future capability must extend the generated contract
and runtime together before accepting the extension.

### Endpoint and request validation

The router exposes POST only. Axum's method handling returns 405 for GET,
DELETE, and OPTIONS; it never creates, echoes, or depends on
`Mcp-Session-Id`, and ignores legacy `Last-Event-ID` rather than implying
resumption.

For every POST, in this order:

1. validate Origin as above;
2. require `Content-Type: application/json` (parameters allowed) and an
   `Accept` value that explicitly includes both `application/json` and
   `text/event-stream`; reject unsupported content with 415 or 406 before
   dispatch;
3. decode exactly one JSON object representing a JSON-RPC 2.0 request, never a
   batch or JSON-RPC response;
4. require `params._meta` with
   `io.modelcontextprotocol/protocolVersion` and
   `io.modelcontextprotocol/clientCapabilities`, then require the
   `MCP-Protocol-Version` header to match the body version;
5. support only `2026-07-28`. A matching unsupported version returns HTTP 400
   with JSON-RPC `UnsupportedProtocolVersionError` (`-32022`) and the one
   supported version. A missing or mismatched metadata header returns HTTP 400
   with `HeaderMismatch` (`-32020`);
6. require `Mcp-Method` to exactly equal JSON-RPC `method`, and for
   `tools/call`, require `Mcp-Name` to equal `params.name`. `Mcp-Name` supports
   the specification's Base64 sentinel decoding and rejects malformed/control
   values before comparison.

The implementation may always return `application/json` for these unary
methods; it does not advertise or implement response-stream subscriptions,
progress, MRTR, or resumable SSE. Requests without an id are not a supported
core interaction in this slice and receive a JSON-RPC invalid-request error
rather than a misleading 202 acceptance.

### Supported operations

- `server/discover` returns `resultType: "complete"`,
  `supportedVersions: ["2026-07-28"]`, `capabilities: { "tools": {} }`, and
  server name/version at
  `_meta["io.modelcontextprotocol/serverInfo"]`.
- `tools/list` returns `resultType: "complete"` and the exact generated tools
  in deterministic manifest order. It does not mutate or filter them by
  connection state.
- `tools/call` requires a declared exact tool name and object arguments
  (omission means an empty object). It invokes the caller's one dispatch
  closure. A successful `Value` becomes both `structuredContent` and the
  backwards-compatible JSON text content. A closure error becomes a
  `resultType: "complete"` tool result with `isError: true`; callers remain
  responsible for returning safe public error text.
- Other methods, including modernly framed `initialize`, return HTTP 404 with
  JSON-RPC `-32601`. The error names the supported modern version so a legacy
  client has an actionable diagnostic; the runtime does not downgrade into
  legacy session behavior.

Malformed envelope/parameters use JSON-RPC invalid-request or invalid-params
errors with HTTP 400. Unknown methods use HTTP 404 with `-32601`. All
transport-generated errors are static, value-free messages; raw headers,
origins, and bodies are not reflected.

### Consumer integration

`examples/notes` becomes the reference public boundary. Its existing argument
location mapping will be factored into a shared MCP dispatch adapter so stdio
and HTTP call exactly the same consumer operation function. A
`notes-mcp-http` binary will bind only `127.0.0.1` by default and mount the
runtime explicitly at `/mcp`; the generic crate itself never binds a listener.
No AnkiMCP source or dependency enters Hydra.

## Verification design

Runtime tests cover construction validation, Origin rejection, media headers,
JSON-RPC envelope validation, metadata/header mismatch, unsupported version,
legacy-method rejection, method/name matching, tool execution result/error
shapes, POST-only behavior, and the absence of session state.

The Notes integration test starts a real loopback Axum listener and uses a
small independent MCP HTTP client fixture to send the full modern metadata and
headers through `server/discover`, `tools/list`, and a declared `tools/call`.
It asserts that generated tool objects are unchanged, the call reaches the
same Notes dispatch implementation, and no session header is minted. This is
agent/consumer evidence, not merely a router unit test.

The full Hydra gate remains build, tests, strict Clippy, fmt, both reference
example codegen checks, Creed diff, and `git diff --check`.

## Alternatives rejected

### Reuse the stdio protocol over HTTP

Rejected: its `initialize` handshake and `2024-11-05` behavior are a different
protocol era. Wrapping it in Axum would falsely promise current Streamable HTTP
conformance.

### Add an AnkiMCP-specific HTTP server

Rejected: transport ownership belongs to Hydra and must remain reusable by any
embedded Rust host.

### Accept `x-mcp-header` as inert schema data

Rejected: current Streamable HTTP requires validation of those body/header
mirrors. Rejecting unsupported annotations is safer than silently advertising
a contract the server cannot enforce.

### Introduce sessions or a GET SSE endpoint for future flexibility

Rejected: `2026-07-28` deliberately removed both. The first transport is
stateless and unary; a later, separately specified capability can add a
request-scoped SSE response only if a Hydra-owned operation needs it.
