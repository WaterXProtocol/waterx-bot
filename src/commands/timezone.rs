use crate::commands::util::*;
use crate::commands::{menu, tg};
use crate::core::i18n;
use telexide::prelude::*;

#[command(description = "set your timezone")]
pub async fn timezone(ctx: Context, message: Message) -> CommandResult {
    let Some((user, lang)) = begin(&ctx, &message).await? else {
        return Ok(());
    };
    let current = db(&ctx).get_tz(user.id).ok().flatten();
    tg::send_with_buttons(
        &ctx,
        message.chat.get_id(),
        i18n::choose_timezone(lang),
        &menu::tz_picker_rows(current, false),
    )
    .await?;
    Ok(())
}
