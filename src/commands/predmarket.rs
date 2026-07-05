//! AMM prediction market — the host `/predict` event card.
//!
//! A shared board shows each outcome's **LMSR price** + buy buttons + a host
//! `[resolve]` control. Tapping an outcome opens the tapper's own **spend
//! builder** (owner-locked; presets accumulate in the callback data) which buys
//! YES shares via `amm_buy`; the board re-renders in place after each trade. The
//! host resolves by declaring the winning outcome (`resolve_event`) or voiding
//! (`void_event`); resolving/voiding **settles all positions immediately**
//! (`settle_event` — winners paid + LP residual returned), with the 5-min
//! auto-settle sweep as the backstop. All board state lives in the DB (no
//! in-memory map) — callbacks use the `pm:` namespace.

use crate::commands::menu;
use crate::commands::tg::{self, answer};
use crate::commands::util::*;
use crate::core::i18n::{self, Lang};
use crate::core::lmsr;
use crate::core::types::OddsFormat;
use crate::database::{AmmBoard, FundOutcome, TradeOutcome, COIN, MIN_SEED};
use telexide::model::CallbackQuery;
use telexide::prelude::*;

// Callback prefixes (all under `pm:`).
pub const PM_BUY: &str = "pm:buy:"; // pm:buy:<event>:<idx>
pub const PM_SIZE: &str = "pm:sz:"; // pm:sz:<event>:<idx>:<owner>:<spend>
pub const PM_PLACE: &str = "pm:go:"; // pm:go:<event>:<idx>:<owner>:<spend>
pub const PM_RESOLVE: &str = "pm:resolve:"; // pm:resolve:<event>
pub const PM_WIN: &str = "pm:win:"; // pm:win:<event>:<idx>
pub const PM_VOID: &str = "pm:void:"; // pm:void:<event>
pub const PM_BACK: &str = "pm:back:"; // pm:back:<event>
                                      // Funding stage (LP price discovery): pick an outcome → amount builder → add.
pub const PM_FUND: &str = "pm:fund:"; // pm:fund:<event>:<idx>
pub const PM_FSIZE: &str = "pm:fsz:"; // pm:fsz:<event>:<idx>:<owner>:<amount>
pub const PM_FPLACE: &str = "pm:fgo:"; // pm:fgo:<event>:<idx>:<owner>:<amount>

fn circled(n: usize) -> String {
    const C: [&str; 10] = ["①", "②", "③", "④", "⑤", "⑥", "⑦", "⑧", "⑨", "⑩"];
    C.get(n - 1).map_or_else(|| format!("{n}."), |c| (*c).to_string())
}

/// Each outcome's YES price in cents from the LMSR curve (`SHARE == COIN`).
fn prices_cents(b: &AmmBoard) -> Vec<f64> {
    let q: Vec<f64> = b.options.iter().map(|(_, q)| *q as f64 / COIN as f64).collect();
    (0..b.options.len())
        .map(|i| lmsr::price(&q, b.b_param as f64, i) * 100.0)
        .collect()
}

/// Implied opening prices (cents) from the funding tally `pₖ = fundedₖ / Σ`.
fn funding_cents(b: &AmmBoard) -> Vec<f64> {
    crate::core::lmsr_fund::opening_prices(&b.funded)
        .iter()
        .map(|p| p * 100.0)
        .collect()
}

