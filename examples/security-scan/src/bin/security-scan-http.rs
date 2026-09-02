//! Security-scan example HTTP server binary.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    security_scan_example::run_http().await
}
