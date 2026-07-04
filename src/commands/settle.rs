//! `/settle` — public **manual** settle of resolved markets, and the periodic
//! [`auto_settle`] task. Both detect Polymarket (Gamma) resolution for every open
//! **sourced** (`/events`) event, cache the winner on the row, then pay out
//! **every** resolved/void position — sourced (oracle-decided) *and* any
//! host-resolved AMM (`/predict`) event whose resolve-time settle didn't land.
//! Settlement is deterministic (the oracle / host already picked the winner), so
//! anyone can run `/settle`; it's the manual fallback if the 5-min auto-settle
//! task ever stalls. (There is no per-user `/claim` anymore — everything settles
//! automatically.)

use crate::commands::util::*;
use crate::commands::{markets, tg};
use crate::core::i18n::{self, Lang};
use crate::database::{ClaimKind, Database, Payout, SettleReport};
use std::collections::HashSet;
use telexide::model::CallbackQuery;
use telexide::prelude::*;

// Owner-only **manual** settlement — the `/settle` fallback for when Gamma can't
// auto-resolve (Polymarket down, or a result it can't map). The owner picks an
// open sourced event, then its winning outcome (or voids it). All plain English
// (an owner-only admin surface, like `/dashboard`).
/// `mstl:pick:<event_id>` — open one event's outcome picker.
pub const MSTL_PICK: &str = "mstl:pick:";
/// `mstl:win:<event_id>:<idx>` — declare the winning outcome + settle.
pub const MSTL_WIN: &str = "mstl:win:";
/// `mstl:void:<event_id>` — void the event + refund every bettor.
pub const MSTL_VOID: &str = "mstl:void:";
/// `mstl:list` — back to the open-event list from an outcome picker.
pub const MSTL_LIST: &str = "mstl:list";

#[command(description = "settle resolved markets")]
pub async fn settle(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let lang = lang_for_msg(&ctx, &message);
    let database = db(&ctx);
    let report = match sweep(&database).await {
        Ok(r) => r,
        Err(e) => {
            alert_owner(&ctx, &format!("[settle] sweep failed to enumerate events: {e}")).await;
            reply(&ctx, &message, i18n::db_error(lang)).await?;
            return Ok(());
        }
    };
    // DM each winner/refundee (same as the auto-settle task).
    notify_settled(&bot_token(&ctx), &database, &report.payouts).await;
    // Any event whose settle tx failed is money that didn't move — alert the owner.
    alert_failures(&ctx, &report.failed).await;

    // Owner: after the auto-sweep, offer a **manual** picker for any sourced event
    // Gamma couldn't resolve (Polymarket down, unmapped result), so settlement
    // never stalls on the oracle. Plain English (owner-only admin surface).
    let caller = message.from.as_ref().map(|u| u.id).unwrap_or(0);
    if is_owner(&ctx, caller) {
        let summary = sweep_summary(&report.payouts);
        let (list_text, rows) = list_view(&database);
        let text = format!("{summary}\n\n{list_text}");
        if rows.is_empty() {
            send_text(&ctx, message.chat.get_id(), text).await?;
        } else {
            tg::send_with_buttons(&ctx, message.chat.get_id(), &text, &rows).await?;
        }
        return Ok(());
    }

    // Non-owner: just the localized auto-sweep summary (settlement is deterministic).
    if report.payouts.is_empty() {
        reply(&ctx, &message, i18n::settle_nothing(lang)).await?;
        return Ok(());
    }
    let events: HashSet<i64> = report.payouts.iter().map(|p| p.event_id).collect();
    let paid: i64 = report.payouts.iter().map(|p| p.coins).sum();
    reply(
        &ctx,
        &message,
        i18n::settle_done(lang, &events.len().to_string(), &fmt_coins(paid)),
    )
    .await?;
    Ok(())
}

/// Alert the owner (log + DM) about events whose settle tx failed this sweep, so a
/// stuck payout doesn't stay buried in stderr. No-op when nothing failed.
async fn alert_failures(ctx: &Context, failed: &[i64]) {
    if failed.is_empty() {
        return;
    }
    let ids = failed
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    alert_owner(
        ctx,
        &format!(
            "[settle] {} event(s) failed to settle (id: {ids}) — will retry.",
            failed.len()
        ),
    )
    .await;
}

/// One plain-English line summarising an auto-sweep's result (owner view).
fn sweep_summary(payouts: &[Payout]) -> String {
    let settled: HashSet<i64> = payouts.iter().map(|p| p.event_id).collect();
    if settled.is_empty() {
        "🔁 Auto-settle: nothing resolved on Polymarket right now.".to_string()
    } else {
        let paid: i64 = payouts.iter().map(|p| p.coins).sum();
        format!(
            "✅ Auto-settled {} event(s) — paid out {} 🪙.",
            settled.len(),
            fmt_coins(paid)
        )
    }
}

