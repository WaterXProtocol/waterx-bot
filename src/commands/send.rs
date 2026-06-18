use crate::bot::BotIdKey;
use crate::commands::util::*;
use crate::database::COIN;
use crate::i18n;
use std::time::Duration;
use telexide::prelude::*;

#[command(description = "send coins to the replied-to user")]
pub async fn send(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let Some(sender) = message.from.clone() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let lang = lang_for(&ctx, &sender);

    let parts = args(&message);
    if parts.is_empty() {
        reply(&ctx, &message, i18n::usage_send(lang)).await?;
        return Ok(());
    }

    let database = db(&ctx);
    let bot_id: i64 = *ctx.data.read().get::<BotIdKey>().expect("BotIdKey missing");
    let reply_target = message
        .reply_to_message
        .as_ref()
        .and_then(|r| r.from.clone());

    // === coin path === — a direct transfer to the replied-to user only; no
    // envelope drops. Requires replying to a real user.
    if let Ok(amount) = parts[0].parse::<i64>() {
        if amount <= 0 {
            reply(&ctx, &message, ERR_NEG_REPLY).await?;
            return Ok(());
        }
        // Validate the recipient *before* debiting.
        let Some(receiver) = reply_target.as_ref().filter(|u| u.id != bot_id).cloned() else {
            reply(&ctx, &message, i18n::usage_send(lang)).await?;
            return Ok(());
        };
        if !database.balance_change(sender.id, -amount * COIN)? {
            reply(&ctx, &message, i18n::not_enough_money(lang)).await?;
            return Ok(());
        }
        database.force_change(receiver.id, amount * COIN)?;
        reply(
            &ctx,
            &message,
            i18n::sent_coins(
                lang,
                &full_name(&sender),
                &full_name(&receiver),
                &format_number(amount),
            ),
        )
        .await?;
        return Ok(());
    }

    // === fruit path === — needs a reply target.
    let Some(receiver) = reply_target else {
        reply(&ctx, &message, i18n::reply_to_send_fruit(lang)).await?;
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
        reply(&ctx, &message, i18n::messing_around(lang)).await?;
        return Ok(());
    }

    if receiver.id != bot_id {
        send_text(
            &ctx,
            message.chat.get_id(),
            i18n::sent_fruits(lang, &full_name(&sender), &full_name(&receiver), &moved),
        )
        .await?;
        return Ok(());
    }

    // bot is the receiver → reaction sequence based on how many fruits were eaten
    let chat_id = message.chat.get_id();
    let n = moved.chars().count();
    let emoji = match n {
        1 => "😋",
        2 => "😋",
        3 => "🤩",
        4 => "🥰",
        _ => "😇", // 5+
    };
    reply(
        &ctx,
        &message,
        i18n::thanks(lang, &full_name(&sender), i18n::eat_reaction(lang, n)),
    )
    .await?;
    send_text(&ctx, chat_id, emoji).await?;
    if n >= 5 {
        send_text(&ctx, chat_id, i18n::service_paused(lang)).await?;
        tokio::time::sleep(Duration::from_secs(60)).await;
        send_text(&ctx, chat_id, i18n::im_back(lang)).await?;
    }
    Ok(())
}
