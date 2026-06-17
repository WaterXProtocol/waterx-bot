//! Real-money match betting: tap a match in `/matches` → the bot DMs a fresh
//! odds quote → pick a side → pick a stake. The quote is valid for
//! [`QUOTE_TTL_SECS`]; after that the user must re-open `/matches` because the
//! odds move. Wagers are stored and settled later by an admin (`/settle`).

use crate::bot::QuotesKey;
use crate::commands::markets;
use crate::commands::tg;
use crate::commands::util::*;
use crate::database::COIN;
use crate::i18n::{self, Lang};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use telexide::api::types::AnswerCallbackQuery;
use telexide::model::CallbackQuery;
use telexide::prelude::*;

/// How long a quoted set of odds stays valid.
const QUOTE_TTL_SECS: i64 = 60;
/// Whole-coin stake presets offered as buttons.
const SIZE_PRESETS: [i64; 5] = [1, 5, 10, 50, 100];

/// Callback-data prefixes (the `bet:` prefix lives in `markets`).
pub const OPT: &str = "opt:";
pub const SIZE: &str = "sz:";

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

fn decimal(cents: f64) -> f64 {
    100.0 / cents
}

/// `[outcome]` rows (one per priced outcome) labelled `name 1.54`.
fn option_rows(lang: Lang, q: &Quote, qid: u64) -> Vec<tg::Row> {
    let mut rows = Vec::new();
    for outcome in ["teamA", "draw", "teamB"] {
        if let Some(c) = q.odds(outcome).filter(|c| *c > 0.0) {
            let label = format!("{} {:.2}", q.side_name(lang, outcome), decimal(c));
            rows.push(vec![(label, format!("{OPT}{qid}:{outcome}"))]);
        }
    }
    rows
}

fn quote_text(lang: Lang, q: &Quote) -> String {
    let mut s = format!("{} vs. {}\n", q.team_a, q.team_b);
    for outcome in ["teamA", "draw", "teamB"] {
        if let Some(c) = q.odds(outcome).filter(|c| *c > 0.0) {
            s.push_str(&format!("· {} — {:.2}\n", q.side_name(lang, outcome), decimal(c)));
        }
    }
    s.push('\n');
    s.push_str(i18n::bet_pick(lang));
    s
}

/// `bet:<market_id>` — DM the user a fresh quote for that match.
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
    };
    let qid = quotes(ctx).lock().insert(q.clone());
    let text = quote_text(lang, &q);
    let rows = option_rows(lang, &q, qid);

    // The bet flow happens in the user's DM.
    match tg::send_with_buttons(ctx, cb.from.id, &text, &rows).await {
        Ok(_) => answer(ctx, cb, i18n::bet_check_dm(lang), false).await,
        Err(_) => {
            quotes(ctx).lock().remove(qid);
            answer(ctx, cb, i18n::bet_dm_first(lang), true).await
        }
    }
}

/// `opt:<qid>:<outcome>` — show the stake presets for the chosen side.
pub async fn handle_opt(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some((qid, outcome)) = parse_qid_outcome(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    let Some(q) = quotes(ctx).lock().get(qid).filter(|q| q.fresh(now())) else {
        return expire(ctx, cb, lang).await;
    };
    let Some(odds) = q.odds(&outcome).filter(|c| *c > 0.0) else {
        return answer(ctx, cb, "", false).await;
    };

    let side = q.side_name(lang, &outcome);
    let rows: Vec<tg::Row> = vec![SIZE_PRESETS
        .iter()
        .map(|s| (s.to_string(), format!("{SIZE}{qid}:{outcome}:{s}")))
        .collect()];
    edit(
        ctx,
        cb,
        &i18n::bet_how_much(lang, &side, &format!("{:.2}", decimal(odds))),
        &rows,
    )
    .await?;
    answer(ctx, cb, "", false).await
}

/// `sz:<qid>:<outcome>:<stake>` — validate freshness + balance, place the wager.
pub async fn handle_size(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let parts: Vec<&str> = rest.split(':').collect();
    let [qid_s, outcome, stake_s] = parts.as_slice() else {
        return answer(ctx, cb, "", false).await;
    };
    let (Ok(qid), Ok(stake_coins)) = (qid_s.parse::<u64>(), stake_s.parse::<i64>()) else {
        return answer(ctx, cb, "", false).await;
    };
    let Some(q) = quotes(ctx).lock().get(qid).filter(|q| q.fresh(now())) else {
        return expire(ctx, cb, lang).await;
    };
    let Some(odds) = q.odds(outcome).filter(|c| *c > 0.0) else {
        return answer(ctx, cb, "", false).await;
    };

    let stake_units = stake_coins * COIN;
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
        outcome,
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

    let payout = (stake_units as f64 * decimal(odds)).round() as i64;
    let side = q.side_name(lang, outcome);
    let text = i18n::bet_placed(
        lang,
        &fmt_coins(stake_units),
        &side,
        &format!("{:.2}", decimal(odds)),
        &fmt_coins(payout),
    );
    let _ = tg::edit_text_only(ctx, cb.message_chat(), cb.message_id(), &text).await;
    answer(ctx, cb, i18n::bet_done(lang), false).await
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
