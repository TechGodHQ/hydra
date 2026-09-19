//! Hydra core: the API definition model shared by the code generator and
//! generated surfaces.
//!
//! An [`ApiDefinition`] is the single source of truth for a project's
//! operations. Hydra projects it onto three surfaces — CLI (clap), HTTP
//! (axum), and MCP (tool schemas + stdio runtime) — without name-based
//! inference: every route, parameter location, and surface allowlist is
//! declared explicitly, so the generated contract cannot drift from the
//! generated router.
//!
//! This model is extracted from iris's `iris-codegen` and generalized for
//! reuse across `TechGodHQ` projects.

use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize, de::Error as DeError};

pub mod paths;
pub mod validate;

/// Default path to a project's API definition file.
pub const DEFAULT_DEFINITION_PATH: &str = "api/operations.yaml";

/// Default directory for committed generated artifacts.
pub const DEFAULT_GENERATED_DIR: &str = "generated";

/// Operation names that would collide with identifiers emitted by the
/// generated HTTP module.
pub const GENERATED_HTTP_RESERVED_NAMES: &[&str] = &[
    "generated_router",
    "generated_route",
    "generated_operation_input",
    "state",
    "path",
    "query",
    "body",
    "get",
    "post",
    "router",
    "value",
    "response",
];

/// Rust 2024 keywords rejected as operation/parameter identifiers.
pub const RUST_KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final", "macro",
    "override", "priv", "try", "typeof", "unsized", "virtual", "yield", "gen",
];

/// An API definition: the operations a project exposes on its surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ApiDefinition {
    /// Operations exposed by the project.
    pub operations: Vec<Operation>,
}

/// A single operation exposed through CLI, HTTP, and MCP.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Operation {
    /// Stable `snake_case` operation name.
    pub name: String,
    /// Human-readable operation description. Flows to CLI help, OpenAPI-style
    /// docs, and MCP tool descriptions.
    pub description: String,
    /// HTTP method used by the REST surface.
    pub method: HttpMethod,
    /// HTTP path used by the REST surface. Path placeholders like
    /// `{thread_id}` must have matching path-location parameters.
    pub path: String,
    /// Whether this is a read-only operation. Reads must be GET, writes POST.
    pub read: bool,
    /// Rust-ish output type documentation for generated surfaces.
    pub output_type: String,
    /// Input parameters for the operation.
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    /// Delivery kind for the response. `sse` marks a streaming operation
    /// whose HTTP surface is a Server-Sent Events stream; the default
    /// `unary` behavior is a single JSON response.
    #[serde(default)]
    pub delivery: Delivery,
    /// Explicit surface allowlist. When present, only the listed surfaces are
    /// generated; when absent, all surfaces (HTTP, CLI, MCP) are generated.
    /// `delivery: sse` requires `http` to be listed and excludes MCP.
    #[serde(default)]
    pub surfaces: Option<Vec<Surface>>,
    /// Override for the generated CLI subcommand name (kebab-case). Defaults
    /// to the operation name. Used where the public CLI contract names a
    /// command differently from the operation.
    #[serde(default)]
    pub cli_command: Option<String>,
    /// Explicit CLI-only boolean presentation flags. These fields are
    /// generated only into the CLI argument struct; they never become HTTP
    /// inputs, MCP inputs, or entries in `parameters_json()`.
    #[serde(default)]
    pub cli_output_flags: Vec<CliOutputFlag>,
    /// Explicit typed HTTP error responses generated for this operation.
    ///
    /// Error declarations affect only the generated HTTP artifact. They give
    /// an existing runtime binding fixed status/body constructors without
    /// changing request inputs, CLI arguments, or MCP schemas.
    #[serde(default)]
    pub http_error_responses: Vec<HttpErrorResponse>,
    /// Opt in to raw-request access on the HTTP surface. The generated
    /// handler receives the exact raw body bytes and a header map instead
    /// of decoded/typed extractors, for consumers that verify signatures
    /// (e.g. webhook HMAC) over the wire representation. Default behavior
    /// (flag absent) is unchanged. Raw-request operations must list `http`
    /// as their only surface, stay unary, and declare no body-location
    /// parameters. The header map lowercases names, drops non-UTF-8
    /// values, and collapses repeated headers to the last value.
    #[serde(default)]
    pub raw_request: bool,
}

