//! Owner-only commands, gated on `BOT_OWNER` via [`is_owner`]. Non-owners are
//! silently ignored (no reply) so the commands stay invisible to regular users.
//! These are intentionally **not** registered in the public `/` command menu.

use crate::commands::tg;
use crate::commands::tg::answer;
use crate::commands::util::*;
use crate::database::OpenMarket;
use crate::i18n::{self, Lang};
use telexide::model::CallbackQuery;
use telexide::prelude::*;

/// Callback-data prefix for the button-driven settle flow (owner-only).
pub const SETTLE_CB: &str = "stl:";

/// Callback-data prefix for the button-driven selective `/reset` flow (owner+dev).
pub const RESET_CB: &str = "rst:";
/// Selectable reset parts, OR'd into a bitmask that rides in the callback data.
const RESET_MATCHES: u8 = 1;
const RESET_PREDICTIONS: u8 = 2;
const RESET_BALANCE: u8 = 4;
const RESET_PROMPT: &str = "🧹 Reset (dev) — tap to select, then Submit:";

/// Marker file written by `/redeploy` (holds the chat id to notify) and read by
/// `bot::run` on the next startup to confirm the bot is back online. Relative to
/// the working directory (same place as the SQLite file).
pub const REDEPLOY_MARKER: &str = "redeploy.notify";

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
        let open = match database.list_open_markets() {
            Ok(o) => o,
            Err(e) => {
                eprintln!("settle list_open_markets error: {e}");
                reply(&ctx, &message, "⚠️ DB error — couldn't list open markets.").await?;
                return Ok(());
            }
        };
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
    let open = match database.list_open_markets() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("settle list_open_markets error: {e}");
            reply(&ctx, &message, "⚠️ DB error — couldn't load markets.").await?;
            return Ok(());
        }
    };
    let Some(target) = open.iter().find(|m| m.market_id == parts[0] || m.slug == parts[0]) else {
        reply(&ctx, &message, "No open market with that id/slug.").await?;
        return Ok(());
    };
    let market_id = target.market_id.clone();
    let (_ok, summary) = run_settle(&ctx, &market_id, winner).await;
    reply(&ctx, &message, summary).await?;
    Ok(())
}

/// Settle one market against `winner` (a stored outcome key: `teamA`/`teamB`/
/// `draw`): pay winners, mark wagers, DM every bettor, and return `(ok, summary)`
/// — `ok` is false when the settlement itself failed, so the button flow can
/// avoid a false "Settled ✅" toast. Shared by the `/settle <id> <a|b|draw>`
/// text path and the button-driven confirm flow.
async fn run_settle(ctx: &Context, market_id: &str, winner: &str) -> (bool, String) {
    let database = db(ctx);
    let settlements = match database.settle_market(market_id, winner) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("settle_market error ({market_id}): {e}");
            return (false, format!("⚠️ Settle error: {e}"));
        }
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
        if let Err(e) = send_text(ctx, st.user, dm).await {
            eprintln!("settle result DM failed (user {}): {e:?}", st.user);
        }
    }
    let summary = format!(
        "Settled {} wager(s): {won} won (paid {}), {lost} lost.",
        settlements.len(),
        fmt_coins(paid)
    );
    (true, summary)
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
        return answer(ctx, cb, "", false).await;
    }
    let Some(message) = cb.message.clone() else {
        return answer(ctx, cb, "", false).await;
    };
    let chat = message.chat.get_id();
    let mid = message.message_id;
    let database = db(ctx);
    let open = match database.list_open_markets() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("handle_settle_cb list_open_markets error: {e}");
            tg::edit_text_only(ctx, chat, mid, "⚠️ DB error — couldn't load markets.").await.ok();
            return answer(ctx, cb, "", false).await;
        }
    };

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
                return answer(ctx, cb, "", false).await;
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
                return answer(ctx, cb, "", false).await;
            };
            let Some(m) = open.iter().find(|m| m.market_id == market_id) else {
                tg::edit_text_only(ctx, chat, mid, "That market is no longer open.").await.ok();
                return answer(ctx, cb, "", false).await;
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
                return answer(ctx, cb, "", false).await;
            };
            let Some(winner) = outcome_key(o) else {
                return answer(ctx, cb, "", false).await;
            };
            if !open.iter().any(|m| m.market_id == market_id) {
                tg::edit_text_only(ctx, chat, mid, "That market is no longer open.").await.ok();
                return answer(ctx, cb, "", false).await;
            }
            let (ok, summary) = run_settle(ctx, market_id, winner).await;
            tg::edit_text_only(ctx, chat, mid, &summary).await.ok();
            return answer(ctx, cb, if ok { "Settled ✅" } else { "⚠️ Settle failed" }, true).await;
        }
        _ => {}
    }
    answer(ctx, cb, "", false).await
}

