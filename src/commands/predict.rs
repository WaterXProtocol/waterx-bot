//! `/predict` — a stateful DM **builder wizard** for creating a prediction.
//!
//! `/predict` (run anywhere) DMs the host a step-by-step flow: question → options
//! → end-time presets, then posts the finished prediction card back to the chat
//! `/predict` was invoked in. Free-text steps (question, options) are captured by
//! the [`on_message`] listener, which routes a host's next plain DM into their
//! in-flight [`PredictDraft`] (`bot::ConvosKey`, the `Convo::Predict` variant). The end time is a button
//! step (`gend:<minutes>`), which finalizes and posts the card.

use crate::bot::Convo;
use crate::commands::predmarket;
use crate::commands::tg;
use crate::commands::tg::answer;
use crate::commands::util::*;
use crate::core::i18n::{self, Lang};
use crate::database::{FEE_BPS_DEFAULT, FEE_BPS_MAX};
use telexide::model::{CallbackQuery, UpdateContent, User};
use telexide::prelude::*;

/// Trading-fee picker prefix (`pmfee:<bps>`), routed in `callbacks::on_callback`.
pub const PREDICT_FEE: &str = "pmfee:";

/// Callback prefix for the builder's end-time presets: `gend:<minutes>` (0 = no
/// deadline). Routed in `callbacks::on_callback`.
pub const PREDICT_END: &str = "gend:";

/// Callback prefix for the builder's funding-window presets: `pmfund:<minutes>`
/// (how long funding stays open before trading lazily opens). Routed in
/// `callbacks::on_callback`.
pub const PREDICT_FUND: &str = "pmfund:";

/// End-time presets shown after options: (minutes-from-now, label). 0 (no
/// deadline) is a separate button.
const END_PRESETS: &[(i64, &str)] = &[(60, "1h"), (360, "6h"), (720, "12h"), (1440, "24h"), (4320, "3d")];

/// Funding-window presets: how long the funding stage stays open (minutes).
const FUND_PRESETS: &[(i64, &str)] = &[(60, "1h"), (360, "6h"), (1440, "24h"), (4320, "3d")];

/// Cap on a custom (or preset) deadline: 30 days from now.
const MAX_PREDICT_MINUTES: i64 = 30 * 24 * 60;

