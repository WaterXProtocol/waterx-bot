//! Real-money match betting: tap a match in `/matches` → a fresh odds quote
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
use crate::commands::tg;
use crate::commands::tg::answer;
use crate::commands::util::*;
use crate::database::{decimal_payout, COIN};
use crate::core::i18n::{self, Lang};
use crate::core::types::OddsFormat;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use telexide::model::CallbackQuery;
use telexide::prelude::*;

/// How long a quoted set of odds stays valid.
const QUOTE_TTL_SECS: i64 = 60;
/// Whole-coin stake presets — each button *adds* this much to the running stake.
const SIZE_PRESETS: [i64; 4] = [1, 5, 10, 50];

/// Callback-data prefixes (the `bet:` prefix lives in `markets`).
pub const OPT: &str = "opt:"; // pick a side → open the stake builder
pub const SIZE: &str = "sz:"; // re-render the builder at an accumulated total
pub const SIZE_PLACE: &str = "szp:"; // Confirm → debit + place the wager

/// A snapshot of one match's odds, locked for [`QUOTE_TTL_SECS`].
#[derive(Clone)]
pub struct Quote {
    market_id: String,
    slug: String,
    team_a: String,
    team_b: String,
    odds_a: Option<f64>,
    odds_draw: Option<f64>,
    odds_b: Option<f64>,
    ends_at: i64,
    quoted_at: i64,
    /// Chat the match was tapped in (the group brief) — the placed bet is
    /// announced back here. Equals the DM when `/markets` was used privately.
    origin_chat: i64,
    /// In a group, the message id of the posted prediction card (A vs B + side
    /// buttons) the placed bet is announced as a reply to. 0 when private.
    origin_msg: i64,
    /// The user this quote's stake board belongs to. In a group the board is a
    /// shared message anyone can *see*, so every builder/confirm/place step is
    /// locked to this owner (others get `not_your_bet`). Set at side-tap time.
    owner: i64,
}

impl Quote {
    fn odds(&self, outcome: &str) -> Option<f64> {
        match outcome {
            "teamA" => self.odds_a,
            "teamB" => self.odds_b,
            "draw" => self.odds_draw,
            _ => None,
        }
    }

