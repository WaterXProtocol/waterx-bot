//! `/predict` — a stateful DM **builder wizard** for creating a prediction.
//!
//! `/predict` (run anywhere) DMs the host a step-by-step flow: question → options
//! → end-time presets, then posts the finished prediction card back to the chat
//! `/predict` was invoked in. Free-text steps (question, options) are captured by
//! the [`on_message`] listener, which routes a host's next plain DM into their
//! in-flight [`PredictDraft`] (`bot::ConvosKey`, the `Convo::Predict` variant). The end time is a button
//! step (`gend:<minutes>`), which finalizes and posts the card.

use crate::bot::{Convo, ConvosKey};
use crate::commands::tg;
use crate::commands::util::*;
use crate::game::BetGame;
use crate::i18n::{self, Lang};
use std::collections::HashMap;
use std::sync::Arc;
use telexide::api::types::AnswerCallbackQuery;
use telexide::model::{CallbackQuery, UpdateContent};
use telexide::prelude::*;
use tokio::sync::Mutex;

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

fn drafts(ctx: &Context) -> Arc<Mutex<HashMap<i64, Convo>>> {
    ctx.data
        .read()
        .get::<ConvosKey>()
        .expect("ConvosKey missing")
        .clone()
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Acknowledge a callback query (optional alert toast). Local copy so the builder
/// doesn't depend on `callbacks`'s private helper.
async fn ack(ctx: &Context, cb: &CallbackQuery, toast: &str) -> Result<(), telexide::Error> {
    let mut a = AnswerCallbackQuery::new(cb.id.clone());
    if !toast.is_empty() {
        a.text = Some(toast.to_string());
        a.show_alert = Some(true);
    }
    ctx.api.answer_callback_query(a).await?;
    Ok(())
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

    // DM the first prompt; only register the draft once we know the DM lands (a
    // user who never started the bot can't run the wizard). Starting `/predict`
    // again just replaces any half-finished draft.
    match send_text(&ctx, host.id, i18n::predict_ask_question(lang)).await {
        Ok(_) => {
            // Inserting overwrites any in-flight `/feedback` flow for this user —
            // the last-started DM flow wins.
            drafts(&ctx).lock().await.insert(
                host.id,
                Convo::Predict(PredictDraft { origin_chat, lang, description: None, options: None }),
            );
            // In a group the prompt went to the host's DM — point them there.
            if is_group_chat(origin_chat) {
                reply(&ctx, &message, i18n::predict_check_dm(lang)).await?;
            }
        }
        Err(_) => {
            reply(&ctx, &message, i18n::bet_dm_first(lang)).await?;
        }
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

    let drafts = drafts(&ctx);
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
/// `BetGame`, post the card to the origin chat, register it, and confirm in the DM.
pub async fn handle_predict_endtime(
    ctx: &Context,
    cb: &CallbackQuery,
    rest: &str,
) -> Result<(), telexide::Error> {
    let Ok(minutes) = rest.parse::<i64>() else {
        return ack(ctx, cb, "").await;
    };
    // Consume the draft (so a double-tap can't post twice).
    let Some(Convo::Predict(draft)) = drafts(ctx).lock().await.remove(&cb.from.id) else {
        return ack(ctx, cb, "").await; // expired / already finalized / wrong flow
    };
    let (Some(description), Some(options)) = (draft.description, draft.options) else {
        return ack(ctx, cb, "").await; // incomplete (shouldn't happen)
    };
    let lang = draft.lang;
    let ends_at = if minutes <= 0 { 0 } else { now() + minutes * 60 };

    let opt_refs: Vec<&str> = options.iter().map(String::as_str).collect();
    let mut game = BetGame::new(cb.from.id, lang, &description, &opt_refs);
    game.ends_at = ends_at;
    // Pin the board to the host's odds format (shared message — like its locale).
    game.odds_fmt = db(ctx).get_odds_fmt(cb.from.id).unwrap_or_default();

    // Post the card to the origin chat; only on success do we register the game.
    let sent = match tg::send_with_buttons(ctx, draft.origin_chat, &game.get_text(), &game.get_buttons()).await {
        Ok(m) => m,
        Err(e) => {
            eprintln!("predict card post failed (chat {}): {e:?}", draft.origin_chat);
            return ack(ctx, cb, i18n::predict_post_failed(lang)).await;
        }
    };
    game.set_id(sent.chat.get_id(), sent.message_id);
    let key = format!("{}:{}", sent.chat.get_id(), sent.message_id);
    // Re-render so the id tail shows (best-effort, like the old `/predict`).
    let _ = tg::edit_with_buttons(ctx, sent.chat.get_id(), sent.message_id, &game.get_text(), &game.get_buttons()).await;
    if let Err(err) = db(ctx).save_bet_game(&game) {
        eprintln!("save_bet_game error (continuing in-memory only): {err}");
    }
    games(ctx).lock().await.insert(key, game);

    // Confirm in the builder DM (edit the end-time message in place).
    if let Some(m) = &cb.message {
        let _ = tg::edit_text_only(ctx, m.chat.get_id(), m.message_id, i18n::predict_created(lang)).await;
    }
    ack(ctx, cb, "").await
}