/// The funding-stage board: question + opening tally (each outcome's implied price
/// + coins funded) + the pool status. Funding buttons live in `funding_board_rows`.
fn funding_text(b: &AmmBoard) -> String {
    let lang = Lang::from_store_code(&b.lang).unwrap_or(Lang::En);
    let fmt = OddsFormat::from_store_code(&b.odds_fmt);
    let mut s = format!("🎲 {}  ·  🌱\n", b.question);
    let header = if b.open_at > now() {
        fmt_local_time(b.open_at, b.tz_offset).map(|t| i18n::funding_until(lang, &t))
    } else {
        None
    };
    s.push_str(&header.unwrap_or_else(|| i18n::funding_open_now(lang).to_string()));
    s.push('\n');
    let cents = funding_cents(b);
    for (i, (name, _)) in b.options.iter().enumerate() {
        s.push_str(&format!(
            "\n{} {name}   {}   {}🪙",
            circled(i + 1),
            format_odds(cents.get(i).copied().unwrap_or(0.0), fmt),
            fmt_coins(b.funded.get(i).copied().unwrap_or(0))
        ));
    }
    s.push_str(&format!("\n\n{}", i18n::funding_pool(lang, &fmt_coins(b.pool))));
    if b.pool >= MIN_SEED.saturating_mul(COIN) {
        s.push_str(&format!("  ·  {}", i18n::funding_ready(lang)));
    } else {
        s.push_str(&format!(
            "  ·  {}",
            i18n::funding_need(lang, &MIN_SEED.to_string())
        ));
    }
    s
}

/// Funding-board buttons: one `[💧 <name>]` per outcome (3/row) + the host's
/// `[✖ Cancel]` (void). Shared message — any user can fund.
fn funding_board_rows(b: &AmmBoard) -> Vec<tg::Row> {
    let lang = Lang::from_store_code(&b.lang).unwrap_or(Lang::En);
    let mut rows: Vec<tg::Row> = b
        .options
        .iter()
        .enumerate()
        .map(|(i, (name, _))| (format!("💧 {name}"), format!("{PM_FUND}{}:{i}", b.event_id)))
        .collect::<Vec<_>>()
        .chunks(3)
        .map(<[_]>::to_vec)
        .collect();
    rows.push(vec![(
        i18n::void_label(lang).to_string(),
        format!("{PM_VOID}{}", b.event_id),
    )]);
    rows
}

/// The board text (host's pinned lang + odds format — a shared message).
fn board_text(b: &AmmBoard) -> String {
    if b.state == "funding" {
        return funding_text(b);
    }
    let lang = Lang::from_store_code(&b.lang).unwrap_or(Lang::En);
    let fmt = OddsFormat::from_store_code(&b.odds_fmt);
    // `resolved`/`void` become `closed` the instant auto-settle pays everyone out,
    // so render `closed` by its outcome (winner set → ✅, else a void → ↩️) rather
    // than a bare 🔴 — the card reads the same before and after settlement.
    let emoji = match b.state.as_str() {
        "open" => "🟢",
        "resolved" => "✅",
        "void" => "↩️",
        "closed" if b.winning_market.is_some() => "✅",
        "closed" => "↩️",
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
        s.push_str(&format!(
            "\n{} {name}   {}",
            circled(i + 1),
            format_odds(cents[i], fmt)
        ));
    }
    s.push_str(&format!(
        "\n\n{}",
        i18n::board_footer_open(lang, &fmt_coins(b.pool))
    ));
    // Terminal result line — shown for resolved/void **and** their settled
    // `closed` form, so an auto-settled AMM card still shows who won / that it
    // voided (a `closed` event with a `winning_market` is a settled win; without,
    // a settled void).
    let winner = b
        .winning_market
        .and_then(|i| b.options.get(i as usize))
        .map(|(n, _)| n.clone());
    match (b.state.as_str(), &winner) {
        ("resolved" | "closed", Some(w)) => s.push_str(&format!("\n\n{}", i18n::pm_resolved(lang, w))),
        ("void" | "closed", _) => s.push_str(&format!("\n\n{}", i18n::pm_voided(lang))),
        _ => {}
    }
    s
}

