//! Validation for API definitions: the guardrail layer that keeps generated
//! contracts honest.
//!
//! Every rule here exists because a generated surface once lied (see the
//! server-less spike notes in iris) — path placeholders must match declared
//! parameters, names must be collision-free, and surface allowlists must be
//! self-consistent.

use anyhow::Result;

use crate::{
    ApiDefinition, GENERATED_HTTP_RESERVED_NAMES, HttpMethod, Operation, ParameterLocation,
    RUST_KEYWORDS, Surface,
};

/// Validate semantic constraints that YAML parsing alone cannot enforce.
pub fn validate_definition(definition: &ApiDefinition) -> Result<()> {
    anyhow::ensure!(
        !definition.operations.is_empty(),
        "API definition must contain at least one operation"
    );

    let mut names = std::collections::BTreeSet::new();
    for operation in &definition.operations {
        anyhow::ensure!(
            is_valid_identifier(&operation.name),
            "operation name must be a Rust-safe snake_case identifier: {}",
            operation.name
        );
        anyhow::ensure!(
            names.insert(operation.name.as_str()),
            "duplicate operation name: {}",
            operation.name
        );
        anyhow::ensure!(
            !GENERATED_HTTP_RESERVED_NAMES.contains(&operation.name.as_str()),
            "operation name is reserved by generated HTTP module: {}",
            operation.name
        );
        anyhow::ensure!(
            operation.path.starts_with('/'),
            "operation {} path must start with /",
            operation.name
        );
        anyhow::ensure!(
            matches!(
                (operation.read, operation.method),
                (true, HttpMethod::Get) | (false, HttpMethod::Post)
            ),
            "operation {} read/method mismatch: reads must be GET, writes must be POST",
            operation.name
        );
        validate_operation_parameters(operation)?;
        validate_operation_surfaces(operation)?;
    }

    // CLI command overrides must not collide with any generated subcommand.
    let mut cli_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for operation in &definition.operations {
        if !operation.generates_cli() {
            continue;
        }
        let name = operation
            .cli_command
            .clone()
            .unwrap_or_else(|| operation.name.clone());
        anyhow::ensure!(
            cli_names.insert(name),
            "operation {} declares a CLI command name that collides with another operation",
            operation.name
        );
    }

    Ok(())
}

/// Validate parameter names and path/parameter consistency for one operation.
fn validate_operation_parameters(operation: &Operation) -> Result<()> {
    let mut parameter_names = std::collections::BTreeSet::new();
    for parameter in &operation.parameters {
        anyhow::ensure!(
            is_valid_identifier(&parameter.name),
            "parameter name must be a Rust-safe snake_case identifier: {}.{}",
            operation.name,
            parameter.name
        );
        anyhow::ensure!(
            parameter_names.insert(parameter.name.as_str()),
            "duplicate parameter name: {}.{}",
            operation.name,
            parameter.name
        );
    }

    let path_parameters = crate::paths::path_parameters(&operation.path)?;
    for path_parameter in &path_parameters {
        anyhow::ensure!(
            operation
                .parameters
                .iter()
                .any(|parameter| parameter.name == *path_parameter
                    && parameter.location == ParameterLocation::Path),
            "operation {} path placeholder {{{}}} has no matching path parameter",
            operation.name,
            path_parameter
        );
    }
    for parameter in operation
        .parameters
        .iter()
        .filter(|parameter| parameter.location == ParameterLocation::Path)
    {
        anyhow::ensure!(
            path_parameters
                .iter()
                .any(|path_parameter| path_parameter == &parameter.name),
            "operation {} path parameter {} is not present in path {}",
            operation.name,
            parameter.name,
            operation.path
        );
    }
    Ok(())
}

/// Validate surface allowlists, delivery-kind rules, and CLI command names.
fn validate_operation_surfaces(operation: &Operation) -> Result<()> {
    if let Some(surfaces) = &operation.surfaces {
        anyhow::ensure!(
            !surfaces.is_empty(),
            "operation {} declares an empty surfaces list",
            operation.name
        );
        let mut unique = std::collections::BTreeSet::new();
        for surface in surfaces.iter().map(|s| format!("{s:?}")) {
            anyhow::ensure!(
                unique.insert(surface),
                "operation {} lists a surface more than once",
                operation.name
            );
        }
    }
    if operation.is_sse() {
        anyhow::ensure!(
            operation.generates_http(),
            "operation {} uses delivery: sse but does not list the http surface",
            operation.name
        );
        anyhow::ensure!(
            operation.method == HttpMethod::Get,
            "operation {} uses delivery: sse but is not a GET",
            operation.name
        );
        anyhow::ensure!(
            !operation.generates_mcp(),
            "operation {} uses delivery: sse but lists the mcp surface; \
             SSE operations are excluded from MCP generation",
            operation.name
        );
    }
    if let Some(cli_command) = &operation.cli_command {
        anyhow::ensure!(
            !cli_command.is_empty(),
            "operation {} declares an empty cli_command",
            operation.name
        );
        anyhow::ensure!(
            is_kebab_case(cli_command),
            "operation {} declares cli_command {cli_command:?} which is not kebab-case",
            operation.name
        );
    }
    let _ = Surface::Http; // surface enum is part of the public contract
    Ok(())
}

/// Whether a value is lowercase kebab-case (letters, digits, single hyphens).
fn is_kebab_case(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub(crate) fn is_valid_identifier(value: &str) -> bool {
    is_snake_case(value) && !value.as_bytes()[0].is_ascii_digit() && !RUST_KEYWORDS.contains(&value)
}

fn is_snake_case(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && !value.starts_with('_')
        && !value.ends_with('_')
}
