//! Notes example CLI binary.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    notes_example::run_cli().await
}
