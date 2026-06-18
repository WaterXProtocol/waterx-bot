//! Real-money match betting: tap a match in `/matches` → a fresh odds quote
//! (replacing the brief in place in a private chat, or DM'd from a group where
//! the brief is shared) → pick a side → **build a stake** (preset buttons add up; `Clear`
//! resets) → **Confirm** (a confirmation screen — the only step that debits the
//! balance and records the wager). The running total rides in the buttons'
//! callback data, so no per-user state is stored server-side. The quote is valid
//! for [`QUOTE_TTL_SECS`]; after that the user must re-open `/matches` because the
//! odds move. Wagers are settled later by an admin (`/settle`).

use crate::bot::QuotesKey;
use crate::commands::markets;
use crate::commands::tg;
use crate::commands::util::*;
use crate::database::{decimal_payout, COIN};
use crate::i18n::{self, Lang};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use telexide::api::types::AnswerCallbackQuery;
use telexide::model::CallbackQuery;
use telexide::prelude::*;

/// How long a quoted set of odds stays valid.
const QUOTE_TTL_SECS: i64 = 60;
/// Whole-coin stake presets — each button *adds* this much to the running stake.
const SIZE_PRESETS: [i64; 4] = [1, 5, 10, 50];

/// Callback-data prefixes (the `bet:` prefix lives in `markets`).
pub const OPT: &str = "opt:"; // pick a side → open the stake builder
pub const SIZE: &str = "sz:"; // re-render the builder at an accumulated total
pub const SIZE_CONFIRM: &str = "szc:"; // builder → confirmation screen
pub const SIZE_PLACE: &str = "szp:"; // confirmation → debit + place the wager

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

/// `[outcome]` rows (one per priced outcome) labelled `name 1.54`.
fn option_rows(lang: Lang, q: &Quote, qid: u64) -> Vec<tg::Row> {
    let mut rows = Vec::new();
    for outcome in ["teamA", "draw", "teamB"] {
        if let Some(c) = q.odds(outcome).filter(|c| *c > 0.0) {
            let label = format!("{} {:.2}", q.side_name(lang, outcome), decimal_odds(c));
            rows.push(vec![(label, format!("{OPT}{qid}:{outcome}"))]);
        }
    }
    rows
}

fn quote_text(lang: Lang, q: &Quote) -> String {
    let mut s = format!("{} vs. {}\n", q.team_a, q.team_b);
    for outcome in ["teamA", "draw", "teamB"] {
        if let Some(c) = q.odds(outcome).filter(|c| *c > 0.0) {
            s.push_str(&format!("· {} — {:.2}\n", q.side_name(lang, outcome), decimal_odds(c)));
        }
    }
    s.push('\n');
    s.push_str(i18n::bet_pick(lang));
    s
}

/// `bet:<market_id>` — open a fresh quote for that match. In a private chat the
/// brief is replaced in place; in a group it's DM'd (the brief is shared).
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

    // Remember where the match was tapped (the group brief) so the *placed* bet
    // can be announced back there; the build flow itself happens in the DM.
    let origin_chat = cb
        .message
        .as_ref()
        .map(|m| m.chat.get_id())
        .unwrap_or(cb.from.id);
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
        origin_chat,
    };
    let qid = quotes(ctx).lock().insert(q.clone());
    let text = quote_text(lang, &q);
    let rows = option_rows(lang, &q, qid);

    // In a private chat the brief is the user's own message → replace it in
    // place with the quote (the rest of the build flow edits in place too). In a
    // group the brief is shared, so editing would clobber it for everyone — DM
    // the per-user build flow instead, and toast where to look.
    if is_group_chat(origin_chat) {
        match tg::send_with_buttons(ctx, cb.from.id, &text, &rows).await {
            Ok(_) => answer(ctx, cb, i18n::bet_check_dm(lang), false).await,
            Err(_) => {
                quotes(ctx).lock().remove(qid);
                answer(ctx, cb, i18n::bet_dm_first(lang), true).await
            }
        }
    } else {
        answer(ctx, cb, "", false).await?;
        let _ = tg::edit_with_buttons(ctx, origin_chat, cb.message_id(), &text, &rows).await;
        Ok(())
    }
}

