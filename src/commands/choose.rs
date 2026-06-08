use crate::commands::util::*;
use crate::utils::old_determine;
use telexide::prelude::*;

#[command(description = "choose one from whitespace-separated options")]
pub async fn choose(ctx: Context, message: Message) -> CommandResult {
    let opts = args(&message);
    match old_determine(&opts) {
        Some(pick) => reply(&ctx, &message, pick).await?,
        None => reply(&ctx, &message, ERR_REPLY).await?,
    }
    Ok(())
}
