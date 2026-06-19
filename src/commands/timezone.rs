use crate::commands::util::*;
use crate::commands::{menu, tg};
use crate::i18n;
use telexide::prelude::*;

#[command(description = "set your timezone")]
pub async fn timezone(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let Some(user) = message.from.as_ref() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let lang = lang_for(&ctx, user);
    let current = db(&ctx).get_tz(user.id).ok().flatten();
    tg::send_with_buttons(
        &ctx,
        message.chat.get_id(),
        i18n::choose_timezone(lang),
        &menu::tz_picker_rows(current),
    )
    .await?;
    Ok(())
}
