//! Hydra code generator: projects an [`ApiDefinition`] onto three committed
//! artifacts — CLI structs (clap), HTTP routes (axum), and MCP tool schemas
//! (JSON) — from one explicit source of truth. No name-based inference: the
//! definition declares method, path, parameter locations, and surface
//! allowlists, and all three surfaces emit from that single declaration.

use std::fmt::Write as _;
use std::{fs, path::Path};

use anyhow::{Context, Result};
use hydra_core::{
    ApiDefinition, Delivery, HttpMethod, Operation, Parameter, ParameterLocation, Surface,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Append a formatted chunk to the output string (pedantic-clean wrapper).
macro_rules! push_fmt {
    ($out:expr, $($arg:tt)*) => {
        let _ = write!($out, $($arg)*);
    };
}

/// Generation options: the per-project knobs that iris hard-coded.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct GenerateConfig {
    /// Where generated handlers send operation inputs, e.g.
    /// `super::execute_generated_operation`. Emits
    /// `<target>(&state, "<op>", GeneratedOperationInput { .. })`.
    pub http_dispatch_fn: String,
    /// The axum state type generated handlers accept, e.g. `crate::app::AppState`.
    pub http_state_type: String,
    /// Where the SSE binding hooks live, e.g. `super::`.
    #[serde(default = "default_sse_binding_prefix")]
    pub sse_binding_prefix: String,
    /// Header line identifying the generator in committed artifacts.
    #[serde(default = "default_generator_name")]
    pub generator_name: String,
}

fn default_sse_binding_prefix() -> String {
    "super::".to_string()
}

fn default_generator_name() -> String {
    "hydra".to_string()
}

impl Default for GenerateConfig {
    fn default() -> Self {
        Self {
            http_dispatch_fn: "super::execute_generated_operation".to_string(),
            http_state_type: "crate::app::AppState".to_string(),
            sse_binding_prefix: default_sse_binding_prefix(),
            generator_name: default_generator_name(),
        }
    }
}

/// Generate all committed artifacts for a definition.
#[must_use]
pub fn generate_all(definition: &ApiDefinition, config: &GenerateConfig) -> GeneratedArtifacts {
    GeneratedArtifacts {
        cli_rs: generate_cli(definition),
        http_rs: generate_http(definition, config),
        mcp_json: generate_mcp(definition),
    }
}

/// Generated artifact bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedArtifacts {
    /// Generated clap command metadata/structs.
    pub cli_rs: String,
    /// Generated axum route metadata and handlers.
    pub http_rs: String,
    /// Generated MCP tool schema JSON.
    pub mcp_json: String,
}

/// Write generated artifacts under a directory.
pub fn write_generated(dir: impl AsRef<Path>, artifacts: &GeneratedArtifacts) -> Result<()> {
    let dir = dir.as_ref();
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    write_if_changed(dir.join("cli.rs"), &artifacts.cli_rs)?;
    write_if_changed(dir.join("http.rs"), &artifacts.http_rs)?;
    write_if_changed(dir.join("mcp.json"), &artifacts.mcp_json)?;
    Ok(())
}

/// Verify committed generated artifacts match the API definition.
pub fn verify_generated(
    definition_path: impl AsRef<Path>,
    generated_dir: impl AsRef<Path>,
    config: &GenerateConfig,
) -> Result<()> {
    let definition = hydra_core::load_api_definition(definition_path)?;
    let expected = generate_all(&definition, config);
    let dir = generated_dir.as_ref();

    compare_file(dir.join("cli.rs"), &expected.cli_rs)?;
    compare_file(dir.join("http.rs"), &expected.http_rs)?;
    compare_file(dir.join("mcp.json"), &expected.mcp_json)?;
    Ok(())
}

// ── CLI surface ────────────────────────────────────────────────────────────

