//! Owner-only commands, gated on `BOT_OWNER` via [`is_owner`]. Non-owners are
//! silently ignored (no reply) so the commands stay invisible to regular users.
//! These are intentionally **not** registered in the public `/` command menu.

use crate::commands::util::*;
use crate::database::COIN;
use crate::i18n::{self, Lang};
use telexide::prelude::*;

/// Normalize an admin-typed winner token into the stored outcome key.
fn normalize_winner(s: &str) -> Option<&'static str> {
    match s.to_ascii_lowercase().as_str() {
        "a" | "teama" | "team_a" | "home" | "1" => Some("teamA"),
        "b" | "teamb" | "team_b" | "away" | "2" => Some("teamB"),
        "draw" | "x" | "d" | "tie" => Some("draw"),
        _ => None,
    }
}

/// `/settle` — owner-only. With no args, lists markets that still have open
/// wagers (copy a market id). `/settle <market_id|slug> <a|b|draw>` pays out
/// winners and DMs every bettor their result.
#[command(description = "owner: settle a match")]
pub async fn settle(ctx: Context, message: Message) -> CommandResult {
    let Some(uid) = from_id(&message) else {
        return Ok(());
    };
    if !is_owner(&ctx, uid) {
        return Ok(());
    }
    let database = db(&ctx);
    let parts = args(&message);

    if parts.len() < 2 {
        let open = database.list_open_markets().unwrap_or_default();
        if open.is_empty() {
            reply(&ctx, &message, "No open wagers.").await?;
            return Ok(());
        }
        let mut s = String::from("Open markets — /settle <id> <a|b|draw>:\n");
        for m in &open {
            s.push_str(&format!(
                "\n{} vs {}\n  {}\n  {} bet(s) · {} staked\n",
                m.team_a,
                m.team_b,
                m.market_id,
                m.count,
                fmt_coins(m.stake)
            ));
        }
        reply(&ctx, &message, s).await?;
        return Ok(());
    }

    let Some(winner) = normalize_winner(&parts[1]) else {
        reply(&ctx, &message, "Winner must be one of: a | b | draw").await?;
        return Ok(());
    };
    let open = database.list_open_markets().unwrap_or_default();
    let Some(target) = open.iter().find(|m| m.market_id == parts[0] || m.slug == parts[0]) else {
        reply(&ctx, &message, "No open market with that id/slug.").await?;
        return Ok(());
    };
    let market_id = target.market_id.clone();
    let settlements = database.settle_market(&market_id, winner)?;

    let (mut won, mut lost, mut paid) = (0u32, 0u32, 0i64);
    for st in &settlements {
        let blang = database.get_lang(st.user).ok().flatten().unwrap_or(Lang::En);
        let dm = if st.won {
            won += 1;
            paid += st.payout;
            i18n::bet_won(blang, &fmt_coins(st.payout))
        } else {
            lost += 1;
            i18n::bet_lost(blang).to_string()
        };
        let _ = send_text(&ctx, st.user, dm).await;
    }
    reply(
        &ctx,
        &message,
        format!(
            "Settled {} wager(s): {won} won (paid {}), {lost} lost.",
            settlements.len(),
            fmt_coins(paid)
        ),
    )
    .await?;
    Ok(())
}

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
