use std::sync::{Arc, Mutex};

use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use hydra_mcp_http::{OriginPolicy, PROTOCOL_VERSION, ServerConfig, router};
use serde_json::{Value, json};
use tower::ServiceExt;

fn manifest(tool_name: &str) -> Value {
    json!([{
        "name": tool_name,
        "description": "fixture tool",
        "inputSchema": {"type": "object", "additionalProperties": false}
    }])
}

fn config(tools: Value) -> ServerConfig {
    ServerConfig::new("fixture", "1.2.3", tools, OriginPolicy::RejectPresented)
}

fn request(method: &str, id: u64, params: &Value) -> Request<Body> {
    Request::post("/")
        .header("content-type", "application/json; charset=utf-8")
        .header("accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", PROTOCOL_VERSION)
        .header("Mcp-Method", method)
        .body(Body::from(
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string(),
        ))
        .unwrap()
}

fn params() -> Value {
    json!({
        "_meta": {
            "io.modelcontextprotocol/protocolVersion": PROTOCOL_VERSION,
            "io.modelcontextprotocol/clientCapabilities": {}
        }
    })
}

async fn json_body(response: Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn discover_list_and_call_use_the_real_router_boundary() {
    let tools = manifest("echo");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let calls_for_dispatch = calls.clone();
    let app = router(config(tools.clone()), move |name, arguments| {
        let calls = calls_for_dispatch.clone();
        async move {
            calls.lock().unwrap().push((name, arguments));
            Ok(json!({"answer": 42}))
        }
    })
    .unwrap();

    let discover = app
        .clone()
        .oneshot(request("server/discover", 1, &params()))
        .await
        .unwrap();
    assert_eq!(discover.status(), StatusCode::OK);
    let discover_body = json_body(discover).await;
    assert_eq!(discover_body["result"]["resultType"], "complete");
    assert_eq!(
        discover_body["result"]["supportedVersions"],
        json!([PROTOCOL_VERSION])
    );
    assert_eq!(
        discover_body["result"]["_meta"]["io.modelcontextprotocol/serverInfo"],
        json!({"name": "fixture", "version": "1.2.3"})
    );

    let list = app
        .clone()
        .oneshot(request("tools/list", 2, &params()))
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    assert_eq!(json_body(list).await["result"]["tools"], tools);

    let mut call_params = params();
    call_params["name"] = json!("echo");
    call_params["arguments"] = json!({"value": "hello"});
    let call_request = Request::post("/")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", PROTOCOL_VERSION)
        .header("Mcp-Method", "tools/call")
        .header("Mcp-Name", "echo")
        .body(Body::from(
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": call_params
            })
            .to_string(),
        ))
        .unwrap();
    let call = app.oneshot(call_request).await.unwrap();
    assert_eq!(call.status(), StatusCode::OK);
    assert!(call.headers().get("Mcp-Session-Id").is_none());
    let call_body = json_body(call).await;
    assert_eq!(
        call_body["result"]["structuredContent"],
        json!({"answer": 42})
    );
    assert_eq!(call_body["result"]["content"][0]["text"], "{\"answer\":42}");
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &[("echo".to_owned(), json!({"value": "hello"}))]
    );
}

#[tokio::test]
async fn dispatch_errors_are_tool_results_and_invalid_ids_are_null() {
    let app = router(config(manifest("echo")), |_, _| async {
        Err::<Value, String>("safe failure".to_owned())
    })
    .unwrap();

    let mut invalid_id = request("tools/list", 1, &params());
    *invalid_id.body_mut() = Body::from(
        json!({
            "jsonrpc": "2.0",
            "id": {"not": "a valid JSON-RPC id"},
            "method": "tools/list",
            "params": params()
        })
        .to_string(),
    );
    let invalid_id_response = app.clone().oneshot(invalid_id).await.unwrap();
    let invalid_id_body = json_body(invalid_id_response).await;
    assert_eq!(invalid_id_body["id"], Value::Null);
    assert_eq!(invalid_id_body["error"]["code"], -32600);

    let mut call_params = params();
    call_params["name"] = json!("echo");
    call_params["arguments"] = json!({});
    let mut call = request("tools/call", 2, &call_params);
    call.headers_mut()
        .insert("Mcp-Name", "echo".parse().unwrap());
    let call_body = app.oneshot(call).await.unwrap();
    assert_eq!(call_body.status(), StatusCode::OK);
    let call_body = json_body(call_body).await;
    assert_eq!(call_body["result"]["isError"], true);
    assert_eq!(call_body["result"]["content"][0]["text"], "safe failure");
}

