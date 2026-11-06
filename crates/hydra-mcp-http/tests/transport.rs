use std::sync::{Arc, Mutex};

use axum::{
    body::Body,
    http::{HeaderValue, Request, StatusCode},
    response::Response,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use hydra_mcp_http::{OriginPolicy, PROTOCOL_VERSION, ServerConfig, router};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
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

fn call_request(name: &str, arguments: Option<Value>) -> Request<Body> {
    let mut call_params = params();
    call_params["name"] = json!(name);
    if let Some(arguments) = arguments {
        call_params["arguments"] = arguments;
    }
    let mut request = request("tools/call", 1, &call_params);
    request
        .headers_mut()
        .insert("Mcp-Name", HeaderValue::from_str(name).unwrap());
    request
}

macro_rules! replace_body {
    ($request:expr, $value:expr $(,)?) => {{
        *$request.body_mut() = Body::from(($value).to_string());
    }};
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
    assert_eq!(
        json_body(legacy).await["error"],
        json!({
            "code": -32601,
            "message": format!("Method not found for protocol {PROTOCOL_VERSION}")
        })
    );

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

struct RejectionCase {
    name: &'static str,
    request: Request<Body>,
    status: StatusCode,
    code: i64,
    message: &'static str,
    id: Value,
}

const fn rejection_case(
    name: &'static str,
    request: Request<Body>,
    status: StatusCode,
    code: i64,
    message: &'static str,
    id: Value,
) -> RejectionCase {
    RejectionCase {
        name,
        request,
        status,
        code,
        message,
        id,
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Keep the rejection matrix's cases and common assertions together.
async fn named_rejection_matrix_is_static_value_free_and_never_dispatches() {
    let dispatches = Arc::new(AtomicUsize::new(0));
    let dispatches_for_closure = dispatches.clone();
    let app = router(config(manifest("echo")), move |_, _| {
        let dispatches = dispatches_for_closure.clone();
        async move {
            dispatches.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"unexpected": true}))
        }
    })
    .unwrap();
    let mut cases = Vec::new();

    let mut origin = Request::post("/")
        .header("Origin", "https://ORIGIN_SECRET_SENTINEL.invalid")
        .body(Body::from("BODY_SECRET_SENTINEL"))
        .unwrap();
    origin
        .headers_mut()
        .insert("Content-Type", HeaderValue::from_static("text/plain"));
    cases.push(rejection_case(
        "presented Origin precedes media and JSON decoding",
        origin,
        StatusCode::FORBIDDEN,
        -32600,
        "Origin rejected",
        Value::Null,
    ));

    let mut missing_content_type = call_request("echo", Some(json!({})));
    missing_content_type.headers_mut().remove("content-type");
    cases.push(rejection_case(
        "missing Content-Type",
        missing_content_type,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        -32600,
        "Content-Type must be application/json",
        Value::Null,
    ));
    let mut wrong_content_type = call_request("echo", Some(json!({})));
    wrong_content_type
        .headers_mut()
        .insert("content-type", HeaderValue::from_static("text/plain"));
    cases.push(rejection_case(
        "unsupported Content-Type",
        wrong_content_type,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        -32600,
        "Content-Type must be application/json",
        Value::Null,
    ));
    let mut missing_accept = call_request("echo", Some(json!({})));
    missing_accept.headers_mut().remove("accept");
    cases.push(rejection_case(
        "missing Accept",
        missing_accept,
        StatusCode::NOT_ACCEPTABLE,
        -32600,
        "Accept must include application/json and text/event-stream",
        Value::Null,
    ));
    let mut incomplete_accept = call_request("echo", Some(json!({})));
    incomplete_accept
        .headers_mut()
        .insert("accept", HeaderValue::from_static("application/json"));
    cases.push(rejection_case(
        "Accept omits text/event-stream",
        incomplete_accept,
        StatusCode::NOT_ACCEPTABLE,
        -32600,
        "Accept must include application/json and text/event-stream",
        Value::Null,
    ));

    let mut malformed_json = request("tools/list", 1, &params());
    *malformed_json.body_mut() = Body::from("BODY_SECRET_SENTINEL {");
    cases.push(rejection_case(
        "malformed JSON",
        malformed_json,
        StatusCode::BAD_REQUEST,
        -32600,
        "Invalid JSON-RPC request",
        Value::Null,
    ));
    let mut batch = request("tools/list", 2, &params());
    replace_body!(
        &mut batch,
        json!([{"jsonrpc":"2.0","id":2,"method":"tools/list","params":params()}]),
    );
    cases.push(rejection_case(
        "JSON-RPC batch",
        batch,
        StatusCode::BAD_REQUEST,
        -32600,
        "Invalid JSON-RPC request",
        Value::Null,
    ));
    let mut response_envelope = request("tools/list", 3, &params());
    replace_body!(
        &mut response_envelope,
        json!({"jsonrpc":"2.0","id":3,"result":{"value":"BODY_SECRET_SENTINEL"}}),
    );
    cases.push(rejection_case(
        "JSON-RPC response is not a request",
        response_envelope,
        StatusCode::BAD_REQUEST,
        -32600,
        "Invalid JSON-RPC request",
        json!(3),
    ));
    let mut notification = request("tools/list", 4, &params());
    replace_body!(
        &mut notification,
        json!({"jsonrpc":"2.0","method":"tools/list","params":params()}),
    );
    cases.push(rejection_case(
        "request id is required",
        notification,
        StatusCode::BAD_REQUEST,
        -32600,
        "Invalid JSON-RPC request",
        Value::Null,
    ));
    let mut invalid_id = request("tools/list", 5, &params());
    replace_body!(
        &mut invalid_id,
        json!({"jsonrpc":"2.0","id":{"value":"BODY_SECRET_SENTINEL"},"method":"tools/list","params":params()}),
    );
    cases.push(rejection_case(
        "invalid request id is null",
        invalid_id,
        StatusCode::BAD_REQUEST,
        -32600,
        "Invalid JSON-RPC request",
        Value::Null,
    ));
    let mut invalid_version = request("tools/list", 6, &params());
    replace_body!(
        &mut invalid_version,
        json!({"jsonrpc":"1.0","id":6,"method":"tools/list","params":params()}),
    );
    cases.push(rejection_case(
        "non-2.0 JSON-RPC envelope",
        invalid_version,
        StatusCode::BAD_REQUEST,
        -32600,
        "Invalid JSON-RPC request",
        json!(6),
    ));
    let mut control_method = request("tools/list", 7, &params());
    replace_body!(
        &mut control_method,
        json!({"jsonrpc":"2.0","id":7,"method":"tools/list\nSECRET_SENTINEL","params":params()}),
    );
    cases.push(rejection_case(
        "control character in method",
        control_method,
        StatusCode::BAD_REQUEST,
        -32600,
        "Invalid JSON-RPC request",
        json!(7),
    ));
    let mut non_object_params = request("tools/list", 8, &params());
    replace_body!(
        &mut non_object_params,
        json!({"jsonrpc":"2.0","id":8,"method":"tools/list","params":[]}),
    );
    cases.push(rejection_case(
        "non-object params",
        non_object_params,
        StatusCode::BAD_REQUEST,
        -32600,
        "Request metadata is required",
        json!(8),
    ));
    let mut missing_meta = request("tools/list", 9, &params());
    replace_body!(
        &mut missing_meta,
        json!({"jsonrpc":"2.0","id":9,"method":"tools/list","params":{}}),
    );
    cases.push(rejection_case(
        "missing _meta",
        missing_meta,
        StatusCode::BAD_REQUEST,
        -32600,
        "Request metadata is required",
        json!(9),
    ));
    let mut missing_body_version = request("tools/list", 10, &params());
    replace_body!(
        &mut missing_body_version,
        json!({"jsonrpc":"2.0","id":10,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/clientCapabilities":{}}}}),
    );
    cases.push(rejection_case(
        "missing body protocol version",
        missing_body_version,
        StatusCode::BAD_REQUEST,
        -32600,
        "Request protocol metadata is required",
        json!(10),
    ));
    let mut non_string_version = request("tools/list", 11, &params());
    replace_body!(
        &mut non_string_version,
        json!({"jsonrpc":"2.0","id":11,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":17,"io.modelcontextprotocol/clientCapabilities":{}}}}),
    );
    cases.push(rejection_case(
        "non-string body protocol version",
        non_string_version,
        StatusCode::BAD_REQUEST,
        -32600,
        "Request protocol metadata is required",
        json!(11),
    ));
    let mut missing_capabilities = request("tools/list", 12, &params());
    replace_body!(
        &mut missing_capabilities,
        json!({"jsonrpc":"2.0","id":12,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":PROTOCOL_VERSION}}}),
    );
    cases.push(rejection_case(
        "missing client capabilities",
        missing_capabilities,
        StatusCode::BAD_REQUEST,
        -32600,
        "Request client capabilities are required",
        json!(12),
    ));
    let mut non_object_capabilities = request("tools/list", 13, &params());
    replace_body!(
        &mut non_object_capabilities,
        json!({"jsonrpc":"2.0","id":13,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":PROTOCOL_VERSION,"io.modelcontextprotocol/clientCapabilities":[]}}}),
    );
    cases.push(rejection_case(
        "non-object client capabilities",
        non_object_capabilities,
        StatusCode::BAD_REQUEST,
        -32600,
        "Request client capabilities are required",
        json!(13),
    ));

    let mut missing_protocol = request("tools/list", 14, &params());
    missing_protocol
        .headers_mut()
        .remove("MCP-Protocol-Version");
    cases.push(rejection_case(
        "missing MCP-Protocol-Version",
        missing_protocol,
        StatusCode::BAD_REQUEST,
        -32020,
        "HeaderMismatch",
        json!(14),
    ));
    let mut malformed_protocol = request("tools/list", 15, &params());
    malformed_protocol.headers_mut().insert(
        "MCP-Protocol-Version",
        HeaderValue::from_bytes(&[0x80]).unwrap(),
    );
    cases.push(rejection_case(
        "non-text MCP-Protocol-Version",
        malformed_protocol,
        StatusCode::BAD_REQUEST,
        -32020,
        "HeaderMismatch",
        json!(15),
    ));
    let mut mismatched_protocol = request("tools/list", 16, &params());
    mismatched_protocol.headers_mut().insert(
        "MCP-Protocol-Version",
        HeaderValue::from_static("HEADER_SECRET_SENTINEL"),
    );
    cases.push(rejection_case(
        "mismatched protocol header",
        mismatched_protocol,
        StatusCode::BAD_REQUEST,
        -32020,
        "HeaderMismatch",
        json!(16),
    ));
    let mut unsupported_meta = params();
    unsupported_meta["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!("2099-01-01");
    let mut unsupported = request("tools/list", 17, &unsupported_meta);
    unsupported.headers_mut().insert(
        "MCP-Protocol-Version",
        HeaderValue::from_static("2099-01-01"),
    );
    cases.push(rejection_case(
        "matching unsupported version",
        unsupported,
        StatusCode::BAD_REQUEST,
        -32022,
        "UnsupportedProtocolVersionError",
        json!(17),
    ));
    let mut missing_method = request("tools/list", 18, &params());
    missing_method.headers_mut().remove("Mcp-Method");
    cases.push(rejection_case(
        "missing Mcp-Method",
        missing_method,
        StatusCode::BAD_REQUEST,
        -32020,
        "HeaderMismatch",
        json!(18),
    ));
    let mut malformed_method = request("tools/list", 19, &params());
    malformed_method
        .headers_mut()
        .insert("Mcp-Method", HeaderValue::from_bytes(&[0x80]).unwrap());
    cases.push(rejection_case(
        "non-text Mcp-Method",
        malformed_method,
        StatusCode::BAD_REQUEST,
        -32020,
        "HeaderMismatch",
        json!(19),
    ));
    let mut mismatched_method = request("tools/list", 20, &params());
    mismatched_method.headers_mut().insert(
        "Mcp-Method",
        HeaderValue::from_static("METHOD_SECRET_SENTINEL"),
    );
    cases.push(rejection_case(
        "mismatched Mcp-Method",
        mismatched_method,
        StatusCode::BAD_REQUEST,
        -32020,
        "HeaderMismatch",
        json!(20),
    ));

    let mut missing_name_params = params();
    missing_name_params["arguments"] = json!({});
    let mut missing_name = request("tools/call", 21, &missing_name_params);
    missing_name
        .headers_mut()
        .insert("Mcp-Name", HeaderValue::from_static("echo"));
    cases.push(rejection_case(
        "missing tools/call name",
        missing_name,
        StatusCode::BAD_REQUEST,
        -32602,
        "tools/call requires a tool name",
        json!(21),
    ));
    let mut non_string_name = call_request("echo", Some(json!({})));
    let mut non_string_name_params = params();
    non_string_name_params["name"] = json!(21);
    non_string_name_params["arguments"] = json!({});
    replace_body!(
        &mut non_string_name,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":non_string_name_params}),
    );
    cases.push(rejection_case(
        "non-string tool name",
        non_string_name,
        StatusCode::BAD_REQUEST,
        -32602,
        "tools/call requires a tool name",
        json!(1),
    ));
    let mut empty_name = call_request("echo", Some(json!({})));
    let mut empty_name_params = params();
    empty_name_params["name"] = json!("");
    empty_name_params["arguments"] = json!({});
    replace_body!(
        &mut empty_name,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":empty_name_params}),
    );
    cases.push(rejection_case(
        "empty tool name",
        empty_name,
        StatusCode::BAD_REQUEST,
        -32602,
        "tools/call has an invalid tool name",
        json!(1),
    ));
    let mut control_name = call_request("echo", Some(json!({})));
    let mut control_name_params = params();
    control_name_params["name"] = json!("echo\nSECRET_SENTINEL");
    control_name_params["arguments"] = json!({});
    replace_body!(
        &mut control_name,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":control_name_params}),
    );
    cases.push(rejection_case(
        "control character in tool name",
        control_name,
        StatusCode::BAD_REQUEST,
        -32602,
        "tools/call has an invalid tool name",
        json!(1),
    ));
    let mut missing_name_header = call_request("echo", Some(json!({})));
    missing_name_header.headers_mut().remove("Mcp-Name");
    cases.push(rejection_case(
        "missing Mcp-Name",
        missing_name_header,
        StatusCode::BAD_REQUEST,
        -32020,
        "HeaderMismatch",
        json!(1),
    ));
    let mut malformed_name_header = call_request("echo", Some(json!({})));
    malformed_name_header
        .headers_mut()
        .insert("Mcp-Name", HeaderValue::from_bytes(&[0x80]).unwrap());
    cases.push(rejection_case(
        "non-text Mcp-Name",
        malformed_name_header,
        StatusCode::BAD_REQUEST,
        -32020,
        "HeaderMismatch",
        json!(1),
    ));
    let mut mismatched_name = call_request("echo", Some(json!({})));
    mismatched_name
        .headers_mut()
        .insert("Mcp-Name", HeaderValue::from_static("NAME_SECRET_SENTINEL"));
    cases.push(rejection_case(
        "mismatched Mcp-Name",
        mismatched_name,
        StatusCode::BAD_REQUEST,
        -32020,
        "HeaderMismatch",
        json!(1),
    ));
    let mut malformed_base64 = call_request("echo", Some(json!({})));
    malformed_base64
        .headers_mut()
        .insert("Mcp-Name", HeaderValue::from_static("=?base64?%%%?="));
    cases.push(rejection_case(
        "malformed Base64 Mcp-Name",
        malformed_base64,
        StatusCode::BAD_REQUEST,
        -32020,
        "HeaderMismatch",
        json!(1),
    ));
    let mut encoded_control = call_request("echo", Some(json!({})));
    let encoded = STANDARD.encode("echo\nSECRET_SENTINEL");
    encoded_control.headers_mut().insert(
        "Mcp-Name",
        HeaderValue::from_str(&format!("=?base64?{encoded}?=")).unwrap(),
    );
    cases.push(rejection_case(
        "Base64 Mcp-Name decodes to control character",
        encoded_control,
        StatusCode::BAD_REQUEST,
        -32020,
        "HeaderMismatch",
        json!(1),
    ));
    let unknown = call_request("not_declared", Some(json!({})));
    cases.push(rejection_case(
        "undeclared tool with matching mirrored name",
        unknown,
        StatusCode::BAD_REQUEST,
        -32602,
        "tools/call names an undeclared tool",
        json!(1),
    ));
    let non_object_arguments = call_request("echo", Some(json!(["not", "an", "object"])));
    cases.push(rejection_case(
        "array tool arguments",
        non_object_arguments,
        StatusCode::BAD_REQUEST,
        -32602,
        "tools/call arguments must be an object",
        json!(1),
    ));
    let null_arguments = call_request("echo", Some(Value::Null));
    cases.push(rejection_case(
        "null tool arguments",
        null_arguments,
        StatusCode::BAD_REQUEST,
        -32602,
        "tools/call arguments must be an object",
        json!(1),
    ));

    // Preserve distinct rows found in the older partial candidate as well.
    let mut sse_only = call_request("echo", Some(json!({})));
    sse_only
        .headers_mut()
        .insert("accept", HeaderValue::from_static("text/event-stream"));
    cases.push(rejection_case(
        "Accept omits application/json",
        sse_only,
        StatusCode::NOT_ACCEPTABLE,
        -32600,
        "Accept must include application/json and text/event-stream",
        Value::Null,
    ));
    for (name, body, message) in [
        (
            "missing method",
            json!({"jsonrpc":"2.0","id":1,"params":params()}),
            "Invalid JSON-RPC request",
        ),
        (
            "non-string method",
            json!({"jsonrpc":"2.0","id":1,"method":17,"params":params()}),
            "Invalid JSON-RPC request",
        ),
        (
            "missing params",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call"}),
            "Request metadata is required",
        ),
        (
            "non-object _meta",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"_meta":[]}}),
            "Request metadata is required",
        ),
    ] {
        let mut request = call_request("echo", Some(json!({})));
        replace_body!(&mut request, body);
        cases.push(rejection_case(
            name,
            request,
            StatusCode::BAD_REQUEST,
            -32600,
            message,
            json!(1),
        ));
    }
    for (name, value) in [
        ("unterminated Base64 sentinel", "=?base64?YWJj"),
        ("Base64 non-UTF8 sentinel", "=?base64?/w==?="),
    ] {
        let mut request = call_request("echo", Some(json!({})));
        request
            .headers_mut()
            .insert("Mcp-Name", HeaderValue::from_static(value));
        cases.push(rejection_case(
            name,
            request,
            StatusCode::BAD_REQUEST,
            -32020,
            "HeaderMismatch",
            json!(1),
        ));
    }

    // Prove this exact router/counter is live before testing pre-dispatch rejections.
    let control = app
        .clone()
        .oneshot(call_request("echo", Some(json!({}))))
        .await
        .unwrap();
    assert_eq!(control.status(), StatusCode::OK);
    assert_eq!(dispatches.swap(0, Ordering::SeqCst), 1);
    for case in cases {
        let response = app.clone().oneshot(case.request).await.unwrap();
        assert_eq!(
            dispatches.load(Ordering::SeqCst),
            0,
            "{} dispatched",
            case.name
        );
        assert_eq!(response.status(), case.status, "{}", case.name);
        let headers = response.headers().clone();
        for header in headers.keys() {
            let name = header.as_str().to_ascii_lowercase();
            assert_ne!(name, "mcp-session-id", "{}", case.name);
            assert!(!name.starts_with("access-control-"), "{}", case.name);
        }
        let body = json_body(response).await;
        assert_eq!(body["jsonrpc"], "2.0", "{}", case.name);
        assert_eq!(body["id"], case.id, "{}", case.name);
        assert_eq!(body["error"]["code"], case.code, "{}", case.name);
        assert_eq!(body["error"]["message"], case.message, "{}", case.name);
        assert!(
            !body.to_string().contains("SECRET_SENTINEL"),
            "{} leaked request input",
            case.name
        );
        if case.code == -32022 {
            assert_eq!(
                body["error"]["data"]["supportedVersions"],
                json!([PROTOCOL_VERSION])
            );
        }
    }
    assert_eq!(dispatches.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn omitted_tool_arguments_dispatch_as_an_empty_object() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let calls_for_dispatch = calls.clone();
    let app = router(config(manifest("echo")), move |name, arguments| {
        let calls = calls_for_dispatch.clone();
        async move {
            calls.lock().unwrap().push((name, arguments));
            Ok(json!({"answer": "default"}))
        }
    })
    .unwrap();
    let mut call_params = params();
    call_params["name"] = json!("echo");
    let mut request = request("tools/call", 1, &call_params);
    request
        .headers_mut()
        .insert("Mcp-Name", HeaderValue::from_static("echo"));
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json_body(response).await["result"]["structuredContent"]["answer"],
        "default"
    );
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &[("echo".to_owned(), json!({}))]
    );
}

#[tokio::test]
async fn exact_origin_allowlist_rejection_is_pre_dispatch_and_emits_no_browser_headers() {
    let dispatches = Arc::new(AtomicUsize::new(0));
    let dispatches_for_closure = dispatches.clone();
    let app = router(
        ServerConfig::new(
            "fixture",
            "1.2.3",
            manifest("echo"),
            OriginPolicy::AllowExact(vec!["https://trusted.example".to_owned()]),
        ),
        move |_, _| {
            let dispatches = dispatches_for_closure.clone();
            async move {
                dispatches.fetch_add(1, Ordering::SeqCst);
                Ok(json!(null))
            }
        },
    )
    .unwrap();
    let mut allowed_request = call_request("echo", Some(json!({})));
    allowed_request.headers_mut().insert(
        "Origin",
        HeaderValue::from_static("https://trusted.example"),
    );
    let allowed_response = app.clone().oneshot(allowed_request).await.unwrap();
    assert_eq!(allowed_response.status(), StatusCode::OK);
    assert_eq!(
        dispatches.swap(0, Ordering::SeqCst),
        1,
        "valid allowed-origin control should dispatch exactly once"
    );

    let mut request = call_request("echo", Some(json!({})));
    request.headers_mut().insert(
        "Origin",
        HeaderValue::from_static("https://UNTRUSTED_SECRET_SENTINEL.example"),
    );
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    for header in response.headers().keys() {
        let name = header.as_str().to_ascii_lowercase();
        assert_ne!(name, "mcp-session-id");
        assert!(!name.starts_with("access-control-"));
    }
    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], -32600);
    assert_eq!(body["error"]["message"], "Origin rejected");
    assert!(!body.to_string().contains("SECRET_SENTINEL"));
    assert_eq!(dispatches.load(Ordering::SeqCst), 0);
}

#[test]
#[allow(clippy::too_many_lines)] // Keep the construction rejection cases together.
fn construction_rejection_matrix_names_tools_without_reflecting_manifest_values() {
    let cases = vec![
        (
            "manifest wrapper shape",
            json!("MANIFEST_SECRET_SENTINEL"),
            "manifest must be a tools array or object wrapper",
            None,
        ),
        (
            "wrapper missing tools",
            json!({"other":"MANIFEST_SECRET_SENTINEL"}),
            "manifest wrapper must contain a tools array",
            Some("MANIFEST_SECRET_SENTINEL"),
        ),
        (
            "wrapper tools shape",
            json!({"tools":"MANIFEST_SECRET_SENTINEL"}),
            "manifest wrapper must contain a tools array",
            Some("MANIFEST_SECRET_SENTINEL"),
        ),
        (
            "tool entry shape",
            json!(["MANIFEST_SECRET_SENTINEL"]),
            "tool 0 must be an object",
            Some("MANIFEST_SECRET_SENTINEL"),
        ),
        (
            "missing tool name",
            json!([{"inputSchema":{"type":"object"}}]),
            "tool 0 must have a string name",
            None,
        ),
        (
            "non-string tool name",
            json!([{"name":17,"inputSchema":{"type":"object"}}]),
            "tool 0 must have a string name",
            None,
        ),
        (
            "empty tool name",
            json!([{"name":"","inputSchema":{"type":"object"}}]),
            "tool 0 has an invalid name",
            None,
        ),
        (
            "control character tool name",
            json!([{"name":"bad\nname","inputSchema":{"type":"object"}}]),
            "tool 0 has an invalid name",
            None,
        ),
        (
            "missing inputSchema",
            json!([{"name":"fixture"}]),
            "tool fixture is missing inputSchema",
            None,
        ),
        (
            "non-object inputSchema",
            json!([{"name":"fixture","inputSchema":"SCHEMA_SECRET_SENTINEL"}]),
            "tool fixture inputSchema must be an object",
            Some("SCHEMA_SECRET_SENTINEL"),
        ),
        (
            "duplicate tool names",
            json!([{"name":"same","inputSchema":{"type":"object"}},{"name":"same","inputSchema":{"type":"object"}}]),
            "tool name is duplicated: same",
            None,
        ),
        (
            "nested annotation",
            json!([{"name":"annotation_fixture","inputSchema":{"properties":{"token":{"x-mcp-header":"ANNOTATION_SECRET_SENTINEL"}}}}]),
            "annotation_fixture uses unsupported x-mcp-header",
            Some("ANNOTATION_SECRET_SENTINEL"),
        ),
        (
            "array-nested annotation",
            json!([{"name":"array_annotation_fixture","inputSchema":{"properties":{"items":{"type":"array","items":{"properties":{"token":{"x-mcp-header":"ARRAY_ANNOTATION_SECRET_SENTINEL"}}}}}}}]),
            "array_annotation_fixture uses unsupported x-mcp-header",
            Some("ARRAY_ANNOTATION_SECRET_SENTINEL"),
        ),
        (
            "oneOf array annotation",
            json!([{"name":"union_annotation_fixture","inputSchema":{"properties":{"variant":{"oneOf":[{"properties":{"token":{"x-mcp-header":"UNION_ANNOTATION_SECRET_SENTINEL"}}}]}}}}]),
            "union_annotation_fixture uses unsupported x-mcp-header",
            Some("UNION_ANNOTATION_SECRET_SENTINEL"),
        ),
    ];
    for (name, tools, expected, secret) in cases {
        let result = router(config(tools), |_, _| async { Ok(json!(null)) });
        let error = result
            .err()
            .unwrap_or_else(|| panic!("{name}: invalid manifest accepted"));
        let message = error.to_string();
        assert!(message.contains(expected), "{name}: {message}");
        assert!(
            !message.contains("SECRET_SENTINEL"),
            "{name}: leaked a manifest value"
        );
        if let Some(secret) = secret {
            assert!(secret.contains("SECRET_SENTINEL"), "test sentinel missing");
        }
        if name.contains("annotation") {
            assert!(message.contains("x-mcp-header"), "{name}: {message}");
        }
    }
}

#[test]
fn construction_rejects_invalid_exact_origin_entries() {
    let cases = vec![
        ("empty", vec![String::new()]),
        (
            "surrounding whitespace",
            vec![" https://trusted.example".to_owned()],
        ),
        (
            "control character",
            vec!["https://trusted.example\n".to_owned()],
        ),
        (
            "duplicate",
            vec![
                "https://trusted.example".to_owned(),
                "https://trusted.example".to_owned(),
            ],
        ),
    ];
    for (name, origins) in cases {
        let result = router(
            ServerConfig::new(
                "fixture",
                "1.2.3",
                manifest("echo"),
                OriginPolicy::AllowExact(origins),
            ),
            |_, _| async { Ok(json!(null)) },
        );
        let error = result
            .err()
            .unwrap_or_else(|| panic!("{name}: invalid policy accepted"));
        let message = error.to_string();
        assert!(
            message.starts_with("invalid MCP Origin policy:"),
            "{name}: {message}"
        );
        assert!(
            !message.contains("trusted.example"),
            "{name}: echoed configuration input"
        );
    }
}

#[tokio::test]
async fn realistic_options_preflight_and_delete_remain_unsupported_without_headers() {
    let app = router(config(manifest("echo")), |_, _| async { Ok(json!(null)) }).unwrap();
    let options = app
        .clone()
        .oneshot(
            Request::options("/")
                .header("Origin", "https://untrusted.example")
                .header("Access-Control-Request-Method", "POST")
                .header(
                    "Access-Control-Request-Headers",
                    "content-type,mcp-protocol-version,mcp-method,mcp-name",
                )
                .header("Access-Control-Request-Private-Network", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(options.status(), StatusCode::METHOD_NOT_ALLOWED);
    for header in options.headers().keys() {
        let name = header.as_str().to_ascii_lowercase();
        assert_ne!(name, "mcp-session-id");
        assert!(!name.starts_with("access-control-"));
    }

    let delete = app
        .oneshot(
            Request::delete("/")
                .header("Mcp-Session-Id", "SESSION_SECRET_SENTINEL")
                .header("Last-Event-ID", "RESUME_SECRET_SENTINEL")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::METHOD_NOT_ALLOWED);
    for header in delete.headers().keys() {
        let name = header.as_str().to_ascii_lowercase();
        assert_ne!(name, "mcp-session-id");
        assert!(!name.starts_with("access-control-"));
    }
}
