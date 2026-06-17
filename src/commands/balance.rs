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
    let fruits = if info.fruit.is_empty() {
        "—".to_string()
    } else {
        info.fruit
    };
    let body = format!(
        "{}\n{}",
        full_name(user),
        i18n::menu_status(lang, &fmt_coins(info.balance), &fruits)
    );
    reply(&ctx, &message, body).await?;
    Ok(())
}
