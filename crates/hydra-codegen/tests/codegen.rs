//! Ported from iris-codegen's test suite, extended for hydra's config knob
//! and location metadata.

use std::{collections::BTreeMap, fs, path::PathBuf};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::Query,
    http::{Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use clap::Parser;
use hydra_codegen::{GenerateConfig, generate_all, write_generated};
use hydra_core::{ApiDefinition, Delivery, HttpMethod, Operation, Parameter, ParameterLocation};
use pretty_assertions::assert_eq;
use tower::ServiceExt;

#[allow(dead_code)]
mod http_error_responses_fixture {
    include!("fixtures/http-error-responses/http.rs");
}

#[derive(Clone, Default)]
struct HttpErrorFixtureState;

#[allow(clippy::unused_async)]
async fn http_error_fixture_dispatch(
    _state: &HttpErrorFixtureState,
    operation: &str,
    input: http_error_responses_fixture::GeneratedOperationInput,
) -> Response {
    assert_eq!(operation, "read_events", "fixture dispatch is unary-only");
    match input.query.get("mode").map(String::as_str) {
        Some("limited") => http_error_responses_fixture::read_events_http_errors::rate_limited(
            "30".to_owned(),
            Some("backoff".to_owned()),
        )
        .into_response(),
        _ => axum::Json(serde_json::json!({"events": []})).into_response(),
    }
}

#[allow(dead_code, clippy::missing_const_for_fn)]
fn bind_runtime_sse_subscribe_events(
    router: Router<HttpErrorFixtureState>,
) -> Router<HttpErrorFixtureState> {
    router.route("/events/stream", get(http_error_fixture_sse))
}

async fn http_error_fixture_sse(Query(query): Query<BTreeMap<String, String>>) -> Response {
    match query.get("mode").map(String::as_str) {
        Some("invalid") => {
            http_error_responses_fixture::subscribe_events_http_errors::invalid_replay_cursor()
                .into_response()
        }
        Some("expired") => {
            http_error_responses_fixture::subscribe_events_http_errors::replay_cursor_expired(Some(
                "cursor-17".to_owned(),
            ))
            .into_response()
        }
        Some("expired-without-oldest") => {
            http_error_responses_fixture::subscribe_events_http_errors::replay_cursor_expired(None)
                .into_response()
        }
        Some("unavailable") => {
            http_error_responses_fixture::subscribe_events_http_errors::stream_unavailable()
                .into_response()
        }
        _ => StatusCode::NO_CONTENT.into_response(),
    }
}

fn context_page_definition() -> ApiDefinition {
    // Pagination retention/replay and the concrete `ContextPage.next_cursor`
    // value are consumer-owned. Hydra only projects this explicit unary
    // operation and preserves its dispatch result without domain semantics.
    ApiDefinition {
        operations: vec![Operation {
            name: "get_context_page".into(),
            description: "Read one cursor-paginated context page.".into(),
            method: HttpMethod::Get,
            path: "/v1/context".into(),
            read: true,
            output_type: "ContextPage".into(),
            parameters: vec![
                Parameter {
                    name: "limit".into(),
                    description: "Maximum records to return.".into(),
                    ty: hydra_core::ParameterType::U32,
                    required: false,
                    location: ParameterLocation::Query,
                    schema: None,
                    cli: None,
                },
                Parameter {
                    name: "cursor".into(),
                    description: "Opaque cursor from ContextPage.next_cursor.".into(),
                    ty: hydra_core::ParameterType::String,
                    required: false,
                    location: ParameterLocation::Query,
                    schema: None,
                    cli: None,
                },
            ],
            delivery: Delivery::Unary,
            surfaces: Some(vec![
                hydra_core::Surface::Cli,
                hydra_core::Surface::Http,
                hydra_core::Surface::Mcp,
            ]),
            cli_command: None,
            cli_output_flags: vec![],
            http_error_responses: vec![],
            raw_request: false,
        }],
    }
}

fn sample_definition() -> ApiDefinition {
    ApiDefinition {
        operations: vec![Operation {
            name: "list_items".into(),
            description: "List items.".into(),
            method: HttpMethod::Get,
            path: "/items".into(),
            read: true,
            output_type: "Vec<Item>".into(),
            parameters: vec![Parameter {
                name: "limit".into(),
                description: "Max items.".into(),
                ty: hydra_core::ParameterType::U32,
                required: false,
                location: ParameterLocation::Query,
                schema: None,
                cli: None,
            }],
            delivery: Delivery::Unary,
            surfaces: None,
            cli_command: None,
            cli_output_flags: vec![],
            http_error_responses: vec![],
            raw_request: false,
        }],
    }
}

#[test]
fn generates_all_three_surfaces_from_definition() {
    let artifacts = generate_all(&sample_definition(), &GenerateConfig::default());
    assert!(artifacts.cli_rs.contains("ListItems(ListItemsArgs)"));
    assert!(
        artifacts
            .http_rs
            .contains(".route(\"/items\", get(list_items))")
    );
    assert!(artifacts.mcp_json.contains("\"list_items\""));
}

#[test]
fn config_dispatch_fn_flows_into_http_handlers() {
    let config = GenerateConfig {
        http_dispatch_fn: "crate::my_dispatch".into(),
        http_state_type: "crate::MyState".into(),
        ..GenerateConfig::default()
    };
    let artifacts = generate_all(&sample_definition(), &config);
    assert!(artifacts.http_rs.contains("crate::my_dispatch("));

    assert!(artifacts.http_rs.contains("State<crate::MyState>"));
    assert!(artifacts.http_rs.contains("Router<crate::MyState>"));
}

#[test]
fn adding_operation_changes_every_surface() {
    let mut definition = sample_definition();
    let before = generate_all(&definition, &GenerateConfig::default());
    definition.operations.push(Operation {
        name: "create_item".into(),
        description: "Create an item.".into(),
        method: HttpMethod::Post,
        path: "/items".into(),
        read: false,
        output_type: "Item".into(),
        parameters: vec![],
        delivery: Delivery::Unary,
        surfaces: None,
        cli_command: None,
        cli_output_flags: vec![],
        http_error_responses: vec![],
        raw_request: false,
    });
    let after = generate_all(&definition, &GenerateConfig::default());
    assert_ne!(before.cli_rs, after.cli_rs);
    assert_ne!(before.http_rs, after.http_rs);
    assert_ne!(before.mcp_json, after.mcp_json);
}

#[test]
fn surface_allowlist_hides_operations_per_surface() {
    let mut definition = sample_definition();
    definition.operations[0].surfaces =
        Some(vec![hydra_core::Surface::Http, hydra_core::Surface::Mcp]);
    let artifacts = generate_all(&definition, &GenerateConfig::default());
    assert!(!artifacts.cli_rs.contains("ListItems"), "CLI must be empty");
    assert!(artifacts.http_rs.contains("list_items"));
    assert!(artifacts.mcp_json.contains("list_items"));
}

#[test]
fn sse_operations_are_excluded_from_mcp_and_unary_routes() {
    let mut definition = sample_definition();
    definition.operations.push(Operation {
        name: "subscribe_events".into(),
        description: "Stream events.".into(),
        method: HttpMethod::Get,
        path: "/events".into(),
        read: true,
        output_type: "Event".into(),
        parameters: vec![],
        delivery: Delivery::Sse,
        surfaces: Some(vec![hydra_core::Surface::Http, hydra_core::Surface::Cli]),
        cli_command: Some("watch".into()),
        cli_output_flags: vec![],
        http_error_responses: vec![],
        raw_request: false,
    });
    let artifacts = generate_all(&definition, &GenerateConfig::default());
    assert!(!artifacts.mcp_json.contains("subscribe_events"));
    assert!(!artifacts.http_rs.contains("async fn subscribe_events("));
    assert!(artifacts.http_rs.contains("bind_subscribe_events"));
    // CLI command override flows through
    assert!(artifacts.cli_rs.contains("Watch(WatchArgs)"));
}

#[test]
fn mcp_locations_metadata_is_emitted() {
    let artifacts = generate_all(&sample_definition(), &GenerateConfig::default());
    let parsed: serde_json::Value = serde_json::from_str(&artifacts.mcp_json).unwrap();
    assert_eq!(
        parsed["locations"]["list_items"]["limit"],
        serde_json::json!("query")
    );
}

#[test]
fn unary_cursor_pagination_projects_deterministically_across_surfaces() {
    let definition = context_page_definition();
    assert!(hydra_core::validate::validate_definition(&definition).is_ok());

    let first = generate_all(&definition, &GenerateConfig::default());
    let second = generate_all(&definition, &GenerateConfig::default());
    assert_eq!(first.cli_rs, second.cli_rs);
    assert_eq!(first.http_rs, second.http_rs);
    assert_eq!(first.mcp_json, second.mcp_json);

    assert!(first.cli_rs.contains("GetContextPage(GetContextPageArgs)"));
    assert!(first.cli_rs.contains("pub limit: Option<u32>"));
    assert!(first.cli_rs.contains("pub cursor: Option<String>"));
    assert!(
        first
            .http_rs
            .contains(".route(\"/v1/context\", get(get_context_page))")
    );
    assert!(
        first
            .http_rs
            .contains("Query(query): Query<BTreeMap<String, String>>")
    );
    assert!(first.http_rs.contains(
        "GeneratedOperationInput {\n            path: BTreeMap::new(),\n            query,"
    ));

    let mcp: serde_json::Value = serde_json::from_str(&first.mcp_json).unwrap();
    assert_eq!(
        mcp["tools"][0]["name"],
        serde_json::json!("get_context_page")
    );
    assert_eq!(
        mcp["tools"][0]["inputSchema"]["properties"]["limit"]["type"],
        serde_json::json!("integer")
    );
    assert_eq!(
        mcp["tools"][0]["inputSchema"]["properties"]["cursor"]["type"],
        serde_json::json!("string")
    );
    assert_eq!(
        mcp["locations"]["get_context_page"]["limit"],
        serde_json::json!("query")
    );
    assert_eq!(
        mcp["locations"]["get_context_page"]["cursor"],
        serde_json::json!("query")
    );
}

#[test]
fn rejects_duplicate_operations() {
    let mut definition = sample_definition();
    definition.operations.push(definition.operations[0].clone());
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
}

#[test]
fn rejects_path_placeholder_without_parameter() {
    let mut definition = sample_definition();
    definition.operations[0].path = "/items/{item_id}".into();
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
}

#[test]
fn rejects_read_post_mismatch() {
    let mut definition = sample_definition();
    definition.operations[0].method = HttpMethod::Post;
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
}

#[test]
fn rejects_reserved_generated_http_operation_names() {
    let mut definition = sample_definition();
    definition.operations[0].name = "generated_router".into();
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
}

#[test]
fn rejects_empty_surfaces_list() {
    let mut definition = sample_definition();
    definition.operations[0].surfaces = Some(vec![]);
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
}

#[test]
fn rejects_duplicate_surface_entries() {
    let mut definition = sample_definition();
    definition.operations[0].surfaces =
        Some(vec![hydra_core::Surface::Http, hydra_core::Surface::Http]);
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
}

#[test]
fn rejects_non_kebab_cli_command() {
    let mut definition = sample_definition();
    definition.operations[0].cli_command = Some("List_Items".into());
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
}

#[test]
fn raw_request_operation_generates_raw_handler() {
    let mut definition = sample_definition();
    definition.operations.push(Operation {
        name: "ingest_webhook".into(),
        description: "Receive a signed webhook.".into(),
        method: HttpMethod::Post,
        path: "/hooks/ingest".into(),
        read: false,
        output_type: "Value".into(),
        parameters: vec![],
        delivery: Delivery::Unary,
        surfaces: Some(vec![hydra_core::Surface::Http]),
        cli_command: None,
        cli_output_flags: vec![],
        http_error_responses: vec![],
        raw_request: true,
    });
    let artifacts = generate_all(&definition, &GenerateConfig::default());
    // Raw input struct emitted
    assert!(
        artifacts
            .http_rs
            .contains("pub struct GeneratedRawOperationInput")
    );
    // Raw handler shape: HeaderMap + Bytes extractors, raw dispatch fn
    assert!(artifacts.http_rs.contains("headers: HeaderMap,"));
    assert!(artifacts.http_rs.contains("raw_body: Bytes,"));
    assert!(
        artifacts
            .http_rs
            .contains("super::execute_generated_raw_operation(")
    );
    assert!(artifacts.http_rs.contains("raw_body: raw_body.to_vec(),"));
    // Route registered under the raw handler
    assert!(
        artifacts
            .http_rs
            .contains(".route(\"/hooks/ingest\", post(ingest_webhook))")
    );
    // Absent from CLI and MCP
    assert!(!artifacts.cli_rs.contains("ingest_webhook"));
    assert!(!artifacts.mcp_json.contains("ingest_webhook"));
    // Default dispatch lane untouched for the regular operation
    assert!(
        artifacts
            .http_rs
            .contains("super::execute_generated_operation(")
    );
}

#[test]
fn raw_request_operation_with_path_parameter_extracts_typed_path() {
    let mut definition = sample_definition();
    definition.operations.push(Operation {
        name: "ingest_hook".into(),
        description: "Receive a signed webhook for a source.".into(),
        method: HttpMethod::Post,
        path: "/hooks/{source}/ingest".into(),
        read: false,
        output_type: "Value".into(),
        parameters: vec![Parameter {
            name: "source".into(),
            description: "Hook source identifier.".into(),
            ty: hydra_core::ParameterType::String,
            required: true,
            location: ParameterLocation::Path,
            schema: None,
            cli: None,
        }],
        delivery: Delivery::Unary,
        surfaces: Some(vec![hydra_core::Surface::Http]),
        cli_command: None,
        cli_output_flags: vec![],
        http_error_responses: vec![],
        raw_request: true,
    });
    let artifacts = generate_all(&definition, &GenerateConfig::default());
    assert!(
        artifacts
            .http_rs
            .contains(".route(\"/hooks/{source}/ingest\", post(ingest_hook))")
    );
    // Path extractor present in the raw handler, body extraction absent
    assert!(
        artifacts
            .http_rs
            .contains("Path(path): Path<BTreeMap<String, String>>,\n    headers: HeaderMap,")
    );
    assert!(!artifacts.http_rs.contains("Json(body)"));
}

#[test]
fn raw_request_byte_identical_when_flag_absent() {
    // The core determinism claim for this feature (COD-402 acceptance):
    // a definition with no raw_request operations must produce
    // byte-identical HTTP output to the pre-feature generator. The
    // fixture pair is the notes example as of v0.1.0 (f6ef2e6), before
    // echo_raw/raw_request existed.
    let expected = include_str!("fixtures/notes-pre-raw-http.rs");
    let definition: ApiDefinition =
        serde_yaml::from_str(include_str!("fixtures/notes-pre-raw-operations.yaml")).unwrap();
    // Config as of v0.1.0 (the fixture's provenance). The raw dispatch
    // knob is left at its default: it must not leak into output for
    // definitions that don't use raw operations.
    let config = GenerateConfig {
        http_dispatch_fn: "crate::execute_operation_http".to_string(),
        http_state_type: "crate::AppState".to_string(),
        sse_binding_prefix: "super::".to_string(),
        ..GenerateConfig::default()
    };
    let artifacts = generate_all(&definition, &config);
    assert!(!artifacts.http_rs.contains("GeneratedRawOperationInput"));
    assert!(!artifacts.http_rs.contains("HeaderMap"));
    assert_eq!(artifacts.http_rs, expected);
}

#[test]
fn rejects_raw_request_with_non_http_surface() {
    let mut definition = sample_definition();
    definition.operations[0].raw_request = true;
    // surfaces: None means all surfaces — raw must be exactly [http]
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
    definition.operations[0].surfaces =
        Some(vec![hydra_core::Surface::Http, hydra_core::Surface::Mcp]);
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
    definition.operations[0].surfaces = Some(vec![hydra_core::Surface::Http]);
    assert!(hydra_core::validate::validate_definition(&definition).is_ok());
}

#[test]
fn rejects_raw_request_with_sse_or_body_params() {
    let mut definition = sample_definition();
    definition.operations[0].raw_request = true;
    definition.operations[0].surfaces = Some(vec![hydra_core::Surface::Http]);
    // body-location parameter is rejected (raw bytes replace Json body)
    definition.operations[0].parameters.push(Parameter {
        name: "payload".into(),
        description: "Body payload.".into(),
        ty: hydra_core::ParameterType::String,
        required: true,
        location: ParameterLocation::Body,
        schema: None,
        cli: None,
    });
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
    definition.operations[0].parameters.pop();
    // delivery: sse is rejected (raw is unary-only)
    definition.operations[0].delivery = Delivery::Sse;
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
    definition.operations[0].delivery = Delivery::Unary;
    assert!(hydra_core::validate::validate_definition(&definition).is_ok());
}

#[test]
fn committed_example_artifacts_are_current() {
    // Guards against editing generated/ by hand or forgetting `hydra write`.
    let root = env!("CARGO_MANIFEST_DIR");
    let err = hydra_codegen::verify_generated(
        format!("{root}/../../examples/notes/api/operations.yaml"),
        format!("{root}/../../examples/notes/generated"),
        &serde_yaml::from_str(include_str!("../../../examples/notes/hydra.yaml")).unwrap(),
    )
    .err()
    .map(|e| format!("prettier: {e:#}"));
    if let Some(e) = err {
        panic!("{e}");
    }
}

// ── json parameters + CLI representation overrides (COD-411) ───────────────

fn attachments_parameter() -> Parameter {
    Parameter {
        name: "attachments".into(),
        description: "Attachments to send.".into(),
        ty: hydra_core::ParameterType::Json,
        required: false,
        location: ParameterLocation::Body,
        schema: Some(serde_json::json!({
            "type": "array",
            "items": {
                "type": "object",
                "oneOf": [
                    {
                        "type": "object",
                        "properties": {
                            "mime_type": {"type": "string"},
                            "filename": {"type": "string"},
                            "data_base64": {"type": "string"}
                        },
                        "required": ["mime_type", "data_base64"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {
                            "stored_id": {"type": "string"}
                        },
                        "required": ["stored_id"],
                        "additionalProperties": false
                    }
                ]
            }
        })),
        cli: Some(hydra_core::CliOverride {
            flag: Some("attach".into()),
            multiple: true,
            companions: vec![hydra_core::CliCompanion {
                flag: "attach-mime".into(),
                field: "attach_mime".into(),
                description: "MIME type override for the corresponding --attach.".into(),
            }],
        }),
    }
}

fn definition_with_attachments() -> ApiDefinition {
    let mut definition = sample_definition();
    definition.operations[0]
        .parameters
        .push(attachments_parameter());
    definition
}

#[test]
fn json_parameter_cli_override_generates_repeatable_and_companion_flags() {
    let artifacts = generate_all(&definition_with_attachments(), &GenerateConfig::default());
    // Repeatable --attach flag with Append action
    assert!(
        artifacts.cli_rs.contains(
            "#[arg(long = \"attach\", action = clap::ArgAction::Append, required = false)]"
        )
    );
    // Companion --attach-mime flag, also repeatable, declared flag emitted
    assert!(artifacts.cli_rs.contains(
        "#[arg(long = \"attach-mime\", action = clap::ArgAction::Append)]\n    pub attach_mime: Option<Vec<String>>"
    ));
    // parameters_json maps the repeatable flag back to the wire name and
    // carries the companion alongside
    assert!(
        artifacts
            .cli_rs
            .contains("\"attachments\": args.attachments.clone().unwrap_or_default()")
    );
    assert!(
        artifacts
            .cli_rs
            .contains("\"attach_mime\": args.attach_mime.clone()")
    );
}

#[test]
fn json_parameter_schema_flows_into_mcp_input_schema() {
    let artifacts = generate_all(&definition_with_attachments(), &GenerateConfig::default());
    let parsed: serde_json::Value = serde_json::from_str(&artifacts.mcp_json).unwrap();
    let schema = &parsed["tools"][0]["inputSchema"]["properties"]["attachments"];
    assert_eq!(schema["type"], "array");
    assert!(schema["items"]["oneOf"].is_array());
    // The declared schema is embedded verbatim with the description merged in
    assert_eq!(schema["description"], "Attachments to send.");
}

#[test]
fn scalar_definitions_unchanged_by_feature() {
    // Definitions that don't use json params or cli overrides must produce
    // byte-identical CLI output (the pre-feature generator).
    let artifacts = generate_all(&sample_definition(), &GenerateConfig::default());
    assert!(
        artifacts
            .cli_rs
            .contains("#[arg(long)]\n    pub limit: Option<u32>,")
    );
    assert!(!artifacts.cli_rs.contains("ArgAction::Append"));
    assert!(!artifacts.cli_rs.contains("long = \""));
}

#[test]
fn cli_flag_override_without_multiple_keeps_scalar_shape() {
    let mut definition = sample_definition();
    definition.operations[0].parameters[0].cli = Some(hydra_core::CliOverride {
        flag: Some("max-items".into()),
        multiple: false,
        companions: vec![],
    });
    let artifacts = generate_all(&definition, &GenerateConfig::default());
    assert!(artifacts.cli_rs.contains("#[arg(long = \"max-items\")]"));
    assert!(artifacts.cli_rs.contains("pub limit: Option<u32>,"));
    // parameters_json keeps the wire name `limit`
    assert!(artifacts.cli_rs.contains("\"limit\": args.limit.clone()"));
}

#[test]
fn rejects_json_parameter_without_schema() {
    let mut definition = definition_with_attachments();
    let parameter = &mut definition.operations[0].parameters[1];
    parameter.schema = None;
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
}

#[test]
fn rejects_json_parameter_on_non_body_location() {
    let mut definition = definition_with_attachments();
    let parameter = &mut definition.operations[0].parameters[1];
    parameter.location = ParameterLocation::Query;
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
}

#[test]
fn rejects_schema_on_scalar_parameter() {
    let mut definition = sample_definition();
    definition.operations[0].parameters[0].schema = Some(serde_json::json!({"type": "integer"}));
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
}

#[test]
fn rejects_json_parameter_without_cli_block_on_cli_operation() {
    let mut definition = definition_with_attachments();
    definition.operations[0].parameters[1].cli = None;
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
    // ...but a json param is fine without cli on an operation that does not
    // generate the CLI surface
    definition.operations[0].surfaces =
        Some(vec![hydra_core::Surface::Http, hydra_core::Surface::Mcp]);
    assert!(hydra_core::validate::validate_definition(&definition).is_ok());
}

#[test]
fn rejects_cli_block_on_non_cli_operation() {
    let mut definition = definition_with_attachments();
    definition.operations[0].surfaces =
        Some(vec![hydra_core::Surface::Http, hydra_core::Surface::Mcp]);
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
}

#[test]
fn rejects_multiple_on_scalar_parameter() {
    let mut definition = sample_definition();
    definition.operations[0].parameters[0].cli = Some(hydra_core::CliOverride {
        flag: None,
        multiple: true,
        companions: vec![],
    });
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
}

#[test]
fn rejects_colliding_cli_flags() {
    let mut definition = definition_with_attachments();
    // The companion flag `attach-mime` collides with... nothing yet; make the
    // parameter's own flag collide with the companion.
    let parameter = &mut definition.operations[0].parameters[1];
    parameter.cli = Some(hydra_core::CliOverride {
        flag: Some("attach-mime".into()),
        multiple: true,
        companions: vec![hydra_core::CliCompanion {
            flag: "attach-mime".into(),
            field: "attach_mime".into(),
            description: "Colliding companion.".into(),
        }],
    });
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
}

#[test]
fn rejects_non_kebab_flag_and_invalid_companion_field() {
    let mut definition = definition_with_attachments();
    let parameter = &mut definition.operations[0].parameters[1];
    parameter.cli = Some(hydra_core::CliOverride {
        flag: Some("Attach_Path".into()),
        multiple: true,
        companions: vec![],
    });
    assert!(hydra_core::validate::validate_definition(&definition).is_err());

    let mut definition = definition_with_attachments();
    let parameter = &mut definition.operations[0].parameters[1];
    parameter.cli = Some(hydra_core::CliOverride {
        flag: None,
        multiple: true,
        companions: vec![hydra_core::CliCompanion {
            flag: "attach-mime".into(),
            field: "attach-mime".into(), // not snake_case
            description: "Bad field.".into(),
        }],
    });
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
}

// ── review-panel regression tests (round 2) ────────────────────────────────

#[test]
fn companion_flag_name_is_emitted_not_derived() {
    // A companion whose flag diverges from the kebab derivation of its
    // field must emit the declared flag verbatim.
    let mut definition = definition_with_attachments();
    let parameter = &mut definition.operations[0].parameters[1];
    parameter.cli = Some(hydra_core::CliOverride {
        flag: Some("attach".into()),
        multiple: true,
        companions: vec![hydra_core::CliCompanion {
            flag: "mime-override".into(),
            field: "attach_mime".into(),
            description: "MIME override.".into(),
        }],
    });
    let artifacts = generate_all(&definition, &GenerateConfig::default());
    assert!(artifacts.cli_rs.contains(
        "#[arg(long = \"mime-override\", action = clap::ArgAction::Append)]\n    pub attach_mime: Option<Vec<String>>"
    ));
}

#[test]
fn rejects_override_flag_colliding_with_default_parameter_flag() {
    // A default-shaped parameter's derived flag must collide with an
    // explicit override on another parameter.
    let mut definition = definition_with_attachments();
    // attachments has cli.flag = attach; rename the scalar `limit`
    // parameter's flag to `attach` — collision via the derived default?
    // No: limit's default flag is `limit`. Set attachments' flag to
    // `limit` instead.
    let parameter = &mut definition.operations[0].parameters[1];
    parameter.cli = Some(hydra_core::CliOverride {
        flag: Some("limit".into()),
        multiple: true,
        companions: vec![],
    });
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
}

#[test]
fn rejects_companion_field_colliding_with_parameter_field() {
    let mut definition = definition_with_attachments();
    let parameter = &mut definition.operations[0].parameters[1];
    parameter.cli = Some(hydra_core::CliOverride {
        flag: Some("attach".into()),
        multiple: true,
        companions: vec![hydra_core::CliCompanion {
            flag: "limit-override".into(),
            field: "limit".into(), // collides with the `limit` parameter
            description: "Bad.".into(),
        }],
    });
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
}

#[test]
fn rejects_cli_block_on_path_parameter() {
    let mut definition = definition_with_attachments();
    // Make the scalar limit parameter a path param with a cli block.
    definition.operations[0].parameters[0].location = ParameterLocation::Path;
    definition.operations[0].parameters[0].cli = Some(hydra_core::CliOverride {
        flag: Some("max".into()),
        multiple: false,
        companions: vec![],
    });
    // Path param needs matching placeholder; adjust the path.
    definition.operations[0].path = "/items/{limit}".into();
    assert!(hydra_core::validate::validate_definition(&definition).is_err());
}

#[test]
fn cli_block_without_flag_uses_default_long_attribute() {
    // cli: present, flag omitted: clap derives the flag from the field
    // name (kebab), so the plain #[arg(long)] attribute is emitted —
    // identical to the default shape.
    let mut definition = sample_definition();
    definition.operations[0].parameters[0].cli = Some(hydra_core::CliOverride {
        flag: None,
        multiple: false,
        companions: vec![],
    });
    let artifacts = generate_all(&definition, &GenerateConfig::default());
    assert!(
        artifacts
            .cli_rs
            .contains("#[arg(long)]\n    pub limit: Option<u32>,")
    );
}

#[test]
fn required_multiple_flag_carries_required_true() {
    let mut definition = definition_with_attachments();
    let parameter = &mut definition.operations[0].parameters[1];
    parameter.required = true;
    let artifacts = generate_all(&definition, &GenerateConfig::default());
    assert!(artifacts.cli_rs.contains("required = true"));
}

// ── CLI-only output flags (COD-486) ────────────────────────────────────────

mod cli_output_flags_fixture {
    include!("fixtures/cli-output-flags/cli.rs");
}

#[derive(Debug, Parser)]
#[command(name = "fixture")]
struct FixtureCli {
    #[command(subcommand)]
    command: cli_output_flags_fixture::GeneratedCommand,
}

fn cli_output_flags_definition() -> ApiDefinition {
    let definition: ApiDefinition =
        serde_yaml::from_str(include_str!("fixtures/cli-output-flags/operations.yaml"))
            .expect("fixture YAML parses");
    hydra_core::validate::validate_definition(&definition).expect("fixture definition validates");
    definition
}

fn parameters_json_section(cli: &str) -> &str {
    let start = cli
        .find("    pub fn parameters_json")
        .expect("generated CLI has parameters_json");
    let end = cli[start..]
        .find("\n}\n\n")
        .expect("GeneratedCommand impl closes after parameters_json");
    &cli[start..start + end + "\n}\n".len()]
}

#[test]
fn output_flags_compile_and_parse_as_boolean_presentation_options() {
    let absent = FixtureCli::try_parse_from([
        "fixture",
        "list-records",
        "--cursor",
        "checkpoint-1",
        "--query",
        "pending",
        "--label",
        "triage",
    ])
    .expect("absent output flag parses");
    match absent.command {
        cli_output_flags_fixture::GeneratedCommand::ListRecords(args) => {
            assert!(!args.include_cursor);
            assert_eq!(args.cursor.as_deref(), Some("checkpoint-1"));
            assert_eq!(args.query.as_deref(), Some("pending"));
            assert_eq!(args.label, Some(vec!["triage".to_owned()]));
        }
        _ => panic!("expected list-records command"),
    }

    let present = FixtureCli::try_parse_from([
        "fixture",
        "list-records",
        "--cursor",
        "checkpoint-1",
        "--include-cursor",
    ])
    .expect("present output flag parses");
    assert_eq!(present.command.operation_name(), "list_records");
    let params = present.command.parameters_json();
    match present.command {
        cli_output_flags_fixture::GeneratedCommand::ListRecords(args) => {
            assert!(args.include_cursor);
        }
        _ => panic!("expected list-records command"),
    }
    assert_eq!(
        params,
        serde_json::json!({"cursor": "checkpoint-1", "query": null, "label": null})
    );

    assert!(
        FixtureCli::try_parse_from(["fixture", "list-records", "--include-cursor=true",]).is_err(),
        "SetTrue must reject a supplied boolean value"
    );

    let zero_parameter = FixtureCli::try_parse_from(["fixture", "status", "--verbose"])
        .expect("zero-parameter output flag parses");
    let params = zero_parameter.command.parameters_json();
    match zero_parameter.command {
        cli_output_flags_fixture::GeneratedCommand::Status(args) => assert!(args.verbose),
        _ => panic!("expected status command"),
    }
    assert_eq!(params, serde_json::json!({}));
}

#[test]
fn output_flags_render_declared_help_from_compiled_cli() {
    let list_records_help = FixtureCli::try_parse_from(["fixture", "list-records", "--help"])
        .expect_err("list-records help exits through clap");
    assert_eq!(
        list_records_help.kind(),
        clap::error::ErrorKind::DisplayHelp,
        "list-records --help must render help"
    );
    let list_records_help = list_records_help.render().to_string();
    assert!(
        list_records_help.contains("--include-cursor"),
        "list-records help must expose the exact output flag: {list_records_help}"
    );
    assert!(
        list_records_help.contains("Include the checkpoint alongside each displayed record"),
        "list-records help must expose the declared output-flag description: {list_records_help}"
    );

    let status_help = FixtureCli::try_parse_from(["fixture", "status", "--help"])
        .expect_err("status help exits through clap");
    assert_eq!(
        status_help.kind(),
        clap::error::ErrorKind::DisplayHelp,
        "status --help must render help"
    );
    let status_help = status_help.render().to_string();
    assert!(
        status_help.contains("--verbose"),
        "status help must expose the exact output flag: {status_help}"
    );
    assert!(
        status_help.contains("Include verbose status presentation details"),
        "status help must expose the declared output-flag description: {status_help}"
    );
}

#[test]
fn output_flags_stay_out_of_request_and_mcp_projections() {
    let definition = cli_output_flags_definition();
    let with_output_flags = generate_all(&definition, &GenerateConfig::default());
    let mut without_output_flags = definition;
    for operation in &mut without_output_flags.operations {
        operation.cli_output_flags.clear();
    }
    let without_output_flags = generate_all(&without_output_flags, &GenerateConfig::default());

    assert_eq!(
        parameters_json_section(&with_output_flags.cli_rs),
        parameters_json_section(&without_output_flags.cli_rs),
        "presentation flags must not change request projection"
    );
    assert!(
        with_output_flags
            .cli_rs
            .contains("pub include_cursor: bool")
    );
    assert!(with_output_flags.cli_rs.contains("pub verbose: bool"));
    assert!(!with_output_flags.http_rs.contains("include_cursor"));
    assert!(!with_output_flags.http_rs.contains("verbose"));

    let mcp: serde_json::Value = serde_json::from_str(&with_output_flags.mcp_json).unwrap();
    assert!(
        mcp["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .any(|tool| tool["name"] == "list_records")
    );
    assert!(
        mcp["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .all(|tool| tool["name"] != "subscribe_events")
    );
    assert_eq!(
        mcp["tools"][0]["inputSchema"]["properties"]["cursor"]["type"],
        "string"
    );
    assert!(
        mcp["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .all(|tool| tool["inputSchema"]["properties"]
                .get("include_cursor")
                .is_none()),
        "output flags must not become MCP inputs"
    );
}

#[test]
fn output_flag_fixture_is_fresh_and_deterministic() {
    let definition = cli_output_flags_definition();
    let first = generate_all(&definition, &GenerateConfig::default());
    let second = generate_all(&definition, &GenerateConfig::default());
    assert_eq!(first.cli_rs, second.cli_rs);
    assert_eq!(first.http_rs, second.http_rs);
    assert_eq!(first.mcp_json, second.mcp_json);

    let temp = tempfile::tempdir().expect("temporary generated-artifact directory");
    let first_dir = temp.path().join("first");
    let second_dir = temp.path().join("second");
    write_generated(&first_dir, &first).expect("write first generated artifact set");
    write_generated(&second_dir, &second).expect("write second generated artifact set");

    for artifact in ["cli.rs", "http.rs", "mcp.json"] {
        assert_eq!(
            fs::read(first_dir.join(artifact)).expect("read first generated artifact"),
            fs::read(second_dir.join(artifact)).expect("read second generated artifact"),
            "two generated writes must be byte-identical for {artifact}"
        );
    }
    assert_eq!(
        fs::read_to_string(first_dir.join("cli.rs")).expect("read written CLI fixture"),
        include_str!("fixtures/cli-output-flags/cli.rs"),
        "committed compiled CLI fixture must match fresh generation"
    );
}

#[test]
fn output_flags_validate_the_complete_cli_namespace() {
    let mut definition = cli_output_flags_definition();
    definition.operations[0].cli_output_flags[0].flag = "include_cursor".into();
    assert!(hydra_core::validate::validate_definition(&definition).is_err());

    let mut definition = cli_output_flags_definition();
    definition.operations[0].cli_output_flags[0].field = "include-cursor".into();
    assert!(hydra_core::validate::validate_definition(&definition).is_err());

    let mut definition = cli_output_flags_definition();
    definition.operations[0].cli_output_flags[0].flag = "cursor".into();
    assert!(hydra_core::validate::validate_definition(&definition).is_err());

    let mut definition = cli_output_flags_definition();
    definition.operations[0].cli_output_flags[0].flag = "label".into();
    assert!(hydra_core::validate::validate_definition(&definition).is_err());

    let mut definition = cli_output_flags_definition();
    definition.operations[0].parameters[0].location = ParameterLocation::Path;
    definition.operations[0].path = "/records/{cursor}".into();
    definition.operations[0].cli_output_flags[0].field = "cursor".into();
    assert!(hydra_core::validate::validate_definition(&definition).is_err());

    let mut definition = cli_output_flags_definition();
    definition.operations[0].cli_output_flags[0].flag = "help".into();
    assert!(hydra_core::validate::validate_definition(&definition).is_err());

    let mut definition = cli_output_flags_definition();
    definition.operations[1].cli_output_flags[0].flag = "show-help-details".into();
    definition.operations[1].cli_output_flags[0].field = "help".into();
    let error = hydra_core::validate::validate_definition(&definition)
        .expect_err("clap's help argument ID must be reserved");
    assert!(error.to_string().contains("reserved help argument"));

    let mut definition = cli_output_flags_definition();
    definition.operations[0].parameters[0].name = "help".into();
    let error = hydra_core::validate::validate_definition(&definition)
        .expect_err("parameter fields must not collide with clap's help argument");
    assert!(error.to_string().contains("colliding CLI fields"));

    let mut definition = cli_output_flags_definition();
    definition.operations[0].parameters[1]
        .cli
        .as_mut()
        .unwrap()
        .companions[0]
        .flag = "help".into();
    let error = hydra_core::validate::validate_definition(&definition)
        .expect_err("companion flags must not collide with clap's --help flag");
    assert!(error.to_string().contains("colliding CLI flags"));

    let mut definition = cli_output_flags_definition();
    definition.operations[0].parameters[1]
        .cli
        .as_mut()
        .unwrap()
        .companions[0]
        .field = "help".into();
    let error = hydra_core::validate::validate_definition(&definition)
        .expect_err("companion fields must not collide with clap's help argument");
    assert!(error.to_string().contains("colliding CLI fields"));

    let mut definition = cli_output_flags_definition();
    definition.operations[1].cli_output_flags[0].field = "gen".into();
    let error = hydra_core::validate::validate_definition(&definition)
        .expect_err("Rust 2024's gen keyword must be rejected before generation");
    assert!(
        error
            .to_string()
            .contains("Rust-safe snake_case identifier")
    );

    for keyword in ["try", "yield"] {
        let mut definition = cli_output_flags_definition();
        definition.operations[1].cli_output_flags[0].field = keyword.into();
        let error = hydra_core::validate::validate_definition(&definition)
            .expect_err("all Rust 2024 keywords must be rejected before generation");
        assert!(
            error
                .to_string()
                .contains("Rust-safe snake_case identifier"),
            "{keyword} must be rejected as a generated field"
        );
    }

    let mut definition = cli_output_flags_definition();
    definition.operations[0]
        .cli_output_flags
        .push(hydra_core::CliOutputFlag {
            flag: "include-cursor".into(),
            field: "other_field".into(),
            description: "Duplicate flag.".into(),
        });
    assert!(hydra_core::validate::validate_definition(&definition).is_err());

    let mut definition = cli_output_flags_definition();
    definition.operations[0]
        .cli_output_flags
        .push(hydra_core::CliOutputFlag {
            flag: "other-flag".into(),
            field: "include_cursor".into(),
            description: "Duplicate field.".into(),
        });
    assert!(hydra_core::validate::validate_definition(&definition).is_err());

    let mut definition = cli_output_flags_definition();
    definition.operations[0].surfaces = Some(vec![hydra_core::Surface::Http]);
    let error = hydra_core::validate::validate_definition(&definition)
        .expect_err("output flags without CLI must fail");
    assert!(error.to_string().contains("cli_output_flags"));
}

#[test]
fn output_flags_use_the_actual_multiple_override_long_namespace() {
    // `multiple` emits an explicit `long = cli.effective_flag(field)`.
    // With no override, that preserves `record_ids` rather than clap's normal
    // kebab-case derivation, so the distinct presentation flag `record-ids`
    // must validate and generate alongside it.
    let mut definition = definition_with_attachments();
    let parameter = &mut definition.operations[0].parameters[1];
    parameter.name = "record_ids".into();
    parameter.cli = Some(hydra_core::CliOverride {
        flag: None,
        multiple: true,
        companions: vec![],
    });
    definition.operations[0].cli_output_flags = vec![hydra_core::CliOutputFlag {
        flag: "record-ids".into(),
        field: "show_record_ids".into(),
        description: "Show record identifiers in CLI output.".into(),
    }];

    hydra_core::validate::validate_definition(&definition)
        .expect("distinct emitted long flags validate");
    let artifacts = generate_all(&definition, &GenerateConfig::default());
    assert!(artifacts.cli_rs.contains(
        "#[arg(long = \"record_ids\", action = clap::ArgAction::Append, required = false)]"
    ));
    assert!(artifacts.cli_rs.contains(
        "#[arg(long = \"record-ids\", action = clap::ArgAction::SetTrue)]\n    pub show_record_ids: bool"
    ));
}

#[test]
fn output_flag_yaml_rejects_unknown_keys() {
    let yaml = r"
operations:
  - name: status
    description: Read status.
    method: GET
    path: /status
    read: true
    output_type: Status
    parameters: []
    cli_output_flags:
      - flag: verbose
        field: verbose
        description: Show details.
        unknown: rejected
";
    assert!(serde_yaml::from_str::<ApiDefinition>(yaml).is_err());
}

// ── Typed HTTP error responses (COD-487) ───────────────────────────────────

fn http_error_responses_definition() -> ApiDefinition {
    let definition: ApiDefinition = serde_yaml::from_str(include_str!(
        "fixtures/http-error-responses/operations.yaml"
    ))
    .expect("HTTP-error fixture YAML parses");
    hydra_core::validate::validate_definition(&definition)
        .expect("HTTP-error fixture definition validates");
    definition
}

fn http_error_responses_config() -> GenerateConfig {
    GenerateConfig {
        http_dispatch_fn: "crate::http_error_fixture_dispatch".into(),
        http_state_type: "crate::HttpErrorFixtureState".into(),
        sse_binding_prefix: "crate::".into(),
        ..GenerateConfig::default()
    }
}

async fn http_error_fixture_response(uri: &str) -> Response {
    http_error_responses_fixture::bind_subscribe_events(
        http_error_responses_fixture::generated_router(),
    )
    .with_state(HttpErrorFixtureState)
    .oneshot(
        Request::builder()
            .uri(uri)
            .body(Body::empty())
            .expect("fixture request builds"),
    )
    .await
    .expect("generated fixture router returns a response")
}

async fn json_fixture_response(uri: &str) -> (StatusCode, String, serde_json::Value) {
    let response = http_error_fixture_response(uri).await;
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read generated response body");
    let value = serde_json::from_slice(&body).expect("generated response is JSON");
    (status, content_type, value)
}

#[tokio::test]
async fn http_error_responses_are_generated_and_execute_through_the_real_router() {
    // The existing SSE binding hook owns the handler. It returns generated
    // error constructors before a stream opens, so these error paths must be
    // normal JSON responses rather than an SSE content type or sentinel.
    let (status, content_type, body) = json_fixture_response("/events/stream?mode=invalid").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(content_type, "application/json");
    assert_eq!(body, serde_json::json!({"error": "invalid_replay_cursor"}));

    let (status, content_type, body) = json_fixture_response("/events/stream?mode=expired").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(content_type, "application/json");
    assert_eq!(
        body,
        serde_json::json!({
            "error": "replay_cursor_expired",
            "oldest_cursor": "cursor-17"
        })
    );

    let (status, content_type, body) =
        json_fixture_response("/events/stream?mode=expired-without-oldest").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(content_type, "application/json");
    assert_eq!(body, serde_json::json!({"error": "replay_cursor_expired"}));
    assert!(
        body.get("oldest_cursor").is_none(),
        "optional fields must be omitted rather than serialized as null"
    );

    // Unary operations use the same generated conversion types, without
    // changing the established generated dispatch signature.
    let (status, content_type, body) = json_fixture_response("/events?mode=limited").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(content_type, "application/json");
    assert_eq!(
        body,
        serde_json::json!({
            "error": "rate \u{1} \"limited\"",
            "retry_after": "30",
            "retry_hint": "backoff"
        })
    );

    let (status, content_type, body) =
        json_fixture_response("/events/stream?mode=unavailable").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(content_type, "application/json");
    assert_eq!(body, serde_json::json!({"error": "stream_unavailable"}));
}

#[test]
fn http_error_responses_preserve_non_http_projections_and_private_state() {
    let definition = http_error_responses_definition();
    let with_errors = generate_all(&definition, &http_error_responses_config());
    let mut without_errors = definition;
    for operation in &mut without_errors.operations {
        operation.http_error_responses.clear();
    }
    let without_errors = generate_all(&without_errors, &http_error_responses_config());

    assert_eq!(with_errors.cli_rs, without_errors.cli_rs);
    assert_eq!(with_errors.mcp_json, without_errors.mcp_json);
    assert!(with_errors.cli_rs.contains("ReadEvents(ReadEventsArgs)"));
    assert!(with_errors.mcp_json.contains("\"read_events\""));
    assert_ne!(with_errors.http_rs, without_errors.http_rs);
    assert!(
        with_errors
            .http_rs
            .contains("pub mod read_events_http_errors")
    );
    assert!(
        with_errors
            .http_rs
            .contains("pub mod subscribe_events_http_errors")
    );
    assert!(with_errors.http_rs.contains(
        "pub fn replay_cursor_expired(oldest_cursor: Option<String>) -> ReplayCursorExpiredResponse"
    ));
    assert!(
        with_errors
            .http_rs
            .contains("pub fn rate_limited(retry_after: String, retry_hint: Option<String>) -> RateLimitedResponse")
    );
    assert!(with_errors.http_rs.contains(
        "pub struct InvalidReplayCursorResponse {\n        body: InvalidReplayCursorBody,"
    ));
    assert!(!with_errors.http_rs.contains(
        "pub struct InvalidReplayCursorResponse {\n        pub body: InvalidReplayCursorBody,"
    ));
    assert!(with_errors.http_rs.contains(
        "(StatusCode::from_u16(400).expect(\"validated HTTP error status\"), Json(self.body)).into_response()"
    ));
    assert!(
        !with_errors.http_rs.contains(".route(\"/events/stream\""),
        "SSE retains its runtime binding rather than gaining a duplicate route"
    );
}

#[test]
fn http_error_response_fixture_is_fresh_and_byte_deterministic() {
    let definition = http_error_responses_definition();
    let config = http_error_responses_config();
    let first = generate_all(&definition, &config);
    let second = generate_all(&definition, &config);
    assert_eq!(first.cli_rs, second.cli_rs);
    assert_eq!(first.http_rs, second.http_rs);
    assert_eq!(first.mcp_json, second.mcp_json);

    let temp = tempfile::tempdir().expect("temporary generated-artifact directory");
    let first_dir = temp.path().join("first");
    let second_dir = temp.path().join("second");
    write_generated(&first_dir, &first).expect("write first generated artifact set");
    write_generated(&second_dir, &second).expect("write second generated artifact set");
    for artifact in ["cli.rs", "http.rs", "mcp.json"] {
        assert_eq!(
            fs::read(first_dir.join(artifact)).expect("read first generated artifact"),
            fs::read(second_dir.join(artifact)).expect("read second generated artifact"),
            "two generated writes must be byte-identical for {artifact}"
        );
    }
    assert_eq!(
        fs::read_to_string(first_dir.join("http.rs")).expect("read written HTTP fixture"),
        include_str!("fixtures/http-error-responses/http.rs"),
        "compiled committed HTTP fixture must match fresh generation"
    );

    // A historical no-error HTTP fixture remains byte-identical even after
    // adding the default-empty declaration to Operation.
    let no_error_definition: ApiDefinition =
        serde_yaml::from_str(include_str!("fixtures/notes-pre-raw-operations.yaml"))
            .expect("historic no-error YAML parses");
    let no_error_config = GenerateConfig {
        http_dispatch_fn: "crate::execute_operation_http".to_string(),
        http_state_type: "crate::AppState".to_string(),
        sse_binding_prefix: "super::".to_string(),
        ..GenerateConfig::default()
    };
    assert_eq!(
        generate_all(&no_error_definition, &no_error_config).http_rs,
        include_str!("fixtures/notes-pre-raw-http.rs")
    );
}

fn write_http_error_response_loader_fixture(
    temp: &tempfile::TempDir,
    name: &str,
    yaml: &str,
) -> PathBuf {
    let path = temp.path().join(format!("{name}.yaml"));
    fs::write(&path, yaml).expect("write temporary HTTP-error loader fixture");
    path
}

fn http_error_response_fixture_with_required_constant(constant: &str) -> String {
    include_str!("fixtures/http-error-responses/operations.yaml").replacen(
        "            const: \"rate \\u0001 \\\"limited\\\"\"",
        &format!("            const: {constant}"),
        1,
    )
}

fn http_error_response_fixture_with_optional_constant(constant: &str) -> String {
    const OPTIONAL_RETRY_HINT: &str =
        "          - name: retry_hint\n            type: string\n            required: false\n";

    include_str!("fixtures/http-error-responses/operations.yaml").replacen(
        OPTIONAL_RETRY_HINT,
        &format!("{OPTIONAL_RETRY_HINT}            const: {constant}\n"),
        1,
    )
}

#[test]
fn http_error_constant_loader_distinguishes_omission_from_explicit_non_strings() {
    let temp = tempfile::tempdir().expect("temporary loader fixture directory");

    // The committed fixture exercises escaped required strings, plus omitted
    // required and optional constants through the real YAML loader.
    let committed_fixture = write_http_error_response_loader_fixture(
        &temp,
        "committed",
        include_str!("fixtures/http-error-responses/operations.yaml"),
    );
    let definition = hydra_core::load_api_definition(&committed_fixture)
        .expect("committed HTTP-error fixture loads through the real boundary");
    let fields = &definition.operations[0].http_error_responses[0].fields;
    assert_eq!(
        fields[0].constant.as_deref(),
        Some("rate \u{0001} \"limited\"")
    );
    assert!(
        fields[1].constant.is_none(),
        "omitted required const is dynamic"
    );
    assert!(
        fields[2].constant.is_none(),
        "omitted optional const is dynamic"
    );
    assert!(generate_all(&definition, &http_error_responses_config()).http_rs.contains(
        "pub fn rate_limited(retry_after: String, retry_hint: Option<String>) -> RateLimitedResponse"
    ));

    // Quoted scalar text is valid declaration data, including empty strings;
    // it remains fixed rather than becoming a generated constructor argument.
    for (name, declaration, expected) in [
        ("empty-string", "\"\"", ""),
        ("quoted-numeric", "\"42\"", "42"),
        ("quoted-boolean", "\"true\"", "true"),
    ] {
        let fixture = write_http_error_response_loader_fixture(
            &temp,
            name,
            &http_error_response_fixture_with_required_constant(declaration),
        );
        let definition = hydra_core::load_api_definition(&fixture)
            .unwrap_or_else(|error| panic!("{name} string constant must load: {error:#}"));
        assert_eq!(
            definition.operations[0].http_error_responses[0].fields[0]
                .constant
                .as_deref(),
            Some(expected)
        );
        assert!(generate_all(&definition, &http_error_responses_config()).http_rs.contains(
            "pub fn rate_limited(retry_after: String, retry_hint: Option<String>) -> RateLimitedResponse"
        ));
    }

    // Every supplied non-string form must stop at load_api_definition before
    // generation, for both required and optional fields.
    for (scope, field_name) in [("required", "error"), ("optional", "retry_hint")] {
        for (name, declaration) in [
            ("null", "null"),
            ("tilde", "~"),
            ("empty-value", ""),
            ("boolean", "true"),
            ("integer", "42"),
            ("float", "4.2"),
            ("sequence", "[unexpected]"),
            ("mapping", "{unexpected: value}"),
        ] {
            let yaml = match scope {
                "required" => http_error_response_fixture_with_required_constant(declaration),
                "optional" => http_error_response_fixture_with_optional_constant(declaration),
                _ => unreachable!("test scope is fixed"),
            };
            let fixture =
                write_http_error_response_loader_fixture(&temp, &format!("{scope}-{name}"), &yaml);
            let error = hydra_core::load_api_definition(&fixture).unwrap_err();
            let rendered = format!("{error:#}");
            assert!(
                rendered.contains(&fixture.display().to_string()),
                "{scope}/{name} error must identify the definition source: {rendered}"
            );
            assert!(
                rendered.contains("const must be a string when supplied"),
                "{scope}/{name} error must identify const: {rendered}"
            );
            assert!(
                rendered.contains(&format!(
                    "operation read_events HTTP error response rate_limited field {field_name}"
                )),
                "{scope}/{name} error must identify the operation, response, and field: {rendered}"
            );
        }
    }

    let optional_string = write_http_error_response_loader_fixture(
        &temp,
        "optional-string",
        &http_error_response_fixture_with_optional_constant("fixed"),
    );
    let error = hydra_core::load_api_definition(&optional_string)
        .expect_err("optional string constants remain invalid by contract");
    let rendered = format!("{error:#}");
    assert!(
        rendered.contains(&optional_string.display().to_string()),
        "optional string error must identify the definition source: {rendered}"
    );
    assert!(rendered.contains(
        "operation read_events HTTP error response rate_limited field retry_hint declares const but is optional"
    ));
}

#[test]
fn http_error_response_validation_rejects_ambiguous_or_malformed_declarations() {
    let mut definition = http_error_responses_definition();
    definition.operations[0].http_error_responses[0].status = 399;
    assert!(hydra_core::validate::validate_definition(&definition).is_err());

    let mut definition = http_error_responses_definition();
    definition.operations[0].http_error_responses[0].status = 600;
    assert!(hydra_core::validate::validate_definition(&definition).is_err());

    let mut definition = http_error_responses_definition();
    definition.operations[0].http_error_responses[0].fields[2].constant = Some("never".into());
    let error = hydra_core::validate::validate_definition(&definition)
        .expect_err("optional constants must be rejected");
    assert!(
        error
            .to_string()
            .contains("constants require required: true")
    );

    let mut definition = http_error_responses_definition();
    let duplicate = definition.operations[0].http_error_responses[0].clone();
    definition.operations[0]
        .http_error_responses
        .push(duplicate);
    assert!(hydra_core::validate::validate_definition(&definition).is_err());

    let mut definition = http_error_responses_definition();
    definition.operations[1].http_error_responses[0].name = "a_b".into();
    definition.operations[1].http_error_responses[1].name = "a__b".into();
    let error = hydra_core::validate::validate_definition(&definition)
        .expect_err("emitted PascalCase response types must not collide");
    assert!(
        error
            .to_string()
            .contains("collide after generated type casing")
    );

    let mut definition = http_error_responses_definition();
    definition.operations[0].surfaces = Some(vec![hydra_core::Surface::Cli]);
    let error = hydra_core::validate::validate_definition(&definition)
        .expect_err("HTTP errors without HTTP generation must fail");
    assert!(
        error
            .to_string()
            .contains("does not generate the HTTP surface")
    );

    let mut definition = http_error_responses_definition();
    let mut colliding_operation = definition.operations[0].clone();
    colliding_operation.name = "read_events_http_errors".into();
    colliding_operation.path = "/other-events".into();
    colliding_operation.http_error_responses.clear();
    definition.operations.push(colliding_operation);
    let error = hydra_core::validate::validate_definition(&definition)
        .expect_err("error module must not collide with a generated handler");
    assert!(
        error
            .to_string()
            .contains("collides with an existing generated HTTP item")
    );

    let mut definition = http_error_responses_definition();
    let mut same_status = definition.operations[0].http_error_responses[0].clone();
    same_status.name = "another_invalid_cursor".into();
    definition.operations[0]
        .http_error_responses
        .push(same_status);
    hydra_core::validate::validate_definition(&definition)
        .expect("differently named errors may intentionally share a status");
}

#[test]
fn http_error_response_yaml_rejects_unknown_keys_and_unsupported_types() {
    let unknown_key = r"
operations:
  - name: status
    description: Read status.
    method: GET
    path: /status
    read: true
    output_type: Status
    parameters: []
    http_error_responses:
      - name: unavailable
        status: 503
        fields:
          - name: error
            type: string
            required: true
            const: unavailable
            unexpected: rejected
";
    assert!(
        serde_yaml::from_str::<ApiDefinition>(unknown_key).is_err(),
        "new HTTP-error types must reject unknown YAML keys"
    );

    let unsupported_type = r"
operations:
  - name: status
    description: Read status.
    method: GET
    path: /status
    read: true
    output_type: Status
    parameters: []
    http_error_responses:
      - name: unavailable
        status: 503
        fields:
          - name: retry_after
            type: integer
            required: true
";
    assert!(
        serde_yaml::from_str::<ApiDefinition>(unsupported_type).is_err(),
        "v1 error fields support only declared string values"
    );
}
