use crate::commands::tg;
use crate::commands::util::*;
use crate::core::i18n;
use telexide::prelude::*;

#[command(description = "list fruit for sale; the buyer presses the inline button")]
pub async fn sell(ctx: Context, message: Message) -> CommandResult {
    let Some((seller, lang)) = begin(&ctx, &message).await? else {
        return Ok(());
    };
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
    let Some(price_units) = to_micro(price) else {
        reply(&ctx, &message, ERR_NEG_REPLY).await?;
        return Ok(());
    };

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
        price_units,
    )?;
    if escrowed.is_empty() {
        // Nothing to escrow — clean up the placeholder.
        let _ = tg::delete_message(&ctx, sent.chat.get_id(), sent.message_id).await;
        reply(&ctx, &message, i18n::messing_around(lang)).await?;
        return Ok(());
    }

    // Replace the placeholder with the real listing — refresh the callback
    // payload too in case `escrowed` is a subset of the requested fruits.
    let final_rows = vec![vec![(
        i18n::sell_button(lang, &format_number(price)),
        format!("sell:{}:{escrowed}:{price}", seller.id),
    )]];
    let listing = i18n::sell_listing(lang, &full_name(seller), &escrowed, &format_number(price));
    // Best-effort: the escrow + offer row are already committed and the
    // placeholder already carries a working (DB-backed) button, so a failed
    // cosmetic refresh must not report the whole `/sell` as failed.
    let _ = tg::edit_with_buttons(
        &ctx,
        sent.chat.get_id(),
        sent.message_id,
        &listing,
        &final_rows,
    )
    .await;
    Ok(())
}
