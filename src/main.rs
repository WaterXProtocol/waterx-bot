#![forbid(unsafe_code)]

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Best-effort dotenv load — env vars set externally take precedence.
    // `ENV_FILE` selects which file to load (e.g. `ENV_FILE=.env.dev cargo run`
    // for the dev bot); unset falls back to the default `.env` lookup.
    match std::env::var("ENV_FILE") {
        Ok(path) => {
            let _ = dotenvy::from_filename(path);
        }
        Err(_) => {
            let _ = dotenvy::dotenv();
        }
    }
    waterx_bot::bot::run().await
}
