//! Notes example MCP stdio server binary.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    notes_example::run_mcp().await
}
