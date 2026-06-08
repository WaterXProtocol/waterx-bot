use crate::commands::util::*;
use crate::utils::wolfram_replace;
use telexide::prelude::*;

#[command(description = "build a Wolfram|Alpha query URL from the args")]
pub async fn wolfram(ctx: Context, message: Message) -> CommandResult {
    let raw = args(&message).join(" ");
    if raw.is_empty() {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    }
    let url = format!(
        "https://www.wolframalpha.com/input/?i={}",
        wolfram_replace(&raw)
    );
    reply(&ctx, &message, url).await?;
    Ok(())
}
