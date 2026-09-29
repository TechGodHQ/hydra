//! Notes example MCP Streamable HTTP server binary.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    notes_example::run_mcp_http().await
}