/// `/redeploy` — owner-only. Fire-and-forget triggers a **separate** systemd
/// oneshot (`waterx-deploy.service`) that pulls, builds, and restarts the bot.
/// It runs in its own unit/cgroup so the restart can't kill the deploy mid-build.
/// The trigger command is overridable via the `REDEPLOY_CMD` env var (default
/// `sudo systemctl start --no-block waterx-deploy.service`). See `DEPLOY.md`.
#[command(description = "owner: pull + rebuild + restart")]
pub async fn redeploy(ctx: Context, message: Message) -> CommandResult {
    let Some(uid) = from_id(&message) else {
        return Ok(());
    };
    if !is_owner(&ctx, uid) {
        return Ok(());
    }
    let cmd = std::env::var("REDEPLOY_CMD")
        .unwrap_or_else(|_| "sudo systemctl start --no-block waterx-deploy.service".to_string());
    // Spawn detached: the trigger returns immediately (--no-block); the actual
    // build/restart happens in waterx-deploy.service, not this process.
    match std::process::Command::new("sh").arg("-c").arg(&cmd).spawn() {
        Ok(_) => {
            // Drop a marker so the *freshly restarted* bot can report "back
            // online" here on startup (this process won't survive the restart).
            let _ = std::fs::write(REDEPLOY_MARKER, message.chat.get_id().to_string());
            reply(
                &ctx,
                &message,
                "🚀 Deploying — pull + build + restart triggered. I'll message here when it's back.",
            )
            .await?;
        }
        Err(e) => {
            eprintln!("redeploy spawn error: {e}");
            reply(
                &ctx,
                &message,
                "⚠️ Couldn't start the deploy — check waterx-deploy.service / sudoers (see DEPLOY.md).",
            )
            .await?;
        }
    }
    Ok(())
}