/// The owner's manual-settle list: a button per still-open sourced event
/// (`mstl:pick:<id>`). Empty rows (`Vec::new()`) when nothing is open — the caller
/// sends the text without a keyboard (Telegram rejects an empty `inline_keyboard`).
fn list_view(database: &Database) -> (String, Vec<tg::Row>) {
    // Only events that have actually ended (`ends_at` past) — a match still in
    // progress is never offered for manual settlement.
    let open = database.open_sourced_events(now()).unwrap_or_default();
    if open.is_empty() {
        return (
            "✅ No finished events awaiting settlement.".to_string(),
            Vec::new(),
        );
    }
    let text = "⚠️ Finished events Polymarket hasn't resolved — tap one to settle it by hand:".to_string();
    let rows = open
        .iter()
        .map(|(id, title)| vec![(title.clone(), format!("{MSTL_PICK}{id}"))])
        .collect();
    (text, rows)
}

/// One event's outcome picker: a button per outcome (`mstl:win:<id>:<idx>`), a void
/// button, and a back-to-list button.
fn outcome_picker(event_id: i64, title: &str, outcomes: &[(i64, String)]) -> (String, Vec<tg::Row>) {
    let text = format!("🎯 Settle by hand: {title}\nPick the winning outcome:");
    let mut rows: Vec<tg::Row> = outcomes
        .iter()
        .map(|(idx, name)| vec![(name.clone(), format!("{MSTL_WIN}{event_id}:{idx}"))])
        .collect();
    rows.push(vec![(
        "🚫 Void (refund all)".to_string(),
        format!("{MSTL_VOID}{event_id}"),
    )]);
    rows.push(vec![("⬅ Back".to_string(), MSTL_LIST.to_string())]);
    (text, rows)
}

/// Edit the callback's message in place to `text` + `rows`, dropping the keyboard
/// when `rows` is empty (Telegram rejects an empty `inline_keyboard` on an edit).
async fn render_in_place(ctx: &Context, cb: &CallbackQuery, text: &str, rows: &[tg::Row]) {
    let (chat, msg) = tg::cb_coords(cb);
    if rows.is_empty() {
        let _ = tg::edit_text_only(ctx, chat, msg, text).await;
    } else {
        let _ = tg::edit_with_buttons(ctx, chat, msg, text, rows).await;
    }
}

/// `mstl:pick:<event_id>` — owner: edit the settle message into one event's
/// winning-outcome picker.
pub async fn handle_manual_pick(
    ctx: &Context,
    cb: &CallbackQuery,
    rest: &str,
) -> Result<(), telexide::Error> {
    if !is_owner(ctx, cb.from.id) {
        return tg::answer(ctx, cb, "", false).await;
    }
    let Ok(event_id) = rest.parse::<i64>() else {
        return tg::answer(ctx, cb, "", false).await;
    };
    match db(ctx).sourced_outcomes(event_id) {
        Ok(Some((title, outcomes))) => {
            let (text, rows) = outcome_picker(event_id, &title, &outcomes);
            render_in_place(ctx, cb, &text, &rows).await;
            tg::answer(ctx, cb, "", false).await
        }
        _ => {
            // Settled/gone since the list was drawn — refresh it.
            let (text, rows) = list_view(&db(ctx));
            render_in_place(ctx, cb, &text, &rows).await;
            tg::answer(ctx, cb, "Already settled.", true).await
        }
    }
}

/// `mstl:win:<event_id>:<idx>` — owner: manually resolve a sourced event to the
/// picked outcome, settle every position, DM winners, and return to the list.
pub async fn handle_manual_win(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    if !is_owner(ctx, cb.from.id) {
        return tg::answer(ctx, cb, "", false).await;
    }
    let Some([event_id, idx]) = parse_ints::<2>(rest) else {
        return tg::answer(ctx, cb, "", false).await;
    };
    let database = db(ctx);
    // The winner's name for the toast (grab it before the outcome rows are gone).
    let winner = database
        .sourced_outcomes(event_id)
        .ok()
        .flatten()
        .and_then(|(_, outs)| outs.into_iter().find(|(i, _)| *i == idx).map(|(_, n)| n))
        .unwrap_or_default();
    let flip = database.resolve_event(event_id, idx, now());
    let toast = settle_now(ctx, &database, flip, event_id, &format!("Settled → {winner}")).await;
    let (text, rows) = list_view(&database);
    render_in_place(ctx, cb, &text, &rows).await;
    tg::answer(ctx, cb, &toast, true).await
}

/// `mstl:void:<event_id>` — owner: void a sourced event (refund every bettor's cost
/// basis), then return to the list.
pub async fn handle_manual_void(
    ctx: &Context,
    cb: &CallbackQuery,
    rest: &str,
) -> Result<(), telexide::Error> {
    if !is_owner(ctx, cb.from.id) {
        return tg::answer(ctx, cb, "", false).await;
    }
    let Ok(event_id) = rest.parse::<i64>() else {
        return tg::answer(ctx, cb, "", false).await;
    };
    let database = db(ctx);
    let flip = database.void_event(event_id, now());
    let toast = settle_now(ctx, &database, flip, event_id, "Voided — all bettors refunded").await;
    let (text, rows) = list_view(&database);
    render_in_place(ctx, cb, &text, &rows).await;
    tg::answer(ctx, cb, &toast, true).await
}

