//! Stateless MCP Streamable HTTP transport for Hydra-generated tools.
//!
//! The runtime owns only the current MCP HTTP/JSON-RPC protocol boundary. It
//! accepts a validated generated tool manifest and one consumer-owned async
//! dispatch closure; it never infers an endpoint path, operation semantics,
//! parameter locations, authentication policy, or host lifecycle.

use std::{collections::BTreeSet, future::Future, pin::Pin, sync::Arc};

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::State,
    http::{
        HeaderMap, Request, StatusCode,
        header::{ACCEPT, CONTENT_TYPE, ORIGIN},
    },
    response::{IntoResponse, Response},
    routing::post,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Map, Value, json};
use thiserror::Error;

/// MCP protocol revision implemented by this transport.
pub const PROTOCOL_VERSION: &str = "2026-07-28";

const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
const HEADER_MISMATCH_CODE: i64 = -32020;
const UNSUPPORTED_PROTOCOL_VERSION_CODE: i64 = -32022;
const BASE64_SENTINEL_PREFIX: &str = "=?base64?";
const BASE64_SENTINEL_SUFFIX: &str = "?=";

/// Origin handling is deliberately explicit and secure by default.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum OriginPolicy {
    /// Allow ordinary non-browser clients without an Origin header, but reject
    /// every request that presents one.
    #[default]
    RejectPresented,
    /// Allow only the exact literal origins in this finite list. This is
    /// inbound validation, not a CORS or Private Network Access policy.
    AllowExact(Vec<String>),
}

/// Configuration owned by the embedding host.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// Human-readable server name returned by `server/discover`.
    pub server_name: String,
    /// Server version returned by `server/discover`.
    pub server_version: String,
    /// Bare generated tool array or the generated `{"tools": [...]}` value.
    pub tools: Value,
    /// Explicit inbound Origin policy.
    pub origin_policy: OriginPolicy,
}

impl ServerConfig {
    /// Construct an HTTP runtime configuration.
    #[must_use]
    pub fn new(
        server_name: impl Into<String>,
        server_version: impl Into<String>,
        tools: Value,
        origin_policy: OriginPolicy,
    ) -> Self {
        Self {
            server_name: server_name.into(),
            server_version: server_version.into(),
            tools,
            origin_policy,
        }
    }
}

/// Errors found before an HTTP endpoint is exposed.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum ConfigurationError {
    /// The generated manifest is not a supported tool manifest.
    #[error("invalid MCP tools manifest: {0}")]
    InvalidManifest(String),
    /// The explicit Origin policy contains an unsafe or duplicate entry.
    #[error("invalid MCP Origin policy: {0}")]
    InvalidOriginPolicy(String),
}

type DispatchFuture = Pin<Box<dyn Future<Output = Result<Value, String>> + Send + 'static>>;
type Dispatch = dyn Fn(String, Value) -> DispatchFuture + Send + Sync + 'static;

struct ServerState {
    server_name: String,
    server_version: String,
    tools: Vec<Value>,
    tool_names: BTreeSet<String>,
    origin_policy: OriginPolicy,
    dispatch: Arc<Dispatch>,
}

