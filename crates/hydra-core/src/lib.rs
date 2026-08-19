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
use serde::{Deserialize, Serialize};

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

/// Rust keywords rejected as operation/parameter identifiers.
pub const RUST_KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while", "async", "await", "dyn",
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
    /// Opt in to raw-request access on the HTTP surface. The generated
    /// handler receives the exact raw body bytes and a header map instead
    /// of decoded/typed extractors, for consumers that verify signatures
    /// (e.g. webhook HMAC) over the wire representation. Default behavior
    /// (flag absent) is unchanged. Raw-request operations must list `http`
    /// as their only surface, stay unary, and declare no body-location
    /// parameters.
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
}

impl ParameterType {
    /// Rust type used in generated CLI argument structs.
    #[must_use]
    pub const fn rust_type(self) -> &'static str {
        match self {
            Self::String => "String",
            Self::U32 => "u32",
            Self::Bool => "bool",
        }
    }

    /// JSON Schema type used in generated MCP tool schemas.
    #[must_use]
    pub const fn json_schema_type(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::U32 => "integer",
            Self::Bool => "boolean",
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
    let definition: ApiDefinition = serde_yaml::from_str(&raw)
        .with_context(|| format!("parse API definition from {}", path.display()))?;
    validate::validate_definition(&definition)?;
    Ok(definition)
}
