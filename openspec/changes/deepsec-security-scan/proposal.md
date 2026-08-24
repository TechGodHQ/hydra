# DeepSec security-scan adapter boundary

## Why

Hydra can project an explicitly declared operation onto CLI, HTTP, and MCP, but it has no reference for an agent-powered security scan. Consumers need a small, safe pattern for exposing a scanner without teaching Hydra scanner-specific semantics or inferring behavior from an operation name.

## What changes

- Define a provider-neutral security-scan tracer bullet as a Hydra consumer example.
- Declare a `run_security_scan` operation explicitly, including its input/output contract and surface locations.
- Define a narrow adapter boundary: generated surfaces decode and validate the declared request, then one consumer-owned dispatch implementation invokes the configured scanner adapter.
- Define environment-only configuration and redacted result rules for scanner credentials and findings.
- Add a runnable example, generated artifacts, deterministic codegen coverage, and documentation after this spec is approved.

## What does not change

- Hydra core will not embed DeepSec, vendor a scanner SDK, manage credentials, execute arbitrary shell commands, or infer a scanner from an operation name.
- This change does not create a general plugin system or a production remote-scanner protocol.
- The operation remains a consumer example/tracer bullet; a scanner is selected by explicit consumer configuration.

## Impact

- New OpenSpec change: `deepsec-security-scan`.
- Planned implementation areas: `examples/security-scan/`, workspace membership, generated artifacts, README documentation, and focused validation/codegen tests.
- Consumers gain an auditable template for a security-scanning operation whose three surfaces share one dispatch implementation.
