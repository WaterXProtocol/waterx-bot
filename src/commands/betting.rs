//! Real-money match betting: tap a match in `/markets` → a fresh odds quote
//! (replacing the brief in place in a private chat, or DM'd from a group where
//! the brief is shared) → pick a side → **build a stake** (preset buttons add up; `Clear`
//! resets) → **Confirm** (a confirmation screen — the only step that debits the
//! balance and records the wager). The running total rides in the buttons'
//! callback data, so no per-user state is stored server-side. The displayed quote
//! is valid for [`QUOTE_TTL_SECS`] (the build/confirm flow auto-renews past that),
//! but the **place** step always re-prices from the live feed (`refetch_quote`)
//! so every wager is booked at current odds, not a locked snapshot. Wagers are
//! settled later by an admin (`/settle`).

use crate::bot::QuotesKey;
use crate::commands::markets;
use crate::commands::menu;
use crate::commands::tg;
use crate::commands::tg::{answer, CbMessage};
use crate::commands::util::*;
use crate::core::i18n::{self, Lang};
use crate::core::types::OddsFormat;
use crate::database::{decimal_payout, TradeOutcome, COIN};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use telexide::model::CallbackQuery;
use telexide::prelude::*;

/// How long a quoted set of odds stays valid.
const QUOTE_TTL_SECS: i64 = 10;
/// Callback-data prefixes (the `bet:` prefix lives in `markets`).
pub const OPT: &str = "opt:"; // pick a side → open the stake builder
pub const SIZE: &str = "sz:"; // re-render the builder at an accumulated total
pub const SIZE_PLACE: &str = "szp:"; // Confirm → debit + place the wager

/// A snapshot of one event's odds, locked for [`QUOTE_TTL_SECS`]. Generalized to
/// any number of outcomes — a bet carries the outcome **index**.
#[derive(Clone)]
pub struct Quote {
    /// Event key (a sport slug) — also the sourced event's `source_ref`, the join
    /// key for Gamma resolution. The quote is re-priced by re-fetching this key.
    key: String,
    /// Display title ("France vs. Sweden").
    title: String,
    /// Outcomes in the event's fixed order (`[teamA, draw?, teamB]` — 3 for a 1X2
    /// match, 2 for a draw-less esports match), matching `gamma_resolution`'s idx
    /// convention. Names are already localized by the feed.
    outcomes: Vec<markets::SourcedOutcome>,
    ends_at: i64,
    quoted_at: i64,
    /// Chat the event was tapped in (the group brief) — the placed bet is
    /// announced back here. Equals the DM when `/events` was used privately.
    origin_chat: i64,
    /// In a group, the message id of the posted prediction card the placed bet is
    /// announced as a reply to. 0 when private.
    origin_msg: i64,
    /// The user this quote's stake board belongs to. In a group the board is a
    /// shared message anyone can *see*, so every builder/confirm/place step is
    /// locked to this owner (others get `not_your_bet`). Set at side-tap time.
    owner: i64,
}

impl Quote {
    /// YES odds (cents) for outcome `idx`, if priced (> 0).
    fn yes(&self, idx: usize) -> Option<f64> {
        self.outcomes
            .get(idx)
            .and_then(|o| o.yes_cents)
            .filter(|c| *c > 0.0)
    }

    /// Display name of outcome `idx` (already localized by the feed).
    fn name(&self, idx: usize) -> String {
        self.outcomes.get(idx).map(|o| o.name.clone()).unwrap_or_default()
    }

    fn fresh(&self, now: i64) -> bool {
        now - self.quoted_at <= QUOTE_TTL_SECS
    }
}

/// In-memory, ephemeral store of live quotes keyed by a short id.
#[derive(Default)]
pub struct QuoteStore {
    next: u64,
    map: HashMap<u64, Quote>,
}