#[tokio::test]
async fn header_mismatch_happens_before_dispatch() {
    let dispatched = Arc::new(Mutex::new(false));
    let dispatched_for_closure = dispatched.clone();
    let app = router(config(manifest("echo")), move |_, _| {
        let dispatched = dispatched_for_closure.clone();
        async move {
            *dispatched.lock().unwrap() = true;
            Ok(json!(null))
        }
    })
    .unwrap();
    let mut req = request("tools/list", 1, &params());
    req.headers_mut()
        .insert("Mcp-Method", "tools/call2".parse().unwrap());
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], -32020);
    assert_eq!(body["error"]["message"], "HeaderMismatch");
    assert!(!*dispatched.lock().unwrap());
}

#[tokio::test]
async fn origin_is_checked_before_decoding_and_exact_allowlists_are_inbound_only() {
    let app = router(config(manifest("echo")), |_, _| async { Ok(json!(null)) }).unwrap();
    let rejected = app
        .clone()
        .oneshot(
            Request::post("/")
                .header("origin", "https://untrusted.example")
                .body(Body::from("not-json"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    assert!(
        rejected
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    );

    let allowed_config = ServerConfig::new(
        "fixture",
        "1.2.3",
        manifest("echo"),
        OriginPolicy::AllowExact(vec!["https://trusted.example".to_owned()]),
    );
    let allowed = router(allowed_config, |_, _| async { Ok(json!(null)) })
        .unwrap()
        .oneshot({
            let mut request = request("tools/list", 1, &params());
            request
                .headers_mut()
                .insert("origin", "https://trusted.example".parse().unwrap());
            request
        })
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::OK);
}

#[tokio::test]
async fn sentinel_encoded_tool_names_are_compared_after_decoding() {
    let name = "世界";
    let encoded = STANDARD.encode(name);
    let app = router(config(manifest(name)), |_, _| async { Ok(json!("ok")) }).unwrap();
    let mut call_params = params();
    call_params["name"] = json!(name);
    let call_request = Request::post("/")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", PROTOCOL_VERSION)
        .header("Mcp-Method", "tools/call")
        .header("Mcp-Name", format!("=?base64?{encoded}?="))
        .body(Body::from(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": call_params
            })
            .to_string(),
        ))
        .unwrap();
    let response = app.oneshot(call_request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json_body(response).await["result"]["structuredContent"],
        "ok"
    );
}

#[tokio::test]
async fn unsupported_version_and_legacy_methods_are_truthful() {
    let app = router(config(manifest("echo")), |_, _| async { Ok(json!(null)) }).unwrap();
    let mut unsupported = request("tools/list", 1, &params());
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {"_meta": {
            "io.modelcontextprotocol/protocolVersion": "2099-01-01",
            "io.modelcontextprotocol/clientCapabilities": {}
        }}
    });
    *unsupported.body_mut() = Body::from(body.to_string());
    unsupported
        .headers_mut()
        .insert("MCP-Protocol-Version", "2099-01-01".parse().unwrap());
    let response = app.clone().oneshot(unsupported).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], -32022);
    assert_eq!(
        body["error"]["data"]["supportedVersions"],
        json!([PROTOCOL_VERSION])
    );

    let get = app
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::METHOD_NOT_ALLOWED);

    let legacy = router(config(manifest("echo")), |_, _| async { Ok(json!(null)) })
        .unwrap()
        .oneshot(request("initialize", 2, &params()))
        .await
        .unwrap();
    assert_eq!(legacy.status(), StatusCode::NOT_FOUND);
    assert_eq!(json_body(legacy).await["error"]["code"], -32601);

    let options = router(config(manifest("echo")), |_, _| async { Ok(json!(null)) })
        .unwrap()
        .oneshot(Request::options("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(options.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert!(
        options
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    );
}

#[test]
fn construction_rejects_duplicate_tools_and_header_annotations() {
    let duplicate = json!([
        {"name": "same", "inputSchema": {"type": "object"}},
        {"name": "same", "inputSchema": {"type": "object"}}
    ]);
    assert!(router(config(duplicate), |_, _| async { Ok(json!(null)) }).is_err());

    let header_annotation = json!([{
        "name": "header",
        "inputSchema": {"type": "object", "properties": {"token": {"x-mcp-header": "X-Token"}}}
    }]);
    let error = router(config(header_annotation), |_, _| async { Ok(json!(null)) })
        .unwrap_err()
        .to_string();
    assert!(error.contains("header"));
    assert!(error.contains("x-mcp-header"));
}
