use crate::bot::BotIdKey;
use crate::commands::tg;
use crate::commands::util::*;
use std::time::Duration;
use telexide::prelude::*;

#[command(description = "send water-coins or fruit to the replied-to user")]
pub async fn send(ctx: Context, message: Message) -> CommandResult {
    let Some(sender) = message.from.clone() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };

    let parts = args(&message);
    if parts.is_empty() {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    }

    let database = db(&ctx);
    let bot_id: i64 = *ctx.data.read().get::<BotIdKey>().expect("BotIdKey missing");
    let reply_target = message
        .reply_to_message
        .as_ref()
        .and_then(|r| r.from.clone());

    // === coin path ===
    if let Ok(amount) = parts[0].parse::<i64>() {
        if amount <= 0 {
            reply(&ctx, &message, ERR_NEG_REPLY).await?;
            return Ok(());
        }
        if !database.balance_change(sender.id, -amount)? {
            reply(&ctx, &message, "錢不夠耶😶").await?;
            return Ok(());
        }

        let direct_target = reply_target
            .as_ref()
            .filter(|u| u.id != bot_id)
            .cloned();
        if let Some(receiver) = direct_target {
            // Reply target is a real user → direct transfer.
            database.force_change(receiver.id, amount)?;
            reply(
                &ctx,
                &message,
                format!(
                    "{} 送給 {}\n{} 顆 水幣",
                    full_name(&sender),
                    full_name(&receiver),
                    format_number(amount),
                ),
            )
            .await?;
            return Ok(());
        }

        // No reply target → bare envelope drop.
        // Reply target is the bot → bot reacts, then drops envelope.
        if reply_target.is_some() {
            send_text(&ctx, message.chat.get_id(), "莎莉亞不需要錢喔 😎").await?;
        }
        let rows = vec![vec![(
            "領取🧧".to_string(),
            format!("envelope:{amount}"),
        )]];
        let title = if reply_target.is_some() {
            "搶紅包囉！".to_string()
        } else {
            format!(
                "{} 發紅包 {} 水幣！",
                full_name(&sender),
                format_number(amount)
            )
        };
        let sent = tg::send_with_buttons(&ctx, message.chat.get_id(), &title, &rows).await?;
        database.insert_buffer(sent.chat.get_id(), sent.message_id)?;
        return Ok(());
    }

    // === fruit path === — needs a reply target.
    let Some(receiver) = reply_target else {
        reply(&ctx, &message, "回覆對方訊息來送水果呦😅").await?;
        return Ok(());
    };

    // Iterate the *characters* of the first arg (e.g. `/send 🍑🍓` = 2 fruits in 1 arg)
    // to match the original Python behaviour. Database::fruit_transfer is bot-aware
    // and consumes fruit silently when receiver is the bot itself.
    let raw = &parts[0];
    let mut moved = String::new();
    for fruit_ch in raw.chars() {
        if database.fruit_transfer(sender.id, receiver.id, &fruit_ch.to_string())? {
            moved.push(fruit_ch);
        }
    }

    if moved.is_empty() {
        reply(&ctx, &message, "來亂的嗎🤨").await?;
        return Ok(());
    }

    if receiver.id != bot_id {
        send_text(
            &ctx,
            message.chat.get_id(),
            format!(
                "{} 送給 {}\n{moved}",
                full_name(&sender),
                full_name(&receiver),
            ),
        )
        .await?;
        return Ok(());
    }

    // bot is the receiver → reaction sequence based on how many fruits were eaten
    let chat_id = message.chat.get_id();
    let n = moved.chars().count();
    let (line, emoji) = match n {
        1 => ("好吃", "😋"),
        2 => ("好好吃", "😋"),
        3 => ("好多好多", "🤩"),
        4 => ("好幸福", "🥰"),
        _ => ("幸福到升天", "😇"), // 5+
    };
    reply(
        &ctx,
        &message,
        format!("謝謝 {}！\n{line}", full_name(&sender)),
    )
    .await?;
    send_text(&ctx, chat_id, emoji).await?;
    if n >= 5 {
        send_text(&ctx, chat_id, "(暫停服務)").await?;
        tokio::time::sleep(Duration::from_secs(60)).await;
        send_text(&ctx, chat_id, "我回來了🙂").await?;
    }
    Ok(())
}
