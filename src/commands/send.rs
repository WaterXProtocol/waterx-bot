use crate::bot::BotIdKey;
use crate::commands::tg;
use crate::commands::util::*;
use crate::core::i18n;
use std::time::Duration;
use telexide::prelude::*;

/// Best-effort refund of `units` micro-coins to `user`, logging (not swallowing)
/// a failure — used when an envelope can't be posted/recorded after the debit.
fn refund_or_log(db: &crate::database::Database, user: i64, units: i64) {
    if let Err(e) = db.force_change(user, units) {
        eprintln!("envelope refund failed (user {user}, +{units}): {e}");
    }
}

#[command(description = "send coins to the replied-to user")]
pub async fn send(ctx: Context, message: Message) -> CommandResult {
    let Some((sender, lang)) = begin(&ctx, &message).await? else {
        return Ok(());
    };

    let parts = args(&message);
    if parts.is_empty() {
        reply(&ctx, &message, i18n::usage_send(lang)).await?;
        return Ok(());
    }

    let database = db(&ctx);
    let bot_id: i64 = *ctx.data.read().get::<BotIdKey>().expect("BotIdKey missing");
    let reply_target = message.reply_to_message.as_ref().and_then(|r| r.from.clone());

    // === coin path ===
    if let Ok(amount) = parts[0].parse::<i64>() {
        let Some(units) = to_micro(amount) else {
            reply(&ctx, &message, ERR_NEG_REPLY).await?;
            return Ok(());
        };

        let direct_target = reply_target.as_ref().filter(|u| u.id != bot_id).cloned();
        if let Some(receiver) = direct_target {
            // Reply target is a real user → **atomic** direct transfer: debit and
            // credit happen in one DB transaction, so coins can never be taken
            // from the sender without landing on the receiver.
            if !database.transfer(sender.id, receiver.id, units)? {
                reply(&ctx, &message, i18n::not_enough_money(lang)).await?;
                return Ok(());
            }
            reply(
                &ctx,
                &message,
                i18n::sent_coins(
                    lang,
                    &full_name(sender),
                    &full_name(&receiver),
                    &format_number(amount),
                ),
            )
            .await?;
            return Ok(());
        }

        // No reply target → bare envelope drop (reply target is the bot → bot
        // reacts, then drops envelope). Debit first, then post + record the
        // buffer row; if either step fails the envelope is unclaimable, so we
        // refund the sender rather than stranding their coins.
        if !database.balance_change(sender.id, -units)? {
            reply(&ctx, &message, i18n::not_enough_money(lang)).await?;
            return Ok(());
        }
        if reply_target.is_some() {
            send_text(&ctx, message.chat.get_id(), i18n::bot_no_money(lang)).await?;
        }
        let rows = vec![vec![(
            i18n::claim_button(lang).to_string(),
            format!("envelope:{amount}"),
        )]];
        let title = if reply_target.is_some() {
            i18n::grab_envelope_title(lang).to_string()
        } else {
            i18n::sent_envelope_title(lang, &full_name(sender), &format_number(amount))
        };
        let sent = match tg::send_with_buttons(&ctx, message.chat.get_id(), &title, &rows).await {
            Ok(sent) => sent,
            Err(e) => {
                // The envelope never went out → refund the debited coins.
                refund_or_log(&database, sender.id, units);
                return Err(e);
            }
        };
        if let Err(e) = database.insert_buffer(sent.chat.get_id(), sent.message_id, sender.id, units) {
            // Button is live but the claim checks for this buffer row, so the
            // coins would be stuck — refund the sender.
            eprintln!(
                "envelope insert_buffer failed, refunding sender {}: {e}",
                sender.id
            );
            refund_or_log(&database, sender.id, units);
            return Ok(());
        }
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
            i18n::sent_fruits(lang, &full_name(sender), &full_name(&receiver), &moved),
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
        i18n::thanks(lang, &full_name(sender), i18n::eat_reaction(lang, n)),
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
