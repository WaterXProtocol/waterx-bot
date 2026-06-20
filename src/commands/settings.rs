use crate::commands::util::*;
use crate::commands::{menu, tg};
use crate::core::i18n;
use telexide::prelude::*;

/// `/settings` — open the settings hub: pick an odds display format, or open the
/// language / timezone pickers. Replaces the old `/language` command (the
/// language picker is now reachable via the hub's `[🌐 Language]` button).
#[command(description = "language, timezone & odds format")]
pub async fn settings(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let Some(user) = message.from.as_ref() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let lang = lang_for(&ctx, user);
    let chat_id = message.chat.get_id();

    // In a group the hub is a per-user, edit-in-place surface, so open it in the
    // user's DM (like `/predict`/`/feedback`) instead of posting it to the group.
    if is_group_chat(chat_id) {
        match tg::send_with_buttons(
            &ctx,
            user.id,
            i18n::settings_title(lang),
            &menu::settings_rows(lang),
        )
        .await
        {
            Ok(_) => reply(&ctx, &message, i18n::settings_check_dm(lang)).await?,
            Err(_) => reply(&ctx, &message, i18n::settings_dm_first(lang)).await?,
        };
        return Ok(());
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
