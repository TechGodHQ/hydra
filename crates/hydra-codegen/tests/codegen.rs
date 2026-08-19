//! Ported from iris-codegen's test suite, extended for hydra's config knob
//! and location metadata.

use hydra_codegen::{GenerateConfig, generate_all};
use hydra_core::{ApiDefinition, Delivery, HttpMethod, Operation, Parameter, ParameterLocation};
use pretty_assertions::assert_eq;

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