/// `mstl:list` — owner: back to the open-event list from an outcome picker.
pub async fn handle_manual_list(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    if !is_owner(ctx, cb.from.id) {
        return tg::answer(ctx, cb, "", false).await;
    }
    let (text, rows) = list_view(&db(ctx));
    render_in_place(ctx, cb, &text, &rows).await;
    tg::answer(ctx, cb, "", false).await
}

/// Shared resolve/void → settle → notify tail for the two manual actions. `flip` is
/// the `resolve_event`/`void_event` result; `label` names the action for the success
/// toast (e.g. `Settled → France`). Winners are DM'd via [`notify_settled`]. Returns
/// the toast text.
async fn settle_now(
    ctx: &Context,
    database: &Database,
    flip: rusqlite::Result<bool>,
    event_id: i64,
    label: &str,
) -> String {
    match flip {
        Ok(true) => match database.settle_event(event_id, None) {
            Ok(payouts) => {
                notify_settled(&bot_token(ctx), database, &payouts).await;
                let paid: i64 = payouts.iter().map(|p| p.coins).sum();
                format!("✅ {label} — paid out {} 🪙.", fmt_coins(paid))
            }
            Err(e) => {
                alert_owner(
                    ctx,
                    &format!("[settle] manual settle failed (event {event_id}): {e}"),
                )
                .await;
                "⚠️ Marked resolved but settle failed — retry /settle.".to_string()
            }
        },
        Ok(false) => "Already settled.".to_string(),
        Err(e) => {
            alert_owner(
                ctx,
                &format!("[settle] manual resolve/void failed (event {event_id}): {e}"),
            )
            .await;
            "⚠️ Couldn't settle — try again.".to_string()
        }
    }
}

/// Detect Gamma resolution for every open sourced event (caching the winner on
/// the row), then settle **all** resolved/void positions — sourced and AMM.
/// Shared by the `/settle` command and the periodic [`auto_settle`] task. The
/// returned [`SettleReport`] carries the payouts + any per-event settle failures.
pub async fn sweep(database: &Database) -> rusqlite::Result<SettleReport> {
    let now = now();
    for (event_id, slug, n_outcomes) in database.all_open_sourced().unwrap_or_default() {
        if slug.is_empty() {
            continue;
        }
        if let Ok(Some(idx)) = markets::gamma_resolution(&slug, n_outcomes as usize).await {
            let _ = database.resolve_event(event_id, idx, now);
        }
    }
    database.settle_all_resolved()
}

/// The periodic auto-settle task body (every `bot::AUTO_SETTLE_INTERVAL`): run the
/// [`sweep`], DM each credited user ([`notify_settled`]), and **alert the owner**
/// (log + DM) if the sweep couldn't enumerate events or any event failed to settle
/// — so a stuck payout surfaces instead of sitting in stderr. `token`/`owner` let
/// it DM directly via the Bot API (the task runs outside the framework, no `Context`).
pub async fn auto_settle(token: &str, owner: i64, database: &Database) {
    match sweep(database).await {
        Ok(report) => {
            notify_settled(token, database, &report.payouts).await;
            if !report.failed.is_empty() {
                let ids = report
                    .failed
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                alert_owner_token(
                    token,
                    owner,
                    &format!(
                        "[auto-settle] {} event(s) failed to settle (id: {ids}) — will retry.",
                        report.failed.len()
                    ),
                )
                .await;
            }
        }
        Err(e) => alert_owner_token(token, owner, &format!("[auto-settle] sweep error: {e}")).await,
    }
}

/// DM each user a settlement credited — winners (`Won`) and void refunds
/// (`Refunded`), i.e. every payout with `coins > 0`. Pure losers (`coins == 0`,
/// which is also all the sweep logs to `/history`) are not messaged. Each renders
/// in the user's saved locale (English if unset). Best-effort per user: a DM to
/// someone who bet in a group but never started the bot just fails and is skipped —
/// the coins already landed on their balance regardless. Shared by the auto-settle
/// task and the manual `/settle` (whichever path settles an event first sees its
/// payouts; the other gets an empty list, so a user is never double-notified).
pub(crate) async fn notify_settled(token: &str, database: &Database, payouts: &[Payout]) {
    for p in payouts.iter().filter(|p| p.coins > 0) {
        let lang = database.get_lang(p.user).ok().flatten().unwrap_or(Lang::En);
        let coins = fmt_coins(p.coins);
        let text = if p.kind == ClaimKind::Refunded {
            i18n::bet_refunded(lang, &p.title, &coins)
        } else {
            i18n::bet_won(lang, &p.title, &coins)
        };
        let _ = tg::send_text_token(token, p.user, &text).await;
    }
}
