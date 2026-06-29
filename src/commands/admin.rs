//! Owner-only commands, gated on `BOT_OWNER` via [`is_owner`]. Non-owners are
//! silently ignored (no reply) so the commands stay invisible to regular users.
//! These are intentionally **not** registered in the public `/` command menu.

use crate::commands::tg;
use crate::commands::tg::answer;
use crate::commands::util::*;
use crate::core::types::BetState;
use crate::database::COIN;
use crate::core::i18n;
use std::time::Duration;
use telexide::model::{CallbackQuery, User};
use telexide::prelude::*;

/// Pace between `/broadcast` sends — ~15 msg/s, well under Telegram's ~30/s
/// global cap, so live users' commands keep rate budget during a broadcast.
const BROADCAST_PACE: Duration = Duration::from_millis(67);
/// Max rate-limit (429) retries per recipient in `send_resilient`.
const SEND_MAX_RETRIES: u32 = 5;
/// Extra slack added to Telegram's reported `retry after N` before retrying.
const RATE_LIMIT_SLACK_SECS: u64 = 1;

/// Callback-data prefix for the button-driven selective `/reset` flow (owner+dev).
pub const RESET_CB: &str = "rst:";
/// Selectable reset parts, OR'd into a bitmask that rides in the callback data.
const RESET_MATCHES: u8 = 1;
const RESET_PREDICTIONS: u8 = 2;
const RESET_EVERYTHING: u8 = 4;
const RESET_PROMPT: &str = "🧹 Reset (dev) — tap to select, then Submit:";

/// Marker file written by `/redeploy` (holds the chat id to notify) and read by
/// `bot::run` on the next startup to confirm the bot is back online. Relative to
/// the working directory (same place as the SQLite file).
pub const REDEPLOY_MARKER: &str = "redeploy.notify";

/// Owner gate for message-triggered admin commands. Returns the sender as a
/// `User` iff they're the configured `BOT_OWNER`; otherwise `None`, meaning the
/// caller should silently `return Ok(())` — non-owners (and the rare from-less
/// message, e.g. a channel post) never learn these commands exist. Centralises
/// the `from_id` + `is_owner` preamble each admin command repeated, and hands
/// back the full `User` so callers needing it (lang, mint target) avoid a
/// second `message.from` unwrap.
fn owner_guard(ctx: &Context, message: &Message) -> Option<User> {
    let user = message.from.clone()?;
    is_owner(ctx, user.id).then_some(user)
}

/// Parse Telegram's `retry after N` (seconds) out of a 429 error description.
/// telexide collapses the API response to its raw `description` string and drops
/// the structured `parameters.retry_after`, so the wait is only available as text
/// (e.g. `"Too Many Requests: retry after 7"`).
fn retry_after_secs(desc: &str) -> Option<u64> {
    let rest = &desc[desc.find("retry after")? + "retry after".len()..];
    rest.split_whitespace().next()?.parse().ok()
}

/// Send `text` to `chat_id`, **retrying through rate limits** so a large fan-out
/// (`/broadcast`, settle DMs) reaches every chat instead of silently dropping the
/// ones that hit Telegram's ~30 msg/s cap. A 429 reports `retry after N` — we
/// sleep that long (plus `RATE_LIMIT_SLACK_SECS`) and try again, up to
/// `SEND_MAX_RETRIES`. Any *other* error (the user blocked the bot, never opened
/// a DM, deleted the account) is permanent, so we give up immediately and return
/// false. Returns true once the message is delivered.
async fn send_resilient(ctx: &Context, chat_id: i64, text: &str) -> bool {
    for _ in 0..=SEND_MAX_RETRIES {
        match send_text(ctx, chat_id, text).await {
            Ok(_) => return true,
            Err(e) => match retry_after_secs(&e.0) {
                Some(secs) => {
                    tokio::time::sleep(Duration::from_secs(secs + RATE_LIMIT_SLACK_SECS)).await;
                }
                None => return false,
            },
        }
    }
    false
}

