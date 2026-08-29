//! Security-scan example MCP stdio binary.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    security_scan_example::run_mcp().await
}