/// `/dashboard` — owner-only snapshot of bot-wide metrics: user/chat counts, the
/// circulating coin supply, real-money match-bet exposure, and open self-host
/// games. Plain English (an operator diagnostic, not a user-facing surface).
#[command(description = "owner: bot-wide dashboard")]
pub async fn dashboard(ctx: Context, message: Message) -> CommandResult {
    let Some(uid) = from_id(&message) else {
        return Ok(());
    };
    if !is_owner(&ctx, uid) {
        return Ok(());
    }
    let database = db(&ctx);
    // Everything — including the open self-host `/predict` game count and the
    // coins committed to them — comes from one DB aggregate now (the `games`
    // table holds only live games, so no in-memory fold is needed).
    let s = match database.dashboard() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("/dashboard query error: {e}");
            reply(&ctx, &message, format!("⚠️ Dashboard error: {e}")).await?;
            return Ok(());
        }
    };
    let status = if database.is_paused().unwrap_or(false) {
        "⏸ PAUSED"
    } else {
        "▶️ running"
    };

    let text = format!(
        "📊 Bot stats\n\
         \n\
         👥 Users: {users} (referred: {referred})\n\
         ✅ Checked in today: {checkin}\n\
         💬 Chats: {groups} group(s) · {private} private\n\
         \n\
         🪙 Coin supply: {supply}\n\
         \n\
         🎟️ Match bets\n\
         · open: {open_w} · {open_stake} staked\n\
         · all-time: {tot_w} · {tot_vol} volume\n\
         🎲 Self-host games\n\
         · open: {open_g} · {open_g_coins} staked\n\
         \n\
         ⚙️ Status: {status}",
        users = format_number(s.users),
        referred = format_number(s.referred_users),
        checkin = format_number(s.checked_in_today),
        groups = format_number(s.groups),
        private = format_number(s.private_chats),
        supply = fmt_coins(s.total_supply),
        open_w = format_number(s.open_wagers),
        open_stake = fmt_coins(s.open_stake),
        tot_w = format_number(s.total_wagers),
        tot_vol = fmt_coins(s.total_volume),
        open_g = format_number(s.open_games),
        open_g_coins = fmt_coins(s.open_game_stake),
    );
    reply(&ctx, &message, text).await?;
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
    let sender = message.from.clone().expect("from_id ensured a sender");

    let parts = args(&message);
    let usage = "/mint <amount> — reply to someone, or omit the reply to mint to yourself 🪄";
    let Some(amount) = parts.first().and_then(|s| s.parse::<i64>().ok()).filter(|n| *n > 0) else {
        reply(&ctx, &message, usage).await?;
        return Ok(());
    };
    let Some(units) = to_micro(amount) else {
        reply(&ctx, &message, usage).await?;
        return Ok(());
    };
    // Target: the replied-to user, or the owner themselves when there's no reply.
    let receiver = message
        .reply_to_message
        .as_ref()
        .and_then(|r| r.from.clone())
        .unwrap_or_else(|| sender.clone());

    db(&ctx).force_change(receiver.id, units)?;
    reply(
        &ctx,
        &message,
        format!("🪄 Minted {} to {}", format_number(amount), full_name(&receiver)),
    )
    .await?;
    Ok(())
}

/// `/reset` — selective, button-driven wipe. Owner-only **and** dev-mode-only, so
/// it can never fire against a production bot. Posts a multi-select picker
/// (Matches / Predictions / Balances + Submit); the actual work happens in
/// [`handle_reset_cb`] when Submit is pressed.
#[command(description = "owner+dev: selective reset")]
pub async fn reset(ctx: Context, message: Message) -> CommandResult {
    let Some(uid) = from_id(&message) else {
        return Ok(());
    };
    if !is_owner(&ctx, uid) || !is_dev(&ctx) {
        return Ok(());
    }
    let _ = tg::send_with_buttons(&ctx, message.chat.get_id(), RESET_PROMPT, &reset_picker_rows(0)).await;
    Ok(())
}

/// `✅`/`⬜`-toggle picker for the selective reset. Each part button carries the
/// **resulting** bitmask (`flags ^ bit`) so a tap re-renders with that part
/// flipped — stateless, the selection rides entirely in the callback data.
fn reset_picker_rows(flags: u8) -> Vec<tg::Row> {
    let part = |bit: u8, label: &str| -> tg::Row {
        let mark = if flags & bit != 0 { "✅" } else { "⬜" };
        vec![(format!("{mark} {label}"), format!("{RESET_CB}t:{}", flags ^ bit))]
    };
    vec![
        part(RESET_MATCHES, "Matches (refund open bets)"),
        part(RESET_PREDICTIONS, "Predictions (refund open bets)"),
        part(RESET_BALANCE, "Balances"),
        vec![("🧹 Submit".to_string(), format!("{RESET_CB}go:{flags}"))],
    ]
}

