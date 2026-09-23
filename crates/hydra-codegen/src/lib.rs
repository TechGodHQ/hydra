//! Hydra code generator: projects an [`ApiDefinition`] onto four committed
//! artifacts — CLI structs (clap), HTTP routes (axum), MCP tool schemas
//! (JSON), and a fetch-based TypeScript client — from one explicit source of
//! truth. No name-based inference: the definition declares method, path,
//! parameter locations, and surface allowlists, and every surface emits from
//! that single declaration.

use std::fmt::Write as _;
use std::{fs, path::Path};

use anyhow::{Context, Result};
use hydra_core::{
    ApiDefinition, Delivery, HttpErrorResponse, HttpMethod, Operation, Parameter,
    ParameterLocation, Surface,
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
    /// Where generated raw-request handlers send operation inputs, e.g.
    /// `super::execute_generated_raw_operation`. Only used when the
    /// definition contains `raw_request: true` operations. Emits
    /// `<target>(&state, "<op>", GeneratedRawOperationInput { .. })`.
    #[serde(default = "default_http_raw_dispatch_fn")]
    pub http_raw_dispatch_fn: String,
    /// Header line identifying the generator in committed artifacts.
    #[serde(default = "default_generator_name")]
    pub generator_name: String,
    /// Exported class name for the generated TypeScript client.
    #[serde(default = "default_ts_client_name")]
    pub ts_client_name: String,
}

fn default_sse_binding_prefix() -> String {
    "super::".to_string()
}

fn default_http_raw_dispatch_fn() -> String {
    "super::execute_generated_raw_operation".to_string()
}

fn default_generator_name() -> String {
    "hydra".to_string()
}

fn default_ts_client_name() -> String {
    "HydraClient".to_string()
}

impl Default for GenerateConfig {
    fn default() -> Self {
        Self {
            http_dispatch_fn: "super::execute_generated_operation".to_string(),
            http_state_type: "crate::app::AppState".to_string(),
            sse_binding_prefix: default_sse_binding_prefix(),
            http_raw_dispatch_fn: default_http_raw_dispatch_fn(),
            generator_name: default_generator_name(),
            ts_client_name: default_ts_client_name(),
        }
    }
}

impl GenerateConfig {
    /// Validate configuration that affects generated TypeScript identifiers.
    ///
    /// Rust dispatch paths remain consumer-owned strings, but the exported
    /// client class is emitted directly into TypeScript and must be a safe,
    /// non-reserved identifier that cannot shadow the shared generated types.
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            is_ts_identifier(&self.ts_client_name),
            "ts_client_name must be a non-reserved TypeScript identifier: {}",
            self.ts_client_name
        );
        anyhow::ensure!(
            !matches!(
                self.ts_client_name.as_str(),
                "ClientOptions" | "ApiError" | "JsonValue"
            ),
            "ts_client_name collides with a generated TypeScript type: {}",
            self.ts_client_name
        );
        Ok(())
    }
}

/// Generate all committed artifacts for a definition.
#[must_use]
pub fn generate_all(definition: &ApiDefinition, config: &GenerateConfig) -> GeneratedArtifacts {
    GeneratedArtifacts {
        cli_rs: generate_cli(definition),
        http_rs: generate_http(definition, config),
        mcp_json: generate_mcp(definition),
        ts_client_ts: generate_ts_client(definition, config),
    }
}

/// Validate cross-definition/configuration collisions before generating
/// TypeScript identifiers.
///
/// CLI callers should run this before `write` or `check` so invalid
/// combinations produce structured errors rather than reaching the generator's
/// defensive assertion.
pub fn validate_generation(definition: &ApiDefinition, config: &GenerateConfig) -> Result<()> {
    validate_ts_client_generation(definition, config)
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
    /// Generated fetch-based TypeScript client module.
    pub ts_client_ts: String,
}

/// Write generated artifacts under a directory.
pub fn write_generated(dir: impl AsRef<Path>, artifacts: &GeneratedArtifacts) -> Result<()> {
    let dir = dir.as_ref();
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    write_if_changed(dir.join("cli.rs"), &artifacts.cli_rs)?;
    write_if_changed(dir.join("http.rs"), &artifacts.http_rs)?;
    write_if_changed(dir.join("mcp.json"), &artifacts.mcp_json)?;
    let ts_dir = dir.join("ts-client");
    fs::create_dir_all(&ts_dir).with_context(|| format!("create {}", ts_dir.display()))?;
    write_if_changed(ts_dir.join("index.ts"), &artifacts.ts_client_ts)?;
    Ok(())
}