/// Response delivery kind for a generated operation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Delivery {
    /// Unary JSON request/response.
    #[default]
    Unary,
    /// Server-Sent Events stream (`text/event-stream`).
    Sse,
}

/// A generated surface an operation may be exposed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Surface {
    /// The HTTP REST surface.
    Http,
    /// The CLI command surface.
    Cli,
    /// The MCP tool surface.
    Mcp,
}

impl Operation {
    /// Whether this operation emits a streaming SSE response.
    #[must_use]
    pub const fn is_sse(&self) -> bool {
        matches!(self.delivery, Delivery::Sse)
    }

    /// Whether this operation opts into raw-request access on the HTTP
    /// surface (exact body bytes + headers instead of typed extraction).
    #[must_use]
    pub const fn is_raw_request(&self) -> bool {
        self.raw_request
    }

    /// Whether the HTTP surface is generated for this operation.
    #[must_use]
    pub fn generates_http(&self) -> bool {
        self.surfaces
            .as_ref()
            .is_none_or(|surfaces| surfaces.contains(&Surface::Http))
    }

    /// Whether the CLI surface is generated for this operation.
    #[must_use]
    pub fn generates_cli(&self) -> bool {
        self.surfaces
            .as_ref()
            .is_none_or(|surfaces| surfaces.contains(&Surface::Cli))
    }

    /// Whether the MCP surface is generated for this operation.
    #[must_use]
    pub fn generates_mcp(&self) -> bool {
        self.surfaces
            .as_ref()
            .is_none_or(|surfaces| surfaces.contains(&Surface::Mcp))
    }
}

/// HTTP method for a generated REST operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    /// HTTP GET.
    Get,
    /// HTTP POST.
    Post,
}

impl HttpMethod {
    /// Uppercase wire name (`GET` / `POST`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

/// A typed operation parameter.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Parameter {
    /// Stable `snake_case` parameter name.
    pub name: String,
    /// Human-readable parameter description.
    pub description: String,
    /// Logical type name.
    #[serde(rename = "type")]
    pub ty: ParameterType,
    /// Whether callers must provide this parameter.
    pub required: bool,
    /// Where this parameter appears in HTTP requests.
    pub location: ParameterLocation,
    /// JSON Schema describing the wire shape of a `json` parameter.
    ///
    /// Declared explicitly — never inferred — and embedded verbatim (plus
    /// the parameter description) into generated MCP tool input schemas.
    /// Required for `json` parameters, forbidden for scalar ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,
    /// Explicit CLI representation override. Declared — never inferred —
    /// for parameters whose CLI shape differs from the default single
    /// `--name` flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli: Option<CliOverride>,
}

/// Explicit CLI representation for a parameter.
///
/// The default CLI shape is one `--name` flag bound to the parameter.
/// When that is wrong (repeatable flags, companion flags that only make
/// sense on the CLI), the definition declares it here. Everything is
/// explicit: flag names, field names, and descriptions.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CliOverride {
    /// Kebab-case flag name override. Defaults to the parameter name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flag: Option<String>,
    /// Repeatable flag → `Vec<String>` clap field. `json` parameters only.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub multiple: bool,
    /// Additional CLI-only repeatable string flags (e.g. `--attach-mime`).
    /// Emitted as `Option<Vec<String>>` clap fields; never part of the
    /// HTTP or MCP surface.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub companions: Vec<CliCompanion>,
}

