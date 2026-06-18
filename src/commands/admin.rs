//! Owner-only commands, gated on `BOT_OWNER` via [`is_owner`]. Non-owners are
//! silently ignored (no reply) so the commands stay invisible to regular users.
//! These are intentionally **not** registered in the public `/` command menu.

use crate::commands::tg;
use crate::commands::util::*;
use crate::database::{OpenMarket, COIN};
use crate::i18n::{self, Lang};
use telexide::api::types::AnswerCallbackQuery;
use telexide::model::CallbackQuery;
use telexide::prelude::*;

/// Callback-data prefix for the button-driven settle flow (owner-only).
pub const SETTLE_CB: &str = "stl:";

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
        // Button-driven picker: one button per market, labelled with the
        // human-readable title (the market id rides in the callback data).
        tg::send_with_buttons(
            &ctx,
            message.chat.get_id(),
            "Settle — pick a market:",
            &market_list_rows(&open),
        )
        .await?;
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
    let summary = run_settle(&ctx, &market_id, winner).await;
    reply(&ctx, &message, summary).await?;
    Ok(())
}

/// Settle one market against `winner` (a stored outcome key: `teamA`/`teamB`/
/// `draw`): pay winners, mark wagers, DM every bettor, and return a one-line
/// admin summary. Shared by the `/settle <id> <a|b|draw>` text path and the
/// button-driven confirm flow.
async fn run_settle(ctx: &Context, market_id: &str, winner: &str) -> String {
    let database = db(ctx);
    let settlements = match database.settle_market(market_id, winner) {
        Ok(s) => s,
        Err(e) => return format!("Settle error: {e}"),
    };
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
        let _ = send_text(ctx, st.user, dm).await;
    }
    format!(
        "Settled {} wager(s): {won} won (paid {}), {lost} lost.",
        settlements.len(),
        fmt_coins(paid)
    )
}

/// Human-readable button label for a market (title, never the raw id).
fn market_label(m: &OpenMarket) -> String {
    if !m.team_a.is_empty() && !m.team_b.is_empty() {
        format!("{} vs {}", m.team_a, m.team_b)
    } else if !m.slug.is_empty() {
        m.slug.clone()
    } else {
        m.market_id.clone()
    }
}

/// One button per open market: visible label is the title, callback data is
/// `stl:p:<market_id>`.
fn market_list_rows(open: &[OpenMarket]) -> Vec<tg::Row> {
    open.iter()
        .map(|m| vec![(market_label(m), format!("{SETTLE_CB}p:{}", m.market_id))])
        .collect()
}

/// The three-outcome keyboard for a chosen market, plus a back button.
fn outcome_rows(m: &OpenMarket) -> Vec<tg::Row> {
    vec![
        vec![(m.team_a.clone(), format!("{SETTLE_CB}o:{}:a", m.market_id))],
        vec![("🤝 Draw".to_string(), format!("{SETTLE_CB}o:{}:d", m.market_id))],
        vec![(m.team_b.clone(), format!("{SETTLE_CB}o:{}:b", m.market_id))],
        vec![("⬅ Back".to_string(), format!("{SETTLE_CB}l"))],
    ]
}

/// `a|b|d` → display name of the winning side for a market.
fn winner_label(o: &str, m: &OpenMarket) -> String {
    match o {
        "a" => m.team_a.clone(),
        "b" => m.team_b.clone(),
        _ => "Draw".to_string(),
    }
}

/// `a|b|d` → stored outcome key for `settle_market`.
fn outcome_key(o: &str) -> Option<&'static str> {
    match o {
        "a" => Some("teamA"),
        "b" => Some("teamB"),
        "d" => Some("draw"),
        _ => None,
    }
}

/// Acknowledge a callback query (optional alert toast).
async fn ack(ctx: &Context, cb: &CallbackQuery, toast: &str) -> Result<(), telexide::Error> {
    let mut a = AnswerCallbackQuery::new(cb.id.clone());
    if !toast.is_empty() {
        a.text = Some(toast.to_string());
        a.show_alert = Some(true);
    }
    ctx.api.answer_callback_query(a).await?;
    Ok(())
}

