use crate::commands::util::*;
use crate::commands::{menu, tg};
use crate::core::i18n;
use telexide::prelude::*;

/// `/settings` — open the settings hub: pick an odds display format, or open the
/// language / timezone pickers. Replaces the old `/language` command (the
/// language picker is now reachable via the hub's `[🌐 Language]` button).
#[command(description = "language, timezone & odds format")]
pub async fn settings(ctx: Context, message: Message) -> CommandResult {
    let Some((user, lang)) = begin(&ctx, &message).await? else {
        return Ok(());
    };
    let chat_id = message.chat.get_id();

    // In a group the hub is a per-user, edit-in-place surface, so open it in the
    // user's DM (like `/predict`/`/feedback`) instead of posting it to the group.
    if is_group_chat(chat_id) {
        let landed = tg::send_with_buttons(
            &ctx,
            user.id,
            i18n::settings_title(lang),
            &menu::settings_rows(lang),
        )
        .await
        .is_ok();
        return dm_pointer(
            &ctx,
            &message,
            landed,
            i18n::settings_check_dm(lang),
            i18n::settings_dm_first(lang),
        )
        .await;
    }

    tg::send_with_buttons(
        &ctx,
        chat_id,
        i18n::settings_title(lang),
        &menu::settings_rows(lang),
    )
    .await?;
    Ok(())
}