/// The live board's buttons: a buy button per outcome (3/row) + a host
/// `[resolve]` row. Empty once the event is no longer open.
fn board_rows(b: &AmmBoard) -> Vec<tg::Row> {
    if b.state == "funding" {
        return funding_board_rows(b);
    }
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
    rows.push(vec![(
        i18n::close_button(lang).to_string(),
        format!("{PM_RESOLVE}{}", b.event_id),
    )]);
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
        (
            i18n::void_label(lang).to_string(),
            format!("{PM_VOID}{}", b.event_id),
        ),
        (
            i18n::bet_btn_back(lang).to_string(),
            format!("{PM_BACK}{}", b.event_id),
        ),
    ]);
    rows
}

/// Re-render the board card in place from the live DB state. A funding card whose
/// window has closed flips to the open market here (first interaction wins).
async fn refresh_card(ctx: &Context, event_id: i64) {
    let _ = db(ctx).finalize_funding_if_due(event_id);
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
    let shares = db(ctx)
        .amm_buy_quote(b.event_id, idx, spend_micro)
        .ok()
        .flatten()
        .unwrap_or(0);
    let body = i18n::bet_build(
        lang,
        name,
        &format_odds(price, fmt),
        &fmt_coins(spend_micro),
        &fmt_coins(shares),
    );
    let text = board_header(chat, owner_name, &body);
    let preset_row: tg::Row = WHOLE_COIN_PRESETS
        .iter()
        .map(|p| {
            (
                format!("+{p}"),
                format!(
                    "{PM_SIZE}{}:{idx}:{owner}:{}",
                    b.event_id,
                    spend_whole.saturating_add(*p)
                ),
            )
        })
        .collect();
    let action_row = vec![
        (
            i18n::bet_btn_confirm(lang).to_string(),
            format!("{PM_PLACE}{}:{idx}:{owner}:{spend_whole}", b.event_id),
        ),
        (
            i18n::bet_btn_clear(lang).to_string(),
            format!("{PM_SIZE}{}:{idx}:{owner}:0", b.event_id),
        ),
    ];
    let mut rows = vec![preset_row, action_row];
    if is_group_chat(chat) {
        rows.push(vec![(
            i18n::bet_btn_dismiss(lang).to_string(),
            format!("bx:{owner}"),
        )]);
    }
    (text, rows)
}

