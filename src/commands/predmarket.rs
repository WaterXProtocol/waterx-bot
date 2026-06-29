//! AMM prediction market — the host `/predict` event card.
//!
//! A shared board shows each outcome's **LMSR price** + buy buttons + a host
//! `[resolve]` control. Tapping an outcome opens the tapper's own **spend
//! builder** (owner-locked; presets accumulate in the callback data) which buys
//! YES shares via `amm_buy`; the board re-renders in place after each trade. The
//! host resolves by declaring the winning outcome (`resolve_event`) or voiding
//! (`void_event`); payouts are collected later via `/claim`. All board state
//! lives in the DB (no in-memory map) — callbacks use the `pm:` namespace.

use crate::commands::tg::{self, answer};
use crate::commands::util::*;
use crate::core::i18n::{self, Lang};
use crate::core::lmsr;
use crate::core::types::OddsFormat;
use crate::database::{AmmBoard, TradeOutcome, COIN};
use telexide::model::CallbackQuery;
use telexide::prelude::*;

/// Whole-coin spend presets for the buy builder.
const SPEND_PRESETS: [i64; 4] = [1, 5, 10, 50];

// Callback prefixes (all under `pm:`).
pub const PM_BUY: &str = "pm:buy:"; // pm:buy:<event>:<idx>
pub const PM_SIZE: &str = "pm:sz:"; // pm:sz:<event>:<idx>:<owner>:<spend>
pub const PM_PLACE: &str = "pm:go:"; // pm:go:<event>:<idx>:<owner>:<spend>
pub const PM_RESOLVE: &str = "pm:resolve:"; // pm:resolve:<event>
pub const PM_WIN: &str = "pm:win:"; // pm:win:<event>:<idx>
pub const PM_VOID: &str = "pm:void:"; // pm:void:<event>
pub const PM_BACK: &str = "pm:back:"; // pm:back:<event>

fn circled(n: usize) -> String {
    const C: [&str; 10] = ["①", "②", "③", "④", "⑤", "⑥", "⑦", "⑧", "⑨", "⑩"];
    C.get(n - 1).map_or_else(|| format!("{n}."), |c| (*c).to_string())
}

fn cb_lang(ctx: &Context, cb: &CallbackQuery) -> Lang {
    db(ctx).get_lang(cb.from.id).ok().flatten().unwrap_or_else(|| Lang::from_user(&cb.from))
}

fn cb_coords(cb: &CallbackQuery) -> (i64, i64) {
    cb.message.as_ref().map(|m| (m.chat.get_id(), m.message_id)).unwrap_or((0, 0))
}

/// Each outcome's YES price in cents from the LMSR curve (`SHARE == COIN`).
fn prices_cents(b: &AmmBoard) -> Vec<f64> {
    let q: Vec<f64> = b.options.iter().map(|(_, q)| *q as f64 / COIN as f64).collect();
    (0..b.options.len()).map(|i| lmsr::price(&q, b.b_param as f64, i) * 100.0).collect()
}

/// The board text (host's pinned lang + odds format — a shared message).
fn board_text(b: &AmmBoard) -> String {
    let lang = Lang::from_store_code(&b.lang).unwrap_or(Lang::En);
    let fmt = OddsFormat::from_store_code(&b.odds_fmt);
    let emoji = match b.state.as_str() {
        "open" => "🟢",
        "resolved" => "✅",
        "void" => "↩️",
        _ => "🔴",
    };
    let mut s = format!("🎲 {}  ·  {emoji}\n", b.question);
    if b.ends_at != 0 && b.state == "open" {
        if let Some(t) = fmt_local_time(b.ends_at, b.tz_offset) {
            s.push_str(&format!("⏰ {t}\n"));
        }
    }
    let cents = prices_cents(b);
    for (i, (name, _)) in b.options.iter().enumerate() {
        s.push_str(&format!("\n{} {name}   {}", circled(i + 1), format_odds(cents[i], fmt)));
    }
    s.push_str(&format!("\n\n{}", i18n::board_footer_open(lang, &fmt_coins(b.pool))));
    // Terminal states append the result line below the board.
    match b.state.as_str() {
        "resolved" => {
            let winner = b
                .winning_market
                .and_then(|i| b.options.get(i as usize))
                .map(|(n, _)| n.clone())
                .unwrap_or_default();
            s.push_str(&format!("\n\n{}", i18n::pm_resolved(lang, &winner)));
        }
        "void" => s.push_str(&format!("\n\n{}", i18n::pm_voided(lang))),
        _ => {}
    }
    s
}

