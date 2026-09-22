# TypeScript client surface example

This example is a small consumer boundary for Iris. `api/operations.yaml` is
checked in from Iris's current operation definition so the generator can be
run and typechecked without making the Hydra repository depend on the Iris
checkout at build time. The follow-up Iris ticket wires the same generated
surface into `TechGodHQ/iris` and publishes the SDK.

## Generate and typecheck

```bash
npm install
npm run generate
npm run check:generated
npm run typecheck
```

The generated module is `generated/ts-client/index.ts`. It has no runtime
package dependencies and uses the platform `fetch` API. `IrisClient` accepts an
absolute `baseUrl`, an optional deployment bearer token, and an injectable
`fetch` implementation for tests. `listThreads` demonstrates optional query
pagination; path parameters are URL-encoded, body parameters are JSON encoded,
and non-2xx responses throw the generated `ApiError` with status and parsed
body.

The fixture intentionally includes Iris's SSE operation, but its explicit
`surfaces: [http, cli]` allowlist keeps it out of this unary client. Streaming
TypeScript support needs its own contract rather than silently pretending an
SSE response is JSON.
