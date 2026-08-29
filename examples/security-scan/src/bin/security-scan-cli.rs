//! Security-scan example CLI binary.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    security_scan_example::run_cli().await
}
