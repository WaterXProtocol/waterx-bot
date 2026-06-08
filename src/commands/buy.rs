use crate::commands::tg;
use crate::commands::util::*;
use telexide::api::types::DeleteMessage;
use telexide::prelude::*;

#[command(description = "post a buy offer; the seller presses the inline button")]
pub async fn buy(ctx: Context, message: Message) -> CommandResult {
    let Some(buyer) = message.from.clone() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let parts = args(&message);
    if parts.len() < 2 {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    }
    // Restrict /buy to the canonical fruit set — matches the original Python
    // (`if fruit in self.sorry_reply`). Garbage characters are silently dropped.
    let fruits: String = parts[0]
        .chars()
        .filter(|c| SORRY_FRUITS.contains(c))
        .collect();
    if fruits.is_empty() {
        reply(&ctx, &message, "來亂的嗎🤨").await?;
        return Ok(());
    }
    let Ok(price) = parts[1].parse::<i64>() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    if price <= 0 {
        reply(&ctx, &message, ERR_NEG_REPLY).await?;
        return Ok(());
    }

    // Placeholder first → claim msg id → escrow coin against it.
    let rows = vec![vec![(
        format!("${} 賣出", format_number(price)),
        format!("buy:{}:{fruits}:{price}", buyer.id),
    )]];
    let sent =
        tg::send_with_buttons(&ctx, message.chat.get_id(), "(loading…)", &rows).await?;

    let database = db(&ctx);
    if !database.open_buy_offer(sent.chat.get_id(), sent.message_id, buyer.id, &fruits, price)? {
        let _ = ctx
            .api
            .delete_message(DeleteMessage::new(
                sent.chat.get_id().into(),
                sent.message_id,
            ))
            .await;
        reply(&ctx, &message, "錢不夠耶😶").await?;
        return Ok(());
    }

    let listing = format!(
        "{} 收購 {fruits}\n出價 {} 水幣",
        full_name(&buyer),
        format_number(price)
    );
    tg::edit_with_buttons(&ctx, sent.chat.get_id(), sent.message_id, &listing, &rows).await?;
    Ok(())
}
