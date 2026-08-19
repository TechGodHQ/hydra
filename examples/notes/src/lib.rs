//! Notes example: the handwritten part of a hydra project.
//!
//! Everything here is ordinary Rust — a store, an operation dispatcher, and
//! thin surface binaries. The generated surfaces (committed under
//! `generated/`) route CLI args, HTTP requests, and MCP tool calls into the
//! same `execute_operation` dispatch. Regenerate with
//! `cargo run -p hydra-codegen -- write` from the workspace root.

use std::sync::{Arc, Mutex};

use axum::{Router, http::StatusCode, response::IntoResponse};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod generated {
    include!("../generated/http.rs");
}

pub mod generated_cli {
    include!("../generated/cli.rs");
}

/// Constant path to generated MCP tool schemas.
pub const GENERATED_MCP_JSON: &str = include_str!("../generated/mcp.json");

/// A note in the store.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Note {
    /// Stable note id.
    pub id: String,
    /// Note title.
    pub title: String,
    /// Note body.
    pub body: String,
}

/// Aggregate stats about the store.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Stats {
    /// Total number of notes.
    pub count: u32,
    /// Total characters across all bodies.
    pub chars: u64,
}

/// Shared application state.
#[derive(Clone, Default)]
pub struct AppState {
    notes: Arc<Mutex<Vec<Note>>>,
}

impl AppState {
    /// Seed the store with fixture notes.
    #[must_use]
    pub fn with_fixtures() -> Self {
        let state = Self::default();
        state.notes.lock().expect("notes lock").extend([
            Note {
                id: "n1".into(),
                title: "hello hydra".into(),
                body: "first fixture note".into(),
            },
            Note {
                id: "n2".into(),
                title: "spike followup".into(),
                body: "extract the codegen".into(),
            },
        ]);
        state
    }
}

/// Typed operation error carrying an HTTP status, so every surface reports
/// the same status/message pair.
#[derive(Debug)]
pub struct OperationError {
    /// HTTP status for the HTTP surface.
    pub status: StatusCode,
    /// Human-readable message used on all surfaces.
    pub message: String,
}

impl std::fmt::Display for OperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for OperationError {}

/// The single dispatch every generated surface funnels through: the one
/// place operation routing lives. Returns a JSON value or a typed error; the
/// HTTP adapter below maps the error to a status code.
///
/// Kept `async` so real projects can await I/O here; the fixture store is
/// synchronous.
#[allow(clippy::unused_async)]
pub async fn execute_operation(
    state: &AppState,
    operation: &str,
    input: generated::GeneratedOperationInput,
) -> Result<Value, OperationError> {
    match operation {
        "list_notes" => {
            let limit = parse_opt_u32(input.query.get("limit"))?;
            let limit = usize::try_from(limit.unwrap_or(50).min(100_000)).unwrap_or(50);
            let notes: Vec<Note> = {
                let guard = state.notes.lock().expect("notes lock");
                guard.iter().rev().take(limit).cloned().collect()
            };
            Ok(serde_json::to_value(notes).expect("notes serialize"))
        }
        "get_note" => {
            let id = path_str(&input, "note_id")?;
            state
                .notes
                .lock()
                .expect("notes lock")
                .iter()
                .find(|n| n.id == id)
                .cloned()
                .map(|n| serde_json::to_value(n).expect("note serialize"))
                .ok_or_else(|| not_found(&format!("note not found: {id}")))
        }
        "create_note" => {
            #[derive(Deserialize)]
            struct CreateArgs {
                title: String,
                body: String,
            }
            let args: CreateArgs = serde_json::from_value(input.body)
                .map_err(|e| bad_request(&format!("invalid body: {e}")))?;
            let note = {
                let mut notes = state.notes.lock().expect("notes lock");
                let note = Note {
                    id: format!("n{}", notes.len() + 1),
                    title: args.title,
                    body: args.body,
                };
                notes.push(note.clone());
                note
            };
            Ok(serde_json::to_value(note).expect("note serialize"))
        }
        "delete_note" => {
            let id = path_str(&input, "note_id")?;
            let removed = {
                let mut notes = state.notes.lock().expect("notes lock");
                let before = notes.len();
                notes.retain(|n| n.id != id);
                before - notes.len()
            };
            if removed == 0 {
                return Err(not_found(&format!("note not found: {id}")));
            }
            Ok(Value::Null)
        }
        "note_stats" => {
            let out_stats = {
                let notes = state.notes.lock().expect("notes lock");
                Stats {
                    count: u32::try_from(notes.len()).unwrap_or(u32::MAX),
                    chars: notes.iter().map(|n| n.body.chars().count() as u64).sum(),
                }
            };
            Ok(serde_json::to_value(out_stats).expect("stats serialize"))
        }
        other => Err(bad_request(&format!("unknown operation: {other}"))),
    }
}

/// CLI-only compaction job (not exposed over HTTP or MCP).
#[allow(clippy::unused_async)]
pub async fn compact_notes(state: &AppState) -> usize {
    let mut notes = state.notes.lock().expect("notes lock");
    let before = notes.len();
    notes.retain(|n| !n.body.trim().is_empty());
    before - notes.len()
}

fn path_str(
    input: &generated::GeneratedOperationInput,
    key: &str,
) -> Result<String, OperationError> {
    input
        .path
        .get(key)
        .cloned()
        .ok_or_else(|| bad_request(&format!("missing path parameter: {key}")))
}

fn parse_opt_u32(value: Option<&String>) -> Result<Option<u32>, OperationError> {
    value
        .map(|raw| {
            raw.parse()
                .map_err(|_| bad_request(&format!("invalid u32: {raw}")))
        })
        .transpose()
}

