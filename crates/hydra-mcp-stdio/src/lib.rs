//! Hydra MCP stdio runtime: a minimal, reusable MCP server over
//! newline-delimited JSON-RPC on stdio.
//!
//! Speaks just enough of the MCP spec for tool-driven agents: `initialize`,
//! `notifications/initialized`, `tools/list`, and `tools/call`. Tool
//! definitions come from the caller (generated `mcp.json`), and dispatch is
//! a single async closure — the project's operation implementation stays the
//! only place business logic lives.
//!
//! Extracted from iris-mcp's transport layer for reuse across hydra projects.

use std::future::Future;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Constant identifying the MCP protocol version implemented here.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// Serve MCP over stdio until stdin closes.
///
/// `tools` is the JSON array from generated `mcp.json` (`{"tools": [...]}` is
/// also accepted — the `tools` member is extracted). `dispatch` receives the
/// tool name and arguments object and returns the tool result value. Dispatch
/// errors are reported to the client as tool results with `isError: true`
/// carrying the error message, preserving the caller's `Display` text.
///
/// # Errors
///
/// Returns an error only on stdio failures; protocol-level errors are
/// reported as JSON-RPC error responses to the client.
pub async fn serve<F, Fut>(
    server_name: &str,
    server_version: &str,
    tools: Value,
    dispatch: F,
) -> Result<(), Box<dyn std::error::Error>>
where
    F: Fn(String, Value) -> Fut + Send + Sync,
    Fut: Future<Output = Result<Value, String>> + Send,
{
    let tools_value = normalize_tools(tools);
    let server_info = json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": server_name, "version": server_version },
    });

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = tokio::io::stdout();
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }
        let Ok(request) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        let is_notification = request.get("id").is_none();
        let Some(response) = handle_request(&request, &tools_value, &server_info, &dispatch).await
        else {
            continue; // notification: no response permitted
        };
        if is_notification {
            // JSON-RPC 2.0: never respond to notifications, even on error.
            continue;
        }
        let mut framed = serde_json::to_string(&response)?;
        framed.push('\n');
        stdout.write_all(framed.as_bytes()).await?;
        stdout.flush().await?;
    }
    Ok(())
}

/// Handle one JSON-RPC request value, returning `None` when no response
/// should be sent (notifications).
async fn handle_request<F, Fut>(
    request: &Value,
    tools: &Value,
    server_info: &Value,
    dispatch: &F,
) -> Option<Value>
where
    F: Fn(String, Value) -> Fut + Send + Sync,
    Fut: Future<Output = Result<Value, String>> + Send,
{
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return Some(error_response(
            &id,
            -32600,
            "invalid request: missing method",
        ));
    };
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));

    let result = match method {
        "initialize" => server_info.clone(),
        "notifications/initialized" | "initialized" => return None,
        "tools/list" => json!({ "tools": tools }),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(error_response(
                    &id,
                    -32602,
                    "tools/call requires params.name",
                ));
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match dispatch(name.to_string(), arguments).await {
                Ok(value) => json!({
                    "content": [{ "type": "text", "text": value.to_string() }],
                }),
                Err(message) => json!({
                    "content": [{ "type": "text", "text": message }],
                    "isError": true,
                }),
            }
        }
        other => {
            return Some(error_response(
                &id,
                -32601,
                &format!("method not found: {other}"),
            ));
        }
    };

    Some(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    }))
}

/// Normalize either a bare tools array or a `{"tools": [...]}` wrapper.
fn normalize_tools(tools: Value) -> Value {
    match tools {
        Value::Array(_) => tools,
        Value::Object(ref map) => map.get("tools").cloned().unwrap_or_else(|| json!([])),
        _ => json!([]),
    }
}

fn error_response(id: &Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    })
}