fn generate_cli(definition: &ApiDefinition) -> String {
    let mut out = generated_header("CLI command structs generated from the API definition");
    out.push_str("use clap::{Args, Subcommand};\n");
    out.push_str("use serde::{Deserialize, Serialize};\n\n");
    out.push_str("#[derive(Debug, Clone, Subcommand)]\n");
    out.push_str("pub enum GeneratedCommand {\n");
    for operation in cli_operations(definition) {
        push_doc_comment(&mut out, "    ", &operation.description);
        out.push_str("    ");
        out.push_str(&cli_variant_name(operation));
        out.push('(');
        out.push_str(&cli_variant_name(operation));
        out.push_str("Args),\n");
    }
    out.push_str("}\n\n");

    out.push_str("impl GeneratedCommand {\n");
    out.push_str("    pub const fn operation_name(&self) -> &'static str {\n");
    out.push_str("        match self {\n");
    for operation in cli_operations(definition) {
        out.push_str("            Self::");
        out.push_str(&cli_variant_name(operation));
        out.push_str("(_) => ");
        out.push_str(&rust_string_literal(&operation.name));
        out.push_str(",\n");
    }
    out.push_str("        }\n");
    out.push_str("    }\n\n");
    out.push_str("    pub fn parameters_json(&self) -> serde_json::Value {\n");
    out.push_str("        match self {\n");
    for operation in cli_operations(definition) {
        out.push_str("            Self::");
        out.push_str(&cli_variant_name(operation));
        // Underscore the binding only for zero-parameter operations so the
        // generated file compiles warning-free.
        let binder = if operation.parameters.is_empty() {
            "_args"
        } else {
            "args"
        };
        out.push('(');
        out.push_str(binder);
        out.push_str(") => serde_json::json!({");
        for (index, parameter) in operation.parameters.iter().enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            out.push_str(&rust_string_literal(&parameter.name));
            out.push_str(": args.");
            out.push_str(&parameter.name);
            if matches!(parameter.ty, hydra_core::ParameterType::String) || !parameter.required {
                out.push_str(".clone()");
            }
        }
        out.push_str("}),\n");
    }
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");

    for operation in cli_operations(definition) {
        out.push_str("#[derive(Debug, Clone, Serialize, Deserialize, Args)]\n");
        out.push_str("pub struct ");
        out.push_str(&cli_variant_name(operation));
        out.push_str("Args {\n");
        for parameter in &operation.parameters {
            push_doc_comment(&mut out, "    ", &parameter.description);
            if parameter.location != ParameterLocation::Path {
                out.push_str("    #[arg(long)]\n");
            }
            out.push_str("    pub ");
            out.push_str(&parameter.name);
            out.push_str(": ");
            if parameter.required {
                out.push_str(parameter.ty.rust_type());
            } else {
                out.push_str("Option<");
                out.push_str(parameter.ty.rust_type());
                out.push('>');
            }
            out.push_str(",\n");
        }
        out.push_str("}\n\n");
    }

    out
}

fn cli_operations(definition: &ApiDefinition) -> impl Iterator<Item = &Operation> {
    definition.operations.iter().filter(|o| o.generates_cli())
}

// ── HTTP surface ───────────────────────────────────────────────────────────

fn generate_http(definition: &ApiDefinition, config: &GenerateConfig) -> String {
    let mut out = generated_header("HTTP route handlers generated from the API definition");
    out.push_str("use std::collections::BTreeMap;\n\n");
    // Emit only the extractor/method imports the definition actually uses.
    let unary_ops: Vec<&Operation> = http_operations(definition)
        .filter(|operation| !operation.is_sse())
        .collect();
    let any_path = unary_ops.iter().any(|o| {
        o.parameters
            .iter()
            .any(|p| p.location == ParameterLocation::Path)
    });
    let any_query = unary_ops.iter().any(|o| {
        o.parameters
            .iter()
            .any(|p| p.location == ParameterLocation::Query)
    });
    let any_body = unary_ops.iter().any(|o| {
        o.parameters
            .iter()
            .any(|p| p.location == ParameterLocation::Body)
    });
    let any_get = unary_ops.iter().any(|o| o.method == HttpMethod::Get);
    let any_post = unary_ops.iter().any(|o| o.method == HttpMethod::Post);
    let mut extractors = vec!["State"];
    if any_path {
        extractors.push("Path");
    }
    if any_query {
        extractors.push("Query");
    }
    let mut methods = Vec::new();
    if any_get {
        methods.push("get");
    }
    if any_post {
        methods.push("post");
    }
    push_fmt!(
        out,
        "use axum::{{extract::{{{}}}, response::Response, routing::{{{}}}, Router}};\n",
        extractors.join(", "),
        methods.join(", ")
    );
    if any_body {
        out.push_str("use axum::Json;\n");
    }
    out.push_str("use serde_json::Value;\n\n");
    out.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n");
    out.push_str("pub struct GeneratedRoute {\n");
    out.push_str("    pub name: &'static str,\n");
    out.push_str("    pub method: &'static str,\n");
    out.push_str("    pub path: &'static str,\n");
    out.push_str("}\n\n");
    out.push_str("#[derive(Debug, Clone, Default, PartialEq, Eq)]\n");
    out.push_str("pub struct GeneratedOperationInput {\n");
    out.push_str("    pub path: BTreeMap<String, String>,\n");
    out.push_str("    pub query: BTreeMap<String, String>,\n");
    out.push_str("    pub body: Value,\n");
    out.push_str("}\n\n");
    out.push_str("pub const GENERATED_ROUTES: &[GeneratedRoute] = &[\n");
    for operation in http_operations(definition).filter(|o| !o.is_sse()) {
        out.push_str("    GeneratedRoute { name: ");
        out.push_str(&rust_string_literal(&operation.name));
        out.push_str(", method: ");
        out.push_str(&rust_string_literal(operation.method.as_str()));
        out.push_str(", path: ");
        out.push_str(&rust_string_literal(&operation.path));
        out.push_str(" },\n");
    }
    out.push_str("];\n\n");
    push_fmt!(
        out,
        "pub fn generated_router() -> Router<{}> {{\n",
        config.http_state_type
    );
    out.push_str("    Router::new()\n");
    for operation in http_operations(definition).filter(|o| !o.is_sse()) {
        out.push_str("        .route(");
        out.push_str(&rust_string_literal(&operation.path));
        out.push_str(", ");
        match operation.method {
            HttpMethod::Get => out.push_str("get("),
            HttpMethod::Post => out.push_str("post("),
        }
        out.push_str(&operation.name);
        out.push_str("))\n");
    }
    out.push_str("}\n\n");

    out.push_str(&generate_sse_surface(definition, config));

    let unary: Vec<&Operation> = http_operations(definition)
        .filter(|operation| !operation.is_sse())
        .collect();
    for (index, operation) in unary.iter().enumerate() {
        push_unary_handler(&mut out, operation, config);
        if index + 1 < unary.len() {
            out.push('\n');
        }
    }

    out
}