/// `opt:<qid>:<outcome>` — chose a side → open the stake builder at 0.
pub async fn handle_opt(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some((qid, outcome)) = parse_qid_outcome(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    render_builder(ctx, cb, lang, qid, &outcome, 0).await
}

/// `sz:<qid>:<outcome>:<total>` — re-render the builder at the accumulated total
/// (preset buttons add, `Clear` resets to 0, `Back` from confirm lands here).
pub async fn handle_size(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some((qid, outcome, total)) = parse_qid_outcome_total(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    render_builder(ctx, cb, lang, qid, &outcome, total).await
}

/// `szc:<qid>:<outcome>:<total>` — the builder's Confirm: show the final
/// confirmation screen (the "modal" before any debit).
pub async fn handle_size_confirm(
    ctx: &Context,
    cb: &CallbackQuery,
    rest: &str,
) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some((qid, outcome, total)) = parse_qid_outcome_total(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    if total <= 0 {
        return answer(ctx, cb, i18n::bad_stake(lang), true).await;
    }
    let Some(q) = quotes(ctx).lock().get(qid).filter(|q| q.fresh(now())) else {
        return expire(ctx, cb, lang).await;
    };
    let Some(odds) = q.odds(&outcome).filter(|c| *c > 0.0) else {
        return answer(ctx, cb, "", false).await;
    };
    let side = q.side_name(lang, &outcome);
    let win = fmt_coins(decimal_payout(total * COIN, odds));
    let text = i18n::bet_confirm(lang, &fmt_coins(total * COIN), &side, &win);
    let rows = vec![
        vec![(i18n::bet_btn_place(lang).to_string(), format!("{SIZE_PLACE}{qid}:{outcome}:{total}"))],
        vec![(i18n::bet_btn_back(lang).to_string(), format!("{SIZE}{qid}:{outcome}:{total}"))],
    ];
    edit(ctx, cb, &text, &rows).await?;
    answer(ctx, cb, "", false).await
}

/// `szp:<qid>:<outcome>:<total>` — the only step that moves money: re-check
/// freshness + balance, debit, and record the wager.
pub async fn handle_size_place(
    ctx: &Context,
    cb: &CallbackQuery,
    rest: &str,
) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some((qid, outcome, total)) = parse_qid_outcome_total(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    if total <= 0 {
        return answer(ctx, cb, i18n::bad_stake(lang), true).await;
    }
    let Some(q) = quotes(ctx).lock().get(qid).filter(|q| q.fresh(now())) else {
        return expire(ctx, cb, lang).await;
    };
    let Some(odds) = q.odds(&outcome).filter(|c| *c > 0.0) else {
        return answer(ctx, cb, "", false).await;
    };

    let stake_units = total * COIN;
    let database = db(ctx);
    if !database.balance_change(cb.from.id, -stake_units).unwrap_or(false) {
        return answer(ctx, cb, i18n::not_enough_money(lang), true).await;
    }
    if let Err(err) = database.place_wager(
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
        // Roll the debit back if the insert failed.
        database.force_change(cb.from.id, stake_units).ok();
        eprintln!("place_wager error: {err}");
        return answer(ctx, cb, i18n::db_error(lang), true).await;
    }
    quotes(ctx).lock().remove(qid);

    let payout = decimal_payout(stake_units, odds);
    let side = q.side_name(lang, &outcome);
    let odds_str = format!("{:.2}", decimal_odds(odds));
    // Confirm privately in the DM (where the builder lives).
    let text = i18n::bet_placed(lang, &fmt_coins(stake_units), &side, &odds_str, &fmt_coins(payout));
    let _ = tg::edit_text_only(ctx, cb.message_chat(), cb.message_id(), &text).await;
    // Announce the placed bet back in the original group (skip if `/markets` was
    // used privately — the origin is then the DM itself).
    if is_group_chat(q.origin_chat) {
        let announce =
            i18n::bet_announce(lang, &full_name(&cb.from), &fmt_coins(stake_units), &side, &odds_str);
        let _ = send_text(ctx, q.origin_chat, announce).await;
    }
    answer(ctx, cb, i18n::bet_done(lang), false).await
}

/// Render the accumulate screen for `total` coins on `outcome`: presets that add,
/// then a `[Confirm] [Clear]` row.
async fn render_builder(
    ctx: &Context,
    cb: &CallbackQuery,
    lang: Lang,
    qid: u64,
    outcome: &str,
    total: i64,
) -> Result<(), telexide::Error> {
    let Some(q) = quotes(ctx).lock().get(qid).filter(|q| q.fresh(now())) else {
        return expire(ctx, cb, lang).await;
    };
    let Some(odds) = q.odds(outcome).filter(|c| *c > 0.0) else {
        return answer(ctx, cb, "", false).await;
    };
    let side = q.side_name(lang, outcome);
    let win = fmt_coins(decimal_payout(total.max(0) * COIN, odds));
    let text = i18n::bet_build(
        lang,
        &side,
        &format!("{:.2}", decimal_odds(odds)),
        &fmt_coins(total.max(0) * COIN),
        &win,
    );
    let add_row: tg::Row = SIZE_PRESETS
        .iter()
        .map(|p| (format!("+{p}"), format!("{SIZE}{qid}:{outcome}:{}", total + p)))
        .collect();
    let rows = vec![
        add_row,
        vec![
            (i18n::bet_btn_confirm(lang).to_string(), format!("{SIZE_CONFIRM}{qid}:{outcome}:{total}")),
            (i18n::bet_btn_clear(lang).to_string(), format!("{SIZE}{qid}:{outcome}:0")),
        ],
    ];
    edit(ctx, cb, &text, &rows).await?;
    answer(ctx, cb, "", false).await
}

async fn expire(ctx: &Context, cb: &CallbackQuery, lang: Lang) -> Result<(), telexide::Error> {
    let _ = tg::edit_text_only(ctx, cb.message_chat(), cb.message_id(), i18n::bet_expired(lang)).await;
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

fn parse_qid_outcome(rest: &str) -> Option<(u64, String)> {
    let (qid, outcome) = rest.split_once(':')?;
    Some((qid.parse().ok()?, outcome.to_string()))
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
