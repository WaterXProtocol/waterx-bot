use crate::commands::assets;
use crate::commands::selling;
use crate::commands::tg;
use crate::commands::util::*;
use crate::core::i18n;
use telexide::prelude::*;

/// `/bets` — show the caller's open match bets + self-host predictions. Works in
/// **both** DM and group (for showing off positions). Shows a "no open bets"
/// notice when there's nothing to flex.
#[command(description = "show your open bets")]
pub async fn bets(ctx: Context, message: Message) -> CommandResult {
    let Some((user, lang)) = begin(&ctx, &message).await? else {
        return Ok(());
    };
    let (body, holds) = assets::bets_body(&ctx, lang, user).await;
    let chat = message.chat.get_id();
    // Offer `[💸 Sell]` when the caller holds positions — but only in a **private**
    // chat (the picker edits the message into the tapper's holdings; in a shared
    // group message that would mix users), matching the `/assets` view.
    if holds && !is_group_chat(chat) {
        let rows = vec![vec![(
            i18n::btn_sell(lang).to_string(),
            selling::SELL_PICK.to_string(),
        )]];
        tg::send_with_buttons(&ctx, chat, &body, &rows).await?;
    } else {
        reply(&ctx, &message, body).await?;
    }
    Ok(())
}
