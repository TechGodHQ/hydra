# Security-scan adapter boundary requirements

## Requirement: Explicit security-scan operation declaration

A consumer SHALL declare its security-scan operation in `api/operations.yaml`, including method, path, read/write semantics, parameter locations, output type, and the explicit `surfaces: [cli, http, mcp]` allowlist.

#### Scenario: Generated surfaces share one declared operation

- **WHEN** the consumer generates artifacts from the security-scan definition
- **THEN** CLI, HTTP, and MCP expose the declared operation without name-based inference
- **AND** all surfaces route to the same consumer dispatch implementation

## Requirement: Consumer-owned scanner adapter

The consumer SHALL own scanner selection and invocation behind a typed adapter boundary. Hydra core SHALL NOT depend on a scanner SDK, select a scanner from an operation name, or execute caller-provided shell commands. The tracer bullet SHALL accept exactly `target = "fixture:demo-repo"` and `profile = "baseline"` (with omitted profile defaulting to `baseline`); all other target/profile values, empty values, control characters, and targets longer than 1024 bytes SHALL be rejected before adapter invocation.

#### Scenario: Fixture scan dispatch

- **WHEN** a valid tracer-bullet request reaches the generated surface
- **THEN** the consumer dispatch invokes its configured `SecurityScanner` adapter with a typed request
- **AND** the result is returned through the generated surface contract

## Requirement: Explicit secret configuration and redaction

A real scanner adapter SHALL accept configuration only through `DEEPSEC_ENDPOINT` (required absolute HTTPS URL) and `DEEPSEC_TOKEN` (required non-empty credential). Source, committed examples, generated artifacts, test fixtures, and user-visible results SHALL NOT contain their values. Adapters SHALL return private internal errors; the consumer SHALL map these to an allowlisted public error code (`invalid_request`, `scanner_unavailable`, or `scan_failed`) with fixed safe text before every CLI, HTTP, MCP, and logging boundary.

#### Scenario: Scanner credentials are absent from an operation result

- **WHEN** an adapter reports a scanner error or finding
- **THEN** the response and logs contain only allowlisted public fields or fixed safe error text
- **AND** never contain authorization headers, token values, endpoint values, raw configuration values, or raw adapter errors

## Requirement: Invalid consumer input does not invoke the scanner

The consumer SHALL validate its target/profile policy before calling the scanner adapter.

#### Scenario: Invalid target rejected before dispatch

- **WHEN** a request supplies an invalid tracer-bullet target or profile
- **THEN** the consumer returns a validation error
- **AND** the configured scanner adapter is not invoked

## Requirement: Generated artifacts remain deterministic

The security-scan example SHALL commit generated artifacts and pass its codegen freshness check. Its deterministic fixture result SHALL be exactly `status=completed`, `target=fixture:demo-repo`, `profile=baseline`, summary counts all zero, and an empty findings list. Future findings SHALL sort by `(severity, rule_id)` and expose only bounded safe summary fields.

#### Scenario: Repeated generation is stable

- **WHEN** the generator writes artifacts twice from unchanged security-scan inputs
- **THEN** the resulting committed artifacts are byte-identical
- **AND** `hydra check` succeeds
