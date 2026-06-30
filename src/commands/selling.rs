//! Selling held YES shares back before settlement. From the private "Check
//! assets" view, `[💸 Sell]` (`slpick`) opens a picker of open holdings → pick
//! one → an **amount builder** (`slb:` presets accumulate a share count, with a
//! live proceeds preview at the current price) → **Confirm** (`slgo:`) sells via
//! the engine: `sourced_sell` at the live Gamma price, or `amm_sell` down the
//! LMSR curve. The sell amount rides in the callback data (no server-side state),
//! and every step re-derives from the held position, so a stale board can't
//! oversell.

use crate::commands::markets;
use crate::commands::menu;
use crate::commands::tg::{self, answer};
use crate::commands::util::*;
use crate::core::i18n::{self, Lang};
use crate::database::{SellContext, TradeOutcome, COIN};
use telexide::model::CallbackQuery;
use telexide::prelude::*;

/// Whole-share presets the builder adds (× `COIN` = micro-shares, since
/// `SHARE == COIN`); `Max` sells the whole holding.
const SELL_PRESETS: [i64; 3] = [10, 50, 100];

/// Callback prefixes — kept clear of the fruit `sell:` namespace.
pub const SELL_PICK: &str = "slpick"; // exact — open the holdings picker
pub const SELL_BUILD: &str = "slb:"; // slb:<event>:<idx>:<micro_shares>
pub const SELL_PLACE: &str = "slgo:"; // slgo:<event>:<idx>:<micro_shares>

fn cb_lang(ctx: &Context, cb: &CallbackQuery) -> Lang {
    db(ctx)
        .get_lang(cb.from.id)
        .ok()
        .flatten()
        .unwrap_or_else(|| Lang::from_user(&cb.from))
}

fn cb_coords(cb: &CallbackQuery) -> (i64, i64) {
    cb.message
        .as_ref()
        .map(|m| (m.chat.get_id(), m.message_id))
        .unwrap_or((0, 0))
}

async fn edit(ctx: &Context, cb: &CallbackQuery, text: &str, rows: &[tg::Row]) {
    let (chat, msg) = cb_coords(cb);
    let _ = tg::edit_with_buttons(ctx, chat, msg, text, rows).await;
}

/// Current YES price (cents, > 0) of a sourced event's outcome `idx`, or `None`
/// when the index isn't listed / unpriced. The held position's `market_idx` lines
/// up with the feed event's outcome order (the same order the bet was placed in).
fn outcome_price(ev: &markets::SourcedEvent, idx: i64) -> Option<f64> {
    let i = usize::try_from(idx).ok()?;
    ev.outcomes.get(i).and_then(|o| o.yes_cents).filter(|x| *x > 0.0)
}

/// `slpick` — edit the assets view into a picker: one button per open holding.
pub async fn handle_sell_pick(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let positions = db(ctx).user_positions(cb.from.id).unwrap_or_default();
    if positions.is_empty() {
        return answer(ctx, cb, i18n::no_open_bets(lang), true).await;
    }
    answer(ctx, cb, "", false).await?;
    let mut rows: Vec<tg::Row> = positions
        .iter()
        .map(|p| {
            vec![(
                format!("{} · {}", p.outcome, fmt_coins(p.shares)),
                format!("{SELL_BUILD}{}:{}:0", p.event_id, p.market_idx),
            )]
        })
        .collect();
    rows.push(vec![(i18n::bet_btn_back(lang).to_string(), menu::MENU_BALANCE.to_string())]);
    edit(ctx, cb, i18n::positions_title(lang), &rows).await;
    Ok(())
}

/// `slb:<event>:<idx>:<micro>` — render the amount builder with a live proceeds
/// preview (the amount is clamped to the held shares).
pub async fn handle_sell_build(
    ctx: &Context,
    cb: &CallbackQuery,
    rest: &str,
) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some((eid, idx, micro)) = parse3(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    let Some(c) = db(ctx).sell_context(eid, idx, cb.from.id).ok().flatten() else {
        return answer(ctx, cb, i18n::bet_expired(lang), true).await;
    };
    let micro = micro.clamp(0, c.held);
    let proceeds = quote_proceeds(ctx, lang, &c, eid, idx, micro).await;
    let (text, rows) = build_screen(lang, eid, idx, &c, micro, proceeds);
    edit(ctx, cb, &text, &rows).await;
    answer(ctx, cb, "", false).await
}

