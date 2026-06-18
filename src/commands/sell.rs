use crate::commands::tg;
use crate::commands::util::*;
use crate::i18n;
use telexide::api::types::DeleteMessage;
use telexide::prelude::*;

#[command(description = "list fruit for sale; the buyer presses the inline button")]
pub async fn sell(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let Some(seller) = message.from.clone() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let lang = lang_for(&ctx, &seller);
    let parts = args(&message);
    if parts.len() < 2 {
        reply(&ctx, &message, i18n::usage_sell(lang)).await?;
        return Ok(());
    }
    let fruits = parts[0].clone();
    let Ok(price) = parts[1].parse::<i64>() else {
        reply(&ctx, &message, i18n::usage_sell(lang)).await?;
        return Ok(());
    };
    if price <= 0 {
        reply(&ctx, &message, ERR_NEG_REPLY).await?;
        return Ok(());
    }

    // Send placeholder with a (cleanly-serialised) inline button first to claim
    // a message id; then escrow fruit against that id atomically.
    let rows = vec![vec![(
        i18n::sell_button(lang, &format_number(price)),
        format!("sell:{}:{fruits}:{price}", seller.id),
    )]];
    let sent =
        tg::send_with_buttons(&ctx, message.chat.get_id(), i18n::loading(lang), &rows).await?;

    let database = db(&ctx);
    let escrowed = database.open_sell_offer(
        sent.chat.get_id(),
        sent.message_id,
        seller.id,
        &fruits,
        price * crate::database::COIN,
    )?;
    if escrowed.is_empty() {
        // Nothing to escrow — clean up the placeholder.
        let _ = ctx
            .api
            .delete_message(DeleteMessage::new(
                sent.chat.get_id().into(),
                sent.message_id,
            ))
            .await;
        reply(&ctx, &message, i18n::messing_around(lang)).await?;
        return Ok(());
    }

    // Replace the placeholder with the real listing — refresh the callback
    // payload too in case `escrowed` is a subset of the requested fruits.
    let final_rows = vec![vec![(
        i18n::sell_button(lang, &format_number(price)),
        format!("sell:{}:{escrowed}:{price}", seller.id),
    )]];
    let listing = i18n::sell_listing(lang, &full_name(&seller), &escrowed, &format_number(price));
    tg::edit_with_buttons(
        &ctx,
        sent.chat.get_id(),
        sent.message_id,
        &listing,
        &final_rows,
    )
    .await?;
    Ok(())
}