impl QuoteStore {
    fn insert(&mut self, q: Quote) -> u64 {
        // Drop anything well past its TTL so the map can't grow unbounded.
        let cutoff = q.quoted_at - 5 * QUOTE_TTL_SECS;
        self.map.retain(|_, v| v.quoted_at >= cutoff);
        self.next += 1;
        self.map.insert(self.next, q);
        self.next
    }
    fn get(&self, id: u64) -> Option<Quote> {
        self.map.get(&id).cloned()
    }
    fn replace(&mut self, id: u64, q: Quote) {
        self.map.insert(id, q);
    }
    fn remove(&mut self, id: u64) {
        self.map.remove(&id);
    }
}

fn quotes(ctx: &Context) -> Arc<Mutex<QuoteStore>> {
    ctx.data
        .read()
        .get::<QuotesKey>()
        .expect("QuotesKey missing")
        .clone()
}

/// **Always** re-price `qid` from the current feed (cache-served, so at most
/// `markets::FEED_CACHE_TTL` old) and store the relocked quote, ignoring the
/// quote's own TTL. Every real-money placement goes through this so a wager is
/// booked at the latest odds, never a stale locked snapshot. Returns `None` if
/// the quote was evicted, the market ended, or the feed is unreachable (a feed
/// error also alerts the owner). The relocked quote keeps its
/// `origin_chat`/`origin_msg` so the announcement still lands.
async fn refetch_quote(ctx: &Context, lang: Lang, qid: u64) -> Option<Quote> {
    let q = quotes(ctx).lock().get(qid)?;
    let m = match markets::fetch_one(lang, &q.key).await {
        Ok(Some(m)) => m,
        Ok(None) => return None,
        Err(e) => {
            alert_owner(ctx, &format!("[bet] reprice feed fetch failed ({}): {e}", q.key)).await;
            return None;
        }
    };
    if m.ends_at != 0 && now() >= m.ends_at {
        return None;
    }
    let renewed = Quote {
        outcomes: m.outcomes,
        ends_at: m.ends_at,
        quoted_at: now(),
        ..q
    };
    quotes(ctx).lock().replace(qid, renewed.clone());
    Some(renewed)
}

/// Fetch the quote for `qid`, **auto-renewing** its odds only if it has gone
/// stale (past `QUOTE_TTL_SECS`) — used by the private DM build/confirm flow (the
/// user's own message, so a silent renew is fine; the shared group card uses the
/// explicit Refresh button instead). The place step uses [`refetch_quote`]
/// directly so it always re-prices. `None` on the same terminal conditions.
async fn fresh_quote(ctx: &Context, lang: Lang, qid: u64) -> Option<Quote> {
    let q = quotes(ctx).lock().get(qid)?;
    if q.fresh(now()) {
        return Some(q);
    }
    refetch_quote(ctx, lang, qid).await
}

/// Whether `uid` owns the stake board behind `qid`. `None` when the quote is gone
/// (the caller then falls through to its normal "expired" handling — there's no
/// owner to leak). `Some(false)` is an explicit "not your board" → reject.
fn quote_owner_ok(ctx: &Context, qid: u64, uid: i64) -> Option<bool> {
    quotes(ctx).lock().get(qid).map(|q| q.owner == uid)
}

/// One row per priced outcome, labelled `name <odds>`. The callback is
/// `opt:<lang>:<fmt>:<key>:<idx>` — it carries the **event key** (not a quote id)
/// so the card is stateless (a tap re-prices on demand, surviving quote eviction /
/// restart) plus the **locale and odds format the card was created in** so every
/// re-render stays in that one language + format (a shared group card must not flip
/// per tapper). `lang`/`fmt` store codes, the key (a slug), and the numeric `idx`
/// are all colon-free, so it parses back cleanly.
fn option_rows(q: &Quote, lang: Lang, fmt: OddsFormat, is_group: bool) -> Vec<tg::Row> {
    let mut rows = Vec::new();
    for (idx, _) in q.outcomes.iter().enumerate() {
        if let Some(c) = q.yes(idx) {
            let label = format!("{} {}", q.name(idx), format_odds(c, fmt));
            rows.push(vec![(
                label,
                format!("{OPT}{}:{}:{}:{idx}", lang.store_code(), fmt.store_code(), q.key),
            )]);
        }
    }
    // Private only: the card replaced the brief in place, so offer a way back to
    // it (today's matches). In a **group** the card is a shared surface — a back
    // tap would yank everyone out of it — so it carries no back row.
    if !is_group {
        rows.push(vec![(
            i18n::bet_btn_back(lang).to_string(),
            menu::MENU_MARKETS.to_string(),
        )]);
    }
    rows
}