    fn side_name(&self, lang: Lang, outcome: &str) -> String {
        match outcome {
            "teamA" => self.team_a.clone(),
            "teamB" => self.team_b.clone(),
            _ => i18n::draw_label(lang).to_string(),
        }
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

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// **Always** re-price `qid` from the current feed (cache-served, so at most
/// `markets::FEED_CACHE_TTL` old) and store the relocked quote, ignoring the
/// quote's own TTL. Every real-money placement goes through this so a wager is
/// booked at the latest odds, never a stale locked snapshot. Returns `None` if
/// the quote was evicted, the match ended, or the feed is unreachable (a feed
/// error also alerts the owner). The relocked quote keeps its
/// `origin_chat`/`origin_msg` so the announcement still lands.
async fn refetch_quote(ctx: &Context, lang: Lang, qid: u64) -> Option<Quote> {
    let q = quotes(ctx).lock().get(qid)?;
    let m = match markets::fetch_one(lang, &q.market_id).await {
        Ok(Some(m)) => m,
        Ok(None) => return None,
        Err(e) => {
            eprintln!("[bet] feed fetch error (reprice {}): {e}", q.market_id);
            notify_owner(ctx, &format!("match-bet reprice fetch failed ({}): {e}", q.market_id)).await;
            return None;
        }
    };
    if m.ends_at != 0 && now() >= m.ends_at {
        return None;
    }
    let renewed = Quote {
        odds_a: m.odds_a,
        odds_draw: m.odds_draw,
        odds_b: m.odds_b,
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

fn cb_lang(ctx: &Context, cb: &CallbackQuery) -> Lang {
    db(ctx)
        .get_lang(cb.from.id)
        .ok()
        .flatten()
        .unwrap_or_else(|| Lang::from_user(&cb.from))
}

/// Whether `uid` owns the stake board behind `qid`. `None` when the quote is gone
/// (the caller then falls through to its normal "expired" handling — there's no
/// owner to leak). `Some(false)` is an explicit "not your board" → reject.
fn quote_owner_ok(ctx: &Context, qid: u64, uid: i64) -> Option<bool> {
    quotes(ctx).lock().get(qid).map(|q| q.owner == uid)
}

/// `[outcome]` rows (one per priced outcome) labelled `name <odds>`. The callback
/// is `opt:<lang>:<fmt>:<market_id>:<outcome>` — it carries the **market id** (not
/// a quote id) so the card is stateless (a tap re-prices on demand, surviving
/// quote eviction / restart), plus the **locale and odds format the card was
/// created in** so every re-render stays in that one language + format (a shared
/// group card must not flip per tapper). `lang`/`fmt` store codes and the market
/// id (a UUID) are all colon-free, so it parses back cleanly.
fn option_rows(lang: Lang, q: &Quote, fmt: OddsFormat) -> Vec<tg::Row> {
    let mut rows = Vec::new();
    for outcome in ["teamA", "draw", "teamB"] {
        if let Some(c) = q.odds(outcome).filter(|c| *c > 0.0) {
            let label = format!("{} {}", q.side_name(lang, outcome), format_odds(c, fmt));
            rows.push(vec![(
                label,
                format!("{OPT}{}:{}:{}:{outcome}", lang.store_code(), fmt.store_code(), q.market_id),
            )]);
        }
    }
    rows
}

fn quote_text(lang: Lang, q: &Quote, fmt: OddsFormat) -> String {
    let mut s = format!("{} vs. {}\n", q.team_a, q.team_b);
    for outcome in ["teamA", "draw", "teamB"] {
        if let Some(c) = q.odds(outcome).filter(|c| *c > 0.0) {
            s.push_str(&format!("· {} — {}\n", q.side_name(lang, outcome), format_odds(c, fmt)));
        }
    }
    s.push('\n');
    s.push_str(i18n::bet_pick(lang));
    s
}

/// `bet:<market_id>` — render that match's card. In both group and private chats
/// the brief is replaced in place with the card, so the chat converges on one
/// focal message. The card is **stateless** — its side buttons carry the market
/// id, so a side tap re-prices on demand (`handle_opt`); nothing is stored in
/// `QuoteStore` here.
pub async fn handle_bet(
    ctx: &Context,
    cb: &CallbackQuery,
    market_id: &str,
) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let m = match markets::fetch_one(lang, market_id).await {
        Ok(Some(m)) => m,
        Ok(None) => {
            // Fetched fine but the match isn't listed — stale button, expected.
            eprintln!("[bet] market {market_id} not in feed (stale button)");
            return answer(ctx, cb, i18n::bet_unavailable(lang), true).await;
        }
        Err(e) => {
            // Genuine feed fetch/parse failure — alert the owner.
            eprintln!("[bet] feed fetch error for {market_id}: {e}");
            notify_owner(ctx, &format!("match-bet feed fetch failed ({market_id}): {e}")).await;
            return answer(ctx, cb, i18n::bet_unavailable(lang), true).await;
        }
    };
    if m.ends_at != 0 && now() >= m.ends_at {
        return answer(ctx, cb, i18n::bet_closed(lang), true).await;
    }

    // Build the card from the freshly-fetched odds. It's stateless: the side
    // buttons carry the market id (`option_rows`), so the brief becoming this card
    // needs no stored quote — a side tap re-prices in `handle_opt`, and the placed
    // bet is anchored/announced from the quote minted there. Replace the brief in
    // place (group and private alike) so the chat converges on one message.
    let q = Quote {
        market_id: m.market_id.clone(),
        slug: m.slug.clone(),
        team_a: m.team_a.clone(),
        team_b: m.team_b.clone(),
        odds_a: m.odds_a,
        odds_draw: m.odds_draw,
        odds_b: m.odds_b,
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
        &quote_text(lang, &q, fmt),
        &option_rows(lang, &q, fmt),
    )
    .await;
    Ok(())
}

/// `opt:<lang>:<market_id>:<outcome>` — a side was tapped on the match card.
/// **Always re-prices** from the live feed (cache-served, so ~free and
/// self-healing — works even after the prior quote was evicted or the bot
/// restarted), then opens the stake builder at 0. The **shared card stays in its
/// creator's locale** (`card_lang`, carried in the button) for both the feed
/// fetch and the render, so it never flips language per tapper — and when the odds
/// are unchanged (still in the cache window) the content is byte-identical, so
/// Telegram returns "not modified" (no flicker, concurrent taps idempotent). In a
/// **group** the builder is **DM'd** in the *tapper's* locale so it never clobbers
/// the card; in a **private** chat the card itself becomes the builder in place. A
/// gone/ended match turns the card into a "finished" notice; a transient feed
/// error keeps the card and alerts the owner.
pub async fn handle_opt(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let tapper_lang = cb_lang(ctx, cb);
    let tapper_fmt = db(ctx).get_odds_fmt(cb.from.id).unwrap_or_default();
    let Some((card_lang, card_fmt, market_id, outcome)) = parse_card_opt(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    // Fetch in the card's locale so its team names stay stable across tappers.
    let m = match markets::fetch_one(card_lang, &market_id).await {
        Ok(Some(m)) => m,
        Ok(None) => return finish_card(ctx, cb, card_lang).await,
        Err(e) => {
            eprintln!("[opt] feed fetch error for {market_id}: {e}");
            notify_owner(ctx, &format!("match-bet odds fetch failed ({market_id}): {e}")).await;
            return answer(ctx, cb, i18n::bet_unavailable(tapper_lang), true).await;
        }
    };
    if m.ends_at != 0 && now() >= m.ends_at {
        return finish_card(ctx, cb, card_lang).await;
    }
    let group = is_group_chat(cb.message_chat());
    let q = Quote {
        market_id: m.market_id.clone(),
        slug: m.slug.clone(),
        team_a: m.team_a.clone(),
        team_b: m.team_b.clone(),
        odds_a: m.odds_a,
        odds_draw: m.odds_draw,
        odds_b: m.odds_b,
        ends_at: m.ends_at,
        quoted_at: now(),
        origin_chat: cb.message_chat(),
        // Anchor the placed-bet announcement to the group card; a private origin
        // is the caller's own DM (no announcement), so leave it 0 there.
        origin_msg: if group { cb.message_id() } else { 0 },
        // The stake board is locked to whoever tapped the side.
        owner: cb.from.id,
    };
    // The tapped side must still be priced; if not, refresh the card (in its own
    // locale) to the live buttons and tell the tapper.
    if q.odds(&outcome).filter(|c| *c > 0.0).is_none() {
        if group {
            let _ = tg::edit_with_buttons(
                ctx,
                cb.message_chat(),
                cb.message_id(),
                &quote_text(card_lang, &q, card_fmt),
                &option_rows(card_lang, &q, card_fmt),
            )
            .await;
        }
        return answer(ctx, cb, i18n::bet_unavailable(tapper_lang), true).await;
    }
    let qid = quotes(ctx).lock().insert(q.clone());
    // Builder is per-user → render it in the tapper's locale + odds format, headed
    // by the tapper's name in a group so members can tell whose board it is.
    let Some((btext, brows)) =
        builder_text_rows(tapper_lang, &q, qid, &outcome, 0, tapper_fmt, &full_name(&cb.from))
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
            &quote_text(card_lang, &q, card_fmt),
            &option_rows(card_lang, &q, card_fmt),
        )
        .await;
        match tg::send_with_buttons_reply(ctx, cb.message_chat(), cb.message_id(), &btext, &brows).await {
            Ok(_) => answer(ctx, cb, "", false).await,
            Err(e) => {
                eprintln!("[opt] stake board post failed (chat {}): {e:?}", cb.message_chat());
                answer(ctx, cb, i18n::bet_unavailable(tapper_lang), true).await
            }
        }
    } else {
        let _ = tg::edit_with_buttons(ctx, cb.message_chat(), cb.message_id(), &btext, &brows).await;
        answer(ctx, cb, "", false).await
    }
}

/// `sz:<qid>:<outcome>:<total>` — re-render the builder at the accumulated total
/// (preset buttons add, `Clear` resets to 0, `Back` from confirm lands here).
pub async fn handle_size(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some((qid, outcome, total)) = parse_qid_outcome_total(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    if quote_owner_ok(ctx, qid, cb.from.id) == Some(false) {
        return answer(ctx, cb, i18n::not_your_bet(lang), true).await;
    }
    render_builder(ctx, cb, lang, qid, &outcome, total).await
}

/// `szp:<qid>:<outcome>:<total>` — fired by the builder's **Confirm**: the only
/// step that moves money. Re-checks freshness + balance, debits, records the
/// wager, and reports the result in a popup alert.
pub async fn handle_size_place(
    ctx: &Context,
    cb: &CallbackQuery,
    rest: &str,
) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some((qid, outcome, total)) = parse_qid_outcome_total(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    if quote_owner_ok(ctx, qid, cb.from.id) == Some(false) {
        return answer(ctx, cb, i18n::not_your_bet(lang), true).await;
    }
    // Guard the whole-coin → micro-coin conversion (caps at MAX_COINS, rejects
    // overflow and non-positive) so a crafted stake can't wrap i64 and mint coins.
    let Some(stake_units) = to_micro(total) else {
        return answer(ctx, cb, i18n::bad_stake(lang), true).await;
    };
    // Re-price every placement from the live feed so the wager is booked at the
    // current odds, never the locked snapshot the user was looking at.
    let Some(q) = refetch_quote(ctx, lang, qid).await else {
        return expire(ctx, cb, lang).await;
    };
    let Some(odds) = q.odds(&outcome).filter(|c| *c > 0.0) else {
        return answer(ctx, cb, "", false).await;
    };

    let database = db(ctx);
    // Atomic debit + record in one DB transaction: `false` = insufficient funds
    // (nothing written), `Err` = DB fault. There's no separate debit to roll
    // back, so the old swallowed-rollback path is gone entirely.
    match database.place_wager(
        cb.from.id,
        &q.market_id,
        &q.slug,
        &q.team_a,
        &q.team_b,
        &outcome,
        stake_units,
        odds,
        q.ends_at,
    ) {
        Ok(true) => {}
        Ok(false) => return answer(ctx, cb, i18n::not_enough_money(lang), true).await,
        Err(err) => {
            eprintln!("place_wager error: {err}");
            return answer(ctx, cb, i18n::db_error(lang), true).await;
        }
    }
    quotes(ctx).lock().remove(qid);

    let payout = decimal_payout(stake_units, odds);
    let side = q.side_name(lang, &outcome);
    // Confirmation + group announce show the odds in the bettor's chosen format.
    let odds_str = format_odds(odds, database.get_odds_fmt(cb.from.id).unwrap_or_default());
    let placed = i18n::bet_placed(lang, &fmt_coins(stake_units), &side, &odds_str, &fmt_coins(payout));
    if is_group_chat(q.origin_chat) {
        // Group: the stake board is its own message — delete it and post the
        // result as a reply to the prediction card it was bet on (falling back to a
        // loose message if that card is gone).
        let _ = tg::delete_message(ctx, cb.message_chat(), cb.message_id()).await;
        let announce =
            i18n::bet_announce(lang, &full_name(&cb.from), &fmt_coins(stake_units), &side, &odds_str);
        if q.origin_msg != 0 {
            let _ = tg::send_text_reply(ctx, q.origin_chat, q.origin_msg, &announce).await;
        } else {
            let _ = send_text(ctx, q.origin_chat, announce).await;
        }
    } else {
        // Private `/markets`: the board edits in place into the placed confirmation
        // (no prediction card to reply to — the origin is the caller's own DM).
        let _ = tg::edit_text_only(ctx, cb.message_chat(), cb.message_id(), &placed).await;
    }
    // Report the placed bet in a popup alert (Confirm places straight away now).
    answer(ctx, cb, &placed, true).await
}

/// Build the stake-builder screen for `total` coins on `outcome`: presets that
/// add, then a `[Confirm] [Clear]` row. `None` if the outcome isn't priced.
/// Shared by the in-place editor (`render_builder`) and the group board path. In
/// a group the text is prefixed with the owner's name (`owner_name`) so members
/// can tell whose owner-locked board it is.
fn builder_text_rows(
    lang: Lang,
    q: &Quote,
    qid: u64,
    outcome: &str,
    total: i64,
    fmt: OddsFormat,
    owner_name: &str,
) -> Option<(String, Vec<tg::Row>)> {
    let odds = q.odds(outcome).filter(|c| *c > 0.0)?;
    let side = q.side_name(lang, outcome);
    // Builder runs from 0 upward, so `to_micro` (which rejects 0) doesn't fit;
    // saturate the display conversion instead so a crafted `total` can't overflow.
    let stake_units = total.max(0).saturating_mul(COIN);
    let win = fmt_coins(decimal_payout(stake_units, odds));
    let text = board_header(
        q.origin_chat,
        owner_name,
        &i18n::bet_build(lang, &side, &format_odds(odds, fmt), &fmt_coins(stake_units), &win),
    );
    let add_row: tg::Row = SIZE_PRESETS
        .iter()
        .map(|p| (format!("+{p}"), format!("{SIZE}{qid}:{outcome}:{}", total.saturating_add(*p))))
        .collect();
    // Confirm places the bet straight away (re-pricing at place time) — no
    // separate confirmation screen. In a group the board is its own message, so
    // the owner's Dismiss rides on the same action row.
    let mut action_row = vec![
        (i18n::bet_btn_confirm(lang).to_string(), format!("{SIZE_PLACE}{qid}:{outcome}:{total}")),
        (i18n::bet_btn_clear(lang).to_string(), format!("{SIZE}{qid}:{outcome}:0")),
    ];
    if is_group_chat(q.origin_chat) {
        action_row.push((i18n::bet_btn_dismiss(lang).to_string(), format!("bx:{}", q.owner)));
    }
    Some((text, vec![add_row, action_row]))
}

/// Render the accumulate screen in place (edits the DM builder message).
async fn render_builder(
    ctx: &Context,
    cb: &CallbackQuery,
    lang: Lang,
    qid: u64,
    outcome: &str,
    total: i64,
) -> Result<(), telexide::Error> {
    let Some(q) = fresh_quote(ctx, lang, qid).await else {
        return expire(ctx, cb, lang).await;
    };
    let fmt = db(ctx).get_odds_fmt(cb.from.id).unwrap_or_default();
    // Owner-locked, so the presser is the owner — head the board with their name.
    let Some((text, rows)) =
        builder_text_rows(lang, &q, qid, outcome, total, fmt, &full_name(&cb.from))
    else {
        return answer(ctx, cb, "", false).await;
    };
    edit(ctx, cb, &text, &rows).await?;
    answer(ctx, cb, "", false).await
}

async fn expire(ctx: &Context, cb: &CallbackQuery, lang: Lang) -> Result<(), telexide::Error> {
    let _ = tg::edit_text_only(ctx, cb.message_chat(), cb.message_id(), i18n::bet_expired(lang)).await;
    answer(ctx, cb, "", false).await
}

/// Replace a group card with a "match finished" notice (no buttons) — used when
/// a refresh can't find the match anymore (it's over / settling).
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

async fn edit(
    ctx: &Context,
    cb: &CallbackQuery,
    text: &str,
    rows: &[tg::Row],
) -> Result<(), telexide::Error> {
    let _ = tg::edit_with_buttons(ctx, cb.message_chat(), cb.message_id(), text, rows).await;
    Ok(())
}

/// Parse `<lang>:<fmt>:<market_id>:<outcome>` from an `opt:` callback. `lang`/`fmt`
/// are the store codes the card was created in (pin the shared card's locale +
/// odds format so it can't flip per tapper); the market id is a colon-free UUID
/// and `outcome` is teamA/teamB/draw, so the three colons split cleanly. Unknown
/// lang → English, unknown fmt → Decimal.
fn parse_card_opt(rest: &str) -> Option<(Lang, OddsFormat, String, String)> {
    let mut it = rest.splitn(4, ':');
    let lang = Lang::from_store_code(it.next()?).unwrap_or(Lang::En);
    let fmt = OddsFormat::from_store_code(it.next()?);
    let market_id = it.next()?.to_string();
    let outcome = it.next()?.to_string();
    if market_id.is_empty() || outcome.is_empty() {
        return None;
    }
    Some((lang, fmt, market_id, outcome))
}

/// Parse `<qid>:<outcome>:<total>` (outcome is `teamA`/`teamB`/`draw`, no colon).
fn parse_qid_outcome_total(rest: &str) -> Option<(u64, String, i64)> {
    let parts: Vec<&str> = rest.split(':').collect();
    let [qid, outcome, total] = parts.as_slice() else {
        return None;
    };
    Some((qid.parse().ok()?, (*outcome).to_string(), total.parse().ok()?))
}

/// Small extension so the handlers can read the message coordinates concisely.
trait CbMessage {
    fn message_chat(&self) -> i64;
    fn message_id(&self) -> i64;
}
impl CbMessage for CallbackQuery {
    fn message_chat(&self) -> i64 {
        self.message.as_ref().map(|m| m.chat.get_id()).unwrap_or(0)
    }
    fn message_id(&self) -> i64 {
        self.message.as_ref().map(|m| m.message_id).unwrap_or(0)
    }
}
