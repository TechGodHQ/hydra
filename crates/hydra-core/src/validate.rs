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
    ParameterType, RUST_KEYWORDS, Surface,
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
        validate_operation_raw_request(operation)?;
        validate_operation_cli_overrides(operation)?;
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

/// Validate json parameters, declared schemas, and CLI representation
/// overrides. Everything is declared explicitly — this layer exists so a
/// generated surface can never guess a parameter's CLI shape or MCP
/// schema.
fn validate_operation_cli_overrides(operation: &Operation) -> Result<()> {
    // Collect CLI field names for collision checks across parameters and
    // companions.
    let mut cli_fields: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut cli_flags: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for parameter in &operation.parameters {
        let label = format!("{}.{}", operation.name, parameter.name);

        // json parameters: body location + declared schema.
        if parameter.ty == ParameterType::Json {
            anyhow::ensure!(
                parameter.location == ParameterLocation::Body,
                "parameter {label} is type json but not located in the body; \
                 json parameters are body-only"
            );
            anyhow::ensure!(
                parameter
                    .schema
                    .as_ref()
                    .is_some_and(serde_json::Value::is_object),
                "parameter {label} is type json but declares no object `schema`; \
                 json parameters must declare their JSON Schema explicitly"
            );
        }
        anyhow::ensure!(
            parameter.ty != ParameterType::Json || parameter.schema.is_some(),
            "parameter {label} is type json but declares no `schema`"
        );
        anyhow::ensure!(
            parameter.ty == ParameterType::Json || parameter.schema.is_none(),
            "parameter {label} declares a `schema` but is not type json; \
             schemas are json-parameter-only"
        );

        let Some(cli) = &parameter.cli else {
            // No CLI override: a json parameter on a CLI-generating
            // operation would fall back to inference, which is forbidden.
            anyhow::ensure!(
                parameter.ty != ParameterType::Json || !operation.generates_cli(),
                "parameter {label} is type json on a CLI-generating operation \
                 but declares no `cli` representation; the CLI shape must be \
                 declared explicitly (flag/multiple/companions)"
            );
            continue;
        };

        anyhow::ensure!(
            operation.generates_cli(),
            "parameter {label} declares a `cli` representation but operation {} \
             does not generate the CLI surface",
            operation.name
        );

        if let Some(flag) = &cli.flag {
            anyhow::ensure!(
                is_kebab_case(flag),
                "parameter {label} declares cli flag {flag:?} which is not kebab-case"
            );
        }
        anyhow::ensure!(
            !cli.multiple || parameter.ty == ParameterType::Json,
            "parameter {label} declares cli multiple: true but is not type json; \
             repeatable flags are json-parameter-only"
        );

        // Effective flag + field bookkeeping.
        let effective_flag = cli.effective_flag(&parameter.name);
        anyhow::ensure!(
            cli_flags.insert(effective_flag.clone()),
            "operation {} declares colliding CLI flags: {effective_flag}",
            operation.name
        );
        anyhow::ensure!(
            cli_fields.insert(parameter.name.clone()),
            "operation {} declares colliding CLI fields: {}",
            operation.name,
            parameter.name
        );

        for companion in &cli.companions {
            anyhow::ensure!(
                is_kebab_case(&companion.flag),
                "parameter {label} companion declares flag {:?} which is not kebab-case",
                companion.flag
            );
            anyhow::ensure!(
                crate::validate::is_valid_identifier(&companion.field),
                "parameter {label} companion declares field {:?} which is not a \
                 Rust-safe snake_case identifier",
                companion.field
            );
            anyhow::ensure!(
                cli_flags.insert(companion.flag.clone()),
                "operation {} declares colliding CLI flags: {}",
                operation.name,
                companion.flag
            );
            anyhow::ensure!(
                cli_fields.insert(companion.field.clone()),
                "operation {} declares colliding CLI fields: {} (companion field)",
                operation.name,
                companion.field
            );
        }
    }
    Ok(())
}

/// Validate the raw-request escape hatch: it is an HTTP-surface-only
/// feature, so raw operations must not appear on CLI or MCP, must not
/// stream, and must not declare body-location parameters (the raw bytes
/// replace JSON body extraction).
fn validate_operation_raw_request(operation: &Operation) -> Result<()> {
    if !operation.is_raw_request() {
        return Ok(());
    }
    // `surfaces: None` means "all surfaces"; raw request access is an
    // HTTP-only escape hatch, so the allowlist must be exactly [http].
    anyhow::ensure!(
        operation
            .surfaces
            .as_ref()
            .is_some_and(|surfaces| surfaces.len() == 1 && surfaces.contains(&Surface::Http)),
        "operation {} uses raw_request but does not list exactly the http surface; \
         raw request access is http-only",
        operation.name
    );
    anyhow::ensure!(
        !operation.is_sse(),
        "operation {} uses raw_request with delivery: sse; \
         raw request access is unary-only",
        operation.name
    );
    anyhow::ensure!(
        operation
            .parameters
            .iter()
            .all(|parameter| parameter.location != ParameterLocation::Body),
        "operation {} uses raw_request but declares a body-location parameter; \
         the raw body bytes replace JSON body extraction",
        operation.name
    );
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