/// Build a reusable Axum router for the stateless MCP HTTP server profile.
///
/// The returned router serves one POST endpoint at `/`. The embedding host
/// chooses the listener and may nest it at an explicit path such as `/mcp`.
///
/// # Errors
///
/// Returns [`ConfigurationError`] when the generated manifest or Origin policy
/// cannot be safely represented by this transport.
pub fn router<F, Fut>(config: ServerConfig, dispatch: F) -> Result<Router, ConfigurationError>
where
    F: Fn(String, Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Value, String>> + Send + 'static,
{
    validate_origin_policy(&config.origin_policy)?;
    let (tools, tool_names) = normalize_tools(config.tools)?;
    let dispatch: Arc<Dispatch> =
        Arc::new(move |name, arguments| Box::pin(dispatch(name, arguments)));
    let state = Arc::new(ServerState {
        server_name: config.server_name,
        server_version: config.server_version,
        tools,
        tool_names,
        origin_policy: config.origin_policy,
        dispatch,
    });

    Ok(Router::new()
        .route("/", post(handle_request))
        .with_state(state))
}

async fn handle_request(State(state): State<Arc<ServerState>>, request: Request<Body>) -> Response {
    let headers = request.headers().clone();
    if !origin_allowed(&headers, &state.origin_policy) {
        return rpc_error(
            Value::Null,
            StatusCode::FORBIDDEN,
            -32600,
            "Origin rejected",
            None,
        );
    }
    if !has_json_content_type(&headers) {
        return rpc_error(
            Value::Null,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            -32600,
            "Content-Type must be application/json",
            None,
        );
    }
    if !accepts_required_media_types(&headers) {
        return rpc_error(
            Value::Null,
            StatusCode::NOT_ACCEPTABLE,
            -32600,
            "Accept must include application/json and text/event-stream",
            None,
        );
    }

    let Ok(body) = to_bytes(request.into_body(), MAX_REQUEST_BYTES).await else {
        return rpc_error(
            Value::Null,
            StatusCode::PAYLOAD_TOO_LARGE,
            -32600,
            "Request body is too large",
            None,
        );
    };
    let Ok(value) = serde_json::from_slice::<Value>(&body) else {
        return rpc_error(
            Value::Null,
            StatusCode::BAD_REQUEST,
            -32600,
            "Invalid JSON-RPC request",
            None,
        );
    };
    process_request(&state, &headers, value).await
}

async fn process_request(state: &ServerState, headers: &HeaderMap, request: Value) -> Response {
    let parsed = match parse_request(&request) {
        Ok(parsed) => parsed,
        Err(error) => {
            return rpc_error(
                error.id,
                StatusCode::BAD_REQUEST,
                -32600,
                error.message,
                None,
            );
        }
    };
    let ParsedRequest {
        id,
        method,
        params,
        body_version,
    } = parsed;
    let Some(header_version) = plain_header(headers, "MCP-Protocol-Version") else {
        return header_mismatch(id);
    };
    if header_version != body_version {
        return header_mismatch(id);
    }
    let Some(header_method) = plain_header(headers, "Mcp-Method") else {
        return header_mismatch(id);
    };
    if header_method != method {
        return header_mismatch(id);
    }
    if body_version != PROTOCOL_VERSION {
        return rpc_error(
            id,
            StatusCode::BAD_REQUEST,
            UNSUPPORTED_PROTOCOL_VERSION_CODE,
            "UnsupportedProtocolVersionError",
            Some(json!({ "supportedVersions": [PROTOCOL_VERSION] })),
        );
    }

    match method {
        "server/discover" => {
            let result = json!({
                "resultType": "complete",
                "supportedVersions": [PROTOCOL_VERSION],
                "capabilities": { "tools": {} },
                "_meta": {
                    "io.modelcontextprotocol/serverInfo": {
                        "name": state.server_name,
                        "version": state.server_version,
                    }
                }
            });
            success_response(id, result)
        }
        "tools/list" => success_response(
            id,
            json!({ "resultType": "complete", "tools": state.tools }),
        ),
        "tools/call" => handle_tool_call(state, headers, id, params).await,
        _ => rpc_error(
            id,
            StatusCode::NOT_FOUND,
            -32601,
            "Method not found for protocol 2026-07-28",
            None,
        ),
    }
}

struct ParsedRequest<'a> {
    id: Value,
    method: &'a str,
    params: &'a Map<String, Value>,
    body_version: &'a str,
}

struct RequestError {
    id: Value,
    message: &'static str,
}

fn parse_request(request: &Value) -> Result<ParsedRequest<'_>, RequestError> {
    let Some(object) = request.as_object() else {
        return Err(RequestError {
            id: Value::Null,
            message: "Invalid JSON-RPC request",
        });
    };
    let raw_id = object.get("id");
    let id = raw_id
        .filter(|value| is_valid_request_id(value))
        .cloned()
        .unwrap_or(Value::Null);
    if raw_id.is_none_or(|value| !is_valid_request_id(value))
        || object.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
    {
        return Err(RequestError {
            id,
            message: "Invalid JSON-RPC request",
        });
    }
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return Err(RequestError {
            id,
            message: "Invalid JSON-RPC request",
        });
    };
    if method.chars().any(char::is_control) {
        return Err(RequestError {
            id,
            message: "Invalid JSON-RPC request",
        });
    }
    let Some(params) = object.get("params").and_then(Value::as_object) else {
        return Err(RequestError {
            id,
            message: "Request metadata is required",
        });
    };
    let Some(metadata) = params.get("_meta").and_then(Value::as_object) else {
        return Err(RequestError {
            id,
            message: "Request metadata is required",
        });
    };
    let Some(body_version) = metadata
        .get("io.modelcontextprotocol/protocolVersion")
        .and_then(Value::as_str)
    else {
        return Err(RequestError {
            id,
            message: "Request protocol metadata is required",
        });
    };
    if !metadata
        .get("io.modelcontextprotocol/clientCapabilities")
        .is_some_and(Value::is_object)
    {
        return Err(RequestError {
            id,
            message: "Request client capabilities are required",
        });
    }
    Ok(ParsedRequest {
        id,
        method,
        params,
        body_version,
    })
}

