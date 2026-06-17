//! Owner-only commands, gated on `BOT_OWNER` via [`is_owner`]. Non-owners are
//! silently ignored (no reply) so the commands stay invisible to regular users.
//! These are intentionally **not** registered in the public `/` command menu.

use crate::commands::util::*;
use crate::database::COIN;
use crate::i18n;
use telexide::prelude::*;

/// `/mint <amount>` — credit `amount` whole water-coins to the sender of the
/// replied-to message (reply required). Positive only (no debt).
#[command(description = "owner: mint coins to the replied-to user")]
pub async fn mint(ctx: Context, message: Message) -> CommandResult {
    let Some(uid) = from_id(&message) else {
        return Ok(());
    };
    if !is_owner(&ctx, uid) {
        return Ok(());
    }
    let lang = lang_for(&ctx, message.from.as_ref().unwrap());

    let parts = args(&message);
    let Some(amount) = parts.first().and_then(|s| s.parse::<i64>().ok()).filter(|n| *n > 0) else {
        reply(&ctx, &message, i18n::mint_usage(lang)).await?;
        return Ok(());
    };
    let target = message
        .reply_to_message
        .as_ref()
        .and_then(|r| r.from.clone());
    let Some(receiver) = target else {
        reply(&ctx, &message, i18n::mint_usage(lang)).await?;
        return Ok(());
    };

    db(&ctx).force_change(receiver.id, amount * COIN)?;
    reply(
        &ctx,
        &message,
        i18n::minted(lang, &full_name(&receiver), &format_number(amount)),
    )
    .await?;
    Ok(())
}

/// `/reset` — wipe the database. Owner-only **and** dev-mode-only, so it can
/// never fire against a production bot. Also clears the in-memory bet games.
#[command(description = "owner+dev: wipe the database")]
pub async fn reset(ctx: Context, message: Message) -> CommandResult {
    let Some(uid) = from_id(&message) else {
        return Ok(());
    };
    if !is_owner(&ctx, uid) || !is_dev(&ctx) {
        return Ok(());
    }
    db(&ctx).reset_all()?;
    games(&ctx).lock().await.clear();
    reply(&ctx, &message, "🧹 Database cleared (dev)").await?;
    Ok(())
}

/// `/pause` — flip the bot into the paused state (every non-owner action is
/// blocked by `paused_block` until `/unpause`).
#[command(description = "owner: pause all actions")]
pub async fn pause(ctx: Context, message: Message) -> CommandResult {
    let Some(uid) = from_id(&message) else {
        return Ok(());
    };
    if !is_owner(&ctx, uid) {
        return Ok(());
    }
    let lang = lang_for(&ctx, message.from.as_ref().unwrap());
    db(&ctx).set_paused(true)?;
    reply(&ctx, &message, i18n::service_paused(lang)).await?;
    Ok(())
}

/// `/unpause` — resume normal operation.
#[command(description = "owner: resume all actions")]
pub async fn unpause(ctx: Context, message: Message) -> CommandResult {
    let Some(uid) = from_id(&message) else {
        return Ok(());
    };
    if !is_owner(&ctx, uid) {
        return Ok(());
    }
    let lang = lang_for(&ctx, message.from.as_ref().unwrap());
    db(&ctx).set_paused(false)?;
    reply(&ctx, &message, i18n::im_back(lang)).await?;
    Ok(())
}

/// `/broadcast <message>` — DM `message` to every known user in their private
/// chat. Failures (users who never opened a DM, blocked the bot, …) are skipped
/// and excluded from the delivered count.
#[command(description = "owner: broadcast a message to all users")]
pub async fn broadcast(ctx: Context, message: Message) -> CommandResult {
    let Some(uid) = from_id(&message) else {
        return Ok(());
    };
    if !is_owner(&ctx, uid) {
        return Ok(());
    }
    let lang = lang_for(&ctx, message.from.as_ref().unwrap());

    // Everything after the first whitespace, verbatim (preserves spacing).
    let body = text_of(&message)
        .split_once(char::is_whitespace)
        .map(|(_, rest)| rest.trim())
        .filter(|s| !s.is_empty());
    let Some(body) = body else {
        reply(&ctx, &message, i18n::broadcast_usage(lang)).await?;
        return Ok(());
    };

    // Every chat the bot knows — private DMs and groups alike.
    let ids = db(&ctx).all_chat_ids().unwrap_or_default();
    let mut delivered = 0usize;
    for id in ids {
        if send_text(&ctx, id, body).await.is_ok() {
            delivered += 1;
        }
    }
    reply(
        &ctx,
        &message,
        i18n::broadcast_sent(lang, &delivered.to_string()),
    )
    .await?;
    Ok(())
}
