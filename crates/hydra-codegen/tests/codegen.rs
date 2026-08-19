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
            }],
            delivery: Delivery::Unary,
            surfaces: None,
            cli_command: None,
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