async fn handle_tool_call(
    state: &ServerState,
    headers: &HeaderMap,
    id: Value,
    params: &serde_json::Map<String, Value>,
) -> Response {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return rpc_error(
            id,
            StatusCode::BAD_REQUEST,
            -32602,
            "tools/call requires a tool name",
            None,
        );
    };
    if name.is_empty() || name.chars().any(char::is_control) {
        return rpc_error(
            id,
            StatusCode::BAD_REQUEST,
            -32602,
            "tools/call has an invalid tool name",
            None,
        );
    }
    let Some(header_name) = encoded_or_plain_header(headers, "Mcp-Name") else {
        return header_mismatch(id);
    };
    if header_name != name {
        return header_mismatch(id);
    }
    if !state.tool_names.contains(name) {
        return rpc_error(
            id,
            StatusCode::BAD_REQUEST,
            -32602,
            "tools/call names an undeclared tool",
            None,
        );
    }
    let arguments = match params.get("arguments") {
        None => json!({}),
        Some(value) if value.is_object() => value.clone(),
        Some(_) => {
            return rpc_error(
                id,
                StatusCode::BAD_REQUEST,
                -32602,
                "tools/call arguments must be an object",
                None,
            );
        }
    };

    match (state.dispatch)(name.to_owned(), arguments).await {
        Ok(value) => success_response(
            id,
            json!({
                "resultType": "complete",
                "structuredContent": value,
                "content": [{ "type": "text", "text": value.to_string() }]
            }),
        ),
        Err(message) => success_response(
            id,
            json!({
                "resultType": "complete",
                "isError": true,
                "content": [{ "type": "text", "text": message }]
            }),
        ),
    }
}

fn normalize_tools(tools: Value) -> Result<(Vec<Value>, BTreeSet<String>), ConfigurationError> {
    let tools = match tools {
        Value::Array(tools) => tools,
        Value::Object(object) => object
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| {
                ConfigurationError::InvalidManifest(
                    "manifest wrapper must contain a tools array".to_owned(),
                )
            })?,
        _ => {
            return Err(ConfigurationError::InvalidManifest(
                "manifest must be a tools array or object wrapper".to_owned(),
            ));
        }
    };

    let mut names = BTreeSet::new();
    for (index, tool) in tools.iter().enumerate() {
        let Some(object) = tool.as_object() else {
            return Err(ConfigurationError::InvalidManifest(format!(
                "tool {index} must be an object"
            )));
        };
        let Some(name) = object.get("name").and_then(Value::as_str) else {
            return Err(ConfigurationError::InvalidManifest(format!(
                "tool {index} must have a string name"
            )));
        };
        if name.is_empty() || name.chars().any(char::is_control) {
            return Err(ConfigurationError::InvalidManifest(format!(
                "tool {index} has an invalid name"
            )));
        }
        if !names.insert(name.to_owned()) {
            return Err(ConfigurationError::InvalidManifest(format!(
                "tool name is duplicated: {name}"
            )));
        }
        let Some(schema) = object.get("inputSchema") else {
            return Err(ConfigurationError::InvalidManifest(format!(
                "tool {name} is missing inputSchema"
            )));
        };
        if !schema.is_object() {
            return Err(ConfigurationError::InvalidManifest(format!(
                "tool {name} inputSchema must be an object"
            )));
        }
        if contains_key(schema, "x-mcp-header") {
            return Err(ConfigurationError::InvalidManifest(format!(
                "tool {name} uses unsupported x-mcp-header"
            )));
        }
    }
    Ok((tools, names))
}

fn contains_key(value: &Value, wanted: &str) -> bool {
    match value {
        Value::Object(object) => object
            .iter()
            .any(|(key, value)| key == wanted || contains_key(value, wanted)),
        Value::Array(values) => values.iter().any(|value| contains_key(value, wanted)),
        _ => false,
    }
}

fn validate_origin_policy(policy: &OriginPolicy) -> Result<(), ConfigurationError> {
    let OriginPolicy::AllowExact(origins) = policy else {
        return Ok(());
    };
    let mut seen = BTreeSet::new();
    for origin in origins {
        if origin.is_empty()
            || origin.chars().any(char::is_control)
            || origin.trim() != origin
            || !seen.insert(origin)
        {
            return Err(ConfigurationError::InvalidOriginPolicy(
                "origins must be non-empty, unique, and free of control or surrounding whitespace"
                    .to_owned(),
            ));
        }
    }
    Ok(())
}

