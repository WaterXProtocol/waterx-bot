use crate::commands::tg;
use crate::commands::util::*;
use telexide::api::types::DeleteMessage;
use telexide::prelude::*;

#[command(description = "list fruit for sale; the buyer presses the inline button")]
pub async fn sell(ctx: Context, message: Message) -> CommandResult {
    let Some(seller) = message.from.clone() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let parts = args(&message);
    if parts.len() < 2 {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    }
    let fruits = parts[0].clone();
    let Ok(price) = parts[1].parse::<i64>() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    if price <= 0 {
        reply(&ctx, &message, ERR_NEG_REPLY).await?;
        return Ok(());
    }

    // Send placeholder with a (cleanly-serialised) inline button first to claim
    // a message id; then escrow fruit against that id atomically.
    let rows = vec![vec![(
        format!("${} 買入", format_number(price)),
        format!("sell:{}:{fruits}:{price}", seller.id),
    )]];
    let sent =
        tg::send_with_buttons(&ctx, message.chat.get_id(), "(loading…)", &rows).await?;

    let database = db(&ctx);
    let escrowed =
        database.open_sell_offer(sent.chat.get_id(), sent.message_id, seller.id, &fruits, price)?;
    if escrowed.is_empty() {
        // Nothing to escrow — clean up the placeholder.
        let _ = ctx
            .api
            .delete_message(DeleteMessage::new(
                sent.chat.get_id().into(),
                sent.message_id,
            ))
            .await;
        reply(&ctx, &message, "來亂的嗎🤨").await?;
        return Ok(());
    }

    // Replace the placeholder with the real listing — refresh the callback
    // payload too in case `escrowed` is a subset of the requested fruits.
    let final_rows = vec![vec![(
        format!("${} 買入", format_number(price)),
        format!("sell:{}:{escrowed}:{price}", seller.id),
    )]];
    let listing = format!(
        "{} 出售 {escrowed}\n要價 {} 水幣",
        full_name(&seller),
        format_number(price)
    );
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