/// The live board's buttons: a buy button per outcome (3/row) + a host
/// `[resolve]` row. Empty once the event is no longer open.
fn board_rows(b: &AmmBoard) -> Vec<tg::Row> {
    if b.state != "open" {
        return Vec::new();
    }
    let lang = Lang::from_store_code(&b.lang).unwrap_or(Lang::En);
    let mut rows: Vec<tg::Row> = b
        .options
        .iter()
        .enumerate()
        .map(|(i, (name, _))| (name.clone(), format!("{PM_BUY}{}:{i}", b.event_id)))
        .collect::<Vec<_>>()
        .chunks(3)
        .map(<[_]>::to_vec)
        .collect();
    rows.push(vec![(i18n::close_button(lang).to_string(), format!("{PM_RESOLVE}{}", b.event_id))]);
    rows
}

/// The host's resolve view: pick-a-winner buttons (3/row) + void + back.
fn resolve_rows(b: &AmmBoard) -> Vec<tg::Row> {
    let lang = Lang::from_store_code(&b.lang).unwrap_or(Lang::En);
    let mut rows: Vec<tg::Row> = b
        .options
        .iter()
        .enumerate()
        .map(|(i, (name, _))| (name.clone(), format!("{PM_WIN}{}:{i}", b.event_id)))
        .collect::<Vec<_>>()
        .chunks(3)
        .map(<[_]>::to_vec)
        .collect();
    rows.push(vec![
        (i18n::void_label(lang).to_string(), format!("{PM_VOID}{}", b.event_id)),
        (i18n::bet_btn_back(lang).to_string(), format!("{PM_BACK}{}", b.event_id)),
    ]);
    rows
}

/// Re-render the board card in place from the live DB state.
async fn refresh_card(ctx: &Context, event_id: i64) {
    let Ok(Some(board)) = db(ctx).amm_board(event_id) else {
        return;
    };
    let Ok(Some((chat, msg))) = db(ctx).event_card(event_id) else {
        return;
    };
    let _ = tg::edit_with_buttons(ctx, chat, msg, &board_text(&board), &board_rows(&board)).await;
}

/// Build the per-tapper spend builder for buying `idx` (owner-locked). Shares are
/// previewed via the read-only `amm_buy_quote`.
#[allow(clippy::too_many_arguments)]
fn builder(
    ctx: &Context,
    lang: Lang,
    fmt: OddsFormat,
    b: &AmmBoard,
    idx: i64,
    owner: i64,
    spend_whole: i64,
    chat: i64,
    owner_name: &str,
) -> (String, Vec<tg::Row>) {
    let name = b.options.get(idx as usize).map(|(n, _)| n.as_str()).unwrap_or("");
    let price = prices_cents(b).get(idx as usize).copied().unwrap_or(0.0);
    let spend_micro = spend_whole.max(0).saturating_mul(COIN);
    let shares = db(ctx).amm_buy_quote(b.event_id, idx, spend_micro).ok().flatten().unwrap_or(0);
    let body = i18n::bet_build(lang, name, &format_odds(price, fmt), &fmt_coins(spend_micro), &fmt_coins(shares));
    let text = board_header(chat, owner_name, &body);
    let preset_row: tg::Row = SPEND_PRESETS
        .iter()
        .map(|p| {
            (format!("+{p}"), format!("{PM_SIZE}{}:{idx}:{owner}:{}", b.event_id, spend_whole.saturating_add(*p)))
        })
        .collect();
    let action_row = vec![
        (i18n::bet_btn_confirm(lang).to_string(), format!("{PM_PLACE}{}:{idx}:{owner}:{spend_whole}", b.event_id)),
        (i18n::bet_btn_clear(lang).to_string(), format!("{PM_SIZE}{}:{idx}:{owner}:0", b.event_id)),
    ];
    let mut rows = vec![preset_row, action_row];
    if is_group_chat(chat) {
        rows.push(vec![(i18n::bet_btn_dismiss(lang).to_string(), format!("bx:{owner}"))]);
    }
    (text, rows)
}

