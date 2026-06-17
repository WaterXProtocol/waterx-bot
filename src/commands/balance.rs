use crate::commands::util::*;
use crate::i18n;
use telexide::prelude::*;

#[command(description = "show the caller's island-coin balance")]
pub async fn balance(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let Some(user) = message.from.as_ref() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let lang = lang_for(&ctx, user);
    let info = db(&ctx).get_user_info(user.id)?;
    let body = i18n::have_coins(lang, &full_name(user), &fmt_coins(info.balance));
    reply(&ctx, &message, body).await?;
    Ok(())
}