fn origin_allowed(headers: &HeaderMap, policy: &OriginPolicy) -> bool {
    let Some(origin_header) = headers.get(ORIGIN) else {
        return true;
    };
    let Ok(origin) = origin_header.to_str() else {
        return false;
    };
    match policy {
        OriginPolicy::RejectPresented => false,
        OriginPolicy::AllowExact(origins) => origins.iter().any(|allowed| allowed == origin),
    }
}

fn has_json_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
}

fn accepts_required_media_types(headers: &HeaderMap) -> bool {
    let accepted: BTreeSet<String> = headers
        .get_all(ACCEPT)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|value| value.trim().split(';').next())
        .map(|value| value.trim().to_ascii_lowercase())
        .collect();
    accepted.contains("application/json") && accepted.contains("text/event-stream")
}

fn plain_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn encoded_or_plain_header(headers: &HeaderMap, name: &str) -> Option<String> {
    let value = plain_header(headers, name)?;
    if value.starts_with(BASE64_SENTINEL_PREFIX) || value.ends_with(BASE64_SENTINEL_SUFFIX) {
        if !(value.starts_with(BASE64_SENTINEL_PREFIX) && value.ends_with(BASE64_SENTINEL_SUFFIX)) {
            return None;
        }
        let encoded = value
            .strip_prefix(BASE64_SENTINEL_PREFIX)?
            .strip_suffix(BASE64_SENTINEL_SUFFIX)?;
        let decoded = STANDARD.decode(encoded).ok()?;
        let decoded = String::from_utf8(decoded).ok()?;
        if decoded.is_empty() || decoded.chars().any(char::is_control) {
            return None;
        }
        Some(decoded)
    } else if value.is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        None
    } else {
        Some(value.to_owned())
    }
}

fn is_valid_request_id(id: &Value) -> bool {
    id.is_string() || id.is_number()
}

fn header_mismatch(id: Value) -> Response {
    rpc_error(
        id,
        StatusCode::BAD_REQUEST,
        HEADER_MISMATCH_CODE,
        "HeaderMismatch",
        None,
    )
}

fn success_response(id: Value, result: Value) -> Response {
    let mut body = Map::new();
    body.insert("jsonrpc".to_owned(), json!("2.0"));
    body.insert("id".to_owned(), id);
    body.insert("result".to_owned(), result);
    response(StatusCode::OK, Value::Object(body))
}

fn rpc_error(
    id: Value,
    status: StatusCode,
    code: i64,
    message: &str,
    data: Option<Value>,
) -> Response {
    let mut error = Map::new();
    error.insert("code".to_owned(), json!(code));
    error.insert("message".to_owned(), json!(message));
    if let Some(data) = data {
        error.insert("data".to_owned(), data);
    }
    let mut body = Map::new();
    body.insert("jsonrpc".to_owned(), json!("2.0"));
    body.insert("id".to_owned(), id);
    body.insert("error".to_owned(), Value::Object(error));
    response(status, Value::Object(body))
}

fn response(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_validation_preserves_tool_order() {
        let manifest = json!({
            "tools": [
                {"name": "first", "inputSchema": {"type": "object"}},
                {"name": "second", "inputSchema": {"type": "object"}}
            ]
        });
        let (tools, names) = normalize_tools(manifest).unwrap();
        assert_eq!(tools[0]["name"], "first");
        assert!(names.contains("second"));
    }

    #[test]
    fn nested_header_annotation_is_rejected() {
        let manifest = json!({
            "tools": [{
                "name": "secret",
                "inputSchema": {"properties": {"nested": {"x-mcp-header": "X-Key"}}}
            }]
        });
        let error = normalize_tools(manifest).unwrap_err().to_string();
        assert!(error.contains("secret"));
        assert!(error.contains("x-mcp-header"));
    }

    #[test]
    fn sentinel_name_decodes_utf8() {
        let encoded = STANDARD.encode("世界");
        let headers = HeaderMap::from_iter([(
            "Mcp-Name".parse().unwrap(),
            format!("=?base64?{encoded}?=").parse().unwrap(),
        )]);
        assert_eq!(
            encoded_or_plain_header(&headers, "Mcp-Name").as_deref(),
            Some("世界")
        );
    }
}
