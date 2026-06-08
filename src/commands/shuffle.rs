use crate::commands::util::*;
use rand::seq::SliceRandom;
use telexide::prelude::*;

#[command(description = "shuffle the args and print them back")]
pub async fn shuffle(ctx: Context, message: Message) -> CommandResult {
    let mut opts = args(&message);
    if opts.is_empty() {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    }
    {
        let mut rng = rand::thread_rng();
        opts.shuffle(&mut rng);
    }
    reply(&ctx, &message, opts.join(" ")).await?;
    Ok(())
}