/// Verify committed generated artifacts match the API definition.
pub fn verify_generated(
    definition_path: impl AsRef<Path>,
    generated_dir: impl AsRef<Path>,
    config: &GenerateConfig,
) -> Result<()> {
    let definition = hydra_core::load_api_definition(definition_path)?;
    validate_generation(&definition, config)
        .context("validate generated TypeScript identifiers")?;
    let expected = generate_all(&definition, config);
    let dir = generated_dir.as_ref();

    compare_file(dir.join("cli.rs"), &expected.cli_rs)?;
    compare_file(dir.join("http.rs"), &expected.http_rs)?;
    compare_file(dir.join("mcp.json"), &expected.mcp_json)?;
    compare_file(
        dir.join("ts-client").join("index.ts"),
        &expected.ts_client_ts,
    )?;
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
            push_parameters_json_entry(&mut out, parameter);
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
            emit_cli_field(
                &mut out,
                &parameter.name,
                parameter.ty.rust_type(),
                parameter.location == ParameterLocation::Path,
                parameter.required,
                parameter.cli.as_ref(),
                None,
            );
            if let Some(cli) = &parameter.cli {
                for companion in &cli.companions {
                    push_doc_comment(&mut out, "    ", &companion.description);
                    emit_cli_field(
                        &mut out,
                        &companion.field,
                        "String",
                        false,
                        false,
                        None,
                        Some(&companion.flag),
                    );
                }
            }
        }
        for output_flag in &operation.cli_output_flags {
            push_doc_comment(&mut out, "    ", &output_flag.description);
            emit_cli_output_flag(&mut out, output_flag);
        }
        out.push_str("}\n\n");
    }

    if out.ends_with("\n\n") {
        out.pop();
    }
    out
}

/// Emit one clap field for a generated CLI args struct.
///
/// Path-location parameters stay positional (no attribute), preserving
/// the pre-existing contract. Companion fields are always optional
/// repeatable strings with an explicitly declared flag name. Parameter
/// fields with a CLI representation override follow it: an explicit flag
/// name when declared, and `multiple` → `Option<Vec<String>>` with
/// `ArgAction::Append`. When no flag is declared, clap derives the long
/// flag from the field name (`snake_case` → kebab-case).
fn emit_cli_field(
    out: &mut String,
    field: &str,
    rust_type: &str,
    positional: bool,
    required: bool,
    cli: Option<&hydra_core::CliOverride>,
    companion_flag: Option<&str>,
) {
    let mut field_type = if required && companion_flag.is_none() && !cli.is_some_and(|c| c.multiple)
    {
        rust_type.to_owned()
    } else {
        format!("Option<{rust_type}>")
    };
    let attribute = if let Some(flag) = companion_flag {
        field_type = "Option<Vec<String>>".to_string();
        format!("    #[arg(long = \"{flag}\", action = clap::ArgAction::Append)]\n")
    } else if let Some(cli) = cli {
        if cli.multiple {
            field_type = "Option<Vec<String>>".to_string();
            format!(
                "    #[arg(long = \"{}\", action = clap::ArgAction::Append, required = {required})]\n",
                cli.effective_flag(field)
            )
        } else if let Some(flag) = &cli.flag {
            // An explicitly declared flag name is emitted verbatim.
            format!("    #[arg(long = \"{flag}\")]\n")
        } else {
            "    #[arg(long)]\n".to_owned()
        }
    } else if positional {
        String::new()
    } else {
        "    #[arg(long)]\n".to_owned()
    };
    out.push_str(&attribute);
    out.push_str("    pub ");
    out.push_str(field);
    out.push_str(": ");
    out.push_str(&field_type);
    out.push_str(",\n");
}

/// Emit one explicit CLI-only boolean presentation flag.
///
/// Output flags are never request parameters, so `parameters_json()` omits
/// them. `SetTrue` gives the public boolean contract directly: absent is
/// `false`, present is `true`, and clap rejects a supplied value.
fn emit_cli_output_flag(out: &mut String, output_flag: &hydra_core::CliOutputFlag) {
    push_fmt!(
        out,
        "    #[arg(long = {}, action = clap::ArgAction::SetTrue)]\n",
        rust_string_literal(&output_flag.flag)
    );
    out.push_str("    pub ");
    out.push_str(&output_flag.field);
    out.push_str(": bool,\n");
}

fn cli_operations(definition: &ApiDefinition) -> impl Iterator<Item = &Operation> {
    definition.operations.iter().filter(|o| o.generates_cli())
}

/// Emit one `"wire_name": args.field` entry of a generated
/// `parameters_json()` match arm.
///
/// CLI representation overrides transform the CLI input back into the
/// wire shape: repeatable flags become arrays (defaulting to empty),
/// companions ride alongside as sibling keys.
fn push_parameters_json_entry(out: &mut String, parameter: &Parameter) {
    out.push_str(&rust_string_literal(&parameter.name));
    out.push_str(": ");
    out.push_str("args.");
    out.push_str(&parameter.name);
    match &parameter.cli {
        Some(cli) => {
            if cli.multiple {
                // `required = true` multiple flags are enforced by clap at
                // parse time; unwrap_or_default() covers the optional case.
                out.push_str(".clone().unwrap_or_default()");
            } else {
                out.push_str(".clone()");
            }
            for companion in &cli.companions {
                out.push_str(", ");
                out.push_str(&rust_string_literal(&companion.field));
                out.push_str(": args.");
                out.push_str(&companion.field);
                out.push_str(".clone()");
            }
        }
        None => {
            if matches!(parameter.ty, hydra_core::ParameterType::String) || !parameter.required {
                out.push_str(".clone()");
            }
        }
    }
}

// ── HTTP surface ───────────────────────────────────────────────────────────

/// Which axum imports the generated HTTP module needs, derived from what
/// the definition's unary operations actually use.
#[derive(Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
struct HttpImportPlan {
    /// Path extractor needed.
    path: bool,
    /// Query extractor needed.
    query: bool,
    /// Json extractor needed (non-raw body params).
    body_json: bool,
    /// `HeaderMap` + `Bytes` needed (raw-request operations).
    raw: bool,
    /// `get` router method needed.
    get: bool,
    /// `post` router method needed.
    post: bool,
}

