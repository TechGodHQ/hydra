//! Fixture-backed security-scan consumer.
//!
//! Hydra projects the declared operation onto CLI, HTTP, and MCP. This crate
//! owns validation, scanner selection, and the public redaction boundary.

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

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
pub const GENERATED_MCP_JSON: &str = include_str!("../generated/mcp.json");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanRequest {
    target: String,
    profile: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FindingSummary {
    pub severity: String,
    pub rule_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityScanResult {
    pub status: String,
    pub target: String,
    pub profile: String,
    pub summary: SeveritySummary,
    pub findings: Vec<FindingSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SeveritySummary {
    pub critical: u32,
    pub high: u32,
    pub medium: u32,
    pub low: u32,
}

pub trait SecurityScanner: Send + Sync {
    fn scan(&self, request: &ScanRequest) -> Result<SecurityScanResult, ScannerError>;
}

#[derive(Debug)]
pub struct ScannerError {
    _private: (),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScanArgs {
    target: String,
    #[serde(default)]
    profile: Option<String>,
}

/// Deterministic checked-in adapter. It never interprets the target as a path,
/// URL, or command and requires neither network access nor configuration.
pub struct FixtureScanner;
impl SecurityScanner for FixtureScanner {
    fn scan(&self, request: &ScanRequest) -> Result<SecurityScanResult, ScannerError> {
        if request.target != "fixture:demo-repo" || request.profile != "baseline" {
            return Err(ScannerError { _private: () });
        }
        Ok(SecurityScanResult {
            status: "completed".into(),
            target: request.target.clone(),
            profile: request.profile.clone(),
            summary: SeveritySummary {
                critical: 0,
                high: 0,
                medium: 0,
                low: 0,
            },
            findings: Vec::new(),
        })
    }
}

#[derive(Clone)]
pub struct AppState {
    scanner: Arc<dyn SecurityScanner>,
    invocations: Arc<AtomicUsize>,
}
impl AppState {
    #[must_use]
    pub fn fixture() -> Self {
        Self {
            scanner: Arc::new(FixtureScanner),
            invocations: Arc::new(AtomicUsize::new(0)),
        }
    }
    #[cfg(test)]
    fn invocation_count(&self) -> usize {
        self.invocations.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicErrorCode {
    InvalidRequest,
    ScannerUnavailable,
    ScanFailed,
}
impl PublicErrorCode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::ScannerUnavailable => "scanner_unavailable",
            Self::ScanFailed => "scan_failed",
        }
    }
}

#[derive(Debug)]
pub struct OperationError {
    status: StatusCode,
    code: PublicErrorCode,
}
impl OperationError {
    const fn invalid() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: PublicErrorCode::InvalidRequest,
        }
    }
    const fn unavailable() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: PublicErrorCode::ScannerUnavailable,
        }
    }
    const fn failed() -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            code: PublicErrorCode::ScanFailed,
        }
    }
    fn public_json(&self) -> Value {
        serde_json::json!({"code": self.code.as_str(), "message": "Security scan could not be completed."})
    }
}
impl std::fmt::Display for OperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.code.as_str())
    }
}
impl std::error::Error for OperationError {}

/// The sole generated-surface dispatch. Unknown operations are rejected
/// explicitly; adapters receive only this validated typed request.
#[allow(clippy::unused_async)]
pub async fn execute_operation(
    state: &AppState,
    operation: &str,
    input: generated::GeneratedOperationInput,
) -> Result<Value, OperationError> {
    if operation != "run_security_scan" {
        return Err(OperationError::invalid());
    }
    let args: ScanArgs =
        serde_json::from_value(input.body).map_err(|_| OperationError::invalid())?;
    let request = validate_request(args.target, args.profile)?;
    state.invocations.fetch_add(1, Ordering::SeqCst);
    let result = state
        .scanner
        .scan(&request)
        .map_err(|_| OperationError::failed())?;
    serde_json::to_value(result).map_err(|_| OperationError::unavailable())
}

fn validate_request(
    target: String,
    profile: Option<String>,
) -> Result<ScanRequest, OperationError> {
    if target != "fixture:demo-repo" || target.len() > 1024 || target.chars().any(char::is_control)
    {
        return Err(OperationError::invalid());
    }
    let profile = profile.unwrap_or_else(|| "baseline".into());
    if profile != "baseline" {
        return Err(OperationError::invalid());
    }
    Ok(ScanRequest { target, profile })
}