/// An in-flight `/predict` builder draft (one per host). The current step is
/// implied by which fields are filled: no `description` → awaiting the question;
/// `description` set, no `options` → awaiting options; both set → awaiting the
/// end-time button.
pub struct PredictDraft {
    /// Chat the finished card is posted to (where `/predict` was invoked).
    pub origin_chat: i64,
    pub lang: Lang,
    pub description: Option<String>,
    pub options: Option<Vec<String>>,
    /// Set when the host tapped `[⌨️ Custom]`: their next DM text is parsed as a
    /// custom duration instead of being ignored.
    pub awaiting_custom: bool,
    /// Deadline (unix secs; 0 = none) chosen at the end-time step — set just
    /// before the fee picker.
    pub ends_at: Option<i64>,
    /// Trading fee (bps) chosen at the fee step — set just before the
    /// funding-window picker, then consumed by `finalize_funding`.
    pub fee_bps: Option<i64>,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Open a fresh `/predict` builder for `host`: DM them the first prompt and (only
/// if the DM lands) register the `Convo::Predict` draft pinned to `origin_chat`
/// (where the finished card will post). Returns `true` when the DM landed and the
/// draft is registered, `false` when it bounced (the host never started the bot).
/// Shared by the `/predict` command and the `menu:predict` home-page button.
/// Inserting overwrites any in-flight `/feedback` flow — the last-started wins.
pub(crate) async fn open_draft(ctx: &Context, host: &User, origin_chat: i64) -> bool {
    let lang = lang_for(ctx, host);
    match send_text(ctx, host.id, i18n::predict_ask_question(lang)).await {
        Ok(_) => {
            convos(ctx).lock().await.insert(
                host.id,
                Convo::Predict(PredictDraft {
                    origin_chat,
                    lang,
                    description: None,
                    options: None,
                    awaiting_custom: false,
                    ends_at: None,
                    fee_bps: None,
                }),
            );
            true
        }
        Err(_) => false,
    }
}

#[command(description = "create a prediction")]
pub async fn predict(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let Some(host) = message.from.clone() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let lang = lang_for(&ctx, &host);
    let origin_chat = message.chat.get_id();

    if open_draft(&ctx, &host, origin_chat).await {
        // In a group the prompt went to the host's DM — point them there.
        if is_group_chat(origin_chat) {
            reply(&ctx, &message, i18n::predict_check_dm(lang)).await?;
        }
    } else {
        reply(&ctx, &message, i18n::bet_dm_first(lang)).await?;
    }
    Ok(())
}

/// DM message listener: routes a host's plain-text reply into their in-flight
/// builder draft (question → options). Only fires for non-command text in a
/// private chat where the user has an active draft; otherwise a no-op.
#[prepare_listener]
pub async fn on_message(ctx: Context, update: Update) {
    let UpdateContent::Message(message) = update.content else {
        return;
    };
    // The builder lives in the host's DM: private chats, plain non-command text.
    if is_group_chat(message.chat.get_id()) {
        return;
    }
    let Some(user) = message.from.as_ref() else {
        return;
    };
    let text = text_of(&message).trim().to_string();
    if text.is_empty() || text.starts_with('/') {
        return;
    }

    let drafts = convos(&ctx);
    let mut guard = drafts.lock().await;
    let Some(Convo::Predict(draft)) = guard.get_mut(&user.id) else {
        return; // not in the predict wizard — an ordinary DM or another flow
    };
    // A paused bot shouldn't advance a non-owner's wizard (fail closed).
    if !is_owner(&ctx, user.id) && db(&ctx).is_paused().unwrap_or(true) {
        return;
    }
    let lang = draft.lang;

    if draft.description.is_none() {
        draft.description = Some(text);
        drop(guard);
        let _ = send_text(&ctx, user.id, i18n::predict_ask_options(lang)).await;
        return;
    }
    if draft.options.is_none() {
        let opts = parse_options(&text);
        if opts.len() < 2 {
            drop(guard);
            let _ = send_text(&ctx, user.id, i18n::predict_need_options(lang)).await;
            return;
        }
        draft.options = Some(opts);
        drop(guard);
        let _ = tg::send_with_buttons(
            &ctx,
            user.id,
            i18n::predict_ask_endtime(lang),
            &end_time_rows(lang),
        )
        .await;
        return;
    }
    // Both filled. If the host tapped [⌨️ Custom], this DM is their typed
    // duration; otherwise they're meant to use the end-time buttons, so ignore.
    if !draft.awaiting_custom {
        return;
    }
    let Some(minutes) = parse_duration(&text) else {
        drop(guard);
        let _ = send_text(&ctx, user.id, i18n::predict_bad_duration(lang)).await;
        return;
    };
    // Valid — store the deadline and advance to the fee picker.
    draft.ends_at = Some(now().saturating_add(minutes.saturating_mul(60)));
    draft.awaiting_custom = false;
    drop(guard);
    let _ = tg::send_with_buttons(&ctx, user.id, i18n::predict_ask_fee(lang), &fee_rows(lang)).await;
}

/// Options: one per line when multi-line, else whitespace-separated. Trimmed,
/// empties dropped — so "Yes No" and "Manchester United\nLiverpool" both work.
fn parse_options(text: &str) -> Vec<String> {
    let by_line: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    if by_line.len() >= 2 {
        return by_line;
    }
    text.split_whitespace().map(String::from).collect()
}

/// End-time preset keyboard: duration presets (3 per row) + a "no deadline" row.
fn end_time_rows(lang: Lang) -> Vec<tg::Row> {
    let mut rows: Vec<tg::Row> = END_PRESETS
        .chunks(3)
        .map(|chunk| {
            chunk
                .iter()
                .map(|(m, label)| ((*label).to_string(), format!("{PREDICT_END}{m}")))
                .collect()
        })
        .collect();
    rows.push(vec![
        (i18n::btn_custom(lang).to_string(), format!("{PREDICT_END}custom")),
        (i18n::btn_no_deadline(lang).to_string(), format!("{PREDICT_END}0")),
    ]);
    rows
}

/// Parse a host-typed custom duration into **minutes from now**. Accepts a bare
/// number (minutes) or any combo of `<n>d`/`<n>h`/`<n>m` (e.g. `2h`, `90m`,
/// `1d12h`), whitespace-insensitive and case-insensitive. `None` on garbage or a
/// non-positive total; capped at `MAX_PREDICT_MINUTES`.
fn parse_duration(text: &str) -> Option<i64> {
    let t: String = text.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase();
    if t.is_empty() {
        return None;
    }
    if let Ok(n) = t.parse::<i64>() {
        return (n > 0).then(|| n.min(MAX_PREDICT_MINUTES));
    }
    let mut total: i64 = 0;
    let mut num = String::new();
    for ch in t.chars() {
        if ch.is_ascii_digit() {
            num.push(ch);
        } else {
            let n: i64 = num.parse().ok()?;
            num.clear();
            let mult = match ch {
                'd' => 1440,
                'h' => 60,
                'm' => 1,
                _ => return None,
            };
            total = total.checked_add(n.checked_mul(mult)?)?;
        }
    }
    // A trailing number with no unit (e.g. "1d12") is ambiguous → reject.
    if !num.is_empty() || total <= 0 {
        return None;
    }
    Some(total.min(MAX_PREDICT_MINUTES))
}

/// Outcome of finalizing a `/predict` draft into a posted funding-stage event.
enum Finalized {
    /// Card posted; the event is open for funding.
    Posted,
    /// The card post or event creation failed.
    Failed,
}

/// Create the host funding-stage event from a completed draft and post its board.
/// A placeholder card is posted first so the card slot exists before the event
/// row references it. Nothing is charged — liquidity arrives via the board's
/// fund flow; the opening prices + `b` are discovered when the funding window
/// closes. `window_minutes` is how long funding stays open. The event is pinned to
/// the host's locale / odds-format / timezone (a shared board can't localize per
/// viewer); the board re-renders from the DB on every fund/trade.
async fn finalize_funding(
    ctx: &Context,
    host_id: i64,
    draft: PredictDraft,
    window_minutes: i64,
) -> Finalized {
    let (Some(description), Some(options)) = (draft.description, draft.options) else {
        return Finalized::Failed;
    };
    let lang = draft.lang;
    let ends_at = draft.ends_at.unwrap_or(0);
    let fee_bps = draft.fee_bps.unwrap_or(FEE_BPS_DEFAULT);
    let fmt = db(ctx).get_odds_fmt(host_id).unwrap_or_default();
    let tz = db(ctx).get_tz(host_id).ok().flatten();
    let open_at = now().saturating_add(window_minutes.saturating_mul(60));

    // Placeholder first, so a card slot exists before the event references it.
    let no_rows: Vec<tg::Row> = Vec::new();
    let sent = match tg::send_with_buttons(ctx, draft.origin_chat, &format!("🎲 {description}"), &no_rows).await
    {
        Ok(m) => m,
        Err(e) => {
            eprintln!("predict placeholder post failed (chat {}): {e:?}", draft.origin_chat);
            return Finalized::Failed;
        }
    };
    let (chat, msg) = (sent.chat.get_id(), sent.message_id);

    let event_id = match db(ctx).create_funding_event(
        host_id,
        &description,
        lang.store_code(),
        fmt.store_code(),
        tz,
        ends_at,
        &options,
        fee_bps,
        open_at,
        now(),
    ) {
        Ok(id) => id,
        Err(e) => {
            eprintln!("create_funding_event error: {e}");
            let _ = tg::delete_message(ctx, chat, msg).await;
            return Finalized::Failed;
        }
    };

    let _ = db(ctx).set_event_card(event_id, chat, msg);
    if let Ok(Some(board)) = db(ctx).amm_board(event_id) {
        let (text, rows) = predmarket::first_board(&board);
        let _ = tg::edit_with_buttons(ctx, chat, msg, &text, &rows).await;
    }
    if is_group_chat(chat) {
        let _ = tg::pin_message(ctx, chat, msg).await;
    }
    Finalized::Posted
}

/// `gend:<minutes>` — the builder's end-time pick: stores the deadline and shows
/// the trading-fee picker (`finalize` then creates the AMM event).
pub async fn handle_predict_endtime(
    ctx: &Context,
    cb: &CallbackQuery,
    rest: &str,
) -> Result<(), telexide::Error> {
    // `[⌨️ Custom]` → don't finalize; flip the draft to await a typed duration.
    if rest == "custom" {
        let convos_map = convos(ctx);
        let lang = {
            let mut g = convos_map.lock().await;
            let Some(Convo::Predict(draft)) = g.get_mut(&cb.from.id) else {
                return answer(ctx, cb, "", false).await;
            };
            draft.awaiting_custom = true;
            draft.lang
        };
        if let Some(m) = &cb.message {
            let _ = tg::edit_text_only(ctx, m.chat.get_id(), m.message_id, i18n::predict_ask_custom(lang)).await;
        }
        return answer(ctx, cb, "", false).await;
    }

    // `minutes` comes from the (forgeable) `gend:<n>` callback; saturating math
    // below means a crafted huge value can't wrap.
    let Ok(minutes) = rest.parse::<i64>() else {
        return answer(ctx, cb, "", false).await;
    };
    // Store the deadline on the draft and advance to the fee picker (don't
    // finalize yet — the fee step does that).
    let lang = {
        let convos_map = convos(ctx);
        let mut g = convos_map.lock().await;
        let Some(Convo::Predict(draft)) = g.get_mut(&cb.from.id) else {
            return answer(ctx, cb, "", false).await;
        };
        draft.ends_at =
            Some(if minutes <= 0 { 0 } else { now().saturating_add(minutes.saturating_mul(60)) });
        draft.lang
    };
    if let Some(m) = &cb.message {
        let _ = tg::edit_with_buttons(
            ctx,
            m.chat.get_id(),
            m.message_id,
            i18n::predict_ask_fee(lang),
            &fee_rows(lang),
        )
        .await;
    }
    answer(ctx, cb, "", false).await
}

/// `pmfee:<bps>` — the host picked their trading fee: store it and show the
/// funding-window picker (`finalize_funding` runs after that).
pub async fn handle_predict_fee(
    ctx: &Context,
    cb: &CallbackQuery,
    rest: &str,
) -> Result<(), telexide::Error> {
    let Ok(bps) = rest.parse::<i64>() else {
        return answer(ctx, cb, "", false).await;
    };
    let fee_bps = bps.clamp(0, FEE_BPS_MAX);
    let lang = {
        let convos_map = convos(ctx);
        let mut g = convos_map.lock().await;
        let Some(Convo::Predict(draft)) = g.get_mut(&cb.from.id) else {
            return answer(ctx, cb, "", false).await;
        };
        draft.fee_bps = Some(fee_bps);
        draft.lang
    };
    if let Some(m) = &cb.message {
        let _ = tg::edit_with_buttons(
            ctx,
            m.chat.get_id(),
            m.message_id,
            i18n::predict_ask_funding(lang),
            &funding_rows(lang),
        )
        .await;
    }
    answer(ctx, cb, "", false).await
}

/// `pmfund:<minutes>` — the host picked the funding-window length: finalize the
/// draft into a posted funding-stage event (confirming in the builder DM).
pub async fn handle_predict_funding(
    ctx: &Context,
    cb: &CallbackQuery,
    rest: &str,
) -> Result<(), telexide::Error> {
    let Ok(minutes) = rest.parse::<i64>() else {
        return answer(ctx, cb, "", false).await;
    };
    let window = minutes.clamp(1, MAX_PREDICT_MINUTES);
    // Consume the draft (so a double-tap can't post twice).
    let Some(Convo::Predict(draft)) = convos(ctx).lock().await.remove(&cb.from.id) else {
        return answer(ctx, cb, "", false).await;
    };
    let lang = draft.lang;
    let msg = match finalize_funding(ctx, cb.from.id, draft, window).await {
        Finalized::Posted => i18n::predict_created(lang).to_string(),
        Finalized::Failed => i18n::predict_post_failed(lang).to_string(),
    };
    if let Some(m) = &cb.message {
        let _ = tg::edit_text_only(ctx, m.chat.get_id(), m.message_id, &msg).await;
    }
    answer(ctx, cb, "", false).await
}

/// Trading-fee picker: 2% / 5% / 10% presets (callback `pmfee:<bps>`).
fn fee_rows(_lang: Lang) -> Vec<tg::Row> {
    vec![vec![
        ("2%".to_string(), format!("{PREDICT_FEE}200")),
        ("5%".to_string(), format!("{PREDICT_FEE}500")),
        ("10%".to_string(), format!("{PREDICT_FEE}1000")),
    ]]
}

/// Funding-window picker: how long the funding stage stays open (callback
/// `pmfund:<minutes>`).
fn funding_rows(_lang: Lang) -> Vec<tg::Row> {
    vec![FUND_PRESETS
        .iter()
        .map(|(m, label)| ((*label).to_string(), format!("{PREDICT_FUND}{m}")))
        .collect()]
}

#[cfg(test)]
mod tests {
    use super::{parse_duration, MAX_PREDICT_MINUTES};

    #[test]
    fn parse_duration_accepts_bare_minutes_and_unit_combos() {
        assert_eq!(parse_duration("90"), Some(90)); // bare = minutes
        assert_eq!(parse_duration("2h"), Some(120));
        assert_eq!(parse_duration("90m"), Some(90));
        assert_eq!(parse_duration("1d"), Some(1440));
        assert_eq!(parse_duration("1d12h"), Some(1440 + 720));
        assert_eq!(parse_duration("1D 12H 30M"), Some(1440 + 720 + 30)); // case/space-insensitive
        assert_eq!(parse_duration("3d"), Some(4320));
    }

    #[test]
    fn parse_duration_rejects_garbage_and_caps() {
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("0"), None); // non-positive
        assert_eq!(parse_duration("0h"), None);
        assert_eq!(parse_duration("soon"), None);
        assert_eq!(parse_duration("2w"), None); // unknown unit
        assert_eq!(parse_duration("1d12"), None); // trailing unit-less number
        // Capped at 30 days.
        assert_eq!(parse_duration("999d"), Some(MAX_PREDICT_MINUTES));
        assert_eq!(parse_duration("100000"), Some(MAX_PREDICT_MINUTES));
    }
}
