use std::{fmt::Write as _, net::SocketAddr, time::Duration};

use notes_example::{AppState, GENERATED_MCP_JSON, mcp_http_router};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::oneshot,
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

async fn post_json(
    address: SocketAddr,
    method: &str,
    name: Option<&str>,
    params: Value,
    id: u64,
) -> (u16, String, Value) {
    post_json_with_origin(address, method, name, params, id, None).await
}

async fn post_json_with_origin(
    address: SocketAddr,
    method: &str,
    name: Option<&str>,
    params: Value,
    id: u64,
    origin: Option<&str>,
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
    if let Some(origin) = origin {
        let _ = write!(request, "Origin: {origin}\r\n");
    }
    request.push_str("\r\n");

    let response = tokio::time::timeout(REQUEST_TIMEOUT, async {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        stream.write_all(body.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        stream
            .take((MAX_RESPONSE_BYTES + 1) as u64)
            .read_to_end(&mut response)
            .await
            .unwrap();
        response
    })
    .await
    .expect("HTTP request exceeded its per-request timeout");
    assert!(
        response.len() <= MAX_RESPONSE_BYTES,
        "HTTP response exceeded the configured size limit"
    );

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

async fn rejected_write_requests_preserve_fixture(address: SocketAddr) {
    let mut stats_params = metadata();
    stats_params["name"] = json!("note_stats");
    stats_params["arguments"] = json!({});
    let (stats_status, _, stats_before) =
        post_json(address, "tools/call", Some("note_stats"), stats_params, 4).await;
    assert_eq!(stats_status, 200);
    let stats_before = stats_before["result"]["structuredContent"].clone();

    let mut create_params = metadata();
    create_params["name"] = json!("create_note");
    create_params["arguments"] = json!({
        "title": "negative fixture sentinel",
        "body": "must not be dispatched"
    });
    let (status, headers, rejected_origin) = post_json_with_origin(
        address,
        "tools/call",
        Some("create_note"),
        create_params.clone(),
        5,
        Some("https://untrusted.example"),
    )
    .await;
    assert_eq!(status, 403);
    assert_eq!(rejected_origin["error"]["code"], -32600);
    assert_eq!(rejected_origin["error"]["message"], "Origin rejected");
    assert!(!headers.to_ascii_lowercase().contains("access-control-"));
    assert!(!headers.to_ascii_lowercase().contains("mcp-session-id"));
    println!("POST tools/call create_note (rejected Origin) => HTTP {status} {rejected_origin}");

    let (status, headers, missing_name) =
        post_json(address, "tools/call", None, create_params.clone(), 6).await;
    assert_eq!(status, 400);
    assert_eq!(missing_name["error"]["code"], -32020);
    assert_eq!(missing_name["error"]["message"], "HeaderMismatch");
    assert!(!headers.to_ascii_lowercase().contains("mcp-session-id"));
    println!("POST tools/call create_note (missing Mcp-Name) => HTTP {status} {missing_name}");

    let (status, headers, mismatched_name) =
        post_json(address, "tools/call", Some("list_notes"), create_params, 7).await;
    assert_eq!(status, 400);
    assert_eq!(mismatched_name["error"]["code"], -32020);
    assert_eq!(mismatched_name["error"]["message"], "HeaderMismatch");
    assert!(!headers.to_ascii_lowercase().contains("mcp-session-id"));
    println!(
        "POST tools/call create_note (mismatched Mcp-Name) => HTTP {status} {mismatched_name}"
    );

    let mut unknown_params = metadata();
    unknown_params["name"] = json!("undeclared_fixture_tool");
    unknown_params["arguments"] = json!({});
    let (status, headers, unknown_tool) = post_json(
        address,
        "tools/call",
        Some("undeclared_fixture_tool"),
        unknown_params,
        8,
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(unknown_tool["error"]["code"], -32602);
    assert_eq!(
        unknown_tool["error"]["message"],
        "tools/call names an undeclared tool"
    );
    assert!(!headers.to_ascii_lowercase().contains("mcp-session-id"));
    println!("POST tools/call undeclared_fixture_tool => HTTP {status} {unknown_tool}");

    let mut stats_params = metadata();
    stats_params["name"] = json!("note_stats");
    stats_params["arguments"] = json!({});
    let (stats_status, _, stats_after) =
        post_json(address, "tools/call", Some("note_stats"), stats_params, 9).await;
    assert_eq!(stats_status, 200);
    assert_eq!(stats_after["result"]["structuredContent"], stats_before);
    println!("Fixture note_stats before/after rejected calls => {stats_before}");
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

    let (discover_status, discover_headers, discover) =
        post_json(address, "server/discover", None, metadata(), 1).await;
    assert_eq!(discover_status, 200);
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
    assert_eq!(discover["result"]["capabilities"]["tools"], json!({}));
    println!("POST server/discover => HTTP {discover_status}; {discover}");

    let (list_status, list_headers, list) =
        post_json(address, "tools/list", None, metadata(), 2).await;
    assert_eq!(list_status, 200);
    assert!(
        !list_headers
            .to_ascii_lowercase()
            .contains("access-control-allow")
    );
    let generated: Value = serde_json::from_str(GENERATED_MCP_JSON).unwrap();
    assert_eq!(list["result"]["tools"], generated["tools"]);
    println!("POST tools/list => HTTP {list_status}; {list}");

    let mut call_params = metadata();
    call_params["name"] = json!("list_notes");
    call_params["arguments"] = json!({"limit": 1});
    let (call_status, call_headers, call) =
        post_json(address, "tools/call", Some("list_notes"), call_params, 3).await;
    assert_eq!(call_status, 200);
    assert!(!call_headers.to_ascii_lowercase().contains("mcp-session-id"));
    assert_eq!(call["result"]["resultType"], "complete");
    assert_eq!(call["result"]["structuredContent"][0]["id"], "n2");
    assert_eq!(
        call["result"]["structuredContent"][0]["title"],
        "spike followup"
    );
    assert_eq!(call["result"]["content"][0]["type"], "text");
    println!("POST tools/call list_notes => HTTP {call_status}; {call}");

    rejected_write_requests_preserve_fixture(address).await;

    shutdown.send(()).unwrap();
    server.await.unwrap();
}