fn quote_text(q: &Quote, lang: Lang, fmt: OddsFormat) -> String {
    let mut s = format!("{}\n", q.title);
    for (idx, _) in q.outcomes.iter().enumerate() {
        if let Some(c) = q.yes(idx) {
            s.push_str(&format!("· {} — {}\n", q.name(idx), format_odds(c, fmt)));
        }
    }
    s.push('\n');
    s.push_str(i18n::bet_pick(lang));
    s
}

/// `bet:<key>` — render that event's card. In both group and private chats the
/// brief is replaced in place with the card, so the chat converges on one focal
/// message. The card is **stateless** — its outcome buttons carry the event key, so
/// a tap re-prices on demand (`handle_opt`); nothing is stored in `QuoteStore` here.
pub async fn handle_bet(ctx: &Context, cb: &CallbackQuery, key: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let m = match markets::fetch_one(lang, key).await {
        Ok(Some(m)) => m,
        Ok(None) => {
            // Fetched fine but the event isn't listed — stale button, expected.
            eprintln!("[bet] event {key} not in feed (stale button)");
            return answer(ctx, cb, i18n::bet_unavailable(lang), true).await;
        }
        Err(e) => {
            // Genuine feed fetch/parse failure — alert the owner.
            alert_owner(ctx, &format!("[bet] feed fetch failed ({key}): {e}")).await;
            return answer(ctx, cb, i18n::bet_unavailable(lang), true).await;
        }
    };
    if m.ends_at != 0 && now() >= m.ends_at {
        return answer(ctx, cb, i18n::bet_closed(lang), true).await;
    }

    // Build the card from the freshly-fetched odds. It's stateless: the outcome
    // buttons carry the event key (`option_rows`), so the brief becoming this card
    // needs no stored quote — a tap re-prices in `handle_opt`, and the placed bet is
    // anchored/announced from the quote minted there. Replace the brief in place
    // (group and private alike) so the chat converges on one message.
    let q = Quote {
        key: m.key.clone(),
        title: m.title.clone(),
        outcomes: m.outcomes.clone(),
        ends_at: m.ends_at,
        quoted_at: now(),
        origin_chat: 0,
        origin_msg: 0,
        owner: 0, // never stored (this quote only renders the card)
    };
    // Pin the card to the creator's odds format (carried in the buttons), so a
    // shared group card renders consistently for everyone, like its locale.
    let fmt = db(ctx).get_odds_fmt(cb.from.id).unwrap_or_default();
    answer(ctx, cb, "", false).await?;
    let _ = tg::edit_with_buttons(
        ctx,
        cb.message_chat(),
        cb.message_id(),
        &quote_text(&q, lang, fmt),
        &option_rows(&q, lang, fmt, is_group_chat(cb.message_chat())),
    )
    .await;
    Ok(())
}

