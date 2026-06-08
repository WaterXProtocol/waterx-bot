use crate::bot::BotIdKey;
use crate::commands::util::*;
use crate::utils::cloth_check;
use telexide::prelude::*;

#[command(description = "throw a random fruit at the replied-to user")]
pub async fn throw(ctx: Context, message: Message) -> CommandResult {
    let Some(sender) = message.from.clone() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let Some(receiver) = message
        .reply_to_message
        .as_ref()
        .and_then(|r| r.from.clone())
    else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };

    let database = db(&ctx);
    let Some(fruit) = database.fruit_pop(sender.id)? else {
        reply(
            &ctx,
            &message,
            format!("{} 沒有水果😶", full_name(&sender)),
        )
        .await?;
        return Ok(());
    };

    // Throwing at the bot itself — short-circuit.
    let bot_id: i64 = *ctx.data.read().get::<BotIdKey>().expect("BotIdKey missing");
    if receiver.id == bot_id {
        reply(&ctx, &message, "莎莉亞怎麼了🥺").await?;
        return Ok(());
    }

    let recv_info = database.get_user_info(receiver.id)?;
    let new_cloth = format!("{}{fruit}", recv_info.cloth);
    let (penalty, remain) = cloth_check(&new_cloth);
    database.set_cloth(receiver.id, &remain)?;

    if sender.id == receiver.id {
        reply(
            &ctx,
            &message,
            format!("{} 自砸了一顆 {fruit} 哇嗚😯", full_name(&sender)),
        )
        .await?;
    } else {
        reply(
            &ctx,
            &message,
            format!(
                "{} 向 {} 砸了一顆 {fruit}",
                full_name(&sender),
                full_name(&receiver),
            ),
        )
        .await?;
    }

    if penalty {
        // NOTE: original Python composed a "fine" image (grayscale profile photo
        // + timestamp + "FINED 50 ISD") via PIL and pinned it. We send a plain
        // text fine notice instead — no image, no pin.
        database.force_change(receiver.id, -50)?;
        send_text(
            &ctx,
            message.chat.get_id(),
            format!(
                "{} 言行多次令人不適\n罰款 50 顆 水幣",
                full_name(&receiver)
            ),
        )
        .await?;
    }
    Ok(())
}