impl HttpImportPlan {
    /// Analyze the definition's unary HTTP operations.
    fn analyze(unary_ops: &[&Operation]) -> Self {
        let any = |predicate: &dyn Fn(&&Operation) -> bool| unary_ops.iter().any(predicate);
        Self {
            path: any(&|o| {
                o.parameters
                    .iter()
                    .any(|p| p.location == ParameterLocation::Path)
            }),
            query: any(&|o| {
                o.parameters
                    .iter()
                    .any(|p| p.location == ParameterLocation::Query)
            }),
            // Raw-request handlers extract the body as bytes themselves,
            // so only non-raw unary operations pull in the Json extractor.
            body_json: any(&|o| {
                !o.is_raw_request()
                    && o.parameters
                        .iter()
                        .any(|p| p.location == ParameterLocation::Body)
            }),
            raw: any(&|o| o.is_raw_request()),
            get: any(&|o| o.method == HttpMethod::Get),
            post: any(&|o| o.method == HttpMethod::Post),
        }
    }
}

fn generate_http(definition: &ApiDefinition, config: &GenerateConfig) -> String {
    let mut out = generated_header("HTTP route handlers generated from the API definition");
    out.push_str("use std::collections::BTreeMap;\n\n");
    // Emit only the extractor/method imports the definition actually uses.
    let unary_ops: Vec<&Operation> = http_operations(definition)
        .filter(|operation| !operation.is_sse())
        .collect();
    let plan = HttpImportPlan::analyze(&unary_ops);
    let mut extractors = vec!["State"];
    if plan.path {
        extractors.push("Path");
    }
    if plan.query {
        extractors.push("Query");
    }
    let mut methods = Vec::new();
    if plan.get {
        methods.push("get");
    }
    if plan.post {
        methods.push("post");
    }
    push_fmt!(
        out,
        "use axum::{{extract::{{{}}}, response::Response, routing::{{{}}}, Router}};\n",
        extractors.join(", "),
        methods.join(", ")
    );
    if plan.body_json {
        out.push_str("use axum::Json;\n");
    }
    if plan.raw {
        // Raw handlers take HeaderMap + Bytes directly; Bytes must come
        // last so the body is fully buffered before extraction.
        out.push_str("use axum::body::Bytes;\n");
        out.push_str("use axum::http::HeaderMap;\n");
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
    if plan.raw {
        push_raw_input_struct(&mut out);
    }
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
    out.push_str(&generate_http_error_responses(definition));

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
    if operation.is_raw_request() {
        push_raw_handler(out, operation, config, has_path, has_query);
        return;
    }
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

/// Emit one raw-request axum handler: the request body arrives as exact
/// bytes with a header map, so consumers can verify signatures over the
/// wire representation. Path/query params still extract normally.
fn push_raw_handler(
    out: &mut String,
    operation: &Operation,
    config: &GenerateConfig,
    has_path: bool,
    has_query: bool,
) {
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
    // HeaderMap before Bytes: extractors run in declaration order and the
    // body must be buffered last.
    out.push_str("    headers: HeaderMap,\n");
    out.push_str("    raw_body: Bytes,\n");
    out.push_str(") -> Response {\n");
    out.push_str("    let headers: BTreeMap<String, String> = headers\n");
    out.push_str("        .iter()\n");
    out.push_str("        .filter_map(|(name, value)| {\n");
    out.push_str("            value\n");
    out.push_str("                .to_str()\n");
    out.push_str("                .ok()\n");
    out.push_str("                .map(|value| (name.as_str().to_owned(), value.to_owned()))\n");
    out.push_str("        })\n");
    out.push_str("        .collect();\n");
    push_fmt!(out, "    {}(\n", config.http_raw_dispatch_fn);
    out.push_str("        &state,\n");
    out.push_str("        ");
    out.push_str(&rust_string_literal(&operation.name));
    out.push_str(",\n");
    out.push_str("        GeneratedRawOperationInput {\n");
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
    out.push_str("            headers,\n");
    out.push_str("            raw_body: raw_body.to_vec(),\n");
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

/// Emit one typed HTTP-error module per operation that declares responses.
///
/// The module is deliberately independent of route generation: unary handlers
/// retain their existing dispatch hook, while SSE keeps its existing runtime
/// binding hook. A consumer chooses when to return one of these generated
/// responses before opening a stream or while handling a unary operation.
fn generate_http_error_responses(definition: &ApiDefinition) -> String {
    let mut out = String::new();
    for operation in
        http_operations(definition).filter(|operation| !operation.http_error_responses.is_empty())
    {
        push_http_error_module(&mut out, operation);
    }
    out
}

/// Emit the typed HTTP-error constructors for one declared operation.
fn push_http_error_module(out: &mut String, operation: &Operation) {
    let module_name = format!("{}_http_errors", operation.name);
    push_fmt!(
        out,
        "/// Typed HTTP error responses declared for the `{}` operation.\n",
        operation.name
    );
    push_fmt!(out, "pub mod {module_name} {{\n");
    out.push_str("    use axum::{\n");
    out.push_str("        http::StatusCode,\n");
    out.push_str("        response::{IntoResponse, Response},\n");
    out.push_str("        Json,\n");
    out.push_str("    };\n");
    out.push_str("    use serde::Serialize;\n\n");

    for response in &operation.http_error_responses {
        push_http_error_response(out, response);
    }

    out.push_str("}\n\n");
}

/// Emit one declared typed HTTP response, with private body/status state and a
/// public constructor returning an `IntoResponse` implementation.
fn push_http_error_response(out: &mut String, response: &HttpErrorResponse) {
    let symbol = pascal_case(&response.name);
    let body_type = format!("{symbol}Body");
    let response_type = format!("{symbol}Response");

    out.push_str("    #[derive(Serialize)]\n");
    push_fmt!(out, "    struct {body_type} {{\n");
    for field in &response.fields {
        if !field.required {
            out.push_str("        #[serde(skip_serializing_if = \"Option::is_none\")]\n");
        }
        push_fmt!(
            out,
            "        {}: {},\n",
            field.name,
            http_error_field_type(field)
        );
    }
    out.push_str("    }\n\n");

    push_fmt!(
        out,
        "    /// Typed HTTP response for the declared `{}` error.\n",
        response.name
    );
    push_fmt!(out, "    pub struct {response_type} {{\n");
    out.push_str("        body: ");
    out.push_str(&body_type);
    out.push_str(",\n");
    out.push_str("    }\n\n");

    push_fmt!(out, "    impl IntoResponse for {response_type} {{\n");
    out.push_str("        fn into_response(self) -> Response {\n");
    push_fmt!(
        out,
        "            (StatusCode::from_u16({}).expect(\"validated HTTP error status\"), Json(self.body)).into_response()\n",
        response.status
    );
    out.push_str("        }\n");
    out.push_str("    }\n\n");

    push_fmt!(
        out,
        "    /// Construct the declared {} `{}` HTTP response.\n",
        response.status,
        response.name
    );
    out.push_str("    pub fn ");
    out.push_str(&response.name);
    out.push('(');
    let mut first = true;
    for field in response
        .fields
        .iter()
        .filter(|field| field.constant.is_none())
    {
        if !first {
            out.push_str(", ");
        }
        first = false;
        push_fmt!(out, "{}: {}", field.name, http_error_field_type(field));
    }
    out.push_str(") -> ");
    out.push_str(&response_type);
    out.push_str(" {\n");
    out.push_str("        ");
    out.push_str(&response_type);
    out.push_str(" {\n");
    out.push_str("            body: ");
    out.push_str(&body_type);
    out.push_str(" {\n");
    for field in &response.fields {
        out.push_str("                ");
        out.push_str(&field.name);
        if let Some(constant) = &field.constant {
            out.push_str(": ");
            out.push_str(&rust_string_literal(constant));
            out.push_str(".to_owned()");
        }
        out.push_str(",\n");
    }
    out.push_str("            },\n");
    out.push_str("        }\n");
    out.push_str("    }\n\n");
}

/// Return the emitted Rust field type for the closed v1 error-field model.
const fn http_error_field_type(field: &hydra_core::HttpErrorField) -> &'static str {
    if field.required {
        "String"
    } else {
        "Option<String>"
    }
}

/// Emit the `GeneratedRawOperationInput` struct, present only when the
/// definition contains raw-request operations.
///
/// The emitted doc notes the header-map contract: names lowercased,
/// non-UTF-8 values dropped, repeated headers last-wins.
fn push_raw_input_struct(out: &mut String) {
    out.push_str("/// Input for raw-request operations: the exact raw body bytes\n");
    out.push_str("/// and a header map, for consumers that verify signatures over\n");
    out.push_str("/// the request as received.\n");
    out.push_str("///\n");
    out.push_str("/// Header contract: names are lowercase (HTTP canonical form),\n");
    out.push_str("/// values must be UTF-8 (non-UTF-8 values are dropped), and\n");
    out.push_str("/// repeated headers collapse to the last value.\n");
    out.push_str("#[derive(Debug, Clone, Default, PartialEq, Eq)]\n");
    out.push_str("pub struct GeneratedRawOperationInput {\n");
    out.push_str("    pub path: BTreeMap<String, String>,\n");
    out.push_str("    pub query: BTreeMap<String, String>,\n");
    out.push_str("    pub headers: BTreeMap<String, String>,\n");
    out.push_str("    pub raw_body: Vec<u8>,\n");
    out.push_str("}\n\n");
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
                let property = if parameter.ty == hydra_core::ParameterType::Json {
                    // Declared JSON Schema subtree, verbatim. The parameter
                    // description is merged in only when the subtree does
                    // not carry its own `description` — a schema-level
                    // description wins, preserving verbatim embedding as
                    // the primary contract.
                    let mut subtree = parameter
                        .schema
                        .clone()
                        .unwrap_or_else(|| json!({"type": "object"}));
                    if let Some(object) = subtree.as_object_mut() {
                        object
                            .entry("description".to_owned())
                            .or_insert_with(|| json!(parameter.description));
                    }
                    subtree
                } else {
                    json!({
                        "type": parameter.ty.json_schema_type(),
                        "description": parameter.description,
                    })
                };
                properties.insert(parameter.name.clone(), property);
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

// ── TypeScript client surface ──────────────────────────────────────────────

/// Generate a zero-runtime-dependency, fetch-based TypeScript client.
///
/// The client intentionally keeps the operation definition visible in the
/// emitted code: path/query/body placement comes from the declared parameter
/// locations, and the generated method names and wire keys are derived only
/// from the explicit operation/parameter names. Named output models remain
/// opaque records because the Hydra definition carries output type names but
/// not their domain schemas; a consumer can refine those aliases at its own
/// boundary without making the generator invent fields.
fn generate_ts_client(definition: &ApiDefinition, config: &GenerateConfig) -> String {
    assert!(
        validate_ts_client_generation(definition, config).is_ok(),
        "invalid TypeScript client generation configuration or identifier collision"
    );
    let operations: Vec<&Operation> = definition
        .operations
        .iter()
        .filter(|operation| operation.generates_ts_client() && !operation.is_sse())
        .collect();
    let mut out = generated_header("TypeScript fetch client generated from the API definition");
    out.push_str("/*\n");
    out.push_str(" * Runtime dependencies: none. This module uses the platform fetch API.\n");
    out.push_str(" * i64/i32-like values are represented as number; consumers that need\n");
    out.push_str(" * exact 64-bit integer precision must validate or replace that alias.\n");
    out.push_str(" */\n\n");
    out.push_str("export type JsonValue =\n");
    out.push_str("  | null\n");
    out.push_str("  | boolean\n");
    out.push_str("  | number\n");
    out.push_str("  | string\n");
    out.push_str("  | JsonValue[]\n");
    out.push_str("  | { [key: string]: JsonValue };\n\n");

    let named_types = ts_named_output_types(&operations, &config.ts_client_name);
    for name in named_types {
        push_fmt!(
            out,
            "/** Opaque domain model `{name}`; refine this alias in the consumer. */\nexport type {name} = Record<string, unknown>;\n\n"
        );
    }

    out.push_str("/** Construction options for the generated client. */\n");
    out.push_str("export interface ClientOptions {\n");
    out.push_str("  /** Absolute service URL, with an optional path prefix. */\n");
    out.push_str("  baseUrl: string;\n");
    out.push_str("  /** Optional deployment-wide bearer token. */\n");
    out.push_str("  token?: string;\n");
    out.push_str("  /** Injectable fetch for tests, Node adapters, and custom transports. */\n");
    out.push_str("  fetch?: typeof globalThis.fetch;\n");
    out.push_str("}\n\n");
    out.push_str("/** Structured error returned for a non-2xx HTTP response. */\n");
    out.push_str("export class ApiError extends Error {\n");
    out.push_str("  readonly status: number;\n");
    out.push_str("  readonly body: unknown;\n\n");
    out.push_str("  constructor(status: number, body: unknown) {\n");
    out.push_str("    super(`HTTP ${status}`);\n");
    out.push_str("    this.name = \"ApiError\";\n");
    out.push_str("    this.status = status;\n");
    out.push_str("    this.body = body;\n");
    out.push_str("  }\n");
    out.push_str("}\n\n");

    for operation in &operations {
        push_ts_operation_types(&mut out, operation);
    }

    push_fmt!(
        out,
        "/** Typed fetch client for the operations declared by this project. */\nexport class {} {{\n",
        config.ts_client_name
    );
    out.push_str("  private readonly baseUrl: string;\n");
    out.push_str("  private readonly token?: string;\n");
    out.push_str("  private readonly fetchImpl: typeof globalThis.fetch;\n\n");
    out.push_str("  constructor(options: ClientOptions) {\n");
    out.push_str("    this.baseUrl = options.baseUrl.replace(/\\/+$/, \"\");\n");
    out.push_str("    this.token = options.token;\n");
    out.push_str("    this.fetchImpl = options.fetch ?? globalThis.fetch.bind(globalThis);\n");
    out.push_str("  }\n\n");

    for operation in &operations {
        push_ts_operation_method(&mut out, operation);
    }

    out.push_str("  private makeUrl(path: string): URL {\n");
    out.push_str("    return new URL(`${this.baseUrl}${path}`);\n");
    out.push_str("  }\n\n");
    out.push_str("  private async request<T>(url: URL, init: RequestInit): Promise<T> {\n");
    out.push_str("    const headers = new Headers(init.headers);\n");
    out.push_str("    headers.set(\"accept\", \"application/json\");\n");
    out.push_str("    if (init.body !== undefined && !headers.has(\"content-type\")) {\n");
    out.push_str("      headers.set(\"content-type\", \"application/json\");\n");
    out.push_str("    }\n");
    out.push_str("    if (this.token !== undefined) {\n");
    out.push_str("      headers.set(\"authorization\", `Bearer ${this.token}`);\n");
    out.push_str("    }\n");
    out.push_str("    const response = await this.fetchImpl(url, { ...init, headers });\n");
    out.push_str("    const text = await response.text();\n");
    out.push_str("    let body: unknown = undefined;\n");
    out.push_str("    if (text.length > 0) {\n");
    out.push_str("      try {\n");
    out.push_str("        body = JSON.parse(text) as unknown;\n");
    out.push_str("      } catch {\n");
    out.push_str("        body = text;\n");
    out.push_str("      }\n");
    out.push_str("    }\n");
    out.push_str("    if (!response.ok) {\n");
    out.push_str("      throw new ApiError(response.status, body);\n");
    out.push_str("    }\n");
    out.push_str("    return body as T;\n");
    out.push_str("  }\n");
    out.push_str("}\n");
    out
}

/// Emit the TypeScript input/result aliases for one operation.
fn push_ts_operation_types(out: &mut String, operation: &Operation) {
    let prefix = pascal_case(&operation.name);
    push_fmt!(out, "/** {} */\n", operation.description);
    push_fmt!(out, "export interface {prefix}Params {{\n");
    for parameter in &operation.parameters {
        push_fmt!(
            out,
            "  {}{}: {};\n",
            parameter.name,
            if parameter.required { "" } else { "?" },
            ts_parameter_type(parameter)
        );
    }
    out.push_str("}\n\n");
    push_fmt!(
        out,
        "/** Result type declared as `{}`. */\nexport type {prefix}Result = {};\n\n",
        operation.output_type,
        ts_output_type(&operation.output_type)
    );
}

/// Emit one client method, preserving each parameter's declared wire location.
fn push_ts_operation_method(out: &mut String, operation: &Operation) {
    let prefix = pascal_case(&operation.name);
    let method_name = camel_case(&operation.name);
    let has_parameters = !operation.parameters.is_empty();
    let all_optional = operation
        .parameters
        .iter()
        .all(|parameter| !parameter.required);
    let signature = if !has_parameters {
        String::new()
    } else if all_optional {
        format!("params: {prefix}Params = {{}}")
    } else {
        format!("params: {prefix}Params")
    };
    push_fmt!(
        out,
        "  /** {} */\n  async {method_name}({signature}): Promise<{prefix}Result> {{\n",
        operation.description
    );
    let path_expression = ts_path_expression(operation);
    push_fmt!(out, "    const url = this.makeUrl({path_expression});\n");
    for parameter in operation
        .parameters
        .iter()
        .filter(|parameter| parameter.location == ParameterLocation::Query)
    {
        push_fmt!(
            out,
            "    if (params.{} !== undefined) {{\n      url.searchParams.set({}, String(params.{}));\n    }}\n",
            parameter.name,
            ts_string_literal(&parameter.name),
            parameter.name
        );
    }
    out.push_str("    return this.request(url, {\n");
    push_fmt!(
        out,
        "      method: {},\n",
        ts_string_literal(operation.method.as_str())
    );
    let body_parameters: Vec<&Parameter> = operation
        .parameters
        .iter()
        .filter(|parameter| parameter.location == ParameterLocation::Body)
        .collect();
    if !body_parameters.is_empty() {
        out.push_str("      body: JSON.stringify({\n");
        for parameter in body_parameters {
            push_fmt!(
                out,
                "        {}: params.{},\n",
                ts_string_literal(&parameter.name),
                parameter.name
            );
        }
        out.push_str("      }),\n");
    }
    out.push_str("    });\n");
    out.push_str("  }\n\n");
}

/// Build a TypeScript expression for a path, encoding every declared path
/// parameter instead of guessing from a path segment's spelling.
fn ts_path_expression(operation: &Operation) -> String {
    let mut expression = String::new();
    let mut cursor = 0;
    while let Some(relative_open) = operation.path[cursor..].find('{') {
        let open = cursor + relative_open;
        let Some(relative_close) = operation.path[open..].find('}') else {
            break;
        };
        let close = open + relative_close;
        if open > cursor {
            expression.push_str(" + ");
            expression.push_str(&ts_string_literal(&operation.path[cursor..open]));
        }
        let name = &operation.path[open + 1..close];
        expression.push_str(" + encodeURIComponent(String(params.");
        expression.push_str(name);
        expression.push_str("))");
        cursor = close + 1;
    }
    if cursor < operation.path.len() {
        expression.push_str(" + ");
        expression.push_str(&ts_string_literal(&operation.path[cursor..]));
    }
    if expression.is_empty() {
        ts_string_literal(&operation.path)
    } else {
        expression.trim_start_matches(" + ").to_owned()
    }
}

/// Convert a declared parameter to a TypeScript type. JSON parameters use
/// their explicit JSON Schema; no shape is inferred from the parameter name.
fn ts_parameter_type(parameter: &Parameter) -> String {
    if parameter.ty == hydra_core::ParameterType::Json {
        parameter
            .schema
            .as_ref()
            .map_or_else(|| "JsonValue".to_owned(), ts_schema_type)
    } else {
        ts_scalar_type(parameter.ty)
    }
}

fn ts_scalar_type(parameter_type: hydra_core::ParameterType) -> String {
    match parameter_type {
        hydra_core::ParameterType::String => "string".to_owned(),
        hydra_core::ParameterType::U32 => "number".to_owned(),
        hydra_core::ParameterType::Bool => "boolean".to_owned(),
        hydra_core::ParameterType::Json => "JsonValue".to_owned(),
    }
}

/// Project the supported JSON Schema subset to a deterministic inline type.
fn ts_schema_type(schema: &Value) -> String {
    ts_schema_expression(schema).text
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum TsTypePrecedence {
    Union,
    Intersection,
    Primary,
}

#[derive(Debug)]
struct TsTypeExpression {
    text: String,
    precedence: TsTypePrecedence,
}

impl TsTypeExpression {
    const fn primary(text: String) -> Self {
        Self {
            text,
            precedence: TsTypePrecedence::Primary,
        }
    }

    fn as_operand(&self, parent: TsTypePrecedence) -> String {
        if self.precedence < parent {
            format!("({})", self.text)
        } else {
            self.text.clone()
        }
    }
}

fn join_ts_types(
    expressions: impl IntoIterator<Item = TsTypeExpression>,
    precedence: TsTypePrecedence,
    separator: &str,
) -> TsTypeExpression {
    let expressions = expressions.into_iter().collect::<Vec<_>>();
    if expressions.len() == 1 {
        return expressions
            .into_iter()
            .next()
            .expect("one expression must be present");
    }
    TsTypeExpression {
        text: expressions
            .iter()
            .map(|expression| expression.as_operand(precedence))
            .collect::<Vec<_>>()
            .join(separator),
        precedence,
    }
}

fn ts_schema_expression(schema: &Value) -> TsTypeExpression {
    if let Some(one_of) = schema.get("oneOf").and_then(Value::as_array) {
        return join_ts_types(
            one_of.iter().map(ts_schema_expression),
            TsTypePrecedence::Union,
            " | ",
        );
    }
    if let Some(any_of) = schema.get("anyOf").and_then(Value::as_array) {
        return join_ts_types(
            any_of.iter().map(ts_schema_expression),
            TsTypePrecedence::Union,
            " | ",
        );
    }
    if let Some(all_of) = schema.get("allOf").and_then(Value::as_array) {
        return join_ts_types(
            all_of.iter().map(ts_schema_expression),
            TsTypePrecedence::Intersection,
            " & ",
        );
    }
    if let Some(value) = schema.get("const") {
        return TsTypeExpression::primary(ts_json_literal(value));
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return join_ts_types(
            values
                .iter()
                .map(|value| TsTypeExpression::primary(ts_json_literal(value))),
            TsTypePrecedence::Union,
            " | ",
        );
    }
    let nullable = schema
        .get("nullable")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || schema
            .get("type")
            .and_then(Value::as_array)
            .is_some_and(|types| types.iter().any(|kind| kind.as_str() == Some("null")));
    let mut type_expression = match schema.get("type") {
        Some(Value::String(kind)) => ts_schema_kind(schema, kind),
        Some(Value::Array(kinds)) => join_ts_types(
            kinds
                .iter()
                .filter_map(Value::as_str)
                .filter(|kind| *kind != "null")
                .map(|kind| ts_schema_kind(schema, kind)),
            TsTypePrecedence::Union,
            " | ",
        ),
        _ => TsTypeExpression::primary("unknown".to_owned()),
    };
    if type_expression.text.is_empty() {
        type_expression = TsTypeExpression::primary("unknown".to_owned());
    }
    if nullable {
        type_expression = join_ts_types(
            [
                type_expression,
                TsTypeExpression::primary("null".to_owned()),
            ],
            TsTypePrecedence::Union,
            " | ",
        );
    }
    type_expression
}

fn ts_schema_kind(schema: &Value, kind: &str) -> TsTypeExpression {
    match kind {
        "string" => TsTypeExpression::primary("string".to_owned()),
        "integer" | "number" => TsTypeExpression::primary("number".to_owned()),
        "boolean" => TsTypeExpression::primary("boolean".to_owned()),
        "null" => TsTypeExpression::primary("null".to_owned()),
        "array" => schema.get("items").map_or_else(
            || TsTypeExpression::primary("Array<unknown>".to_owned()),
            |items| TsTypeExpression::primary(format!("Array<{}>", ts_schema_type(items))),
        ),
        "object" => {
            let mut fields = Vec::new();
            if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
                let required = schema
                    .get("required")
                    .and_then(Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<std::collections::BTreeSet<_>>()
                    })
                    .unwrap_or_default();
                for (name, property) in properties {
                    fields.push(format!(
                        "{}{}: {}",
                        ts_property_key(name),
                        if required.contains(name.as_str()) {
                            ""
                        } else {
                            "?"
                        },
                        ts_schema_type(property)
                    ));
                }
            }
            if schema.get("additionalProperties").and_then(Value::as_bool) != Some(false) {
                fields.push("[key: string]: unknown".to_owned());
            }
            TsTypeExpression::primary(format!("{{ {} }}", fields.join("; ")))
        }
        _ => TsTypeExpression::primary("unknown".to_owned()),
    }
}

/// Map the Rust-ish output type notation currently carried by Hydra's model
/// to a TypeScript expression without pretending that unknown domain models
/// have fields the definition never declared.
fn ts_output_type(output_type: &str) -> String {
    let output_type = output_type.trim();
    if let Some(inner) = generic_inner(output_type, "Vec") {
        return format!("Array<{}>", ts_output_type(inner));
    }
    if let Some(inner) = generic_inner(output_type, "Option") {
        return format!("{} | null", ts_output_type(inner));
    }
    if let Some(inner) = generic_inner(output_type, "Result") {
        return ts_output_type(
            split_generic_args(inner)
                .first()
                .copied()
                .unwrap_or("unknown"),
        );
    }
    match output_type {
        "()" => "void".to_owned(),
        "String" | "str" | "string" => "string".to_owned(),
        "bool" | "boolean" => "boolean".to_owned(),
        "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64" | "i128"
        | "isize" | "f32" | "f64" | "number" => "number".to_owned(),
        "Value" | "Json" | "json" | "serde_json::Value" => "JsonValue".to_owned(),
        value
            if is_type_identifier(value)
                && !matches!(value, "ClientOptions" | "ApiError" | "JsonValue") =>
        {
            value.to_owned()
        }
        _ => "unknown".to_owned(),
    }
}

fn ts_named_output_types(operations: &[&Operation], reserved_name: &str) -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    for operation in operations {
        collect_named_output_types(operation.output_type.trim(), &mut names);
    }
    names
        .into_iter()
        .filter(|name| {
            name != reserved_name
                && !matches!(name.as_str(), "ClientOptions" | "ApiError" | "JsonValue")
        })
        .collect()
}

fn validate_ts_client_generation(
    definition: &ApiDefinition,
    config: &GenerateConfig,
) -> Result<()> {
    config.validate()?;
    let operations: Vec<&Operation> = definition
        .operations
        .iter()
        .filter(|operation| operation.generates_ts_client() && !operation.is_sse())
        .collect();
    let mut generated = std::collections::BTreeSet::from([
        config.ts_client_name.clone(),
        "ClientOptions".to_owned(),
        "ApiError".to_owned(),
        "JsonValue".to_owned(),
    ]);
    for operation in &operations {
        let method = camel_case(&operation.name);
        anyhow::ensure!(
            !matches!(
                method.as_str(),
                "constructor" | "baseUrl" | "token" | "fetchImpl" | "makeUrl" | "request"
            ),
            "operation {} generates a reserved TypeScript client member {}",
            operation.name,
            method
        );
        let prefix = pascal_case(&operation.name);
        for suffix in ["Params", "Result"] {
            let name = format!("{prefix}{suffix}");
            anyhow::ensure!(
                generated.insert(name.clone()),
                "operation {} collides with generated TypeScript identifier {}",
                operation.name,
                name
            );
        }
    }

    let mut model_names = std::collections::BTreeSet::new();
    for operation in &operations {
        collect_named_output_types(operation.output_type.trim(), &mut model_names);
    }
    for name in model_names {
        anyhow::ensure!(
            generated.insert(name.clone()),
            "output type {name} collides with a generated TypeScript identifier"
        );
    }
    Ok(())
}

fn collect_named_output_types(output_type: &str, names: &mut std::collections::BTreeSet<String>) {
    if let Some(inner) = generic_inner(output_type, "Vec") {
        collect_named_output_types(inner.trim(), names);
        return;
    }
    if let Some(inner) = generic_inner(output_type, "Option") {
        collect_named_output_types(inner.trim(), names);
        return;
    }
    if let Some(inner) = generic_inner(output_type, "Result") {
        for argument in split_generic_args(inner) {
            collect_named_output_types(argument.trim(), names);
        }
        return;
    }
    if is_type_identifier(output_type)
        && !matches!(
            output_type,
            "String" | "str" | "string" | "bool" | "boolean" | "Value" | "Json" | "json" | "number"
        )
    {
        names.insert(output_type.to_owned());
    }
}

fn generic_inner<'a>(value: &'a str, name: &str) -> Option<&'a str> {
    let prefix = format!("{name}<");
    value
        .strip_prefix(&prefix)
        .and_then(|rest| rest.strip_suffix('>'))
}

