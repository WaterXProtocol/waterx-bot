use crate::commands::util::*;
use telexide::prelude::*;

#[command(description = "say hi and make sure the user exists in the DB")]
pub async fn start(ctx: Context, message: Message) -> CommandResult {
    if let Some(uid) = from_id(&message) {
        db(&ctx).balance_change(uid, 0)?;
    }
    reply(&ctx, &message, "嗨？").await?;
    Ok(())
}