/// A CLI-only companion flag paired with a parameter's own CLI flags.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CliCompanion {
    /// Kebab-case flag name, e.g. `attach-mime`.
    pub flag: String,
    /// Stable `snake_case` struct field name, e.g. `attach_mime`.
    pub field: String,
    /// Human-readable description for CLI help.
    pub description: String,
}

/// An explicit CLI-only boolean presentation flag declared by an operation.
///
/// Output flags are intentionally separate from [`Parameter`] and
/// [`CliCompanion`]: they are not request data and therefore do not flow into
/// HTTP, MCP, or the generated `parameters_json()` value. V1 is boolean-only;
/// callers access the generated `bool` field to select their own presentation
/// behavior.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CliOutputFlag {
    /// Kebab-case long flag name without a leading `--`.
    pub flag: String,
    /// Stable Rust-safe `snake_case` generated field name.
    pub field: String,
    /// Human-readable help text emitted with the generated field.
    pub description: String,
}

/// One fixed HTTP error response declared by an operation.
///
/// The response name becomes a generated constructor inside that operation's
/// HTTP-error module. Its status and body shape are declaration data rather
/// than runtime guesses; v1 supports only flat string fields.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HttpErrorResponse {
    /// Rust-safe `snake_case` constructor name.
    pub name: String,
    /// HTTP status code emitted by the generated response (400 through 599).
    pub status: u16,
    /// Ordered flat JSON-object fields for this response.
    pub fields: Vec<HttpErrorField>,
}

/// One declared field in an [`HttpErrorResponse`] body.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HttpErrorField {
    /// Rust-safe `snake_case` field name and exact JSON key.
    pub name: String,
    /// The supported v1 field type.
    #[serde(rename = "type")]
    pub ty: HttpErrorFieldType,
    /// Whether this field is always present in the generated JSON object.
    pub required: bool,
    /// A fixed string value supplied by the declaration rather than callers.
    ///
    /// Constants are permitted only for required fields, so optional fields
    /// retain their `Option<String>` omission semantics.
    #[serde(
        rename = "const",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_http_error_constant"
    )]
    pub constant: Option<String>,
}

/// Deserialize a declared HTTP-error constant without collapsing an explicit
/// YAML null into an omitted declaration.
///
/// `default` supplies `None` only when the `const` key is absent. A supplied
/// key must deserialize as a string, so YAML null and every non-string node
/// fail at the definition-loader boundary rather than becoming a dynamic
/// generated constructor argument.
fn deserialize_http_error_constant<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    match serde_yaml::Value::deserialize(deserializer)? {
        serde_yaml::Value::String(value) => Ok(Some(value)),
        _ => Err(D::Error::custom(
            "HTTP error field const must be a string when supplied",
        )),
    }
}

/// Field types supported by typed HTTP error responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpErrorFieldType {
    /// UTF-8 string field.
    String,
}

impl CliOverride {
    /// Effective flag name: the declared override or the parameter name.
    #[must_use]
    pub fn effective_flag(&self, parameter_name: &str) -> String {
        self.flag
            .clone()
            .unwrap_or_else(|| parameter_name.to_owned())
    }
}

/// Parameter type supported by the first-generation codegen contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterType {
    /// UTF-8 string.
    String,
    /// Unsigned 32-bit integer.
    U32,
    /// Boolean flag.
    Bool,
    /// Arbitrary JSON value on the body location. Requires a declared
    /// `schema`, which flows into generated MCP tool input schemas; the
    /// HTTP body already arrives as an untyped JSON value, so the
    /// runtime owns validation. `json` parameters on CLI-generating
    /// operations must declare a `cli:` representation override.
    Json,
}

impl ParameterType {
    /// Rust type used in generated CLI argument structs.
    #[must_use]
    pub const fn rust_type(self) -> &'static str {
        match self {
            Self::String => "String",
            Self::U32 => "u32",
            Self::Bool => "bool",
            Self::Json => "serde_json::Value",
        }
    }

    /// JSON Schema type used in generated MCP tool schemas.
    #[must_use]
    pub const fn json_schema_type(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::U32 => "integer",
            Self::Bool => "boolean",
            Self::Json => "object",
        }
    }
}