/// Emit one unary axum handler for a non-streaming HTTP operation.
fn push_unary_handler(out: &mut String, operation: &Operation, config: &GenerateConfig) {
    let has_path = operation
        .parameters
        .iter()
        .any(|parameter| parameter.location == ParameterLocation::Path);
    let has_query = operation
        .parameters
        .iter()
        .any(|parameter| parameter.location == ParameterLocation::Query);
    let has_body = operation
        .parameters
        .iter()
        .any(|parameter| parameter.location == ParameterLocation::Body);

    out.push_str("async fn ");
    out.push_str(&operation.name);
    out.push_str("(\n");
    push_fmt!(
        out,
        "    State(state): State<{}>,\n",
        config.http_state_type
    );
    if has_path {
        out.push_str("    Path(path): Path<BTreeMap<String, String>>,\n");
    }
    if has_query {
        out.push_str("    Query(query): Query<BTreeMap<String, String>>,\n");
    }
    if has_body {
        out.push_str("    Json(body): Json<Value>,\n");
    }
    out.push_str(") -> Response {\n");
    push_fmt!(out, "    {}(\n", config.http_dispatch_fn);
    out.push_str("        &state,\n");
    out.push_str("        ");
    out.push_str(&rust_string_literal(&operation.name));
    out.push_str(",\n");
    out.push_str("        GeneratedOperationInput {\n");
    if has_path {
        out.push_str("            path,\n");
    } else {
        out.push_str("            path: BTreeMap::new(),\n");
    }
    if has_query {
        out.push_str("            query,\n");
    } else {
        out.push_str("            query: BTreeMap::new(),\n");
    }
    if has_body {
        out.push_str("            body,\n");
    } else {
        out.push_str("            body: Value::Null,\n");
    }
    out.push_str("        },\n");
    out.push_str("    )\n");
    out.push_str("    .await\n");
    out.push_str("}\n");
}

/// Emit the SSE surface: route metadata plus a named runtime binding hook per
/// streaming operation. The handwritten server binds the actual handler, so
/// no duplicate axum route exists in generated code.
fn generate_sse_surface(definition: &ApiDefinition, config: &GenerateConfig) -> String {
    let sse_operations: Vec<&Operation> = http_operations(definition)
        .filter(|operation| operation.is_sse())
        .collect();
    if sse_operations.is_empty() {
        return String::new();
    }
    let state_path = &config.http_state_type;
    let mut out = String::new();
    out.push_str("/// Streaming (SSE) operations declared in the API definition. The\n");
    out.push_str("/// generated surface exposes only this metadata plus the binding hooks\n");
    out.push_str("/// below; the handwritten server supplies the handler.\n");
    out.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n");
    out.push_str("pub struct GeneratedSseRoute {\n");
    out.push_str("    pub name: &'static str,\n");
    out.push_str("    pub method: &'static str,\n");
    out.push_str("    pub path: &'static str,\n");
    out.push_str("}\n\n");
    out.push_str("pub const GENERATED_SSE_ROUTES: &[GeneratedSseRoute] = &[\n");
    for operation in &sse_operations {
        out.push_str("    GeneratedSseRoute { name: ");
        out.push_str(&rust_string_literal(&operation.name));
        out.push_str(", method: ");
        out.push_str(&rust_string_literal(operation.method.as_str()));
        out.push_str(", path: ");
        out.push_str(&rust_string_literal(&operation.path));
        out.push_str(" },\n");
    }
    out.push_str("];\n\n");
    for operation in &sse_operations {
        out.push_str("/// Runtime binding hook for the `");
        out.push_str(&operation.name);
        out.push_str("` SSE operation. The handwritten server implements this\n");
        out.push_str("/// binding; generated code does not generate a duplicate route.\n");
        push_fmt!(
            out,
            "pub fn bind_{}(router: Router<{state_path}>) -> Router<{state_path}> {{\n",
            operation.name
        );
        push_fmt!(
            out,
            "    {}bind_runtime_sse_{}(router)\n",
            config.sse_binding_prefix,
            operation.name
        );
        out.push_str("}\n\n");
    }
    out
}

