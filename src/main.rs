#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Best-effort `.env` load — env vars set externally take precedence.
    let _ = dotenvy::dotenv();
    waterx_bot::bot::run().await
}