/// `pm:buy:<event>:<idx>` — open the tapper's spend builder (a reply to the card
/// in a group; a fresh DM message privately). Owner-locked to the tapper.
pub async fn handle_buy(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some((event_id, idx)) = parse2(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    let Some(board) = db(ctx).amm_board(event_id).ok().flatten() else {
        return answer(ctx, cb, i18n::bet_expired(lang), true).await;
    };
    if board.state != "open" {
        return answer(ctx, cb, i18n::bet_closed(lang), true).await;
    }
    if idx < 0 || idx as usize >= board.options.len() {
        return answer(ctx, cb, "", false).await;
    }
    let (chat, msg) = cb_coords(cb);
    let fmt = db(ctx).get_odds_fmt(cb.from.id).unwrap_or_default();
    let (text, rows) = builder(ctx, lang, fmt, &board, idx, cb.from.id, 0, chat, &full_name(&cb.from));
    answer(ctx, cb, "", false).await?;
    if is_group_chat(chat) {
        let _ = tg::send_with_buttons_reply(ctx, chat, msg, &text, &rows).await;
    } else {
        let _ = tg::send_with_buttons(ctx, chat, &text, &rows).await;
    }
    Ok(())
}

/// `pm:sz:<event>:<idx>:<owner>:<spend>` — re-render the builder at the new total.
pub async fn handle_size(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some((event_id, idx, owner, spend)) = parse4(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    if owner != cb.from.id {
        return answer(ctx, cb, i18n::not_your_bet(lang), true).await;
    }
    let Some(board) = db(ctx).amm_board(event_id).ok().flatten() else {
        return answer(ctx, cb, i18n::bet_expired(lang), true).await;
    };
    let (chat, msg) = cb_coords(cb);
    let fmt = db(ctx).get_odds_fmt(cb.from.id).unwrap_or_default();
    let (text, rows) = builder(ctx, lang, fmt, &board, idx, owner, spend, chat, &full_name(&cb.from));
    let _ = tg::edit_with_buttons(ctx, chat, msg, &text, &rows).await;
    answer(ctx, cb, "", false).await
}

/// `pm:go:<event>:<idx>:<owner>:<spend>` — buy YES shares via `amm_buy`, refresh
/// the board, and announce.
pub async fn handle_place(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some((event_id, idx, owner, spend_whole)) = parse4(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    if owner != cb.from.id {
        return answer(ctx, cb, i18n::not_your_bet(lang), true).await;
    }
    let Some(spend) = to_micro(spend_whole) else {
        return answer(ctx, cb, i18n::bad_stake(lang), true).await;
    };
    let fmt = db(ctx).get_odds_fmt(cb.from.id).unwrap_or_default();
    let (name, price) = db(ctx)
        .amm_board(event_id)
        .ok()
        .flatten()
        .and_then(|b| {
            let p = prices_cents(&b).get(idx as usize).copied()?;
            Some((b.options.get(idx as usize)?.0.clone(), p))
        })
        .unwrap_or_default();
    let shares = match db(ctx).amm_buy(event_id, idx, cb.from.id, spend) {
        Ok(TradeOutcome::Filled { shares, .. }) => shares,
        Ok(TradeOutcome::Rejected) => return answer(ctx, cb, i18n::not_enough_money(lang), true).await,
        Ok(TradeOutcome::Unavailable) => return answer(ctx, cb, i18n::bet_closed(lang), true).await,
        Err(e) => {
            eprintln!("amm_buy error (event {event_id}): {e}");
            return answer(ctx, cb, i18n::db_error(lang), true).await;
        }
    };
    refresh_card(ctx, event_id).await;
    let odds_str = format_odds(price, fmt);
    let placed = i18n::bet_placed(lang, &fmt_coins(spend), &name, &odds_str, &fmt_coins(shares));
    let (chat, msg) = cb_coords(cb);
    if is_group_chat(chat) {
        let _ = tg::delete_message(ctx, chat, msg).await;
        let announce = i18n::bet_announce(lang, &full_name(&cb.from), &fmt_coins(spend), &name, &odds_str);
        if let Ok(Some((cchat, cmsg))) = db(ctx).event_card(event_id) {
            let _ = tg::send_text_reply(ctx, cchat, cmsg, &announce).await;
        } else {
            let _ = send_text(ctx, chat, announce).await;
        }
    } else {
        let _ = tg::edit_text_only(ctx, chat, msg, &placed).await;
    }
    answer(ctx, cb, &placed, true).await
}

/// `pm:resolve:<event>` — host only: edit the board into the pick-a-winner view.
pub async fn handle_resolve(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(board) = host_board(ctx, cb, rest, lang).await? else {
        return Ok(());
    };
    let (chat, msg) = cb_coords(cb);
    let _ = tg::edit_with_buttons(ctx, chat, msg, &board_text(&board), &resolve_rows(&board)).await;
    answer(ctx, cb, "", false).await
}

/// `pm:back:<event>` — host only: back to the live board from the resolve view.
pub async fn handle_back(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    if host_board(ctx, cb, rest, lang).await?.is_none() {
        return Ok(());
    }
    if let Ok(id) = rest.parse::<i64>() {
        refresh_card(ctx, id).await;
    }
    answer(ctx, cb, "", false).await
}

/// `pm:win:<event>:<idx>` — host only: declare the winner (`resolve_event`).
pub async fn handle_win(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some((event_id, idx)) = parse2(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    let Some(board) = db(ctx).amm_board(event_id).ok().flatten() else {
        return answer(ctx, cb, i18n::bet_expired(lang), true).await;
    };
    if cb.from.id != board.host {
        return answer(ctx, cb, i18n::not_your_bet(lang), true).await;
    }
    let winner = board.options.get(idx as usize).map(|(n, _)| n.clone()).unwrap_or_default();
    match db(ctx).resolve_event(event_id, idx, now()) {
        Ok(true) => {
            refresh_card(ctx, event_id).await;
            answer(ctx, cb, &i18n::pm_resolved(lang, &winner), true).await
        }
        _ => answer(ctx, cb, "", false).await,
    }
}

/// `pm:void:<event>` — host only: void the event (refund cost basis on `/claim`).
pub async fn handle_void(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(board) = host_board(ctx, cb, rest, lang).await? else {
        return Ok(());
    };
    match db(ctx).void_event(board.event_id, now()) {
        Ok(true) => {
            refresh_card(ctx, board.event_id).await;
            answer(ctx, cb, i18n::pm_voided(lang), true).await
        }
        _ => answer(ctx, cb, "", false).await,
    }
}

/// Load the board for `rest` = `<event>` and verify the presser is the host.
/// Returns `Ok(None)` (after acking) when missing or not the host.
async fn host_board(
    ctx: &Context,
    cb: &CallbackQuery,
    rest: &str,
    lang: Lang,
) -> Result<Option<AmmBoard>, telexide::Error> {
    let Ok(event_id) = rest.parse::<i64>() else {
        answer(ctx, cb, "", false).await?;
        return Ok(None);
    };
    let Some(board) = db(ctx).amm_board(event_id).ok().flatten() else {
        answer(ctx, cb, i18n::bet_expired(lang), true).await?;
        return Ok(None);
    };
    if cb.from.id != board.host {
        answer(ctx, cb, i18n::not_your_bet(lang), true).await?;
        return Ok(None);
    }
    Ok(Some(board))
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn parse2(rest: &str) -> Option<(i64, i64)> {
    let mut it = rest.split(':');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

fn parse4(rest: &str) -> Option<(i64, i64, i64, i64)> {
    let mut it = rest.split(':');
    Some((
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
    ))
}

/// Render a freshly-created AMM event's board for its first post (used by the
/// `/predict` builder's `finalize`).
pub fn first_board(b: &AmmBoard) -> (String, Vec<tg::Row>) {
    (board_text(b), board_rows(b))
}
