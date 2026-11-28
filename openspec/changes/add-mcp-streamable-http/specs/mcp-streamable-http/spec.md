# MCP Streamable HTTP transport specification

## ADDED Requirements

### Requirement: Hydra exposes a stateless current-protocol MCP HTTP runtime

Hydra SHALL provide an Axum-compatible reusable MCP server runtime for protocol
version `2026-07-28`. The runtime SHALL be constructible from generated MCP
tool objects, explicit server identity, explicit Origin policy, and one
consumer-owned asynchronous tool dispatch closure. It SHALL not infer endpoint
paths, operation behavior, headers, or origins from tool names or request data.

#### Scenario: an embedded host mounts an explicit endpoint

- **WHEN** a consumer constructs the runtime with generated tools and nests its
  router at an explicit `/mcp` path in a host-owned Axum application
- **THEN** the runtime SHALL not bind a listener or own host lifecycle
- **AND** the consumer's one supplied dispatch closure SHALL remain the only
  business-operation path for MCP tool calls.

#### Scenario: generated tool schemas stay transport-identical

- **WHEN** a modern client sends a valid `tools/list` request
- **THEN** the response SHALL contain the same generated tool objects in their
  deterministic manifest order
- **AND** the runtime SHALL not reconstruct schemas or infer parameter
  locations.

### Requirement: the HTTP endpoint validates current Streamable HTTP requests

The runtime SHALL accept only POST requests carrying one JSON-RPC 2.0 request.
It SHALL validate a present Origin before request decoding, require JSON content
and both JSON/SSE response media types, and validate per-request protocol
metadata. It SHALL support only `2026-07-28` and SHALL require
`MCP-Protocol-Version`, `Mcp-Method`, and, for `tools/call`, `Mcp-Name` headers
to agree with their request-body values.

#### Scenario: valid modern discovery succeeds without a session

- **WHEN** a client POSTs `server/discover` with matching `2026-07-28` body
  metadata and required HTTP headers
- **THEN** the server SHALL return a JSON-RPC complete result naming
  `2026-07-28`, the tools capability, and configured server identity
- **AND** the response SHALL not mint or echo a protocol session identifier.

#### Scenario: a mismatched mirrored header is rejected

- **WHEN** a request's `MCP-Protocol-Version`, `Mcp-Method`, or `Mcp-Name`
  header is missing, malformed, or differs from the body value
- **THEN** the server SHALL return HTTP 400 and JSON-RPC error code `-32020`
- **AND** the error SHALL not reflect raw request header or body values.

#### Scenario: an unsupported version is rejected truthfully

- **WHEN** matching body and header metadata request a protocol version other
  than `2026-07-28`
- **THEN** the server SHALL return HTTP 400 and error code `-32022`
- **AND** the error data SHALL list only the supported version.

#### Scenario: a browser Origin is not silently trusted

- **WHEN** an Origin is presented and is not in the consumer's exact static
  allowlist (including the default reject-presented policy)
- **THEN** the server SHALL return HTTP 403 before dispatch
- **AND** it SHALL not reflect or dynamically allow that Origin.

#### Scenario: Origin validation does not accidentally grant browser access

- **WHEN** a request attempts CORS or Private Network Access preflight through
  `OPTIONS`
- **THEN** this first runtime SHALL return HTTP 405 without emitting
  `Access-Control-Allow-*` or `Access-Control-Allow-Private-Network` headers
- **AND** an exact Origin allowlist SHALL remain inbound request validation,
  not an implicit browser-authorization policy.

### Requirement: the runtime remains stateless and does not emulate legacy transport

The runtime SHALL not implement protocol-level initialization, sessions,
standalone GET streams, DELETE session termination, or resumption through
`Last-Event-ID`. It MAY return a single JSON object for all supported unary
requests.

#### Scenario: legacy transport methods do not gain accidental support

- **WHEN** a caller sends GET, DELETE, or OPTIONS to the mounted MCP endpoint
- **THEN** the endpoint SHALL return HTTP 405
- **WHEN** a modernly framed `initialize` request is sent
- **THEN** the endpoint SHALL return HTTP 404 with JSON-RPC method-not-found
  information that identifies the supported modern protocol version.

### Requirement: tool calls preserve the one dispatch boundary

The runtime SHALL serve `tools/call` only for an exactly declared generated
tool name. It SHALL pass the tool name and object arguments to the supplied
consumer closure. Successful results SHALL expose both structured content and a
backwards-compatible text representation; consumer-declared execution failures
SHALL be MCP tool results with `isError: true` rather than transport success
claims.

#### Scenario: a valid generated tool call reaches the consumer

- **WHEN** a valid request calls a declared tool with object arguments
- **THEN** the runtime SHALL invoke the one consumer dispatch closure once
- **AND** return its JSON value as structured content and JSON text content.

#### Scenario: an undeclared tool cannot dispatch

- **WHEN** a client names a tool absent from the generated manifest
- **THEN** the runtime SHALL return an invalid-params JSON-RPC error
- **AND** it SHALL not call the consumer closure.

### Requirement: unsupported header annotations fail closed

The first runtime SHALL reject configuration containing an `x-mcp-header`
annotation anywhere in a tool input schema until Hydra generates and validates
that declared mirror contract.

#### Scenario: a manifest asks for an unsupported parameter header

- **WHEN** a generated/constructed manifest includes an `x-mcp-header`
  annotation
- **THEN** runtime construction SHALL fail before an endpoint is exposed
- **AND** it SHALL name the affected declared tool without accepting traffic.
