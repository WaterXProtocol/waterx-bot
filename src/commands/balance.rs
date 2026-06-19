use crate::commands::assets;
use crate::commands::util::*;
use telexide::prelude::*;

/// `/balance` — show the caller's coin balance. Works in **both** DM and group:
/// the caller opts into showing it by typing the command (e.g. to flex), so —
/// unlike `/assets` — it does **not** hide the balance in a group.
#[command(description = "show your coin balance")]
pub async fn balance(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let Some(user) = message.from.as_ref() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let lang = lang_for(&ctx, user);
    let body = assets::balance_block(&ctx, lang, user).await;
    reply(&ctx, &message, body).await?;
    Ok(())
}