fn not_found(message: &str) -> OperationError {
    OperationError {
        status: StatusCode::NOT_FOUND,
        message: message.to_string(),
    }
}

fn bad_request(message: &str) -> OperationError {
    OperationError {
        status: StatusCode::BAD_REQUEST,
        message: message.to_string(),
    }
}

/// HTTP adapter: run the operation and map typed errors to status codes.
/// This is the shape generated handlers call into.
pub async fn execute_operation_http(
    state: &AppState,
    operation: &str,
    input: generated::GeneratedOperationInput,
) -> axum::response::Response {
    match execute_operation(state, operation, input).await {
        Ok(value) => axum::Json(value).into_response(),
        Err(err) => (
            err.status,
            axum::Json(serde_json::json!({ "error": err.message })),
        )
            .into_response(),
    }
}

/// HTTP adapter for raw-request operations: the wire bytes and headers
/// arrive untouched, so signature verification over the exact received
/// representation is possible.
///
/// This example echoes them back as JSON.
#[allow(clippy::unused_async)]
pub async fn execute_operation_raw_http(
    state: &AppState,
    operation: &str,
    input: generated::GeneratedRawOperationInput,
) -> axum::response::Response {
    match operation {
        "echo_raw" => axum::Json(serde_json::json!({
            "bytes": input.raw_body,
            "bytes_len": input.raw_body.len(),
            "headers": input.headers,
        }))
        .into_response(),
        _ => {
            execute_operation_http(
                state,
                operation,
                generated::GeneratedOperationInput {
                    path: input.path,
                    query: input.query,
                    body: Value::Null,
                },
            )
            .await
        }
    }
}

// ── Surface wiring ────────────────────────────────────────────────────────

/// Build the full HTTP router for the notes service.
pub fn http_router(state: AppState) -> Router {
    generated::generated_router().with_state(state)
}

/// HTTP entry: `notes-http` serves on 127.0.0.1:8941.
pub async fn run_http() -> anyhow::Result<()> {
    let app = http_router(AppState::with_fixtures());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8941").await?;
    println!("listening on http://127.0.0.1:8941");
    axum::serve(listener, app).await?;
    Ok(())
}

/// CLI entry: parse args, dispatch through the same operation path.
pub async fn run_cli() -> anyhow::Result<()> {
    #[derive(Parser)]
    #[command(name = "notes")]
    struct Cli {
        #[command(subcommand)]
        command: generated_cli::GeneratedCommand,
    }

    let cli = Cli::parse();
    let state = AppState::with_fixtures();

    if let generated_cli::GeneratedCommand::CompactNotes(_) = cli.command {
        let removed = compact_notes(&state).await;
        println!("compacted: removed {removed} empty notes");
        return Ok(());
    }

    let operation = cli.command.operation_name();
    let params = cli.command.parameters_json();
    let input = generated::GeneratedOperationInput {
        path: params
            .as_object()
            .map(|o| {
                o.iter()
                    .filter(|(_, v)| v.is_string())
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        query: params
            .as_object()
            .map(|o| {
                o.iter()
                    .filter(|(_, v)| v.is_number() || v.is_boolean())
                    .map(|(k, v)| (k.clone(), v.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        body: params.clone(),
    };

    match execute_operation(&state, operation, input).await {
        Ok(value) => {
            println!("{value}");
            Ok(())
        }
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
}

/// MCP entry: serve tools from generated mcp.json over stdio.
pub async fn run_mcp() -> anyhow::Result<()> {
    let tools: Value = serde_json::from_str(GENERATED_MCP_JSON)?;
    let state = AppState::with_fixtures();
    // Route tool arguments into path/query/body using the location metadata
    // emitted alongside the tool schemas — no name inference.
    let locations = tools
        .get("locations")
        .and_then(|l| l.get("tools"))
        .cloned()
        .unwrap_or_else(|| tools.get("locations").cloned().unwrap_or(Value::Null));

    hydra_mcp_stdio::serve(
        "notes",
        env!("CARGO_PKG_VERSION"),
        tools,
        move |name, args| {
            let state = state.clone();
            let locations = locations.clone();
            async move {
                let mut path = std::collections::BTreeMap::new();
                let mut query = std::collections::BTreeMap::new();
                let mut body = Value::Null;
                let op_locations = locations
                    .as_object()
                    .and_then(|o| o.get(&name))
                    .and_then(|v| v.as_object())
                    .cloned()
                    .unwrap_or_default();
                let mut body_map = serde_json::Map::new();
                for (key, location) in &op_locations {
                    let value = args.get(key).cloned().unwrap_or(Value::Null);
                    match location.as_str().unwrap_or("body") {
                        "path" => {
                            path.insert(
                                key.clone(),
                                value.as_str().unwrap_or_default().to_string(),
                            );
                        }
                        "query" => {
                            query.insert(
                                key.clone(),
                                value.to_string().trim_matches('"').to_string(),
                            );
                        }
                        _ => {
                            body_map.insert(key.clone(), value);
                        }
                    }
                }
                if !body_map.is_empty() {
                    body = Value::Object(body_map);
                }
                let input = generated::GeneratedOperationInput { path, query, body };
                match execute_operation(&state, &name, input).await {
                    Ok(value) => Ok(value),
                    Err(err) => Err(err.message),
                }
            }
        },
    )
    .await
    .map_err(|e| anyhow::anyhow!("mcp stdio error: {e}"))?;
    Ok(())
}