pub async fn execute_operation_http(
    state: &AppState,
    operation: &str,
    input: generated::GeneratedOperationInput,
) -> axum::response::Response {
    match execute_operation(state, operation, input).await {
        Ok(value) => axum::Json(value).into_response(),
        Err(error) => (error.status, axum::Json(error.public_json())).into_response(),
    }
}

pub fn http_router(state: AppState) -> Router {
    generated::generated_router().with_state(state)
}
pub async fn run_http() -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8942").await?;
    axum::serve(listener, http_router(AppState::fixture())).await?;
    Ok(())
}

pub async fn run_cli() -> anyhow::Result<()> {
    #[derive(Parser)]
    #[command(name = "security-scan")]
    struct Cli {
        #[command(subcommand)]
        command: generated_cli::GeneratedCommand,
    }
    let command = Cli::parse().command;
    let params = command.parameters_json();
    let input = generated::GeneratedOperationInput {
        path: BTreeMap::default(),
        query: BTreeMap::default(),
        body: params,
    };
    match execute_operation(&AppState::fixture(), command.operation_name(), input).await {
        Ok(value) => {
            println!("{value}");
            Ok(())
        }
        Err(error) => {
            eprintln!("{}", error.public_json());
            std::process::exit(1);
        }
    }
}

pub async fn run_mcp() -> anyhow::Result<()> {
    let tools: Value = serde_json::from_str(GENERATED_MCP_JSON)?;
    let state = AppState::fixture();
    hydra_mcp_stdio::serve(
        "security-scan",
        env!("CARGO_PKG_VERSION"),
        tools,
        move |name, args| {
            let state = state.clone();
            async move {
                let input = generated::GeneratedOperationInput {
                    path: BTreeMap::default(),
                    query: BTreeMap::default(),
                    body: args,
                };
                execute_operation(&state, &name, input)
                    .await
                    .map_err(|error| error.public_json().to_string())
            }
        },
    )
    .await
    .map_err(|error| anyhow::anyhow!("mcp stdio error: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(target: &str, profile: Option<&str>) -> generated::GeneratedOperationInput {
        generated::GeneratedOperationInput {
            path: BTreeMap::default(),
            query: BTreeMap::default(),
            body: serde_json::json!({"target": target, "profile": profile}),
        }
    }
    #[tokio::test]
    async fn fixture_dispatch_is_deterministic() {
        let state = AppState::fixture();
        let result = execute_operation(
            &state,
            "run_security_scan",
            input("fixture:demo-repo", None),
        )
        .await
        .unwrap();
        assert_eq!(result["status"], "completed");
        assert_eq!(result["profile"], "baseline");
        assert_eq!(state.invocation_count(), 1);
    }
    #[tokio::test]
    async fn invalid_input_never_invokes_adapter() {
        let state = AppState::fixture();
        assert!(
            execute_operation(
                &state,
                "run_security_scan",
                input("https://token:sentinel@example.test", None)
            )
            .await
            .is_err()
        );
        assert_eq!(state.invocation_count(), 0);
    }
    #[tokio::test]
    async fn undeclared_fields_never_invoke_adapter() {
        let state = AppState::fixture();
        let unexpected = generated::GeneratedOperationInput {
            path: BTreeMap::default(),
            query: BTreeMap::default(),
            body: serde_json::json!({"target": "fixture:demo-repo", "DEEPSEC_TOKEN": "sentinel-secret"}),
        };
        assert!(
            execute_operation(&state, "run_security_scan", unexpected)
                .await
                .is_err()
        );
        assert_eq!(state.invocation_count(), 0);
    }
    #[test]
    fn fixture_rejects_bypassed_invalid_typed_request() {
        let request = ScanRequest {
            target: "https://example.test".into(),
            profile: "baseline".into(),
        };
        assert!(FixtureScanner.scan(&request).is_err());
    }
    #[tokio::test]
    async fn public_errors_do_not_leak_secret() {
        let state = AppState::fixture();
        let error = execute_operation(
            &state,
            "unknown_operation",
            input("DEEPSEC_TOKEN=sentinel-secret", None),
        )
        .await
        .unwrap_err();
        let public = error.public_json().to_string();
        assert!(!public.contains("sentinel-secret"));
        assert!(!public.contains("DEEPSEC_TOKEN"));
    }
    #[test]
    fn generated_artifacts_project_all_three_surfaces() {
        let cli = include_str!("../generated/cli.rs");
        let mcp: Value = serde_json::from_str(include_str!("../generated/mcp.json")).unwrap();
        assert!(cli.contains("RunSecurityScan"));
        assert!(include_str!("../generated/http.rs").contains("/security/scans"));
        assert_eq!(mcp["tools"][0]["name"], "run_security_scan");
    }
}
