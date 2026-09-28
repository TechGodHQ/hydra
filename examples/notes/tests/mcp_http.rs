use std::{fmt::Write as _, net::SocketAddr};

use notes_example::{AppState, GENERATED_MCP_JSON, mcp_http_router};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::oneshot,
};

async fn post_json(
    address: SocketAddr,
    method: &str,
    name: Option<&str>,
    params: Value,
    id: u64,
) -> (u16, String, Value) {
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    })
    .to_string();
    let mut request = format!(
        "POST /mcp HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: 2026-07-28\r\nMcp-Method: {method}\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(name) = name {
        let _ = write!(request, "Mcp-Name: {name}\r\n");
    }
    request.push_str("\r\n");

    let mut stream = TcpStream::connect(address).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    stream.write_all(body.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();

    let separator = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("HTTP response headers");
    let header_bytes = &response[..separator];
    let body_bytes = &response[separator + 4..];
    let header_text = String::from_utf8(header_bytes.to_vec()).unwrap();
    let status = header_text
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let value = serde_json::from_slice(body_bytes).unwrap();
    (status, header_text, value)
}

fn metadata() -> Value {
    json!({
        "_meta": {
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {}
        }
    })
}

#[tokio::test]
async fn independent_loopback_client_discovers_lists_and_calls_notes() {
    let app = mcp_http_router(AppState::with_fixtures()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (shutdown, shutdown_signal) = oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_signal.await;
            })
            .await
            .unwrap();
    });

    let (status, discover_headers, discover) =
        post_json(address, "server/discover", None, metadata(), 1).await;
    assert_eq!(status, 200);
    assert!(discover_headers.contains("content-type: application/json"));
    assert!(
        !discover_headers
            .to_ascii_lowercase()
            .contains("mcp-session-id")
    );
    assert_eq!(discover["result"]["resultType"], "complete");
    assert_eq!(
        discover["result"]["supportedVersions"],
        json!(["2026-07-28"])
    );

    let (status, list_headers, list) = post_json(address, "tools/list", None, metadata(), 2).await;
    assert_eq!(status, 200);
    assert!(
        !list_headers
            .to_ascii_lowercase()
            .contains("access-control-allow")
    );
    let generated: Value = serde_json::from_str(GENERATED_MCP_JSON).unwrap();
    assert_eq!(list["result"]["tools"], generated["tools"]);

    let mut call_params = metadata();
    call_params["name"] = json!("list_notes");
    call_params["arguments"] = json!({"limit": 1});
    let (status, call_headers, call) =
        post_json(address, "tools/call", Some("list_notes"), call_params, 3).await;
    assert_eq!(status, 200);
    assert!(!call_headers.to_ascii_lowercase().contains("mcp-session-id"));
    assert_eq!(call["result"]["resultType"], "complete");
    assert_eq!(call["result"]["structuredContent"][0]["id"], "n2");
    assert_eq!(
        call["result"]["structuredContent"][0]["title"],
        "spike followup"
    );
    assert_eq!(call["result"]["content"][0]["type"], "text");

    shutdown.send(()).unwrap();
    server.await.unwrap();
}
