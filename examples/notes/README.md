# Hydra Notes Example

This small synthetic consumer demonstrates how one declared operation is projected through Hydra's generated surfaces. Build and regenerate from the Hydra workspace root; the complete workspace gates are in `AGENTS.md`.

## HTTP server-sent events

`api/operations.yaml` declares `subscribe_events` as `GET /events/stream`, `delivery: sse`, and `surfaces: [http]`. Hydra generates the `GENERATED_SSE_ROUTES` metadata and `generated::bind_subscribe_events` hook, but deliberately does not generate a second Axum route or the event producer.

The consumer implements and wires the named hook exactly once:

```rust
pub fn bind_runtime_sse_subscribe_events(
    router: axum::Router<AppState>,
) -> axum::Router<AppState>
```

That function owns the Axum handler and its finite fixture stream. The generated binding is applied to `generated::generated_router()` before state is attached. The stream contains only deterministic synthetic note events; it is not backed by persistent state or a provider. SSE remains HTTP-only and is not added to MCP or the unary dispatcher.

The integration test uses an ephemeral `127.0.0.1` listener and a real TCP HTTP request. It checks the status and `text/event-stream` MIME type, exact event order/payload, the terminal HTTP chunk, connection EOF, a request timeout, and graceful server shutdown:

```sh
cargo run -p hydra-codegen --bin hydra-codegen -- check
cargo test -p notes-example --test sse
```

To regenerate after changing the operation definition:

```sh
cargo run -p hydra-codegen --bin hydra-codegen -- write
```

No MCP streaming, TypeScript streaming, raw-request SSE, deployment, or live event source is implied by this fixture.