/// `pm:buy:<event>:<idx>` — open the tapper's spend builder (a reply to the card
/// in a group; a fresh DM message privately). Owner-locked to the tapper.
pub async fn handle_buy(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some([event_id, idx]) = parse_ints::<2>(rest) else {
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
    let (chat, msg) = tg::cb_coords(cb);
    let fmt = db(ctx).get_odds_fmt(cb.from.id).unwrap_or_default();
    let (text, rows) = builder(
        ctx,
        lang,
        fmt,
        &board,
        idx,
        cb.from.id,
        0,
        chat,
        &full_name(&cb.from),
    );
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
    let Some([event_id, idx, owner, spend]) = parse_ints::<4>(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    if owner != cb.from.id {
        return answer(ctx, cb, i18n::not_your_bet(lang), true).await;
    }
    let Some(board) = db(ctx).amm_board(event_id).ok().flatten() else {
        return answer(ctx, cb, i18n::bet_expired(lang), true).await;
    };
    // Cap the running spend at the tapper's affordable whole coins (the `amm_buy`
    // debit guards it too, but this keeps the shown number honest): going over lands
    // on the cap with a "not enough" toast, reaching it exactly is silent.
    let cap = db(ctx)
        .get_user_info(cb.from.id)
        .map(|u| u.balance / COIN)
        .unwrap_or(0);
    let over = spend > cap;
    let (chat, msg) = tg::cb_coords(cb);
    let fmt = db(ctx).get_odds_fmt(cb.from.id).unwrap_or_default();
    let (text, rows) = builder(
        ctx,
        lang,
        fmt,
        &board,
        idx,
        owner,
        spend.min(cap).max(0),
        chat,
        &full_name(&cb.from),
    );
    let _ = tg::edit_with_buttons(ctx, chat, msg, &text, &rows).await;
    if over {
        answer(ctx, cb, i18n::not_enough_money(lang), true).await
    } else {
        answer(ctx, cb, "", false).await
    }
}

/// `pm:go:<event>:<idx>:<owner>:<spend>` — buy YES shares via `amm_buy`, refresh
/// the board, and announce.
pub async fn handle_place(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some([event_id, idx, owner, spend_whole]) = parse_ints::<4>(rest) else {
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
            alert_owner(ctx, &format!("amm_buy error (event {event_id}): {e}")).await;
            return answer(ctx, cb, i18n::db_error(lang), true).await;
        }
    };
    refresh_card(ctx, event_id).await;
    let odds_str = format_odds(price, fmt);
    let placed = i18n::bet_placed(lang, &fmt_coins(spend), &name, &odds_str, &fmt_coins(shares));
    let (chat, msg) = tg::cb_coords(cb);
    if is_group_chat(chat) {
        let _ = tg::delete_message(ctx, chat, msg).await;
        let announce = i18n::bet_announce(lang, &full_name(&cb.from), &fmt_coins(spend), &name, &odds_str);
        if let Ok(Some((cchat, cmsg))) = db(ctx).event_card(event_id) {
            let _ = tg::send_text_reply(ctx, cchat, cmsg, &announce).await;
        } else {
            let _ = send_text(ctx, chat, announce).await;
        }
    } else {
        let _ = tg::edit_with_buttons(ctx, chat, msg, &placed, &menu::home_row(lang)).await;
    }
    answer(ctx, cb, &placed, true).await
}

// --- Funding stage: LP liquidity provision ---------------------------------

/// The per-LP funding amount builder for outcome `idx` (owner-locked). Previews
/// the opening price this allocation would set.
#[allow(clippy::too_many_arguments)]
fn fund_builder(
    lang: Lang,
    fmt: OddsFormat,
    b: &AmmBoard,
    idx: i64,
    owner: i64,
    amount_whole: i64,
    chat: i64,
    owner_name: &str,
) -> (String, Vec<tg::Row>) {
    let name = b.options.get(idx as usize).map(|(n, _)| n.as_str()).unwrap_or("");
    let amount_micro = amount_whole.max(0).saturating_mul(COIN);
    // Implied opening price *after* this allocation (price discovery preview).
    let mut funded = b.funded.clone();
    if let Some(f) = funded.get_mut(idx as usize) {
        *f = f.saturating_add(amount_micro);
    }
    let cents = crate::core::lmsr_fund::opening_prices(&funded)
        .get(idx as usize)
        .copied()
        .unwrap_or(0.0)
        * 100.0;
    let body = i18n::fund_build(lang, name, &fmt_coins(amount_micro), &format_odds(cents, fmt));
    let text = board_header(chat, owner_name, &body);
    let preset_row: tg::Row = WHOLE_COIN_PRESETS
        .iter()
        .map(|p| {
            (
                format!("+{p}"),
                format!(
                    "{PM_FSIZE}{}:{idx}:{owner}:{}",
                    b.event_id,
                    amount_whole.saturating_add(*p)
                ),
            )
        })
        .collect();
    let action_row = vec![
        (
            i18n::bet_btn_confirm(lang).to_string(),
            format!("{PM_FPLACE}{}:{idx}:{owner}:{amount_whole}", b.event_id),
        ),
        (
            i18n::bet_btn_clear(lang).to_string(),
            format!("{PM_FSIZE}{}:{idx}:{owner}:0", b.event_id),
        ),
    ];
    let mut rows = vec![preset_row, action_row];
    if is_group_chat(chat) {
        rows.push(vec![(
            i18n::bet_btn_dismiss(lang).to_string(),
            format!("bx:{owner}"),
        )]);
    }
    (text, rows)
}

/// `pm:fund:<event>:<idx>` — open the tapper's funding builder for outcome `idx`.
pub async fn handle_fund(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some([event_id, idx]) = parse_ints::<2>(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    // If the window just closed, open the market and bounce the tapper to trading.
    let _ = db(ctx).finalize_funding_if_due(event_id);
    let Some(board) = db(ctx).amm_board(event_id).ok().flatten() else {
        return answer(ctx, cb, i18n::bet_expired(lang), true).await;
    };
    if board.state != "funding" {
        refresh_card(ctx, event_id).await;
        return answer(ctx, cb, i18n::fund_closed(lang), true).await;
    }
    if idx < 0 || idx as usize >= board.options.len() {
        return answer(ctx, cb, "", false).await;
    }
    let (chat, msg) = tg::cb_coords(cb);
    let fmt = db(ctx).get_odds_fmt(cb.from.id).unwrap_or_default();
    let (text, rows) = fund_builder(lang, fmt, &board, idx, cb.from.id, 0, chat, &full_name(&cb.from));
    answer(ctx, cb, "", false).await?;
    if is_group_chat(chat) {
        let _ = tg::send_with_buttons_reply(ctx, chat, msg, &text, &rows).await;
    } else {
        let _ = tg::send_with_buttons(ctx, chat, &text, &rows).await;
    }
    Ok(())
}

/// `pm:fsz:<event>:<idx>:<owner>:<amount>` — re-render the funding builder.
pub async fn handle_fund_size(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some([event_id, idx, owner, amount]) = parse_ints::<4>(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    if owner != cb.from.id {
        return answer(ctx, cb, i18n::not_your_bet(lang), true).await;
    }
    let Some(board) = db(ctx).amm_board(event_id).ok().flatten() else {
        return answer(ctx, cb, i18n::bet_expired(lang), true).await;
    };
    // Cap the LP allocation at the tapper's affordable whole coins (the
    // `add_liquidity` debit guards it too): going over lands on the cap with a "not
    // enough" toast, reaching it exactly is silent.
    let cap = db(ctx)
        .get_user_info(cb.from.id)
        .map(|u| u.balance / COIN)
        .unwrap_or(0);
    let over = amount > cap;
    let (chat, msg) = tg::cb_coords(cb);
    let fmt = db(ctx).get_odds_fmt(cb.from.id).unwrap_or_default();
    let (text, rows) = fund_builder(
        lang,
        fmt,
        &board,
        idx,
        owner,
        amount.min(cap).max(0),
        chat,
        &full_name(&cb.from),
    );
    let _ = tg::edit_with_buttons(ctx, chat, msg, &text, &rows).await;
    if over {
        answer(ctx, cb, i18n::not_enough_money(lang), true).await
    } else {
        answer(ctx, cb, "", false).await
    }
}

/// `pm:fgo:<event>:<idx>:<owner>:<amount>` — provide liquidity via `add_liquidity`,
/// refresh the board, and announce.
pub async fn handle_fund_place(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some([event_id, idx, owner, amount_whole]) = parse_ints::<4>(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    if owner != cb.from.id {
        return answer(ctx, cb, i18n::not_your_bet(lang), true).await;
    }
    let Some(amount) = to_micro(amount_whole) else {
        return answer(ctx, cb, i18n::bad_stake(lang), true).await;
    };
    let Some(board) = db(ctx).amm_board(event_id).ok().flatten() else {
        return answer(ctx, cb, i18n::bet_expired(lang), true).await;
    };
    if idx < 0 || idx as usize >= board.options.len() {
        return answer(ctx, cb, "", false).await;
    }
    let name = board
        .options
        .get(idx as usize)
        .map(|(n, _)| n.clone())
        .unwrap_or_default();
    let mut alloc = vec![0i64; board.options.len()];
    alloc[idx as usize] = amount;
    match db(ctx).add_liquidity(event_id, cb.from.id, &alloc, now()) {
        Ok(FundOutcome::Funded { total }) => {
            refresh_card(ctx, event_id).await;
            let done = i18n::fund_done(lang, &name, &fmt_coins(total));
            let (chat, msg) = tg::cb_coords(cb);
            if is_group_chat(chat) {
                let _ = tg::delete_message(ctx, chat, msg).await;
                let announce = i18n::fund_announce(lang, &full_name(&cb.from), &fmt_coins(total), &name);
                if let Ok(Some((cchat, cmsg))) = db(ctx).event_card(event_id) {
                    let _ = tg::send_text_reply(ctx, cchat, cmsg, &announce).await;
                } else {
                    let _ = send_text(ctx, chat, announce).await;
                }
            } else {
                let _ = tg::edit_with_buttons(ctx, chat, msg, &done, &menu::home_row(lang)).await;
            }
            answer(ctx, cb, &done, true).await
        }
        Ok(FundOutcome::Rejected) => answer(ctx, cb, i18n::not_enough_money(lang), true).await,
        Ok(FundOutcome::Unavailable) => {
            refresh_card(ctx, event_id).await;
            answer(ctx, cb, i18n::fund_closed(lang), true).await
        }
        Err(e) => {
            alert_owner(ctx, &format!("add_liquidity error (event {event_id}): {e}")).await;
            answer(ctx, cb, i18n::db_error(lang), true).await
        }
    }
}

/// `pm:resolve:<event>` — host only: edit the board into the pick-a-winner view.
pub async fn handle_resolve(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(board) = host_board(ctx, cb, rest, lang).await? else {
        return Ok(());
    };
    let (chat, msg) = tg::cb_coords(cb);
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
    let Some([event_id, idx]) = parse_ints::<2>(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    let Some(board) = db(ctx).amm_board(event_id).ok().flatten() else {
        return answer(ctx, cb, i18n::bet_expired(lang), true).await;
    };
    if cb.from.id != board.host {
        return answer(ctx, cb, i18n::not_your_bet(lang), true).await;
    }
    let winner = board
        .options
        .get(idx as usize)
        .map(|(n, _)| n.clone())
        .unwrap_or_default();
    match db(ctx).resolve_event(event_id, idx, now()) {
        Ok(true) => {
            // Pay all winners + return the LP residual immediately (no manual
            // /claim), then DM each winner. The 5-min auto-settle task is the
            // backstop if the settle errors.
            match db(ctx).settle_event(event_id, None) {
                Ok(payouts) => {
                    crate::commands::settle::notify_settled(&bot_token(ctx), &db(ctx), &payouts).await
                }
                Err(e) => {
                    alert_owner(
                        ctx,
                        &format!("[predict] settle on resolve failed (event {event_id}): {e}"),
                    )
                    .await
                }
            }
            refresh_card(ctx, event_id).await;
            answer(ctx, cb, &i18n::pm_resolved(lang, &winner), true).await
        }
        _ => answer(ctx, cb, "", false).await,
    }
}

/// `pm:void:<event>` — host only: void the event (refunds every trader's cost
/// basis + returns the seed to LPs immediately).
pub async fn handle_void(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(board) = host_board(ctx, cb, rest, lang).await? else {
        return Ok(());
    };
    match db(ctx).void_event(board.event_id, now()) {
        Ok(true) => {
            // Refund every trader's cost basis + return the seed to LPs immediately,
            // then DM each refundee.
            match db(ctx).settle_event(board.event_id, None) {
                Ok(payouts) => {
                    crate::commands::settle::notify_settled(&bot_token(ctx), &db(ctx), &payouts).await
                }
                Err(e) => {
                    alert_owner(
                        ctx,
                        &format!("[predict] settle on void failed (event {}): {e}", board.event_id),
                    )
                    .await
                }
            }
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

/// Render a freshly-created AMM event's board for its first post (used by the
/// `/predict` builder's `finalize`).
pub fn first_board(b: &AmmBoard) -> (String, Vec<tg::Row>) {
    (board_text(b), board_rows(b))
}
