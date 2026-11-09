# Tasks

## Spec review

- [x] 1. Review and merge this OpenSpec-only contract before any runtime,
  workspace, example, or generated-artifact implementation begins.

## Implementation (blocked by spec review)

- [x] 2. Add `hydra-mcp-http` to the workspace with the explicit configuration,
  Origin policy, manifest validation, and Axum router boundary described here.
- [x] 3. Implement current `2026-07-28` stateless POST handling for
  `server/discover`, `tools/list`, and `tools/call`, including media,
  JSON-RPC, metadata, protocol-version, method/name, and safe-error handling.
- [x] 4. Add focused runtime tests for protocol/security failures, POST-only
  and no-implicit-CORS/PNA-preflight behavior, statelessness, tool result/error
  envelopes, and unsupported `x-mcp-header` annotations.
- [x] 5. Wire the Notes reference consumer through the same MCP dispatch
  adapter as stdio; add `notes-mcp-http` and a real loopback modern-client
  acceptance test for discover → list → call.
- [x] 6. Update README and committed guidance with explicit embedding,
  localhost, Origin-policy, protocol-version, and intentionally unsupported
  capability boundaries.
- [x] 7. Run the complete Hydra gates: build, tests, strict Clippy, fmt, both
  example codegen checks, Creed diff, and `git diff --check`.

## Verification evidence

