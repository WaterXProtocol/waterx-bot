use crate::commands::tg;
use crate::commands::util::*;
use crate::i18n;
use telexide::api::types::DeleteMessage;
use telexide::prelude::*;

#[command(description = "post a buy offer; the seller presses the inline button")]
pub async fn buy(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let Some(buyer) = message.from.clone() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let lang = lang_for(&ctx, &buyer);
    let parts = args(&message);
    if parts.len() < 2 {
        reply(&ctx, &message, i18n::usage_buy(lang)).await?;
        return Ok(());
    }
    // Restrict /buy to the canonical fruit set — matches the original Python
    // (`if fruit in self.sorry_reply`). Garbage characters are silently dropped.
    let fruits: String = parts[0]
        .chars()
        .filter(|c| SORRY_FRUITS.contains(c))
        .collect();
    if fruits.is_empty() {
        reply(&ctx, &message, i18n::messing_around(lang)).await?;
        return Ok(());
    }
    let Ok(price) = parts[1].parse::<i64>() else {
        reply(&ctx, &message, i18n::usage_buy(lang)).await?;
        return Ok(());
    };
    let Some(price_units) = to_micro(price) else {
        reply(&ctx, &message, ERR_NEG_REPLY).await?;
        return Ok(());
    };

    // Placeholder first → claim msg id → escrow coin against it.
    let rows = vec![vec![(
        i18n::buy_button(lang, &format_number(price)),
        format!("buy:{}:{fruits}:{price}", buyer.id),
    )]];
    let sent =
        tg::send_with_buttons(&ctx, message.chat.get_id(), i18n::loading(lang), &rows).await?;

    let database = db(&ctx);
    if !database.open_buy_offer(
        sent.chat.get_id(),
        sent.message_id,
        buyer.id,
        &fruits,
        price_units,
    )? {
        let _ = ctx
            .api
            .delete_message(DeleteMessage::new(
                sent.chat.get_id().into(),
                sent.message_id,
            ))
            .await;
        reply(&ctx, &message, i18n::not_enough_money(lang)).await?;
        return Ok(());
    }

    let listing = i18n::buy_listing(lang, &full_name(&buyer), &fruits, &format_number(price));
    tg::edit_with_buttons(&ctx, sent.chat.get_id(), sent.message_id, &listing, &rows).await?;
    Ok(())
}
