use crate::commands::tg;
use crate::commands::util::*;
use telexide::prelude::*;

#[command(description = "/envelope <amount> — drop a red envelope of N water-coins")]
pub async fn envelope(ctx: Context, message: Message) -> CommandResult {
    let Some(sender) = message.from.clone() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let parts = args(&message);
    if parts.is_empty() {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    }
    let Ok(amount) = parts[0].parse::<i64>() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    if amount <= 0 {
        reply(&ctx, &message, ERR_NEG_REPLY).await?;
        return Ok(());
    }

    let database = db(&ctx);
    if !database.balance_change(sender.id, -amount)? {
        reply(&ctx, &message, "錢不夠耶😶").await?;
        return Ok(());
    }

    let rows = vec![vec![("領取🧧".to_string(), format!("envelope:{amount}"))]];
    let sent = tg::send_with_buttons(
        &ctx,
        message.chat.get_id(),
        &format!(
            "{} 發紅包 {} 水幣！",
            full_name(&sender),
            format_number(amount)
        ),
        &rows,
    )
    .await?;
    database.insert_buffer(sent.chat.get_id(), sent.message_id)?;
    Ok(())
}