/// Owner-only button-driven settle flow (callback prefix [`SETTLE_CB`]). Each
/// step edits the same message in place: pick market → pick outcome → confirm
/// 1/2 → confirm 2/2 → settle. Re-reads the open-market list at every step so a
/// market that's already been settled (or whose id doesn't match) fails safe
/// rather than paying out the wrong market.
pub async fn handle_settle_cb(
    ctx: &Context,
    cb: &CallbackQuery,
    rest: &str,
) -> Result<(), telexide::Error> {
    // Only the owner may drive settlement — others get a silent ack even if
    // they somehow see the buttons (e.g. /settle was run in a group).
    if !is_owner(ctx, cb.from.id) {
        return ack(ctx, cb, "").await;
    }
    let Some(message) = cb.message.clone() else {
        return ack(ctx, cb, "").await;
    };
    let chat = message.chat.get_id();
    let mid = message.message_id;
    let database = db(ctx);
    let open = database.list_open_markets().unwrap_or_default();

    let (action, arg) = rest.split_once(':').unwrap_or((rest, ""));
    match action {
        // Back to the market list.
        "l" => {
            if open.is_empty() {
                tg::edit_text_only(ctx, chat, mid, "No open wagers.").await.ok();
            } else {
                tg::edit_with_buttons(ctx, chat, mid, "Settle — pick a market:", &market_list_rows(&open))
                    .await
                    .ok();
            }
        }
        // Picked a market → show its outcome buttons.
        "p" => {
            let Some(m) = open.iter().find(|m| m.market_id == arg) else {
                tg::edit_text_only(ctx, chat, mid, "That market is no longer open.").await.ok();
                return ack(ctx, cb, "").await;
            };
            let text = format!(
                "{} vs {}\n{} bet(s) · {} staked\n\nWho won?",
                m.team_a,
                m.team_b,
                m.count,
                fmt_coins(m.stake)
            );
            tg::edit_with_buttons(ctx, chat, mid, &text, &outcome_rows(m)).await.ok();
        }
        // Picked an outcome → first confirmation; or first confirm → second.
        "o" | "1" => {
            let Some((market_id, o)) = arg.rsplit_once(':') else {
                return ack(ctx, cb, "").await;
            };
            let Some(m) = open.iter().find(|m| m.market_id == market_id) else {
                tg::edit_text_only(ctx, chat, mid, "That market is no longer open.").await.ok();
                return ack(ctx, cb, "").await;
            };
            let wl = winner_label(o, m);
            if action == "o" {
                let text = format!(
                    "Settle this market?\n\n{} vs {}\n→ winner: {}\n\nThis pays out real coins.\n(confirm 1 of 2)",
                    m.team_a, m.team_b, wl
                );
                let rows = vec![
                    vec![("✅ Yes, continue".to_string(), format!("{SETTLE_CB}1:{market_id}:{o}"))],
                    vec![("⬅ Back".to_string(), format!("{SETTLE_CB}p:{market_id}"))],
                ];
                tg::edit_with_buttons(ctx, chat, mid, &text, &rows).await.ok();
            } else {
                let text = format!(
                    "⚠️ FINAL CONFIRMATION\n\n{} vs {}\n→ winner: {}\n\nThis is irreversible.\n(confirm 2 of 2)",
                    m.team_a, m.team_b, wl
                );
                let rows = vec![
                    vec![(format!("⚠️ Settle: {wl} wins"), format!("{SETTLE_CB}2:{market_id}:{o}"))],
                    vec![("❌ Cancel".to_string(), format!("{SETTLE_CB}l"))],
                ];
                tg::edit_with_buttons(ctx, chat, mid, &text, &rows).await.ok();
            }
        }
        // Second confirmation → execute.
        "2" => {
            let Some((market_id, o)) = arg.rsplit_once(':') else {
                return ack(ctx, cb, "").await;
            };
            let Some(winner) = outcome_key(o) else {
                return ack(ctx, cb, "").await;
            };
            if !open.iter().any(|m| m.market_id == market_id) {
                tg::edit_text_only(ctx, chat, mid, "That market is no longer open.").await.ok();
                return ack(ctx, cb, "").await;
            }
            let summary = run_settle(ctx, market_id, winner).await;
            tg::edit_text_only(ctx, chat, mid, &summary).await.ok();
            return ack(ctx, cb, "Settled ✅").await;
        }
        _ => {}
    }
    ack(ctx, cb, "").await
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
    let sender = message.from.clone().expect("from_id ensured a sender");
    let lang = lang_for(&ctx, &sender);

    let parts = args(&message);
    let Some(amount) = parts.first().and_then(|s| s.parse::<i64>().ok()).filter(|n| *n > 0) else {
        reply(&ctx, &message, i18n::mint_usage(lang)).await?;
        return Ok(());
    };
    // Target: the replied-to user, or the owner themselves when there's no reply.
    let receiver = message
        .reply_to_message
        .as_ref()
        .and_then(|r| r.from.clone())
        .unwrap_or_else(|| sender.clone());

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