/// Parameter location for HTTP and generated surface mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ParameterLocation {
    /// Path parameter.
    Path,
    /// Query-string parameter.
    Query,
    /// JSON body parameter.
    Body,
}

/// Load an API definition from YAML.
pub fn load_api_definition(path: impl AsRef<Path>) -> Result<ApiDefinition> {
    let path = path.as_ref();
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read API definition from {}", path.display()))?;
    let raw_definition: serde_yaml::Value = serde_yaml::from_str(&raw)
        .with_context(|| format!("parse API definition from {}", path.display()))?;
    validate_http_error_constant_declarations(&raw_definition)
        .with_context(|| format!("validate HTTP error constants in {}", path.display()))?;
    let definition: ApiDefinition = serde_yaml::from_str(&raw)
        .with_context(|| format!("parse API definition from {}", path.display()))?;
    validate::validate_definition(&definition)
        .with_context(|| format!("validate API definition from {}", path.display()))?;
    Ok(definition)
}

/// Reject supplied non-string HTTP-error constants with declaration names.
///
/// The public model intentionally keeps `constant` as `Option<String>` for
/// programmatic construction. Inspecting the raw YAML before typed
/// deserialization preserves the otherwise-lost distinction between an absent
/// key and an explicit YAML null while giving users named operation, response,
/// and field diagnostics.
fn validate_http_error_constant_declarations(definition: &serde_yaml::Value) -> Result<()> {
    let Some(root) = definition.as_mapping() else {
        return Ok(());
    };
    let Some(operations) =
        yaml_mapping_value(root, "operations").and_then(serde_yaml::Value::as_sequence)
    else {
        return Ok(());
    };

    for (operation_index, operation) in operations.iter().enumerate() {
        let Some(operation) = operation.as_mapping() else {
            continue;
        };
        let operation_name = yaml_mapping_value(operation, "name")
            .and_then(serde_yaml::Value::as_str)
            .map_or_else(
                || format!("operations[{operation_index}]"),
                ToOwned::to_owned,
            );
        let Some(responses) = yaml_mapping_value(operation, "http_error_responses")
            .and_then(serde_yaml::Value::as_sequence)
        else {
            continue;
        };

        for (response_index, response) in responses.iter().enumerate() {
            let Some(response) = response.as_mapping() else {
                continue;
            };
            let response_name = yaml_mapping_value(response, "name")
                .and_then(serde_yaml::Value::as_str)
                .map_or_else(
                    || format!("http_error_responses[{response_index}]"),
                    ToOwned::to_owned,
                );
            let Some(fields) =
                yaml_mapping_value(response, "fields").and_then(serde_yaml::Value::as_sequence)
            else {
                continue;
            };

            for (field_index, field) in fields.iter().enumerate() {
                let Some(field) = field.as_mapping() else {
                    continue;
                };
                let field_name = yaml_mapping_value(field, "name")
                    .and_then(serde_yaml::Value::as_str)
                    .map_or_else(|| format!("fields[{field_index}]"), ToOwned::to_owned);
                if let Some(constant) = yaml_mapping_value(field, "const")
                    && !matches!(constant, serde_yaml::Value::String(_))
                {
                    anyhow::bail!(
                        "operation {operation_name} HTTP error response {response_name} field {field_name} declares const with a non-string value; const must be a string when supplied"
                    );
                }
            }
        }
    }

    Ok(())
}

/// Look up a string key in a YAML mapping without coercing YAML values.
fn yaml_mapping_value<'a>(
    mapping: &'a serde_yaml::Mapping,
    key: &str,
) -> Option<&'a serde_yaml::Value> {
    mapping.get(serde_yaml::Value::String(key.to_owned()))
}