/// `opt:<lang>:<fmt>:<key>:<idx>` — an outcome was tapped on the event card.
/// **Always re-prices** from the live feed (cache-served, so ~free and
/// self-healing — works even after the prior quote was evicted or the bot
/// restarted), then opens the stake builder at 0. The **shared card stays in its
/// creator's locale** (`card_lang`, carried in the button) for both the feed
/// fetch and the render, so it never flips language per tapper — and when the odds
/// are unchanged (still in the cache window) the content is byte-identical, so
/// Telegram returns "not modified" (no flicker, concurrent taps idempotent). In a
/// **group** the builder is **DM'd** in the *tapper's* locale so it never clobbers
/// the card; in a **private** chat the card itself becomes the builder in place. A
/// gone/ended event turns the card into a "finished" notice; a transient feed
/// error keeps the card and alerts the owner.
pub async fn handle_opt(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let tapper_lang = cb_lang(ctx, cb);
    let tapper_fmt = db(ctx).get_odds_fmt(cb.from.id).unwrap_or_default();
    let Some((card_lang, card_fmt, key, idx)) = parse_card_opt(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    // Fetch in the card's locale so its outcome names stay stable across tappers.
    let m = match markets::fetch_one(card_lang, &key).await {
        Ok(Some(m)) => m,
        Ok(None) => return finish_card(ctx, cb, card_lang).await,
        Err(e) => {
            alert_owner(ctx, &format!("[opt] odds feed fetch failed ({key}): {e}")).await;
            return answer(ctx, cb, i18n::bet_unavailable(tapper_lang), true).await;
        }
    };
    if m.ends_at != 0 && now() >= m.ends_at {
        return finish_card(ctx, cb, card_lang).await;
    }
    let group = is_group_chat(cb.message_chat());
    let q = Quote {
        key: m.key.clone(),
        title: m.title.clone(),
        outcomes: m.outcomes.clone(),
        ends_at: m.ends_at,
        quoted_at: now(),
        origin_chat: cb.message_chat(),
        // Anchor the placed-bet announcement to the group card; a private origin
        // is the caller's own DM (no announcement), so leave it 0 there.
        origin_msg: if group { cb.message_id() } else { 0 },
        // The stake board is locked to whoever tapped the outcome.
        owner: cb.from.id,
    };
    // The tapped outcome must still be priced; if not, refresh the card (in its own
    // locale) to the live buttons and tell the tapper.
    if q.yes(idx).is_none() {
        if group {
            let _ = tg::edit_with_buttons(
                ctx,
                cb.message_chat(),
                cb.message_id(),
                &quote_text(&q, card_lang, card_fmt),
                &option_rows(&q, card_lang, card_fmt, group),
            )
            .await;
        }
        return answer(ctx, cb, i18n::bet_unavailable(tapper_lang), true).await;
    }
    let qid = quotes(ctx).lock().insert(q.clone());
    // Builder is per-user → render it in the tapper's locale + odds format, headed
    // by the tapper's name in a group so members can tell whose board it is.
    let Some((btext, brows)) =
        builder_text_rows(tapper_lang, &q, qid, idx, 0, tapper_fmt, &full_name(&cb.from))
    else {
        return answer(ctx, cb, "", false).await;
    };
    if group {
        // (b) Refresh the shared card to current odds in its OWN locale (no-op when
        // unchanged), then post the tapper their OWN stake board as a reply to the
        // card — so the shared card stays a card (others can tap it for their own
        // boards) and each board is owner-locked + threaded under the prediction.
        let _ = tg::edit_with_buttons(
            ctx,
            cb.message_chat(),
            cb.message_id(),
            &quote_text(&q, card_lang, card_fmt),
            &option_rows(&q, card_lang, card_fmt, group),
        )
        .await;
        match tg::send_with_buttons_reply(ctx, cb.message_chat(), cb.message_id(), &btext, &brows).await {
            Ok(_) => answer(ctx, cb, "", false).await,
            Err(e) => {
                alert_owner(
                    ctx,
                    &format!(
                        "[opt] stake board post failed (chat {}): {e:?}",
                        cb.message_chat()
                    ),
                )
                .await;
                answer(ctx, cb, i18n::bet_unavailable(tapper_lang), true).await
            }
        }
    } else {
        let _ = tg::edit_with_buttons(ctx, cb.message_chat(), cb.message_id(), &btext, &brows).await;
        answer(ctx, cb, "", false).await
    }
}

/// `sz:<qid>:<idx>:<total>` — re-render the builder at the accumulated total
/// (preset buttons add, `Clear` resets to 0, `Back` from confirm lands here).
pub async fn handle_size(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some([qid, idx, total]) = parse_ints::<3>(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    let (qid, idx) = (qid as u64, idx as usize);
    if quote_owner_ok(ctx, qid, cb.from.id) == Some(false) {
        return answer(ctx, cb, i18n::not_your_bet(lang), true).await;
    }
    // Cap the running stake at the tapper's affordable whole coins so the builder
    // can never propose more than they hold. The Confirm debit guards it too, but
    // capping here keeps the shown number honest: going over lands on the cap with a
    // "not enough" toast, while reaching it exactly is silent.
    let cap = db(ctx)
        .get_user_info(cb.from.id)
        .map(|u| u.balance / COIN)
        .unwrap_or(0);
    let toast = (total > cap).then_some(i18n::not_enough_money(lang));
    render_builder(ctx, cb, lang, qid, idx, total.min(cap).max(0), toast).await
}

/// `szp:<qid>:<idx>:<total>` — fired by the builder's **Confirm**: the only
/// step that moves money. Re-prices from the live feed, then **buys YES shares**
/// of the picked outcome at that price (`spend → ⌊spend/price⌋ shares`,
/// house-banked) via the unified market engine. The sourced event is materialised
/// on first trade (keyed by the event key for later Gamma resolution). Because a
/// share settles to 1 coin, the "potential payout" shown is the share count — the
/// same number the old fixed-odds flow displayed — but the position can now also
/// be sold back before settlement.
pub async fn handle_size_place(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some([qid, idx, total]) = parse_ints::<3>(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    let (qid, idx) = (qid as u64, idx as usize);
    if quote_owner_ok(ctx, qid, cb.from.id) == Some(false) {
        return answer(ctx, cb, i18n::not_your_bet(lang), true).await;
    }
    // Guard the whole-coin → micro-coin conversion (caps at MAX_COINS, rejects
    // overflow and non-positive) so a crafted spend can't wrap i64 and mint coins.
    let Some(spend_units) = to_micro(total) else {
        return answer(ctx, cb, i18n::bad_stake(lang), true).await;
    };
    // Re-price from the live feed so shares are bought at the current price, never
    // the locked snapshot the user was looking at.
    let Some(q) = refetch_quote(ctx, lang, qid).await else {
        return expire(ctx, cb, lang).await;
    };
    let Some(price_cents) = q.yes(idx) else {
        return answer(ctx, cb, "", false).await;
    };

    let database = db(ctx);
    // Materialise the sourced event on first trade — keyed by the event key so the
    // auto-settle sweep (and `/settle`) can later resolve it against Polymarket.
    // Outcomes keep the feed's fixed order (sport: `[teamA, draw, teamB]`), matching
    // the card's idx.
    let outcomes: Vec<String> = q.outcomes.iter().map(|o| o.name.clone()).collect();
    let event_id = match database.get_or_create_sourced_event(
        &q.key,
        &q.title,
        lang.store_code(),
        "",
        None,
        q.ends_at,
        &outcomes,
        now(),
    ) {
        Ok(id) => id,
        Err(err) => {
            alert_owner(ctx, &format!("get_or_create_sourced_event error: {err}")).await;
            return answer(ctx, cb, i18n::db_error(lang), true).await;
        }
    };
    // Atomic debit + share credit in one transaction (house-banked).
    let shares = match database.sourced_buy(event_id, idx as i64, cb.from.id, spend_units, price_cents) {
        Ok(TradeOutcome::Filled { shares, .. }) => shares,
        Ok(TradeOutcome::Rejected) => return answer(ctx, cb, i18n::not_enough_money(lang), true).await,
        Ok(TradeOutcome::Unavailable) => return expire(ctx, cb, lang).await,
        Err(err) => {
            alert_owner(ctx, &format!("sourced_buy error: {err}")).await;
            return answer(ctx, cb, i18n::db_error(lang), true).await;
        }
    };
    quotes(ctx).lock().remove(qid);

    // 1 share settles to 1 coin, so the share count *is* the potential payout.
    let side = q.name(idx);
    let odds_str = format_odds(price_cents, database.get_odds_fmt(cb.from.id).unwrap_or_default());
    let placed = i18n::bet_placed(
        lang,
        &fmt_coins(spend_units),
        &side,
        &odds_str,
        &fmt_coins(shares),
    );
    if is_group_chat(q.origin_chat) {
        // Group: the stake board is its own message — delete it and post the
        // result as a reply to the event card it was bet on (falling back to a
        // loose message if that card is gone).
        let _ = tg::delete_message(ctx, cb.message_chat(), cb.message_id()).await;
        let announce = i18n::bet_announce(
            lang,
            &full_name(&cb.from),
            &fmt_coins(spend_units),
            &side,
            &odds_str,
        );
        if q.origin_msg != 0 {
            let _ = tg::send_text_reply(ctx, q.origin_chat, q.origin_msg, &announce).await;
        } else {
            let _ = send_text(ctx, q.origin_chat, announce).await;
        }
    } else {
        // Private `/events`: the board edits in place into the placed confirmation,
        // with a way back to the home menu (otherwise it dead-ends).
        let _ = tg::edit_with_buttons(
            ctx,
            cb.message_chat(),
            cb.message_id(),
            &placed,
            &menu::home_row(lang),
        )
        .await;
    }
    answer(ctx, cb, &placed, true).await
}

/// Build the stake-builder screen for `total` coins on outcome `idx`: presets that
/// add, then a `[Confirm] [Clear]` row. `None` if the outcome isn't priced. Shared
/// by the in-place editor (`render_builder`) and the group board path. In a group
/// the text is prefixed with the owner's name (`owner_name`) so members can tell
/// whose owner-locked board it is.
fn builder_text_rows(
    lang: Lang,
    q: &Quote,
    qid: u64,
    idx: usize,
    total: i64,
    fmt: OddsFormat,
    owner_name: &str,
) -> Option<(String, Vec<tg::Row>)> {
    let odds = q.yes(idx)?;
    let side = q.name(idx);
    // Builder runs from 0 upward, so `to_micro` (which rejects 0) doesn't fit;
    // saturate the display conversion instead so a crafted `total` can't overflow.
    let stake_units = total.max(0).saturating_mul(COIN);
    let win = fmt_coins(decimal_payout(stake_units, odds));
    let text = board_header(
        q.origin_chat,
        owner_name,
        &i18n::bet_build(
            lang,
            &side,
            &format_odds(odds, fmt),
            &fmt_coins(stake_units),
            &win,
        ),
    );
    let add_row: tg::Row = WHOLE_COIN_PRESETS
        .iter()
        .map(|p| {
            (
                format!("+{p}"),
                format!("{SIZE}{qid}:{idx}:{}", total.saturating_add(*p)),
            )
        })
        .collect();
    // Confirm places the bet straight away (re-pricing at place time) — no
    // separate confirmation screen. In a group the board is its own message, so
    // the owner's Dismiss rides on the same action row.
    let mut action_row = vec![
        (
            i18n::bet_btn_confirm(lang).to_string(),
            format!("{SIZE_PLACE}{qid}:{idx}:{total}"),
        ),
        (
            i18n::bet_btn_clear(lang).to_string(),
            format!("{SIZE}{qid}:{idx}:0"),
        ),
    ];
    if is_group_chat(q.origin_chat) {
        action_row.push((i18n::bet_btn_dismiss(lang).to_string(), format!("bx:{}", q.owner)));
    }
    let mut rows = vec![add_row, action_row];
    // Private: the builder replaced the card in place, so offer a way back to it
    // (re-render via `bet:<key>`). In a group the builder is its own board with a
    // Dismiss button, and the shared card is still there, so no back row is needed.
    if !is_group_chat(q.origin_chat) {
        rows.push(vec![(
            i18n::bet_btn_back(lang).to_string(),
            format!("{}{}", markets::BET, q.key),
        )]);
    }
    Some((text, rows))
}

/// Render the accumulate screen in place (edits the DM builder message). `toast`,
/// when set, is shown as an alert after the re-render (e.g. the stake was capped at
/// the tapper's balance).
async fn render_builder(
    ctx: &Context,
    cb: &CallbackQuery,
    lang: Lang,
    qid: u64,
    idx: usize,
    total: i64,
    toast: Option<&str>,
) -> Result<(), telexide::Error> {
    let Some(q) = fresh_quote(ctx, lang, qid).await else {
        return expire(ctx, cb, lang).await;
    };
    let fmt = db(ctx).get_odds_fmt(cb.from.id).unwrap_or_default();
    // Owner-locked, so the presser is the owner — head the board with their name.
    let Some((text, rows)) = builder_text_rows(lang, &q, qid, idx, total, fmt, &full_name(&cb.from)) else {
        return answer(ctx, cb, "", false).await;
    };
    tg::edit_cb(ctx, cb, &text, &rows).await;
    match toast {
        Some(t) => answer(ctx, cb, t, true).await,
        None => answer(ctx, cb, "", false).await,
    }
}

async fn expire(ctx: &Context, cb: &CallbackQuery, lang: Lang) -> Result<(), telexide::Error> {
    let _ = tg::edit_with_buttons(
        ctx,
        cb.message_chat(),
        cb.message_id(),
        i18n::bet_expired(lang),
        &menu::home_row(lang),
    )
    .await;
    answer(ctx, cb, "", false).await
}

/// Replace a group card with a "market finished" notice (no buttons) — used when
/// a refresh can't find the market anymore (it's over / settling).
async fn finish_card(ctx: &Context, cb: &CallbackQuery, lang: Lang) -> Result<(), telexide::Error> {
    let no_rows: &[tg::Row] = &[];
    let _ = tg::edit_with_buttons(
        ctx,
        cb.message_chat(),
        cb.message_id(),
        i18n::market_finished(lang),
        no_rows,
    )
    .await;
    answer(ctx, cb, "", false).await
}

/// Parse `<lang>:<fmt>:<key>:<idx>` from an `opt:` callback. `lang`/`fmt` are the
/// store codes the card was created in (pin the shared card's locale + odds format
/// so it can't flip per tapper); the key (a slug) is parsed off the front and the
/// trailing `idx` split from it on the **last** colon, so a key with a stray colon
/// still parses. Unknown lang → English, unknown fmt → Decimal.
fn parse_card_opt(rest: &str) -> Option<(Lang, OddsFormat, String, usize)> {
    let mut it = rest.splitn(3, ':');
    let lang = Lang::from_store_code(it.next()?).unwrap_or(Lang::En);
    let fmt = OddsFormat::from_store_code(it.next()?);
    let (key, idx) = it.next()?.rsplit_once(':')?;
    if key.is_empty() {
        return None;
    }
    Some((lang, fmt, key.to_string(), idx.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quote() -> Quote {
        Quote {
            key: "sport-x".into(),
            title: "A vs B".into(),
            outcomes: vec![
                markets::SourcedOutcome {
                    name: "A".into(),
                    yes_cents: Some(60.0),
                },
                markets::SourcedOutcome {
                    name: "B".into(),
                    yes_cents: Some(40.0),
                },
            ],
            ends_at: 0,
            quoted_at: 0,
            origin_chat: 0,
            origin_msg: 0,
            owner: 0,
        }
    }

    #[test]
    fn card_has_back_to_brief_only_in_private() {
        let q = quote();
        // Private: the two outcome rows + a back-to-brief row.
        let private = option_rows(&q, Lang::En, OddsFormat::Decimal, false);
        assert_eq!(private.len(), 3);
        assert_eq!(private.last().unwrap()[0].1.as_str(), menu::MENU_MARKETS);
        // Group: a shared card — outcome rows only, no back row.
        let group = option_rows(&q, Lang::En, OddsFormat::Decimal, true);
        assert_eq!(group.len(), 2);
        assert!(group.iter().all(|r| r[0].1.as_str() != menu::MENU_MARKETS));
    }
}
