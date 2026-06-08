use crate::commands::util::*;
use std::time::Duration;
use telexide::api::types::SendDice;
use telexide::model::MessageContent;
use telexide::prelude::*;

#[command(description = "/dice <guess 1-6> <wager> — 6x payout on a match")]
pub async fn dice(ctx: Context, message: Message) -> CommandResult {
    let parts = args(&message);

    // No args → just roll a dice for fun, no betting.
    if parts.is_empty() {
        let mut sd = SendDice::new(message.chat.get_id().into());
        sd.reply_to_message_id = Some(message.message_id);
        ctx.api.send_dice(sd).await?;
        return Ok(());
    }

    if parts.len() < 2 {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    }
    let Ok(guess) = parts[0].parse::<u8>() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    if guess == 0 || guess > 6 {
        reply(&ctx, &message, "就這麼想輸嗎？🤔").await?;
        return Ok(());
    }
    let Ok(wager) = parts[1].parse::<i64>() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    if wager <= 0 {
        reply(&ctx, &message, ERR_NEG_REPLY).await?;
        return Ok(());
    }

    let Some(user) = message.from.clone() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };

    let database = db(&ctx);
    if !database.balance_change(user.id, -wager)? {
        reply(&ctx, &message, "錢不夠耶😶").await?;
        return Ok(());
    }

    // Send the dice; Telegram returns the rolled value in the response.
    let mut sd = SendDice::new(message.chat.get_id().into());
    sd.reply_to_message_id = Some(message.message_id);
    let dice_msg = ctx.api.send_dice(sd).await?;
    let MessageContent::Dice { content: dice } = &dice_msg.content else {
        // Shouldn't happen, but refund and bail out gracefully.
        database.force_change(user.id, wager)?;
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let value = dice.value;

    // Let the animation play before announcing.
    tokio::time::sleep(Duration::from_secs(4)).await;

    if guess == value {
        // 6x payout — wager was already deducted, so net gain is 5x.
        database.force_change(user.id, wager * 6)?;
        reply(
            &ctx,
            &message,
            format!(
                "猜中了😮 {} 贏得 {} 顆 水幣",
                full_name(&user),
                format_number(wager * 5)
            ),
        )
        .await?;
    } else {
        reply(&ctx, &message, "沒猜中呦😐").await?;
    }
    Ok(())
}