/// `/redeploy` — owner-only. Fire-and-forget triggers a **separate** systemd
/// oneshot (`waterx-deploy.service`) that pulls, builds, and restarts the bot.
/// It runs in its own unit/cgroup so the restart can't kill the deploy mid-build.
/// The trigger command is overridable via the `REDEPLOY_CMD` env var (default
/// `sudo systemctl start --no-block waterx-deploy.service`). See `DEPLOY.md`.
#[command(description = "owner: pull + rebuild + restart")]
pub async fn redeploy(ctx: Context, message: Message) -> CommandResult {
    if owner_guard(&ctx, &message).is_none() {
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
    if owner_guard(&ctx, &message).is_none() {
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
        open_g = format_number(s.open_predictions),
        open_g_coins = fmt_coins(s.open_game_stake),
    );
    reply(&ctx, &message, text).await?;
    Ok(())
}

/// `/profile` — owner-only inspector for one user's state: balance, open match
/// bets, self-host prediction stakes, referral links, and referee count. Target =
/// the replied-to user (best — we get their name), a numeric id argument, or the
/// owner themselves. Sent as HTML so the user id rides in a tap-to-copy `<code>`
/// span. Plain English (an operator diagnostic, not a user-facing surface).
#[command(description = "owner: inspect a user's profile")]
pub async fn profile(ctx: Context, message: Message) -> CommandResult {
    let Some(owner) = owner_guard(&ctx, &message) else {
        return Ok(());
    };
    // Replied-to user wins (carries a name); else a numeric id arg; else the
    // owner inspecting themselves.
    let replied = message.reply_to_message.as_ref().and_then(|r| r.from.clone());
    let arg_id = args(&message).first().and_then(|s| s.parse::<i64>().ok());
    let (target_id, named) = match (replied, arg_id) {
        (Some(u), _) => (u.id, Some(u)),
        (None, Some(id)) => (id, None),
        (None, None) => (owner.id, Some(owner)),
    };

    let body = profile_text(&ctx, target_id, named.as_ref()).await;
    tg::send_html(&ctx, message.chat.get_id(), &body).await?;
    Ok(())
}

/// Assemble the `/profile` body for `id` (`user` supplies the name when known —
/// a bare-id target stays anonymous). Every dynamic field is HTML-escaped so it
/// can't break the `<code>`-wrapped id; the id itself is the copyable part.
async fn profile_text(ctx: &Context, id: i64, user: Option<&User>) -> String {
    use crate::commands::tg::escape as esc;
    let database = db(ctx);

    let mut s = String::from("👤 Profile\n");
    if let Some(u) = user {
        s.push_str(&format!("{}\n", esc(&full_name(u))));
        if let Some(un) = u.username.as_deref().filter(|u| !u.is_empty()) {
            s.push_str(&format!("@{}\n", esc(un)));
        }
    }
    s.push_str(&format!("🆔 <code>{id}</code>\n"));

    if !database.user_exists(id).unwrap_or(false) {
        s.push_str("\n⚠️ No record — this user hasn't interacted with the bot yet.");
        return s;
    }

    // Balance.
    match database.get_user_info(id) {
        Ok(info) => s.push_str(&format!("\n🪙 Balance: {}", fmt_coins(info.balance))),
        Err(e) => s.push_str(&format!("\n🪙 Balance: ⚠️ {e}")),
    }
    // Chosen locale (only when explicitly set).
    if let Ok(Some(lang)) = database.get_lang(id) {
        s.push_str(&format!("\n🌐 Lang: {}", lang.store_code()));
    }
    // Check-in availability (read-only).
    if let Ok(avail) = database.checkin_available(id) {
        s.push_str(&format!("\n✅ Check-in: {}", if avail { "available" } else { "claimed today" }));
    }

    // Referrals.
    let (referrer, co) = database.get_referrers(id).unwrap_or((0, 0));
    let invited = database.count_referrals(id).unwrap_or(0);
    s.push_str("\n\n🤝 Referrals");
    match (referrer > 0, co > 0) {
        (true, true) => s.push_str(&format!("\n· referred by <code>{referrer}</code> (co <code>{co}</code>)")),
        (true, false) => s.push_str(&format!("\n· referred by <code>{referrer}</code>")),
        _ => s.push_str("\n· referred by —"),
    }
    s.push_str(&format!("\n· invited {invited} user(s)"));

    // Open (unsettled) match bets.
    let bets = database.list_open_wagers(id).unwrap_or_default();
    s.push_str(&format!("\n\n🎟️ Open match bets ({})", bets.len()));
    if bets.is_empty() {
        s.push_str("\n· none");
    } else {
        for p in &bets {
            let side = match p.outcome.as_str() {
                "teamA" => p.team_a.clone(),
                "teamB" => p.team_b.clone(),
                _ => "Draw".to_string(),
            };
            s.push_str(&format!(
                "\n• {} vs. {}\n  {} · 🪙{} → 🏆{}",
                esc(&p.team_a),
                esc(&p.team_b),
                esc(&side),
                fmt_coins(p.stake),
                fmt_coins(p.potential_payout()),
            ));
        }
    }

    // Stakes locked in still-open self-host (`/predict`) games (in-memory map).
    let mut pred = String::new();
    {
        let games = predictions(ctx);
        let guard = games.lock().await;
        for g in guard.values() {
            if !matches!(g.state, BetState::betting | BetState::closed) {
                continue;
            }
            let staked: Vec<(&String, i64)> = g
                .option_order
                .iter()
                .filter_map(|opt| {
                    g.options
                        .get(opt)
                        .and_then(|d| d.detail.get(&id).copied())
                        .filter(|&v| v > 0)
                        .map(|v| (opt, v))
                })
                .collect();
            if staked.is_empty() {
                continue;
            }
            let desc = g
                .description
                .split_once('\n')
                .map_or(g.description.as_str(), |(_, rest)| rest);
            pred.push_str(&format!("\n🎲 {}", esc(desc)));
            for (opt, stake) in staked {
                pred.push_str(&format!("\n  {} · 🪙{}", esc(opt), fmt_coins(stake * COIN)));
            }
        }
    }
    if !pred.is_empty() {
        s.push_str(&format!("\n\n🎲 Open predictions{pred}"));
    }

    s
}

/// `/mint <amount>` — credit `amount` whole water-coins to the sender of the
/// replied-to message (reply required). Positive only (no debt).
#[command(description = "owner: mint coins to the replied-to user")]
pub async fn mint(ctx: Context, message: Message) -> CommandResult {
    let Some(sender) = owner_guard(&ctx, &message) else {
        return Ok(());
    };

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
/// (Matches / Predictions / Everything + Submit); the actual work happens in
/// [`handle_reset_cb`] when Submit is pressed.
#[command(description = "owner+dev: selective reset")]
pub async fn reset(ctx: Context, message: Message) -> CommandResult {
    if owner_guard(&ctx, &message).is_none() || !is_dev(&ctx) {
        return Ok(());
    }
    let _ = tg::send_with_buttons(&ctx, message.chat.get_id(), RESET_PROMPT, &reset_picker_rows(0)).await;
    Ok(())
}

/// `/delete` — owner **and** dev-mode only (`is_dev`, so it can never fire on a
/// production bot). Deletes a user's profile (their `balance` row + any wagers,
/// via `Database::delete_user`) so they count as brand-new again — handy to
/// re-test referral binding, which gates on a `balance` row's existence. Target =
/// the replied-to user (gives a name in the confirmation) or a numeric id
/// argument (`/delete <id>`).
#[command(description = "owner+dev: delete a user to re-test referral")]
pub async fn delete(ctx: Context, message: Message) -> CommandResult {
    if owner_guard(&ctx, &message).is_none() || !is_dev(&ctx) {
        return Ok(());
    }
    // Replied-to user wins (carries a name); else a numeric id arg. Unlike
    // `/profile`, there's no "self" default — deleting requires an explicit target.
    let replied = message.reply_to_message.as_ref().and_then(|r| r.from.clone());
    let arg_id = args(&message).first().and_then(|s| s.parse::<i64>().ok());
    let Some(target) = replied.as_ref().map(|u| u.id).or(arg_id) else {
        reply(
            &ctx,
            &message,
            "/delete <id> — or reply to someone. Removes their profile so referral can re-bind.",
        )
        .await?;
        return Ok(());
    };
    match db(&ctx).delete_user(target) {
        Ok(true) => {
            let who = replied.as_ref().map_or_else(|| target.to_string(), full_name);
            reply(
                &ctx,
                &message,
                format!("🗑️ Deleted {who} (id {target}) — brand-new now, referral can re-bind."),
            )
            .await?;
        }
        Ok(false) => {
            reply(&ctx, &message, format!("No profile on record for id {target}.")).await?;
        }
        Err(e) => {
            eprintln!("delete_user error ({target}): {e}");
            reply(&ctx, &message, format!("⚠️ Delete failed: {e}")).await?;
        }
    }
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
        part(RESET_EVERYTHING, "Everything (refund + backup + wipe)"),
        vec![("🧹 Submit".to_string(), format!("{RESET_CB}go:{flags}"))],
    ]
}

/// Owner+dev button-driven selective reset (callback prefix [`RESET_CB`]).
/// `t:<flags>` re-renders the picker with a part toggled; `go:<flags>` executes
/// the picked parts. **Everything** returns all open bets, snapshots balances to a
/// backup file (`/load`), then wipes every table — aborting the wipe if the
/// backup can't be written, so balances are never lost.
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
            if flags & RESET_EVERYTHING != 0 {
                // (1) Return every open bet (match + self-host) so the snapshot
                // captures those coins back in the balances…
                match database.reset_wagers() {
                    Ok((n, refunded)) => lines.push(format!("🎟️ Returned {} to {n} open match bet(s)", fmt_coins(refunded))),
                    Err(e) => {
                        eprintln!("reset_wagers (everything) error: {e}");
                        lines.push("🎟️ Match refunds — ⚠️ error".to_string());
                    }
                }
                match database.reset_predictions() {
                    Ok((n, refunded)) => {
                        predictions(ctx).lock().await.clear();
                        lines.push(format!("🎲 Returned {} from {n} prediction(s)", fmt_coins(refunded)));
                    }
                    Err(e) => {
                        eprintln!("reset_predictions (everything) error: {e}");
                        lines.push("🎲 Prediction refunds — ⚠️ error".to_string());
                    }
                }
                // (2) Snapshot balances to a backup file — the safety net for /load.
                // If it fails, **abort the wipe** (balances are kept, no data loss).
                match backup_balances(ctx) {
                    Ok(file) => {
                        // (3) …then wipe everything.
                        match database.reset_all() {
                            Ok(()) => {
                                predictions(ctx).lock().await.clear();
                                lines.push(format!(
                                    "💾 Backup saved: {file} (restore with /load {file})\n🗑️ Everything wiped — kick + re-add the bot to re-record the group adder, then members re-refer."
                                ));
                            }
                            Err(e) => {
                                eprintln!("reset_all error: {e}");
                                lines.push("🗑️ Wipe — ⚠️ error".to_string());
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("backup_balances error: {e}");
                        lines.push(format!("💾 Backup FAILED — wipe aborted, balances kept ({e})"));
                    }
                }
            } else {
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
                            predictions(ctx).lock().await.clear();
                            lines.push(format!("🎲 Predictions cleared — {n} prediction(s), refunded {}", fmt_coins(refunded)))
                        }
                        Err(e) => {
                            eprintln!("reset_predictions error: {e}");
                            lines.push("🎲 Predictions — ⚠️ error".to_string());
                        }
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

/// Target schema version stamped by `/migrate`.
const MIGRATE_VERSION: i64 = 1;

/// `/migrate` (owner) — one-time cutover wash to the Polymarket share-trading
/// schema. **Version-guarded** (idempotent): a re-run after a successful
/// migration is a no-op. It refunds every unsettled *legacy* bet (open match
/// wagers + self-host prediction stakes) back to balances, snapshots balances to
/// a rollback file (aborting if it can't be written — never proceed without a
/// backup), then stamps the schema version. The `balance` table (lang / timezone
/// / referrals / check-in) **and** the new events/markets/positions tables are
/// left untouched — only the legacy bet data is drained. Prod-capable (unlike the
/// dev-only `/reset`).
#[command(description = "(owner) one-time cutover wash to the new schema")]
pub async fn migrate(ctx: Context, message: Message) -> CommandResult {
    if owner_guard(&ctx, &message).is_none() {
        return Ok(());
    }
    let database = db(&ctx);
    let ver = database.schema_version().unwrap_or(0);
    if ver >= MIGRATE_VERSION {
        reply(&ctx, &message, format!("✅ Already migrated (schema v{ver}). Nothing to do.")).await?;
        return Ok(());
    }
    // (1) Snapshot balances FIRST — never proceed without a rollback file.
    let snapshot = match backup_balances(&ctx) {
        Ok(file) => file,
        Err(e) => {
            reply(&ctx, &message, format!("⚠️ Backup failed — migration aborted, nothing changed ({e})")).await?;
            return Ok(());
        }
    };
    // (2) Refund all unsettled legacy funds back into balances.
    let (w_n, w_ref) = database.reset_wagers().unwrap_or((0, 0));
    let (p_n, p_ref) = database.reset_predictions().unwrap_or((0, 0));
    predictions(&ctx).lock().await.clear();
    // (3) Stamp the version so a re-run is a no-op.
    let _ = database.set_schema_version(MIGRATE_VERSION);
    let refunded = w_ref.saturating_add(p_ref);
    reply(
        &ctx,
        &message,
        format!(
            "✅ Migrated to schema v{MIGRATE_VERSION}.\n\
             🎟️ {w_n} open match bet(s) + 🎲 {p_n} prediction(s) refunded = {} returned to balances.\n\
             💾 Balances snapshotted to {snapshot} (rollback via /load {snapshot}).\n\
             Legacy bet data drained; the balance table (lang / timezone / referrals) is preserved.",
            fmt_coins(refunded)
        ),
    )
    .await?;
    Ok(())
}

/// Snapshot every non-zero coin balance to a timestamped `balances-*.json` file in
/// the working directory (next to the SQLite DB), returning the filename. The
/// safety net for the `[Everything]` wipe — restored via `/load`. `Err(String)` on
/// a DB or filesystem failure so the caller can abort the wipe.
fn backup_balances(ctx: &Context) -> Result<String, String> {
    let rows = db(ctx).export_balances().map_err(|e| e.to_string())?;
    let json = serde_json::to_string(&rows).map_err(|e| e.to_string())?;
    let file = format!("balances-{}.json", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
    std::fs::write(&file, json).map_err(|e| e.to_string())?;
    Ok(file)
}

/// A safe backup filename: our `balances-*.json` pattern with no path separators —
/// so a `/load <name>` can't traverse out of the working directory.
fn valid_backup_name(name: &str) -> bool {
    name.starts_with("balances-")
        && name.ends_with(".json")
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
}

/// Balance-backup files in the working directory, newest first (timestamped names
/// sort lexicographically, so a reversed sort puts the newest on top).
fn list_backup_files() -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(".")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| valid_backup_name(n))
        .collect();
    files.sort();
    files.reverse();
    files
}

/// `/load` — owner-only. With no argument, list the `balances-*.json` backups (the
/// snapshots written by an `[Everything]` reset). With a filename, restore those
/// balances (upsert each user's coin balance from the file).
#[command(description = "owner: list/restore balance backups")]
pub async fn load(ctx: Context, message: Message) -> CommandResult {
    if owner_guard(&ctx, &message).is_none() {
        return Ok(());
    }
    let parts = args(&message);
    let Some(name) = parts.first() else {
        // No arg → list the available backups.
        let files = list_backup_files();
        if files.is_empty() {
            reply(&ctx, &message, "💾 No balance backups found.").await?;
        } else {
            let mut s = String::from("💾 Balance backups (newest first):\n");
            for f in &files {
                s.push_str(&format!("\n· {f}"));
            }
            s.push_str("\n\nRestore with: /load <filename>");
            reply(&ctx, &message, s).await?;
        }
        return Ok(());
    };
    // With an arg → restore that backup.
    if !valid_backup_name(name) {
        reply(&ctx, &message, "⚠️ Invalid backup name — run /load to list valid files.").await?;
        return Ok(());
    }
    let content = match std::fs::read_to_string(name) {
        Ok(c) => c,
        Err(e) => {
            reply(&ctx, &message, format!("⚠️ Can't read {name}: {e}")).await?;
            return Ok(());
        }
    };
    let rows: Vec<(i64, i64)> = match serde_json::from_str(&content) {
        Ok(r) => r,
        Err(e) => {
            reply(&ctx, &message, format!("⚠️ Bad backup file ({name}): {e}")).await?;
            return Ok(());
        }
    };
    match db(&ctx).import_balances(&rows) {
        Ok(n) => reply(&ctx, &message, format!("✅ Restored {n} balance(s) from {name}")).await?,
        Err(e) => reply(&ctx, &message, format!("⚠️ Restore failed: {e}")).await?,
    }
    Ok(())
}

/// `/pause` — flip the bot into the paused state (every non-owner action is
/// blocked by `paused_block` until `/unpause`).
#[command(description = "owner: pause all actions")]
pub async fn pause(ctx: Context, message: Message) -> CommandResult {
    let Some(user) = owner_guard(&ctx, &message) else {
        return Ok(());
    };
    let lang = lang_for(&ctx, &user);
    db(&ctx).set_paused(true)?;
    reply(&ctx, &message, i18n::service_paused(lang)).await?;
    Ok(())
}

/// `/unpause` — resume normal operation.
#[command(description = "owner: resume all actions")]
pub async fn unpause(ctx: Context, message: Message) -> CommandResult {
    let Some(user) = owner_guard(&ctx, &message) else {
        return Ok(());
    };
    let lang = lang_for(&ctx, &user);
    db(&ctx).set_paused(false)?;
    reply(&ctx, &message, i18n::im_back(lang)).await?;
    Ok(())
}

/// `/broadcast <message>` — DM `message` to every known user in their private
/// chat. Failures (users who never opened a DM, blocked the bot, …) are skipped
/// and excluded from the delivered count.
#[command(description = "owner: broadcast a message to all users")]
pub async fn broadcast(ctx: Context, message: Message) -> CommandResult {
    if owner_guard(&ctx, &message).is_none() {
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
    // Reliability over speed: this can take a while for a big audience, but the
    // goal is that **every** chat receives it. We pace sends to stay under
    // Telegram's ~30 msg/s global cap and retry through any 429s, so nothing is
    // dropped to rate limiting (only genuinely-unreachable chats are skipped).
    let total = ids.len();
    reply(&ctx, &message, format!("📣 Sending to {total} chats…")).await?;
    let mut delivered = 0usize;
    for id in ids {
        if send_resilient(&ctx, id, body).await {
            delivered += 1;
        }
        // Leave headroom under the 30/s cap so live users' commands aren't
        // starved of rate budget during a broadcast.
        tokio::time::sleep(BROADCAST_PACE).await;
    }
    reply(
        &ctx,
        &message,
        format!("📣 Broadcast delivered to {delivered}/{total} chats"),
    )
    .await?;
    Ok(())
}
