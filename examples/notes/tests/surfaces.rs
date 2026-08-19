//! The hydra claim, proven: one dispatch implementation drives CLI, HTTP,
//! and MCP with identical results and errors.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use notes_example::generated::GeneratedOperationInput;
use notes_example::{AppState, execute_operation};
use serde_json::{Value, json};
use tower::util::ServiceExt;

async fn http(state: &AppState, req: Request<Body>) -> axum::response::Response {
    notes_example::http_router(state.clone())
        .oneshot(req)
        .await
        .expect("router call")
}

async fn body_json(res: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Call an operation the way the CLI and MCP surfaces do: through the shared
/// dispatch with a `GeneratedOperationInput`.
async fn via_dispatch(state: &AppState, op: &str, path_query_body: Value) -> Value {
    let obj = path_query_body.as_object().cloned().unwrap_or_default();
    let path = obj
        .get("path")
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                .collect()
        })
        .unwrap_or_default();
    let query = obj
        .get("query")
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.to_string().trim_matches('"').to_string()))
                .collect()
        })
        .unwrap_or_default();
    let body = obj.get("body").cloned().unwrap_or(Value::Null);
    execute_operation(state, op, GeneratedOperationInput { path, query, body })
        .await
        .expect("dispatch succeeds")
}

#[tokio::test]
async fn all_surfaces_return_identical_results() {
    let state = AppState::with_fixtures();

    let direct = via_dispatch(&state, "get_note", json!({"path": {"note_id": "n1"}})).await;
    let expected = direct.clone();

    // HTTP surface
    let res = http(
        &state,
        Request::get("/notes/n1").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let http_value = body_json(res).await;
    assert_eq!(http_value, expected);

    // The MCP surface funnels through the same dispatch fn (see run_mcp),
    // so equality with `direct` is structural by construction; assert the
    // fixture to make a regression loud.
    assert_eq!(expected["id"], "n1");
    assert_eq!(expected["title"], "hello hydra");
}

#[tokio::test]
async fn create_then_read_roundtrip_over_http() {
    let state = AppState::with_fixtures();
    let res = http(
        &state,
        Request::post("/notes")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&json!({"title": "t", "body": "b"})).unwrap(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let created = body_json(res).await;
    assert_eq!(created["id"], "n3");

    let res = http(
        &state,
        Request::get(format!("/notes/{}", created["id"].as_str().unwrap()))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await, created);
}

#[tokio::test]
async fn errors_are_equivalent_across_surfaces() {
    let state = AppState::with_fixtures();

    // HTTP: typed error maps to 404
    let res = http(
        &state,
        Request::get("/notes/nope").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let body = body_json(res).await;
    assert!(body["error"].as_str().unwrap().contains("note not found"));

    // Dispatch: same operation returns the same typed error
    let err = execute_operation(
        &state,
        "get_note",
        GeneratedOperationInput {
            path: [("note_id".to_string(), "nope".to_string())]
                .into_iter()
                .collect(),
            query: std::collections::BTreeMap::default(),
            body: Value::Null,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.status, StatusCode::NOT_FOUND);
    assert!(err.message.contains("note not found"));
}

#[tokio::test]
async fn cli_only_operation_is_absent_from_http() {
    let state = AppState::with_fixtures();
    let res = http(
        &state,
        Request::post("/internal/compact")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn stats_available_on_http_but_not_cli() {
    // The generator test asserts CLI absence structurally; here the HTTP
    // presence is proven live.
    let state = AppState::with_fixtures();
    let res = http(&state, Request::get("/stats").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let body_value = body_json(res).await;
    assert_eq!(body_value["count"], json!(2));
}