/// `slgo:<event>:<idx>:<micro>` — re-price and sell the shares via the engine.
pub async fn handle_sell_place(
    ctx: &Context,
    cb: &CallbackQuery,
    rest: &str,
) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some((eid, idx, micro)) = parse3(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    if micro <= 0 {
        return answer(ctx, cb, "", false).await;
    }
    let Some(c) = db(ctx).sell_context(eid, idx, cb.from.id).ok().flatten() else {
        return answer(ctx, cb, i18n::bet_expired(lang), true).await;
    };
    let sell = micro.min(c.held);
    let outcome = c.outcome.clone();
    let result = if c.kind == "amm" {
        db(ctx).amm_sell(eid, idx, cb.from.id, sell)
    } else {
        // Sourced: re-price from the live feed at place time (by the event key).
        let Some(m) = markets::fetch_one(lang, &c.source_ref).await.ok().flatten() else {
            return answer(ctx, cb, i18n::bet_unavailable(lang), true).await;
        };
        let Some(price) = outcome_price(&m, idx) else {
            return answer(ctx, cb, i18n::bet_unavailable(lang), true).await;
        };
        db(ctx).sourced_sell(eid, idx, cb.from.id, sell, price)
    };
    match result {
        Ok(TradeOutcome::Filled { shares, coins, .. }) => {
            let sold = i18n::sold(lang, &fmt_coins(-shares), &outcome, &fmt_coins(coins));
            let rows = vec![vec![(
                i18n::bet_btn_back(lang).to_string(),
                menu::MENU_BALANCE.to_string(),
            )]];
            edit(ctx, cb, &sold, &rows).await;
            answer(ctx, cb, &sold, true).await
        }
        Ok(_) => answer(ctx, cb, i18n::bet_unavailable(lang), true).await,
        Err(e) => {
            eprintln!("sell error (event {eid}, idx {idx}): {e}");
            answer(ctx, cb, i18n::db_error(lang), true).await
        }
    }
}

/// Read-only proceeds at the current price for selling `micro` shares — `None`
/// when the sourced feed can't be reached (the builder then shows "—").
async fn quote_proceeds(
    ctx: &Context,
    lang: Lang,
    c: &SellContext,
    eid: i64,
    idx: i64,
    micro: i64,
) -> Option<i64> {
    if micro <= 0 {
        return Some(0);
    }
    if c.kind == "amm" {
        db(ctx).amm_sell_quote(eid, idx, micro).ok().flatten()
    } else {
        let m = markets::fetch_one(lang, &c.source_ref).await.ok().flatten()?;
        let price = outcome_price(&m, idx)?;
        Some((micro as f64 * price / 100.0).floor() as i64)
    }
}

fn build_screen(
    lang: Lang,
    eid: i64,
    idx: i64,
    c: &SellContext,
    micro: i64,
    proceeds: Option<i64>,
) -> (String, Vec<tg::Row>) {
    let proceeds_str = proceeds.map(fmt_coins).unwrap_or_else(|| "—".to_string());
    let text = i18n::sell_build(lang, &c.outcome, &fmt_coins(c.held), &fmt_coins(micro), &proceeds_str);
    let preset_row: tg::Row = SELL_PRESETS
        .iter()
        .map(|p| {
            let next = micro.saturating_add(p.saturating_mul(COIN)).min(c.held);
            (format!("+{p}"), format!("{SELL_BUILD}{eid}:{idx}:{next}"))
        })
        .chain(std::iter::once(("Max".to_string(), format!("{SELL_BUILD}{eid}:{idx}:{}", c.held))))
        .collect();
    let action_row = vec![
        (i18n::bet_btn_confirm(lang).to_string(), format!("{SELL_PLACE}{eid}:{idx}:{micro}")),
        (i18n::bet_btn_clear(lang).to_string(), format!("{SELL_BUILD}{eid}:{idx}:0")),
    ];
    let back_row = vec![(i18n::bet_btn_back(lang).to_string(), SELL_PICK.to_string())];
    (text, vec![preset_row, action_row, back_row])
}

/// Parse `<event>:<idx>:<micro>` (all integers).
fn parse3(rest: &str) -> Option<(i64, i64, i64)> {
    let mut it = rest.split(':');
    let a = it.next()?.parse().ok()?;
    let b = it.next()?.parse().ok()?;
    let c = it.next()?.parse().ok()?;
    Some((a, b, c))
}
