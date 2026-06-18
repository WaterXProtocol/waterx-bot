//! Real-money match betting — **in the group, on a shared board** (no DM).
//! Tapping a match in `/markets` edits the list message into that match's board;
//! every member bets on the same message: per-outcome amount buttons accumulate
//! each user's pending stake (private toast — a shared message can't show
//! per-user totals), `Confirm` debits + records the wager, `Clear` resets, and
//! `Back` re-renders the match list. Odds are re-fetched at confirm time so the
//! locked price is fresh. Wagers are settled later by an admin (`/settle`).

use crate::commands::markets;
use crate::commands::tg;
use crate::commands::util::*;
use crate::database::{decimal_payout, COIN};
use crate::i18n::{self, Lang};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::OnceLock;
use telexide::api::types::AnswerCallbackQuery;
use telexide::model::CallbackQuery;
use telexide::prelude::*;

/// Whole-coin stake presets — each button *adds* this much to the running stake.
const SIZE_PRESETS: [i64; 5] = [1, 5, 10, 50, 100];

/// Callback-data prefixes for the in-group board (the `bet:` prefix lives in
/// `markets`). Outcome is a single char in the data: `a`/`d`/`b`.
pub const MB_ADD: &str = "mba:"; // mba:<market_id>:<a|d|b>:<amt> — accumulate
pub const MB_CONFIRM: &str = "mbc:"; // mbc:<market_id> — debit + place
pub const MB_CLEAR: &str = "mbx:"; // mbx:<market_id> — clear pending
pub const MB_BACK: &str = "mbb"; // re-render the /markets list

/// Per-(market_id, user) pending stake (whole coins) + chosen outcome — nothing
/// is debited until `Confirm`, so a restart just drops uncommitted drafts.
type MatchDrafts = HashMap<(String, i64), (String, i64)>;

fn match_drafts() -> &'static Mutex<MatchDrafts> {
    static D: OnceLock<Mutex<MatchDrafts>> = OnceLock::new();
    D.get_or_init(|| Mutex::new(MatchDrafts::new()))
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn cb_lang(ctx: &Context, cb: &CallbackQuery) -> Lang {
    db(ctx)
        .get_lang(cb.from.id)
        .ok()
        .flatten()
        .unwrap_or_else(|| Lang::from_user(&cb.from))
}

async fn answer(
    ctx: &Context,
    cb: &CallbackQuery,
    text: &str,
    alert: bool,
) -> Result<(), telexide::Error> {
    let mut a = AnswerCallbackQuery::new(cb.id.clone());
    if !text.is_empty() {
        a.text = Some(text.to_string());
    }
    a.show_alert = Some(alert);
    ctx.api.answer_callback_query(a).await?;
    Ok(())
}

/// Display name of an outcome (`teamA`/`teamB`/`draw`) for a match.
fn side_name(lang: Lang, m: &markets::MatchInfo, outcome: &str) -> String {
    match outcome {
        "teamA" => m.team_a.clone(),
        "teamB" => m.team_b.clone(),
        _ => i18n::draw_label(lang).to_string(),
    }
}

/// `a|d|b` → stored outcome key.
fn outcome_from_char(c: &str) -> Option<&'static str> {
    match c {
        "a" => Some("teamA"),
        "b" => Some("teamB"),
        "d" => Some("draw"),
        _ => None,
    }
}

fn board_text(lang: Lang, m: &markets::MatchInfo) -> String {
    let mut s = format!("{} vs. {}\n", m.team_a, m.team_b);
    for outcome in ["teamA", "draw", "teamB"] {
        if let Some(c) = m.odds(outcome).filter(|c| *c > 0.0) {
            s.push_str(&format!("· {} — {:.2}\n", side_name(lang, m, outcome), decimal_odds(c)));
        }
    }
    s.push('\n');
    s.push_str(i18n::bet_pick(lang));
    s
}

/// The board keyboard: one amount row per priced outcome, then Confirm/Clear and
/// Back. Amount/outcome ride in the callback data (`mba:<id>:<a|d|b>:<amt>`); the
/// per-user running total lives in memory and is echoed via a private toast.
fn board_rows(lang: Lang, m: &markets::MatchInfo) -> Vec<tg::Row> {
    let mut rows: Vec<tg::Row> = Vec::new();
    for (oc, outcome) in [("a", "teamA"), ("d", "draw"), ("b", "teamB")] {
        if m.odds(outcome).filter(|c| *c > 0.0).is_some() {
            let side = side_name(lang, m, outcome);
            let row: tg::Row = SIZE_PRESETS
                .iter()
                .map(|p| (format!("{side} +{p}"), format!("{MB_ADD}{}:{oc}:{p}", m.market_id)))
                .collect();
            rows.push(row);
        }
    }
    rows.push(vec![
        (i18n::bet_btn_confirm(lang).to_string(), format!("{MB_CONFIRM}{}", m.market_id)),
        (i18n::bet_btn_clear(lang).to_string(), format!("{MB_CLEAR}{}", m.market_id)),
    ]);
    rows.push(vec![(i18n::bet_btn_back(lang).to_string(), MB_BACK.to_string())]);
    rows
}

