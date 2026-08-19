//! Notes example HTTP server binary.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    notes_example::run_http().await
}