fn http_operations(definition: &ApiDefinition) -> impl Iterator<Item = &Operation> {
    definition.operations.iter().filter(|o| o.generates_http())
}

// ── MCP surface ────────────────────────────────────────────────────────────

fn generate_mcp(definition: &ApiDefinition) -> String {
    let tools: Vec<_> = definition
        .operations
        .iter()
        .filter(|operation| operation.generates_mcp() && !operation.is_sse())
        .map(|operation| {
            let mut required = Vec::new();
            let mut properties = serde_json::Map::new();
            for parameter in &operation.parameters {
                if parameter.required {
                    required.push(parameter.name.clone());
                }
                properties.insert(
                    parameter.name.clone(),
                    json!({
                        "type": parameter.ty.json_schema_type(),
                        "description": parameter.description,
                    }),
                );
            }
            json!({
                "name": operation.name,
                "description": operation.description,
                "inputSchema": {
                    "type": "object",
                    "properties": properties,
                    "required": required,
                    "additionalProperties": false,
                },
            })
        })
        .collect();

    // Parameter-location metadata so MCP adapters can route tool arguments
    // into path/query/body without inferring anything from names.
    let locations: serde_json::Map<String, Value> = definition
        .operations
        .iter()
        .filter(|operation| operation.generates_mcp() && !operation.is_sse())
        .map(|operation| {
            let params: serde_json::Map<String, Value> = operation
                .parameters
                .iter()
                .map(|parameter| {
                    (
                        parameter.name.clone(),
                        json!(match parameter.location {
                            ParameterLocation::Path => "path",
                            ParameterLocation::Query => "query",
                            ParameterLocation::Body => "body",
                        }),
                    )
                })
                .collect();
            (operation.name.clone(), Value::Object(params))
        })
        .collect();

    let value = json!({ "tools": tools, "locations": locations });
    let mut out = serde_json::to_string_pretty(&value).expect("serializing MCP JSON cannot fail");
    out.push('\n');
    out
}

// ── shared emission helpers ────────────────────────────────────────────────

fn generated_header(purpose: &str) -> String {
    format!("// Code generated by hydra. DO NOT EDIT.\n// {purpose}\n\n")
}

fn push_doc_comment(out: &mut String, indent: &str, text: &str) {
    for line in text.lines() {
        out.push_str(indent);
        out.push_str("/// ");
        out.push_str(line.trim());
        out.push('\n');
    }
}

fn write_if_changed(path: impl AsRef<Path>, content: &str) -> Result<()> {
    let path = path.as_ref();
    if fs::read_to_string(path).is_ok_and(|current| current == content) {
        return Ok(());
    }
    fs::write(path, content).with_context(|| format!("write {}", path.display()))
}

fn compare_file(path: impl AsRef<Path>, expected: &str) -> Result<()> {
    let path = path.as_ref();
    let actual = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    anyhow::ensure!(
        actual == expected,
        "generated artifact is stale: {} (run `hydra write`)",
        path.display()
    );
    Ok(())
}

fn rust_string_literal(value: &str) -> String {
    serde_json::to_string(value).expect("string literal serialization cannot fail")
}

fn cli_variant_name(operation: &Operation) -> String {
    pascal_case(
        &operation
            .cli_command
            .clone()
            .unwrap_or_else(|| operation.name.clone())
            .replace('-', "_"),
    )
}

fn pascal_case(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut upper_next = true;
    for ch in value.chars() {
        if ch == '_' {
            upper_next = true;
        } else if upper_next {
            out.extend(ch.to_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

// Keep unused-import checker satisfied for symbols referenced only from
// generated output or doc examples.
const _: Option<Surface> = None;
const _: Option<Delivery> = None;
const _: Option<&Parameter> = None;
