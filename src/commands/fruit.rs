use crate::commands::util::*;
use telexide::prelude::*;

#[command(description = "show the caller's fruit inventory")]
pub async fn fruit(ctx: Context, message: Message) -> CommandResult {
    let Some(user) = message.from.as_ref() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let info = db(&ctx).get_user_info(user.id)?;
    let body = if info.fruit.is_empty() {
        format!("{}\n想要水果🤤", full_name(user))
    } else {
        format!("{}\n的水果庫:\n{}", full_name(user), info.fruit)
    };
    reply(&ctx, &message, body).await?;
    Ok(())
}
