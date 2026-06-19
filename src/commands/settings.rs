use crate::commands::util::*;
use crate::commands::{menu, tg};
use crate::i18n;
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
    let fmt = db(&ctx).get_odds_fmt(user.id).unwrap_or_default();
    tg::send_with_buttons(
        &ctx,
        message.chat.get_id(),
        i18n::settings_title(lang),
        &menu::settings_rows(lang, fmt),
    )
    .await?;
    Ok(())
}
