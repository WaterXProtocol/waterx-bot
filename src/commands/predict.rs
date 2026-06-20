//! `/predict` — a stateful DM **builder wizard** for creating a prediction.
//!
//! `/predict` (run anywhere) DMs the host a step-by-step flow: question → options
//! → end-time presets, then posts the finished prediction card back to the chat
//! `/predict` was invoked in. Free-text steps (question, options) are captured by
//! the [`on_message`] listener, which routes a host's next plain DM into their
//! in-flight [`PredictDraft`] (`bot::ConvosKey`, the `Convo::Predict` variant). The end time is a button
//! step (`gend:<minutes>`), which finalizes and posts the card.

use crate::bot::Convo;
use crate::commands::tg;
use crate::commands::tg::answer;
use crate::commands::util::*;
use crate::core::prediction::Prediction;
use crate::core::i18n::{self, Lang};
use telexide::model::{CallbackQuery, UpdateContent, User};
use telexide::prelude::*;

/// Callback prefix for the builder's end-time presets: `gend:<minutes>` (0 = no
/// deadline). Routed in `callbacks::on_callback`.
pub const PREDICT_END: &str = "gend:";

/// End-time presets shown after options: (minutes-from-now, label). 0 (no
/// deadline) is a separate button.
const END_PRESETS: &[(i64, &str)] = &[(60, "1h"), (360, "6h"), (720, "12h"), (1440, "24h"), (4320, "3d")];

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
                Convo::Predict(PredictDraft { origin_chat, lang, description: None, options: None }),
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
    }
    // Both filled → awaiting the end-time button; further text is ignored.
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
    rows.push(vec![(i18n::btn_no_deadline(lang).to_string(), format!("{PREDICT_END}0"))]);
    rows
}

/// `gend:<minutes>` — the builder's end-time pick: finalize the draft into a
/// `Prediction`, post the card to the origin chat, register it, and confirm in the DM.
pub async fn handle_predict_endtime(
    ctx: &Context,
    cb: &CallbackQuery,
    rest: &str,
) -> Result<(), telexide::Error> {
    let Ok(minutes) = rest.parse::<i64>() else {
        return answer(ctx, cb, "", false).await;
    };
    // Consume the draft (so a double-tap can't post twice).
    let Some(Convo::Predict(draft)) = convos(ctx).lock().await.remove(&cb.from.id) else {
        return answer(ctx, cb, "", false).await; // expired / already finalized / wrong flow
    };
    let (Some(description), Some(options)) = (draft.description, draft.options) else {
        return answer(ctx, cb, "", false).await; // incomplete (shouldn't happen)
    };
    let lang = draft.lang;
    // `minutes` comes from the (forgeable) `gend:<n>` callback, so saturate
    // rather than risk a wrapping `* 60` / `+ now` on a crafted huge value.
    let ends_at = if minutes <= 0 { 0 } else { now().saturating_add(minutes.saturating_mul(60)) };

    let opt_refs: Vec<&str> = options.iter().map(String::as_str).collect();
    let mut prediction = Prediction::new(cb.from.id, lang, &description, &opt_refs);
    prediction.ends_at = ends_at;
    // Pin the board to the host's odds format (shared message — like its locale).
    prediction.odds_fmt = db(ctx).get_odds_fmt(cb.from.id).unwrap_or_default();

    // Post the card to the origin chat; only on success do we register the prediction.
    let sent = match tg::send_with_buttons(ctx, draft.origin_chat, &prediction.get_text(), &prediction.get_buttons()).await {
        Ok(m) => m,
        Err(e) => {
            eprintln!("predict card post failed (chat {}): {e:?}", draft.origin_chat);
            return answer(ctx, cb, i18n::predict_post_failed(lang), true).await;
        }
    };
    prediction.set_id(sent.chat.get_id(), sent.message_id);
    let key = format!("{}:{}", sent.chat.get_id(), sent.message_id);
    // Re-render so the id tail shows (best-effort, like the old `/predict`).
    let _ = tg::edit_with_buttons(ctx, sent.chat.get_id(), sent.message_id, &prediction.get_text(), &prediction.get_buttons()).await;
    if let Err(err) = db(ctx).save_prediction(&prediction) {
        eprintln!("save_prediction error (continuing in-memory only): {err}");
    }
    predictions(ctx).lock().await.insert(key, prediction);

    // Confirm in the builder DM (edit the end-time message in place).
    if let Some(m) = &cb.message {
        let _ = tg::edit_text_only(ctx, m.chat.get_id(), m.message_id, i18n::predict_created(lang)).await;
    }
    answer(ctx, cb, "", false).await
}
