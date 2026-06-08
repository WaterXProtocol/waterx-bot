use crate::commands::util::*;
use telexide::prelude::*;

#[command(description = "owner-only: 😴")]
pub async fn sleep(ctx: Context, message: Message) -> CommandResult {
    let Some(uid) = from_id(&message) else {
        return Ok(());
    };
    if !is_owner(&ctx, uid) {
        return Ok(());
    }
    reply(&ctx, &message, "😴").await?;
    // Mirror the original Python's SIGINT-kill behaviour.
    std::process::exit(0);
}