/// `bet:<market_id>` — edit the match list **in place** into that match's shared
/// betting board (stays in the group; every member bets on it).
pub async fn handle_bet(
    ctx: &Context,
    cb: &CallbackQuery,
    market_id: &str,
) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(m) = markets::fetch_one(lang, market_id).await else {
        return answer(ctx, cb, i18n::bet_unavailable(lang), true).await;
    };
    if m.ends_at != 0 && now() >= m.ends_at {
        return answer(ctx, cb, i18n::bet_unavailable(lang), true).await;
    }
    edit(ctx, cb, &board_text(lang, &m), &board_rows(lang, &m)).await?;
    answer(ctx, cb, "", false).await
}

/// `mba:<market_id>:<a|d|b>:<amt>` — add to the tapper's pending stake (private
/// toast; switching outcome resets). Nothing is debited.
pub async fn handle_match_add(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let parts: Vec<&str> = rest.split(':').collect();
    let [mid, oc, amt_s] = parts.as_slice() else {
        return answer(ctx, cb, "", false).await;
    };
    let (Some(outcome), Ok(amt)) = (outcome_from_char(oc), amt_s.parse::<i64>()) else {
        return answer(ctx, cb, "", false).await;
    };
    let Some(m) = markets::fetch_one(lang, mid).await else {
        return answer(ctx, cb, i18n::bet_unavailable(lang), true).await;
    };
    if m.ends_at != 0 && now() >= m.ends_at {
        return answer(ctx, cb, i18n::bet_unavailable(lang), true).await;
    }
    let pending = {
        let mut d = match_drafts().lock();
        let e = d
            .entry((mid.to_string(), cb.from.id))
            .or_insert_with(|| (outcome.to_string(), 0));
        if e.0 != outcome {
            *e = (outcome.to_string(), 0); // switched side → start fresh
        }
        e.1 += amt;
        e.1
    };
    answer(ctx, cb, &i18n::bet_pending(lang, &pending.to_string(), &side_name(lang, &m, outcome)), false)
        .await
}

/// `mbx:<market_id>` — drop the tapper's pending stake.
pub async fn handle_match_clear(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    match_drafts().lock().remove(&(rest.to_string(), cb.from.id));
    answer(ctx, cb, i18n::bet_cleared(lang), false).await
}

/// `mbc:<market_id>` — the only money-moving step: re-fetch fresh odds, debit,
/// record the wager (locked odds), and announce it in the group.
pub async fn handle_match_confirm(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let mid = rest;
    let Some((outcome, amount)) = match_drafts().lock().remove(&(mid.to_string(), cb.from.id)) else {
        return answer(ctx, cb, i18n::bad_stake(lang), true).await;
    };
    if amount <= 0 {
        return answer(ctx, cb, i18n::bad_stake(lang), true).await;
    }
    let Some(m) = markets::fetch_one(lang, mid).await else {
        return answer(ctx, cb, i18n::bet_unavailable(lang), true).await;
    };
    let Some(odds) = m.odds(&outcome).filter(|c| *c > 0.0) else {
        return answer(ctx, cb, i18n::bet_unavailable(lang), true).await;
    };

    let stake_units = amount * COIN;
    let database = db(ctx);
    if !database.balance_change(cb.from.id, -stake_units).unwrap_or(false) {
        // Restore the pending so the user can adjust/clear it.
        match_drafts().lock().insert((mid.to_string(), cb.from.id), (outcome, amount));
        return answer(ctx, cb, i18n::not_enough_money(lang), true).await;
    }
    if let Err(err) = database.place_wager(
        cb.from.id,
        &m.market_id,
        &m.slug,
        &m.team_a,
        &m.team_b,
        &outcome,
        stake_units,
        odds,
        m.ends_at,
    ) {
        database.force_change(cb.from.id, stake_units).ok();
        eprintln!("place_wager error: {err}");
        return answer(ctx, cb, i18n::db_error(lang), true).await;
    }

    let side = side_name(lang, &m, &outcome);
    let odds_str = format!("{:.2}", decimal_odds(odds));
    let payout = decimal_payout(stake_units, odds);
    // Announce the bet in the board's chat so members see the action.
    if let Some(message) = &cb.message {
        let announce =
            i18n::bet_announce(lang, &full_name(&cb.from), &fmt_coins(stake_units), &side, &odds_str);
        let _ = send_text(ctx, message.chat.get_id(), announce).await;
    }
    answer(
        ctx,
        cb,
        &i18n::bet_placed(lang, &fmt_coins(stake_units), &side, &odds_str, &fmt_coins(payout)),
        true,
    )
    .await
}

/// `mbb` — re-render the `/markets` list in place (Back from a match board).
pub async fn handle_match_back(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    let chat = message.chat.get_id();
    let tz = if is_group_chat(chat) {
        0
    } else {
        db(ctx).get_tz(cb.from.id).ok().flatten().unwrap_or(0)
    };
    let (text, rows) = markets::brief(lang, tz).await;
    let _ = tg::edit_with_buttons(ctx, chat, message.message_id, &text, &rows).await;
    answer(ctx, cb, "", false).await
}

/// Edit the callback's own message in place.
async fn edit(
    ctx: &Context,
    cb: &CallbackQuery,
    text: &str,
    rows: &[tg::Row],
) -> Result<(), telexide::Error> {
    let (chat, msg) = cb
        .message
        .as_ref()
        .map(|m| (m.chat.get_id(), m.message_id))
        .unwrap_or((cb.from.id, 0));
    let _ = tg::edit_with_buttons(ctx, chat, msg, text, rows).await;
    Ok(())
}