/// Owner+dev button-driven selective reset (callback prefix [`RESET_CB`]).
/// `t:<flags>` re-renders the picker with a part toggled; `go:<flags>` executes
/// the picked parts. Refunds run **before** the balance wipe, so a combined
/// "balances + matches/predictions" reset cleanly ends at zero.
pub async fn handle_reset_cb(
    ctx: &Context,
    cb: &CallbackQuery,
    rest: &str,
) -> Result<(), telexide::Error> {
    // Destructive + dev-only — silently ack anyone who isn't the owner on a dev bot.
    if !is_owner(ctx, cb.from.id) || !is_dev(ctx) {
        return answer(ctx, cb, "", false).await;
    }
    let Some(message) = cb.message.clone() else {
        return answer(ctx, cb, "", false).await;
    };
    let chat = message.chat.get_id();
    let mid = message.message_id;
    let (action, arg) = rest.split_once(':').unwrap_or((rest, ""));
    match action {
        "t" => {
            let flags: u8 = arg.parse().unwrap_or(0);
            tg::edit_with_buttons(ctx, chat, mid, RESET_PROMPT, &reset_picker_rows(flags)).await.ok();
        }
        "go" => {
            let flags: u8 = arg.parse().unwrap_or(0);
            if flags == 0 {
                return answer(ctx, cb, "Nothing selected", true).await;
            }
            let database = db(ctx);
            let mut lines: Vec<String> = Vec::new();
            // Refunds first (they credit balances)…
            if flags & RESET_MATCHES != 0 {
                match database.reset_wagers() {
                    Ok((n, refunded)) => {
                        lines.push(format!("🎟️ Matches cleared — refunded {} to {n} open bet(s)", fmt_coins(refunded)))
                    }
                    Err(e) => {
                        eprintln!("reset_wagers error: {e}");
                        lines.push("🎟️ Matches — ⚠️ error".to_string());
                    }
                }
            }
            if flags & RESET_PREDICTIONS != 0 {
                match database.reset_predictions() {
                    Ok((n, refunded)) => {
                        games(ctx).lock().await.clear();
                        lines.push(format!("🎲 Predictions cleared — {n} game(s), refunded {}", fmt_coins(refunded)))
                    }
                    Err(e) => {
                        eprintln!("reset_predictions error: {e}");
                        lines.push("🎲 Predictions — ⚠️ error".to_string());
                    }
                }
            }
            // …then the balance wipe, so it's the final state when both are picked.
            if flags & RESET_BALANCE != 0 {
                match database.reset_balances() {
                    Ok(n) => lines.push(format!("🪙 Balances zeroed — {n} account(s)")),
                    Err(e) => {
                        eprintln!("reset_balances error: {e}");
                        lines.push("🪙 Balances — ⚠️ error".to_string());
                    }
                }
            }
            let summary = format!("🧹 Reset done (dev)\n\n{}", lines.join("\n"));
            tg::edit_text_only(ctx, chat, mid, &summary).await.ok();
            return answer(ctx, cb, "Done ✅", true).await;
        }
        _ => {}
    }
    answer(ctx, cb, "", false).await
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

    // Everything after the first whitespace, verbatim (preserves spacing).
    let body = text_of(&message)
        .split_once(char::is_whitespace)
        .map(|(_, rest)| rest.trim())
        .filter(|s| !s.is_empty());
    let Some(body) = body else {
        reply(&ctx, &message, "/broadcast <message>").await?;
        return Ok(());
    };

    // Every chat the bot knows — private DMs and groups alike.
    let ids = match db(&ctx).all_chat_ids() {
        Ok(ids) => ids,
        Err(e) => {
            eprintln!("broadcast all_chat_ids error: {e}");
            reply(&ctx, &message, "⚠️ DB error — couldn't load chat list; nothing sent.").await?;
            return Ok(());
        }
    };
    let mut delivered = 0usize;
    for id in ids {
        if send_text(&ctx, id, body).await.is_ok() {
            delivered += 1;
        }
    }
    reply(&ctx, &message, format!("📣 Broadcast sent to {delivered} chats")).await?;
    Ok(())
}
