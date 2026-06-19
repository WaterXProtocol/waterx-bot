use crate::commands::assets;
use crate::commands::util::*;
use crate::i18n;
use telexide::prelude::*;

/// `/bets` — show the caller's open match bets + self-host predictions. Works in
/// **both** DM and group (for showing off positions). Shows a "no open bets"
/// notice when there's nothing to flex.
#[command(description = "show your open bets")]
pub async fn bets(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let Some(user) = message.from.as_ref() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let lang = lang_for(&ctx, user);
    // `positions_block` prefixes each section with "\n\n", so it appends cleanly
    // after the name header; empty means no open positions.
    let pos = assets::positions_block(&ctx, lang, user).await;
    let body = if pos.is_empty() {
        format!("{}\n{}", full_name(user), i18n::no_open_bets(lang))
    } else {
        format!("{}{pos}", full_name(user))
    };
    reply(&ctx, &message, body).await?;
    Ok(())
}