fn split_generic_args(value: &str) -> Vec<&str> {
    let mut args = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (index, character) in value.char_indices() {
        match character {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => {
                args.push(value[start..index].trim());
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    args.push(value[start..].trim());
    args
}

fn is_type_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_uppercase())
}

fn ts_json_literal(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => ts_string_literal(value),
        _ => "unknown".to_owned(),
    }
}

fn ts_property_key(value: &str) -> String {
    if is_ts_property_identifier(value) {
        value.to_owned()
    } else {
        ts_string_literal(value)
    }
}

fn is_ts_property_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_' || first == '$')
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '$'
        })
}

fn is_ts_identifier(value: &str) -> bool {
    is_ts_property_identifier(value)
        && !matches!(
            value,
            "any"
                | "as"
                | "boolean"
                | "break"
                | "case"
                | "catch"
                | "class"
                | "const"
                | "constructor"
                | "continue"
                | "debugger"
                | "default"
                | "delete"
                | "do"
                | "else"
                | "enum"
                | "export"
                | "extends"
                | "false"
                | "finally"
                | "for"
                | "function"
                | "if"
                | "implements"
                | "import"
                | "in"
                | "instanceof"
                | "interface"
                | "let"
                | "module"
                | "new"
                | "null"
                | "number"
                | "object"
                | "package"
                | "private"
                | "protected"
                | "public"
                | "readonly"
                | "return"
                | "static"
                | "string"
                | "super"
                | "switch"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "type"
                | "typeof"
                | "undefined"
                | "unknown"
                | "var"
                | "void"
                | "while"
                | "with"
                | "yield"
        )
}

fn ts_string_literal(value: &str) -> String {
    serde_json::to_string(value).expect("serializing a TypeScript string literal cannot fail")
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
    // `Debug` for `str` emits a Rust string literal, including Rust's
    // `\u{...}` form for control characters. JSON's `\u0001` form cannot be
    // inserted directly into generated Rust source because Rust rejects that
    // escape spelling.
    format!("{value:?}")
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

fn camel_case(value: &str) -> String {
    let pascal = pascal_case(value);
    let mut characters = pascal.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_lowercase().collect::<String>() + characters.as_str()
    })
}

// Keep unused-import checker satisfied for symbols referenced only from
// generated output or doc examples.
const _: Option<Surface> = None;
const _: Option<Delivery> = None;
const _: Option<&Parameter> = None;
