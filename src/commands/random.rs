use crate::commands::util::*;
use rand::seq::SliceRandom;
use telexide::prelude::*;

#[command(description = "pick a uniformly random option")]
pub async fn random(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let opts = args(&message);
    if opts.is_empty() {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    }
    let pick = {
        let mut rng = rand::thread_rng();
        opts.choose(&mut rng).cloned().unwrap_or_default()
    };
    reply(&ctx, &message, pick).await?;
    Ok(())
}
