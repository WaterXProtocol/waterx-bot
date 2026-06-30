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
use crate::database::{basis_for_sold, SellContext, TradeOutcome};
use telexide::model::CallbackQuery;
use telexide::prelude::*;

/// Quick-pick fractions of the holding (percent) the builder offers; each button
/// **sets** the sell amount to that fraction, and `Max` sells the whole holding.
const SELL_FRACTIONS: [i64; 3] = [25, 50, 75];

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
        Ok(TradeOutcome::Filled { shares, coins, basis, .. }) => {
            let sold = i18n::sold(lang, &fmt_coins(-shares), &outcome, &fmt_coins(coins));
            // Realized P&L on the sold shares = proceeds − their cost basis.
            let body = format!("{sold}\n{}", pnl_line(lang, coins - basis));
            let rows = vec![vec![(
                i18n::bet_btn_back(lang).to_string(),
                menu::MENU_BALANCE.to_string(),
            )]];
            edit(ctx, cb, &body, &rows).await;
            answer(ctx, cb, &body, true).await
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

/// Profit/loss line, realized (on confirm) or estimated (in the builder):
/// `📈 P&L: +5 🪙` (📉 loss / ➖ flat).
fn pnl_line(lang: Lang, pnl: i64) -> String {
    let emoji = if pnl > 0 {
        "📈"
    } else if pnl < 0 {
        "📉"
    } else {
        "➖"
    };
    i18n::sell_pnl(lang, emoji, &fmt_signed_coins(pnl))
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
    let base =
        i18n::sell_build(lang, &c.outcome, &fmt_coins(c.held), &fmt_coins(micro), &proceeds_str);
    // Estimated P&L for the selected amount at the current price = proceeds minus
    // the pro-rata cost basis of those shares (same `basis_for_sold` the sell
    // executes with, so the estimate matches the result). Slot it right under the
    // proceeds line, above the trailing "tap to add" hint (the template's last
    // line in every locale).
    let text = match (micro > 0).then_some(proceeds).flatten() {
        Some(p) => {
            let line = pnl_line(lang, p - basis_for_sold(c.basis, micro, c.held));
            match base.rsplit_once('\n') {
                Some((head, hint)) => format!("{head}\n{line}\n{hint}"),
                None => format!("{base}\n{line}"),
            }
        }
        None => base,
    };
    let preset_row: tg::Row = SELL_FRACTIONS
        .iter()
        .map(|pct| {
            // Each fraction button SETS the amount to that share of the holding.
            let target = (c.held as i128 * *pct as i128 / 100) as i64;
            (format!("{pct}%"), format!("{SELL_BUILD}{eid}:{idx}:{target}"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::COIN;

    fn ctx(held: i64, basis: i64) -> SellContext {
        SellContext {
            kind: "sourced".into(),
            source_ref: "k".into(),
            b_param: 0,
            fee_bps: 0,
            q_shares: vec![],
            held,
            basis,
            outcome: "France".into(),
            title: "t".into(),
        }
    }

    #[test]
    fn builder_shows_pnl_between_proceeds_and_hint() {
        // Hold 40 shares costing 20 coins; sell 20 at proceeds 16 → basis 10, P&L +6.
        let c = ctx(40 * COIN, 20 * COIN);
        let (text, rows) = build_screen(Lang::En, 1, 0, &c, 20 * COIN, Some(16 * COIN));
        let pnl = text.find("📈").expect("P&L line shown");
        let proceeds = text.find("16").expect("proceeds shown");
        let hint = text.find("Choose amount").expect("hint shown");
        assert!(proceeds < pnl, "P&L comes after the proceeds line");
        assert!(pnl < hint, "P&L comes before the choose-amount hint");
        assert!(text.contains("+6"), "shows the +6 profit");
        // The preset buttons are percentage-of-holding, ending in Max.
        let labels: Vec<&str> = rows[0].iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(labels, ["25%", "50%", "75%", "Max"]);
    }

    #[test]
    fn builder_hides_pnl_when_nothing_selected() {
        let c = ctx(40 * COIN, 20 * COIN);
        let (text, _) = build_screen(Lang::En, 1, 0, &c, 0, Some(0));
        assert!(!text.contains("📈") && !text.contains("📉") && !text.contains("➖"));
    }
}