- `cargo build --locked --all-targets` passed.
- `cargo test --locked --all-targets` passed: 60 codegen tests, 3 runtime unit
  tests, 7 runtime transport tests, 1 real Notes loopback test, 14 Notes
  surface tests, and 6 security-scan tests.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`, `creed diff`, and `git diff --check` passed.
- `cargo run --locked -p hydra-codegen --bin hydra-codegen -- check` passed in
  `examples/notes`, `examples/security-scan`, and `examples/ts-client`.
  Two `write` passes in each example were byte-identical to the committed
  four-artifact snapshots and to each other; the final `check` passed.
- In `examples/ts-client`, `npm ci --ignore-scripts --no-audit --no-fund`,
  `npm run check:generated`, `npm run typecheck`, `npm run build`, and
  `npm run test:nested-schema` passed.

## COD-545 rejection-matrix and loopback evidence

The earlier COD-525 verification summary reports test counts, but the count alone
is not evidence that every frozen scenario had an executable named assertion.
This follow-up maps the previously missing negative paths to tests; it does not
change the frozen proposal, design, spec, runtime, generated artifacts, or policy.

- [x] `crates/hydra-mcp-http/tests/transport.rs::named_rejection_matrix_is_static_value_free_and_never_dispatches` — actual router requests for media, malformed JSON/envelopes/batches/notifications, metadata, version/method/name headers (missing, non-text, mismatch), Base64/control names, undeclared tools and invalid arguments; assert status/code/message/ID, redaction, no CORS/session headers and zero dispatches.
- [x] `crates/hydra-mcp-http/tests/transport.rs::call_metadata_and_mirrored_headers_reject_before_dispatch` — six missing/non-text/mismatched protocol/method-header cases and six missing/invalid metadata cases start from a declared, otherwise-valid `tools/call`; assert exact HTTP/RPC/message/ID, redaction, absent CORS/session headers and zero dispatch after a same-router valid call dispatches once. Retain the separate list/envelope rows rather than treating their zero-dispatch result as call-specific evidence.
- [x] `crates/hydra-mcp-http/tests/transport.rs::exact_origin_allowlist_rejection_is_pre_dispatch_and_emits_no_browser_headers` — a valid exact-allowed Origin first dispatches once on the same router/counter (then resets), and its successful response has no CORS/PNA/session grant headers. Realistic OPTIONS with that same allowed Origin/policy returns 405 with no grants or dispatch; the rejected Origin also proves zero dispatch/reflection/CORS/session headers.
- [x] `crates/hydra-mcp-http/tests/transport.rs::construction_rejection_matrix_names_tools_without_reflecting_manifest_values` and `construction_rejects_invalid_exact_origin_entries` — wrapper/array/tool/name/schema shapes, duplicates, nested and array-schema annotations, a `oneOf` annotation exercising JSON-array recursion, and unsafe/duplicate explicit origin entries.
- [x] `crates/hydra-mcp-http/tests/transport.rs::realistic_options_preflight_and_delete_remain_unsupported_without_headers` — realistic CORS/PNA OPTIONS and DELETE return 405 without session/CORS/PNA headers.
- [x] `examples/notes/tests/mcp_http.rs::independent_loopback_client_discovers_lists_and_calls_notes` — bounded independent real-TCP client (five-second per-request timeout and 1 MiB response cap), exact discovery tools capability, full positive response envelopes, plus rejected Origin, missing/mismatched `Mcp-Name`, matching mirrored undeclared tool, and exact synthetic note-stats before/after equality.
- [x] README explains the transcript-producing test command, sample host start/stop and host-owned shutdown, and caller responsibility for value-free tool-error text. The PR retains fresh actual command output and the exact tested head SHA; no historical transcript is reconstructed.

### Frozen scenario index

All transport names below refer to `crates/hydra-mcp-http/tests/transport.rs`;
Notes acceptance refers to `examples/notes/tests/mcp_http.rs`.

| Frozen spec scenario | Executable coverage |
| --- | --- |
| embedded host mounts explicit endpoint | Notes `independent_loopback_client_discovers_lists_and_calls_notes` starts the host-owned listener and calls the nested `/mcp` endpoint, then shuts down the host |
| generated tool schemas stay transport-identical | `discover_list_and_call_use_the_real_router_boundary` plus Notes acceptance compares all tools and ordering to `GENERATED_MCP_JSON` |
| valid modern discovery without a session | `discover_list_and_call_use_the_real_router_boundary` plus Notes acceptance, including `capabilities.tools == {}` |
| mismatched mirrored header | `call_metadata_and_mirrored_headers_reject_before_dispatch` proves protocol/method rejection on dispatchable calls; `named_rejection_matrix_is_static_value_free_and_never_dispatches` retains list/name coverage including malformed/unterminated/control/non-UTF8 Base64 name representations |
| unsupported version | `unsupported_version_and_legacy_methods_are_truthful` plus named matrix, including exact supported-version error data |
| browser Origin not silently trusted | named matrix (default reject-presented, before decoding) and `exact_origin_allowlist_rejection_is_pre_dispatch_and_emits_no_browser_headers` |
| Origin validation does not grant browser access | `exact_origin_allowlist_rejection_is_pre_dispatch_and_emits_no_browser_headers` checks successful allowed-Origin and same-policy OPTIONS response headers; `realistic_options_preflight_and_delete_remain_unsupported_without_headers` retains default-policy OPTIONS/DELETE coverage |
| legacy transport methods unsupported | `unsupported_version_and_legacy_methods_are_truthful` (GET, OPTIONS, and initialize with exact `-32601` method-not-found message naming `PROTOCOL_VERSION`) and realistic OPTIONS/DELETE test |
| valid generated tool reaches consumer | `discover_list_and_call_use_the_real_router_boundary`, `omitted_tool_arguments_dispatch_as_an_empty_object`, `sentinel_encoded_tool_names_are_compared_after_decoding`, and Notes acceptance |
| undeclared tool cannot dispatch | named matrix and Notes rejected-call sequence with fixture-state before/after equality |
| unsupported parameter-header annotation | `construction_rejects_duplicate_tools_and_header_annotations` and construction matrix, including nested array schemas |

Additional rejection families not individually titled in the frozen spec are
mapped by the named matrix: HTTP 415/406; malformed JSON, response/batch/envelope,
missing/invalid ID or method, missing/non-object params/metadata; missing/wrong
protocol or capabilities; missing/non-string/empty/control/unknown tool names;
non-object arguments. Each row checks exact HTTP/RPC code, safe static message,
invalid-ID null handling where applicable, no CORS/PNA/session headers, and no
consumer dispatch. The instrumented router first dispatches a valid control call
exactly once. The dedicated call-metadata/header matrix supplies otherwise-valid
dispatchable call baselines; list-only negatives do not establish that property.
Construction tests
cover wrapper, array, tool, name and schema shapes, duplicates and explicit Origin
configuration. `dispatch_errors_are_tool_results_and_invalid_ids_are_null` retains
safe execution-error (`isError`) and structured/text result behavior.

### Fresh recovery verification

Run on 2026-11-06 as the hosted coding service UID against base
`303ba7af54b2c54752c59c1e3a60828290f79e75` plus this tests/docs-only change:

- Locked all-target build and tests passed: 60 codegen, 3 runtime unit,
  13 transport, 1 real TCP Notes, 14 Notes surface, and 6 security-scan tests.
- Both targeted MCP HTTP and Notes acceptance commands passed; Notes was run
  with `-- --nocapture` to retain actual synthetic positive/negative responses.
- Strict workspace/all-target Clippy, formatting, Creed diff and Git diff checks
  passed.
- All three examples passed check, write, write, check; all four artifacts were
  byte-identical to their committed snapshots and to each other after each write.
- TypeScript fixture `npm ci --ignore-scripts --no-audit --no-fund`, generated
  freshness, typecheck, build and nested-schema tests passed.

No production runtime, frozen proposal/design/spec, generated artifact, dependency,
release, consumer or security-policy changes are part of this recovery.
