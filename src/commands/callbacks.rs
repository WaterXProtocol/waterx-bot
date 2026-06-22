use crate::commands::util::{
    board_header, bot_token, db, fmt_coins, full_name, is_group_chat, predictions, to_micro,
    SORRY_FRUITS,
};
use crate::database::COIN;
use crate::commands::{admin, assets, betting, markets, menu, predict, referral, tg};
use crate::commands::tg::answer;
use crate::database::OfferOutcome;
use crate::core::i18n::{self, Lang};
use crate::core::types::BetState;
use rand::seq::SliceRandom;
use std::collections::HashMap;
use telexide::model::{CallbackQuery, ChatMember, UpdateContent};
use telexide::prelude::*;

/// The bot's own membership changed in a chat. When it's *added to a group*,
/// record the adder so new users who check in there bind to them as a referral.
#[prepare_listener]
pub async fn on_my_chat_member(ctx: Context, update: Update) {
    let UpdateContent::MyChatMember(upd) = update.content else {
        return;
    };
    let chat_id = upd.chat.get_id();
    if chat_id >= 0 {
        return; // groups / supergroups only
    }
    let now_in = matches!(
        upd.new_chat_member,
        ChatMember::Member(_) | ChatMember::Administrator(_) | ChatMember::Creator(_)
    );
    let was_out = matches!(
        upd.old_chat_member,
        ChatMember::Left(_) | ChatMember::Kicked(_)
    );
    if now_in && was_out {
        let _ = db(&ctx).set_group_adder(chat_id, upd.from.id);
    }
}

#[prepare_listener]
pub async fn on_callback(ctx: Context, update: Update) {
    let UpdateContent::CallbackQuery(cb) = update.content else {
        return;
    };
    let Some(data) = cb.data.clone() else {
        return;
    };
    // Learn this chat so /broadcast can reach it later.
    if let Some(m) = &cb.message {
        let _ = db(&ctx).touch_chat(m.chat.get_id());
    }
    // Admin pause kill-switch: block every non-owner button press while paused.
    // Fail **closed** — if we can't read the flag, treat the bot as paused (a
    // kill-switch that can't confirm "off" should stop, not pass through).
    if !crate::commands::util::is_owner(&ctx, cb.from.id) {
        let paused = db(&ctx).is_paused().unwrap_or_else(|e| {
            eprintln!("on_callback is_paused error (failing closed): {e}");
            true
        });
        if paused {
            let _ = answer(&ctx, &cb, i18n::service_paused(cb_lang(&ctx, &cb)), false).await;
            return;
        }
    }
    // Group-add referral: a brand-new user's first interaction in a group binds
    // them to whoever added the bot. Runs before dispatch so the row is created
    // here (not by the handler that follows). The text-command path is mirrored
    // in `util::paused_block`.
    if let Some(m) = &cb.message {
        referral::maybe_bind_group(&ctx, m.chat.get_id(), &cb.from).await;
    }
    let result = if let Some(rest) = data.strip_prefix("envelope:") {
        handle_envelope(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix("gamble:") {
        handle_gamble(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix("gsz:") {
        handle_prediction_size(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix("gsp:") {
        handle_prediction_place(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix("bx:") {
        handle_board_dismiss(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(predict::PREDICT_END) {
        predict::handle_predict_endtime(&ctx, &cb, rest).await
    } else if data.starts_with("sell:") {
        handle_sell(&ctx, &cb).await
    } else if data.starts_with("buy:") {
        handle_buy(&ctx, &cb).await
    } else if let Some(rest) = data.strip_prefix(menu::STZ) {
        handle_settings_tz(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(menu::SET_TZ) {
        handle_set_tz(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(menu::SET_FMT) {
        handle_set_fmt(&ctx, &cb, rest).await
    } else if data == menu::CFG_LANG {
        handle_cfg_lang(&ctx, &cb).await
    } else if data == menu::CFG_TZ {
        handle_cfg_tz(&ctx, &cb).await
    } else if data == menu::CFG_ODDS {
        handle_cfg_odds(&ctx, &cb).await
    } else if data == menu::CFG_HOME {
        handle_cfg_home(&ctx, &cb).await
    } else if let Some(rest) = data.strip_prefix(menu::SLANG) {
        handle_settings_lang(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(menu::SET_LANG) {
        handle_set_lang(&ctx, &cb, rest).await
    } else if data == menu::MENU_CHECKIN {
        handle_menu_checkin(&ctx, &cb).await
    } else if data == menu::MENU_HOME {
        handle_menu_home(&ctx, &cb).await
    } else if data == menu::MENU_BALANCE {
        handle_menu_balance(&ctx, &cb).await
    } else if data == menu::MENU_MARKETS {
        handle_menu_markets(&ctx, &cb).await
    } else if data == menu::MENU_RULE {
        handle_menu_rule(&ctx, &cb).await
    } else if data == menu::MENU_PREDICT {
        handle_menu_predict(&ctx, &cb).await
    } else if data == menu::MENU_REPOST {
        handle_menu_repost(&ctx, &cb).await
    } else if data == menu::MENU_INVITE {
        handle_menu_invite(&ctx, &cb).await
    } else if data == menu::MENU_SETTLE {
        handle_menu_settle(&ctx, &cb).await
    } else if let Some(rest) = data.strip_prefix(PSTL) {
        handle_predict_settle_cb(&ctx, &cb, rest).await
    } else if data == menu::INVITE_LINK {
        handle_invite_link(&ctx, &cb).await
    } else if data == menu::INVITE_FWD {
        handle_invite_fwd(&ctx, &cb).await
    } else if data == menu::INVITE_QR {
        handle_invite_qr(&ctx, &cb).await
    } else if let Some(rest) = data.strip_prefix(markets::BET) {
        betting::handle_bet(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(betting::OPT) {
        betting::handle_opt(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(betting::SIZE_PLACE) {
        betting::handle_size_place(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(betting::SIZE) {
        betting::handle_size(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(admin::SETTLE_CB) {
        admin::handle_settle_cb(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(admin::RESET_CB) {
        admin::handle_reset_cb(&ctx, &cb, rest).await
    } else {
        Ok(())
    };
    if let Err(err) = result {
        eprintln!("callback handler error: {err}");
    }
}

async fn handle_envelope(
    ctx: &Context,
    cb: &CallbackQuery,
    rest: &str,
) -> Result<(), telexide::Error> {
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    let chat_id = message.chat.get_id();
    let msg_id = message.message_id;
    let lang = Lang::from_user(&cb.from);
    let db = db(ctx);
    if !db.has_buffer(chat_id, msg_id).unwrap_or(false) {
        return answer(ctx, cb, i18n::someone_took_it(lang), false).await;
    }
    // The amount in the callback data is **untrusted** (a crafted callback could
    // claim to be any value). It only selects the branch; the actual coin credit
    // comes from the escrow stored in the buffer row (see `claim_envelope`).
    let amount: i64 = rest.parse().unwrap_or(0);
    if amount <= 0 {
        // A forged non-positive claim: hand out a consolation fruit and **refund**
        // the envelope's escrow to its owner (so the coins are neither minted nor
        // stranded), cancelling the envelope.
        let fruit = {
            let mut rng = rand::thread_rng();
            // SORRY_FRUITS is a non-empty const, so `choose` is always `Some`;
            // fall back to a fixed fruit rather than `unwrap` to keep this path
            // panic-free regardless.
            SORRY_FRUITS.choose(&mut rng).copied().unwrap_or('🍑')
        };
        let s = fruit.to_string();
        match db.fruit_change(cb.from.id, &s, true) {
            Ok(true) => {
                db.refund_envelope(chat_id, msg_id).ok();
                let _ = tg::edit_text_only(
                    ctx,
                    chat_id,
                    msg_id,
                    &i18n::received_fruit(lang, &cb.from.first_name, &s),
                )
                .await;
                return answer(ctx, cb, i18n::grabbed_it(lang), false).await;
            }
            Ok(false) => return answer(ctx, cb, i18n::too_many_fruits(lang), false).await,
            Err(_) => return answer(ctx, cb, i18n::db_error(lang), true).await,
        }
    }
    // Atomically claim: the buffer row is deleted and the **escrowed** amount
    // credited only if THIS tap won the delete, so a concurrent double-tap can't
    // double-credit and a crafted callback can't change the credited amount.
    let credited = match db.claim_envelope(chat_id, msg_id, cb.from.id) {
        Ok(Some(units)) => units,
        Ok(None) => return answer(ctx, cb, i18n::someone_took_it(lang), false).await,
        Err(e) => {
            eprintln!("claim_envelope error (chat {chat_id}, msg {msg_id}): {e}");
            return answer(ctx, cb, i18n::db_error(lang), true).await;
        }
    };
    let _ = tg::edit_text_only(
        ctx,
        chat_id,
        msg_id,
        &i18n::received_coins(lang, &cb.from.first_name, &fmt_coins(credited)),
    )
    .await;
    answer(ctx, cb, i18n::grabbed_it(lang), false).await
}

async fn handle_gamble(
    ctx: &Context,
    cb: &CallbackQuery,
    rest: &str,
) -> Result<(), telexide::Error> {
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    let chat_id = message.chat.get_id();
    let msg_id = message.message_id;
    let key = format!("{chat_id}:{msg_id}");
    // Tapper's locale for the private toast; the shared board uses the host's
    // `prediction.lang`.
    let lang = cb_lang(ctx, cb);
    let predictions = predictions(ctx);
    let db = db(ctx);

    // close: gamble:  (host only)
    if rest.is_empty() {
        let (text, rows) = {
            let mut g = predictions.lock().await;
            let Some(prediction) = g.get_mut(&key) else {
                return answer(ctx, cb, i18n::prediction_invalid(lang), true).await;
            };
            if prediction.host != cb.from.id {
                return answer(ctx, cb, i18n::not_host(lang), true).await;
            }
            // A deadline means betting runs until then — the host can't close early.
            if !prediction.can_close(chrono::Utc::now().timestamp()) {
                let when = crate::commands::util::fmt_local_time(prediction.ends_at, prediction.tz_offset)
                    .unwrap_or_default();
                return answer(ctx, cb, &i18n::close_before_deadline(lang, &when), true).await;
            }
            if !prediction.close() {
                return answer(ctx, cb, i18n::already_closed(lang), true).await;
            }
            if let Err(err) = db.save_prediction(prediction) {
                eprintln!("save_prediction(close) error: {err}");
            }
            (prediction.get_text(), prediction.get_buttons())
        };
        let _ = tg::edit_with_buttons(ctx, chat_id, msg_id, &text, &rows).await;
        return answer(ctx, cb, i18n::close_prediction_toast(lang), false).await;
    }

    // pick: gamble:pick:<idx> — post the tapper their OWN stake board as a reply
    // to the shared prediction card (owner-locked, so only they can use it; the placed
    // bet is announced back under the card). The board stays out of the shared
    // card so every member can tap for their own board.
    if let Some(idx_s) = rest.strip_prefix("pick:") {
        let Ok(idx) = idx_s.parse::<usize>() else {
            return answer(ctx, cb, "", false).await;
        };
        // The builder shows the odds in the *bettor's* format (not the host's).
        let fmt = db.get_odds_fmt(cb.from.id).unwrap_or_default();
        let label = {
            let g = predictions.lock().await;
            let Some(prediction) = g.get(&key) else {
                return answer(ctx, cb, i18n::prediction_invalid(lang), true).await;
            };
            if prediction.state != BetState::betting || prediction.ended(chrono::Utc::now().timestamp()) {
                return answer(ctx, cb, i18n::already_closed(lang), true).await;
            }
            match prediction.option_order.get(idx) {
                Some(o) => opt_label(o, &prediction.option_odds(o, fmt)),
                None => return answer(ctx, cb, "", false).await,
            }
        };
        // Head the board with the tapper's name in a group so members can tell
        // whose owner-locked board it is.
        let text = board_header(chat_id, &full_name(&cb.from), &i18n::prediction_build(lang, &label, "0"));
        let rows = prediction_builder_rows(lang, &key, idx, cb.from.id, 0);
        return match tg::send_with_buttons_reply(ctx, chat_id, msg_id, &text, &rows).await {
            Ok(_) => answer(ctx, cb, "", false).await,
            Err(e) => {
                eprintln!("[gamble] stake board post failed (chat {chat_id}): {e:?}");
                answer(ctx, cb, i18n::bet_failed(lang), true).await
            }
        };
    }

    // settle: gamble:<outcome>  (host only, after close). Validate, then hand off
    // to the shared `finish_settle` (also used by the menu-list settle flow).
    let outcome = rest;
    {
        let g = predictions.lock().await;
        let Some(prediction) = g.get(&key) else {
            return answer(ctx, cb, i18n::prediction_invalid(lang), true).await;
        };
        if prediction.host != cb.from.id {
            return answer(ctx, cb, i18n::not_host(lang), true).await;
        }
        if prediction.state != BetState::closed {
            return answer(ctx, cb, i18n::not_closed_yet(lang), true).await;
        }
    }
    if !finish_settle(ctx, &key, outcome).await {
        return answer(ctx, cb, i18n::prediction_invalid(lang), true).await;
    }
    answer(ctx, cb, i18n::settle_success(lang), false).await
}

/// Settle the prediction at `key` (`chat:msg`) for `outcome` (`$draw$` or an
/// option name): pay winners, drop it from the DB + in-memory map, strip the
/// card's now-dead buttons, unpin it, and post the result (header + top-10
/// winners, in the host's locale) as a reply to the card. The caller must have
/// validated host + closed state. Returns `false` if the prediction was already
/// gone (raced away). Shared by the card settle buttons and the `pstl:` menu flow.
async fn finish_settle(ctx: &Context, key: &str, outcome: &str) -> bool {
    let predictions = predictions(ctx);
    let db = db(ctx);
    let Some((chat_s, msg_s)) = key.split_once(':') else {
        return false;
    };
    let (Ok(chat_id), Ok(msg_id)) = (chat_s.parse::<i64>(), msg_s.parse::<i64>()) else {
        return false;
    };
    let (outputs, result_text) = {
        let mut g = predictions.lock().await;
        let Some(prediction) = g.get_mut(key) else {
            return false;
        };
        let (outputs, header) = prediction.settle(outcome);
        // A terminal state → `save_prediction` drops the prediction's rows from the DB.
        if let Err(err) = db.save_prediction(prediction) {
            eprintln!("save_prediction(settle) error: {err}");
        }
        // Bounded readout: header + at most the top 10 winners (or a break-even
        // note for a one-sided settle).
        let result_text = format!("{header}{}", prediction.winners_readout(10));
        // Drop the settled prediction from the in-memory map too.
        g.remove(key);
        (outputs, result_text)
    };
    for (user, win) in &outputs {
        if *win > 0 {
            // Log a failed winner payout instead of silently swallowing it.
            if let Err(e) = db.force_change(*user, win.saturating_mul(COIN)) {
                eprintln!("gamble settle credit failed (user {user}, +{win} coins): {e}");
            }
        }
    }
    // Strip the now-dead settle buttons from the card (keyboard only — the card
    // text is left as-is), then post the result as a reply to it (falling back to
    // a loose message if the card is gone).
    let _ = tg::clear_buttons(ctx, chat_id, msg_id).await;
    // Undo the pin set when the card was created (best-effort; no-op if it was
    // never pinned — e.g. a DM prediction or the bot lacks the right).
    if is_group_chat(chat_id) {
        let _ = tg::unpin_message(ctx, chat_id, msg_id).await;
    }
    let _ = tg::send_text_reply(ctx, chat_id, msg_id, &result_text).await;
    true
}

/// In-group stake-builder keyboard for a `/predict` option at `total` whole coins.
/// The prediction ref + owner + total all ride in the callback data
/// (`gsz/gsc/gsp:<chat>:<msg>:<idx>:<owner>:<total>`) — no server-side per-user
/// state. `owner` locks the board to the tapper; the Dismiss row (`bx:<owner>`)
/// lets them delete it.
fn prediction_builder_rows(lang: Lang, key: &str, idx: usize, owner: i64, total: i64) -> Vec<Vec<(String, String)>> {
    let add: Vec<(String, String)> = crate::core::prediction::STAKE_AMOUNTS
        .iter()
        .map(|p| (format!("+{p}"), format!("gsz:{key}:{idx}:{owner}:{}", total + p)))
        .collect();
    // Confirm places the bet straight away (no separate confirmation screen); the
    // board is its own group message, so Dismiss rides on the same action row.
    vec![
        add,
        vec![
            (i18n::bet_btn_confirm(lang).to_string(), format!("gsp:{key}:{idx}:{owner}:{total}")),
            (i18n::bet_btn_clear(lang).to_string(), format!("gsz:{key}:{idx}:{owner}:0")),
            (i18n::bet_btn_dismiss(lang).to_string(), format!("bx:{owner}")),
        ],
    ]
}

/// Parse `<chat>:<msg>:<idx>:<owner>:<total>` → (prediction key `"chat:msg"`, option
/// idx, board owner, total).
fn parse_prediction_draft(rest: &str) -> Option<(String, usize, i64, i64)> {
    let parts: Vec<&str> = rest.split(':').collect();
    let [chat, msg, idx, owner, total] = parts.as_slice() else {
        return None;
    };
    Some((format!("{chat}:{msg}"), idx.parse().ok()?, owner.parse().ok()?, total.parse().ok()?))
}

/// The option's `(name, odds-in-`fmt`)` for `(key, idx)` if the prediction is still
/// open; else the toast to show (`prediction_invalid`/`already_closed`). The odds are
/// the pari-mutuel multiplier rendered in the *bettor's* format for the DM builder.
async fn prediction_option(
    ctx: &Context,
    lang: Lang,
    key: &str,
    idx: usize,
    fmt: crate::core::types::OddsFormat,
) -> Result<(String, String), &'static str> {
    let predictions = predictions(ctx);
    let g = predictions.lock().await;
    let Some(prediction) = g.get(key) else {
        return Err(i18n::prediction_invalid(lang));
    };
    if prediction.state != BetState::betting || prediction.ended(chrono::Utc::now().timestamp()) {
        return Err(i18n::already_closed(lang));
    }
    let name = prediction.option_order.get(idx).cloned().ok_or_else(|| i18n::prediction_invalid(lang))?;
    let odds = prediction.option_odds(&name, fmt);
    Ok((name, odds))
}

/// `[name (odds)]` — the option label shown in the DM stake builder/confirm.
fn opt_label(name: &str, odds: &str) -> String {
    format!("{name} ({odds})")
}

/// `gsz:…` — re-render the DM builder at the accumulated total.
async fn handle_prediction_size(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some((key, idx, owner, total)) = parse_prediction_draft(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    if owner != cb.from.id {
        return answer(ctx, cb, i18n::not_your_bet(lang), true).await;
    }
    let fmt = db(ctx).get_odds_fmt(cb.from.id).unwrap_or_default();
    let (opt, odds) = match prediction_option(ctx, lang, &key, idx, fmt).await {
        Ok(p) => p,
        Err(t) => return answer(ctx, cb, t, true).await,
    };
    if let Some(m) = &cb.message {
        let body = i18n::prediction_build(lang, &opt_label(&opt, &odds), &total.max(0).to_string());
        let text = board_header(m.chat.get_id(), &full_name(&cb.from), &body);
        let _ = tg::edit_with_buttons(
            ctx,
            m.chat.get_id(),
            m.message_id,
            &text,
            &prediction_builder_rows(lang, &key, idx, owner, total),
        )
        .await;
    }
    answer(ctx, cb, "", false).await
}

/// Best-effort refund of `units` micro-coins to `user`, logging (not swallowing)
/// a failure — used when a self-host stake debit can't be turned into a placed
/// bet (prediction gone / option missing / stake rejected) after the debit succeeded.
fn log_refund(db: &crate::database::Database, user: i64, units: i64) {
    if let Err(e) = db.force_change(user, units) {
        eprintln!("prediction stake refund failed (user {user}, +{units}): {e}");
    }
}

/// `gsp:…` — fired by the board's **Confirm**: the only money-moving step. Debit,
/// `prediction.stake`, edit the group board with new totals, report the result in a
/// popup alert, and announce in the group.
async fn handle_prediction_place(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some((key, idx, owner, total)) = parse_prediction_draft(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    if owner != cb.from.id {
        return answer(ctx, cb, i18n::not_your_bet(lang), true).await;
    }
    // Guard the whole-coin → micro-coin conversion (caps at MAX_COINS, rejects
    // overflow and non-positive): `total` rides in the callback data, so a
    // crafted value must not be able to wrap i64 and mint/destroy coins.
    let Some(units) = to_micro(total) else {
        return answer(ctx, cb, i18n::bad_stake(lang), true).await;
    };
    let db = db(ctx);
    if !db.balance_change(cb.from.id, -units).unwrap_or(false) {
        return answer(ctx, cb, i18n::not_enough_money(lang), true).await;
    }
    let predictions = predictions(ctx);
    let placed = {
        let mut g = predictions.lock().await;
        let Some(prediction) = g.get_mut(&key) else {
            log_refund(&db, cb.from.id, units);
            return answer(ctx, cb, i18n::prediction_invalid(lang), true).await;
        };
        // Reject (and refund) a place on a closed/ended prediction — the builder DM may
        // have been opened before the deadline passed.
        if prediction.state != BetState::betting || prediction.ended(chrono::Utc::now().timestamp()) {
            log_refund(&db, cb.from.id, units);
            return answer(ctx, cb, i18n::already_closed(lang), true).await;
        }
        let Some(option) = prediction.option_order.get(idx).cloned() else {
            log_refund(&db, cb.from.id, units);
            return answer(ctx, cb, i18n::bet_failed(lang), true).await;
        };
        if !prediction.stake(cb.from.id, &option, total, &full_name(&cb.from)) {
            log_refund(&db, cb.from.id, units);
            return answer(ctx, cb, i18n::bet_failed(lang), true).await;
        }
        if let Err(err) = db.save_prediction(prediction) {
            eprintln!("save_prediction(stake) error: {err}");
        }
        (option, prediction.get_text(), prediction.get_buttons())
    };
    let (option, board_text, board_rows) = placed;
    // Edit the group board with the new totals and announce the bet there — as a
    // reply to the board card (like match betting replies to its prediction card), so
    // the announcement is threaded under the prediction it belongs to. Falls back
    // to a loose message if the card is gone (`send_text_reply` sets
    // `allow_sending_without_reply`).
    if let Some((c, m)) = key.split_once(':') {
        if let (Ok(chat), Ok(msg)) = (c.parse::<i64>(), m.parse::<i64>()) {
            let _ = tg::edit_with_buttons(ctx, chat, msg, &board_text, &board_rows).await;
            let announce = i18n::prediction_announce(lang, &full_name(&cb.from), &total.to_string(), &option);
            let _ = tg::send_text_reply(ctx, chat, msg, &announce).await;
        }
    }
    // The stake board is its own message — delete it now that the bet is placed
    // (the result rides in the announcement reply under the prediction card above).
    if let Some(m) = &cb.message {
        let _ = tg::delete_message(ctx, m.chat.get_id(), m.message_id).await;
    }
    answer(ctx, cb, i18n::bet_success(lang), true).await
}

/// `bx:<owner>` — dismiss an in-group personal stake board by deleting it.
/// Owner-locked: only the user the board was opened for can delete it (anyone
/// else sees a toast). Shared by match-bet and self-host `/predict` boards.
async fn handle_board_dismiss(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Ok(owner) = rest.parse::<i64>() else {
        return answer(ctx, cb, "", false).await;
    };
    if owner != cb.from.id {
        return answer(ctx, cb, i18n::not_your_bet(lang), true).await;
    }
    if let Some(m) = &cb.message {
        let _ = tg::delete_message(ctx, m.chat.get_id(), m.message_id).await;
    }
    answer(ctx, cb, "", false).await
}

/// Resolve the locale for a callback presser: their saved choice if any, else
/// the Telegram-reported language.
fn cb_lang(ctx: &Context, cb: &CallbackQuery) -> Lang {
    db(ctx)
        .get_lang(cb.from.id)
        .ok()
        .flatten()
        .unwrap_or_else(|| Lang::from_user(&cb.from))
}

/// `setlang:<store_code>` — persist the chosen locale and swap the picker for
/// the Xaliah main menu in place.
async fn handle_set_lang(
    ctx: &Context,
    cb: &CallbackQuery,
    rest: &str,
) -> Result<(), telexide::Error> {
    let Some(lang) = Lang::from_store_code(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    let db = db(ctx);
    if db.set_lang(cb.from.id, lang).is_err() {
        return answer(ctx, cb, i18n::db_error(lang), true).await;
    }
    if let Some(message) = cb.message.clone() {
        let chat = message.chat.get_id();
        let in_group = is_group_chat(chat);
        // Private chats then pick a timezone — always, so `/language` doubles as
        // "change my timezone too" (first-timers and returning users alike).
        // Groups skip it (shared message) and go straight to the menu.
        if in_group {
            let available = true;
            let _ = tg::edit_with_buttons(
                ctx,
                chat,
                message.message_id,
                &menu::menu_text(lang, &full_name(&cb.from)),
                &menu::main_menu_rows(lang, available, in_group, 0),
            )
            .await;
        } else {
            let _ = tg::edit_with_buttons(
                ctx,
                chat,
                message.message_id,
                i18n::choose_timezone(lang),
                &menu::tz_picker_rows(db.get_tz(cb.from.id).ok().flatten(), false),
            )
            .await;
        }
    }
    answer(ctx, cb, "", false).await
}

/// `settz:<minutes>` — save the picked UTC offset and open the main menu.
async fn handle_set_tz(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let Ok(minutes) = rest.parse::<i64>() else {
        return answer(ctx, cb, "", false).await;
    };
    let db = db(ctx);
    let lang = cb_lang(ctx, cb);
    if db.set_tz(cb.from.id, minutes).is_err() {
        return answer(ctx, cb, i18n::db_error(lang), true).await;
    }
    if let Some(message) = cb.message.clone() {
        let chat = message.chat.get_id();
        let in_group = is_group_chat(chat);
        let available = in_group || db.checkin_available(cb.from.id).unwrap_or(true);
        let pending = predict::pending_settle_count(ctx, cb.from.id).await;
        let _ = tg::edit_with_buttons(
            ctx,
            chat,
            message.message_id,
            &menu::menu_text(lang, &full_name(&cb.from)),
            &menu::main_menu_rows(lang, available, in_group, pending),
        )
        .await;
    }
    answer(ctx, cb, "", false).await
}

/// `setfmt:<code>` — settings-variant odds pick: persist the chosen format and
/// return to the `/settings` hub in place (uniform with `slang:`/`stz:`).
async fn handle_set_fmt(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let fmt = crate::core::types::OddsFormat::from_store_code(rest);
    let db = db(ctx);
    let lang = cb_lang(ctx, cb);
    if db.set_odds_fmt(cb.from.id, fmt).is_err() {
        return answer(ctx, cb, i18n::db_error(lang), true).await;
    }
    if let Some(message) = cb.message.clone() {
        let _ = tg::edit_with_buttons(
            ctx,
            message.chat.get_id(),
            message.message_id,
            i18n::settings_title(lang),
            &menu::settings_rows(lang),
        )
        .await;
    }
    answer(ctx, cb, "", false).await
}

/// `cfg:home` — re-render the `/settings` hub in place (from a sub-picker's Back).
async fn handle_cfg_home(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    if let Some(message) = cb.message.clone() {
        let _ = tg::edit_with_buttons(
            ctx,
            message.chat.get_id(),
            message.message_id,
            i18n::settings_title(lang),
            &menu::settings_rows(lang),
        )
        .await;
    }
    answer(ctx, cb, "", false).await
}

/// `cfg:lang` — open the language picker from the `/settings` hub, ✅-marking the
/// current locale. The picker's `slang:` buttons persist + return to the hub.
async fn handle_cfg_lang(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    if let Some(message) = cb.message.clone() {
        let _ = tg::edit_with_buttons(
            ctx,
            message.chat.get_id(),
            message.message_id,
            i18n::CHOOSE_LANGUAGE,
            &menu::lang_picker_rows(Some(lang), true),
        )
        .await;
    }
    answer(ctx, cb, "", false).await
}

/// `cfg:tz` — open the timezone picker from the `/settings` hub, ✅-marking the
/// current offset. The picker's `stz:` buttons persist + return to the hub.
async fn handle_cfg_tz(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let current = db(ctx).get_tz(cb.from.id).ok().flatten();
    if let Some(message) = cb.message.clone() {
        let _ = tg::edit_with_buttons(
            ctx,
            message.chat.get_id(),
            message.message_id,
            i18n::choose_timezone(lang),
            &menu::tz_picker_rows(current, true),
        )
        .await;
    }
    answer(ctx, cb, "", false).await
}

/// `slang:<store_code>` — settings-variant language pick: persist the locale and
/// return to the `/settings` hub in place (re-rendered in the *new* locale).
async fn handle_settings_lang(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let Some(lang) = Lang::from_store_code(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    let db = db(ctx);
    if db.set_lang(cb.from.id, lang).is_err() {
        return answer(ctx, cb, i18n::db_error(lang), true).await;
    }
    if let Some(message) = cb.message.clone() {
        let _ = tg::edit_with_buttons(
            ctx,
            message.chat.get_id(),
            message.message_id,
            i18n::settings_title(lang),
            &menu::settings_rows(lang),
        )
        .await;
    }
    answer(ctx, cb, "", false).await
}

/// `stz:<minutes>` — settings-variant timezone pick: persist the UTC offset and
/// return to the `/settings` hub in place.
async fn handle_settings_tz(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let Ok(minutes) = rest.parse::<i64>() else {
        return answer(ctx, cb, "", false).await;
    };
    let db = db(ctx);
    let lang = cb_lang(ctx, cb);
    if db.set_tz(cb.from.id, minutes).is_err() {
        return answer(ctx, cb, i18n::db_error(lang), true).await;
    }
    if let Some(message) = cb.message.clone() {
        let _ = tg::edit_with_buttons(
            ctx,
            message.chat.get_id(),
            message.message_id,
            i18n::settings_title(lang),
            &menu::settings_rows(lang),
        )
        .await;
    }
    answer(ctx, cb, "", false).await
}

/// `cfg:odds` — open the odds-format picker from the `/settings` hub, ✅-marking
/// the current format.
async fn handle_cfg_odds(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let current = db(ctx).get_odds_fmt(cb.from.id).unwrap_or_default();
    if let Some(message) = cb.message.clone() {
        let _ = tg::edit_with_buttons(
            ctx,
            message.chat.get_id(),
            message.message_id,
            i18n::btn_odds(lang),
            &menu::odds_picker_rows(lang, current),
        )
        .await;
    }
    answer(ctx, cb, "", false).await
}

/// Group-add referral bind: the first time a **brand-new** user taps *any* button
/// inside a group, bind them to whoever added the bot there and pay both sides.
/// No-op outside groups, when there's no recorded adder, or for existing users
/// (`set_referrer_if_new` only inserts a fresh row — so no farming).
/// `menu:checkin` — grant the daily reward; result shown as an alert. In a
/// private chat the menu refreshes so the now-spent button drops off; in a group
/// the menu is shared, so the button stays for everyone else to claim.
async fn handle_menu_checkin(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let db = db(ctx);

    // (Group-add referral binding now happens for ANY button press in a group,
    // up in `on_callback` before dispatch — see `maybe_bind_group_referral`.)
    match db.try_checkin(cb.from.id, crate::commands::checkin::CHECKIN_REWARD) {
        Ok(true) => {
            if let Some(message) = cb.message.clone() {
                if !is_group_chat(message.chat.get_id()) {
                    let pending = predict::pending_settle_count(ctx, cb.from.id).await;
                    let _ = tg::edit_with_buttons(
                        ctx,
                        message.chat.get_id(),
                        message.message_id,
                        &menu::menu_text(lang, &full_name(&cb.from)),
                        &menu::main_menu_rows(lang, false, false, pending),
                    )
                    .await;
                }
            }
            let amt = fmt_coins(crate::commands::checkin::CHECKIN_REWARD);
            answer(ctx, cb, &i18n::checkin_done(lang, &amt), true).await
        }
        Ok(false) => {
            let t = crate::commands::checkin::time_until_reset();
            answer(ctx, cb, &i18n::checkin_already(lang, &t), true).await
        }
        Err(_) => answer(ctx, cb, i18n::db_error(lang), true).await,
    }
}

/// `menu:invite` — show a chooser: copyable link / forwardable message / QR.
/// The actual artefact is generated only when the user picks one.
async fn handle_menu_invite(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    answer(ctx, cb, "", false).await?;
    // Show the caller's referral count above the format chooser — edited in
    // place over the current message (home menu, or a previous invite result).
    let count = db(ctx).count_referrals(cb.from.id).unwrap_or(0);
    let text = format!(
        "{}\n\n{}",
        i18n::invite_count(lang, &count.to_string()),
        i18n::invite_how(lang)
    );
    let rows = vec![
        vec![(i18n::btn_invite_link(lang).to_string(), menu::INVITE_LINK.to_string())],
        vec![(i18n::btn_invite_fwd(lang).to_string(), menu::INVITE_FWD.to_string())],
        vec![(i18n::btn_invite_qr(lang).to_string(), menu::INVITE_QR.to_string())],
        vec![(i18n::bet_btn_back(lang).to_string(), menu::MENU_HOME.to_string())],
    ];
    let _ = tg::edit_with_buttons(ctx, message.chat.get_id(), message.message_id, &text, &rows).await;
    Ok(())
}

/// `menu:home` — re-render the main menu in place (used by the invite chooser's
/// Back button). Private-chat flow, so `is_group` is false.
async fn handle_menu_home(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    answer(ctx, cb, "", false).await?;
    let chat = message.chat.get_id();
    let in_group = is_group_chat(chat);
    let available = in_group || db(ctx).checkin_available(cb.from.id).unwrap_or(true);
    let pending = predict::pending_settle_count(ctx, cb.from.id).await;
    let _ = tg::edit_with_buttons(
        ctx,
        chat,
        message.message_id,
        &menu::menu_text(lang, &full_name(&cb.from)),
        &menu::main_menu_rows(lang, available, in_group, pending),
    )
    .await;
    Ok(())
}

/// Prefix for the menu-driven prediction settle flow (host-facing, in-place
/// double-confirm). Each step edits the menu message: `pstl:p:<chat>:<msg>` →
/// outcome picker; `pstl:o:<chat>:<msg>:<oc>` → confirm 1/2; `pstl:c:…:<oc>` →
/// confirm 2/2; `pstl:g:…:<oc>` → settle. `<oc>` is an option index, or `d` for
/// void/draw. Settling here never needs a message link, so it works in any chat
/// type (incl. plain groups where `t.me/c` deep links don't exist).
const PSTL: &str = "pstl:";

/// Build the host's open-prediction settle list (title + rows) for `menu:settle`.
/// Each entry is a `pstl:p:` button (no deep link), 🔴 ready / 🟢 still running.
fn settle_list_view(lang: Lang, preds: &[predict::HostPred]) -> (String, Vec<tg::Row>) {
    let mut rows: Vec<tg::Row> = preds
        .iter()
        .map(|p| {
            // 🔴 ready to settle (deadline passed / none), 🟢 still running.
            let mark = if p.ready { "🔴" } else { "🟢" };
            let title = truncate_label(&p.question, 40);
            vec![(format!("{mark} {title}"), format!("{PSTL}p:{}:{}", p.chat_id, p.msg_id))]
        })
        .collect();
    rows.push(vec![(i18n::bet_btn_back(lang).to_string(), menu::MENU_HOME.to_string())]);
    (i18n::settle_list_title(lang).to_string(), rows)
}

/// `menu:settle` — edit the menu in place into the host's list of open
/// predictions, each a `pstl:p:` button that drives the settle flow **in place**
/// (pick outcome → confirm 1/2 → 2/2 → settle), plus `[⬅ Back]`. Private-only
/// (the button isn't shown in groups). Works for every chat type since it never
/// relies on a `t.me/c` message link to reach the card.
async fn handle_menu_settle(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    let chat = message.chat.get_id();
    let preds = predict::host_open_predictions(ctx, cb.from.id).await;
    // Raced to empty (all settled since the badge rendered) — just drop back to the
    // home menu in place (handles its own ack), no "nothing to settle" notice.
    if preds.is_empty() {
        return handle_menu_home(ctx, cb).await;
    }
    answer(ctx, cb, "", false).await?;
    let (body, rows) = settle_list_view(lang, &preds);
    let _ = tg::edit_with_buttons(ctx, chat, message.message_id, &body, &rows).await;
    Ok(())
}

/// Settle-flow outcome code → the string `Prediction::settle` expects
/// (`d` = void/draw, else an option index into `option_order`).
fn resolve_outcome(p: &crate::core::prediction::Prediction, oc: &str) -> Option<String> {
    if oc == "d" {
        return Some("$draw$".to_string());
    }
    p.option_order.get(oc.parse::<usize>().ok()?).cloned()
}

/// Display label for a settle-flow outcome code (option name, or the localized
/// void label for `d`).
fn outcome_label(p: &crate::core::prediction::Prediction, oc: &str, lang: Lang) -> Option<String> {
    if oc == "d" {
        return Some(i18n::void_label(lang).to_string());
    }
    p.option_order.get(oc.parse::<usize>().ok()?).cloned()
}

/// `pstl:` — the host-facing, in-place double-confirm settle flow opened from the
/// `menu:settle` list (see `PSTL`). Each step re-reads the in-memory prediction
/// and re-checks host + deadline, so a stale press fails safe.
async fn handle_predict_settle_cb(
    ctx: &Context,
    cb: &CallbackQuery,
    rest: &str,
) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    let mchat = message.chat.get_id();
    let mid = message.message_id;
    let predictions = predictions(ctx);
    let db = db(ctx);
    let now = chrono::Utc::now().timestamp();
    let (action, tail) = rest.split_once(':').unwrap_or((rest, ""));
    match action {
        // Picked a prediction → its management screen: the outcome picker if it's
        // settleable, otherwise a "still open" note. Both offer `[♻️ Repost card]`
        // (re-posts a fresh copy if the original was deleted from the chat).
        "p" => {
            let (title, rows) = {
                let g = predictions.lock().await;
                let Some(p) = g.get(tail) else {
                    return answer(ctx, cb, i18n::prediction_invalid(lang), true).await;
                };
                if p.host != cb.from.id {
                    return answer(ctx, cb, i18n::not_host(lang), true).await;
                }
                let key = tail;
                let mut rows: Vec<tg::Row> = Vec::new();
                // Settleable = deadline passed/none, or the host is the only staker
                // (force-settle early); otherwise betting is still live.
                let title = if p.can_close(now) || p.only_host_staked() {
                    for (i, o) in p.option_order.iter().enumerate() {
                        rows.push(vec![(o.clone(), format!("{PSTL}o:{key}:{i}"))]);
                    }
                    rows.push(vec![(i18n::void_label(lang).to_string(), format!("{PSTL}o:{key}:d"))]);
                    i18n::settle_pick_outcome(lang).to_string()
                } else {
                    let when = crate::commands::util::fmt_local_time(p.ends_at, p.tz_offset)
                        .unwrap_or_default();
                    i18n::settle_not_ready(lang, &when)
                };
                rows.push(vec![(i18n::repost_card_btn(lang).to_string(), format!("{PSTL}r:{key}"))]);
                rows.push(vec![(i18n::bet_btn_back(lang).to_string(), menu::MENU_SETTLE.to_string())]);
                (title, rows)
            };
            let _ = tg::edit_with_buttons(ctx, mchat, mid, &title, &rows).await;
        }
        // Repost the card to its origin chat (shared `repost_card` helper), then
        // refresh the settle list in place so its row callbacks carry the new msg id.
        "r" => {
            let key = tail;
            {
                let g = predictions.lock().await;
                let Some(p) = g.get(key) else {
                    return answer(ctx, cb, i18n::prediction_invalid(lang), true).await;
                };
                if p.host != cb.from.id {
                    return answer(ctx, cb, i18n::not_host(lang), true).await;
                }
            }
            if !predict::repost_card(ctx, key).await {
                return answer(ctx, cb, i18n::predict_post_failed(lang), true).await;
            }
            answer(ctx, cb, i18n::card_reposted(lang), false).await?;
            let preds = predict::host_open_predictions(ctx, cb.from.id).await;
            if preds.is_empty() {
                return handle_menu_home(ctx, cb).await;
            }
            let (body, rows) = settle_list_view(lang, &preds);
            let _ = tg::edit_with_buttons(ctx, mchat, mid, &body, &rows).await;
            return Ok(());
        }
        // Picked an outcome → confirm 1/2; or first confirm → confirm 2/2.
        "o" | "c" => {
            let Some((key, oc)) = tail.rsplit_once(':') else {
                return answer(ctx, cb, "", false).await;
            };
            let label = {
                let g = predictions.lock().await;
                let Some(p) = g.get(key) else {
                    return answer(ctx, cb, i18n::prediction_invalid(lang), true).await;
                };
                if p.host != cb.from.id {
                    return answer(ctx, cb, i18n::not_host(lang), true).await;
                }
                match outcome_label(p, oc, lang) {
                    Some(l) => l,
                    None => return answer(ctx, cb, i18n::prediction_invalid(lang), true).await,
                }
            };
            let (text, rows) = if action == "o" {
                (
                    i18n::settle_confirm1(lang, &label),
                    vec![
                        vec![(i18n::settle_btn_continue(lang).to_string(), format!("{PSTL}c:{key}:{oc}"))],
                        vec![(i18n::bet_btn_back(lang).to_string(), format!("{PSTL}p:{key}"))],
                    ],
                )
            } else {
                (
                    i18n::settle_confirm2(lang, &label),
                    vec![
                        vec![(i18n::settle_btn_settle(lang).to_string(), format!("{PSTL}g:{key}:{oc}"))],
                        vec![(i18n::bet_btn_back(lang).to_string(), format!("{PSTL}p:{key}"))],
                    ],
                )
            };
            let _ = tg::edit_with_buttons(ctx, mchat, mid, &text, &rows).await;
        }
        // Final confirm → settle (closing the pool first if betting's still open).
        "g" => {
            let Some((key, oc)) = tail.rsplit_once(':') else {
                return answer(ctx, cb, "", false).await;
            };
            let outcome = {
                let mut g = predictions.lock().await;
                let Some(p) = g.get_mut(key) else {
                    return answer(ctx, cb, i18n::prediction_invalid(lang), true).await;
                };
                if p.host != cb.from.id {
                    return answer(ctx, cb, i18n::not_host(lang), true).await;
                }
                if !p.can_close(now) && !p.only_host_staked() {
                    let when = crate::commands::util::fmt_local_time(p.ends_at, p.tz_offset)
                        .unwrap_or_default();
                    return answer(ctx, cb, &i18n::close_before_deadline(lang, &when), true).await;
                }
                let Some(outcome) = resolve_outcome(p, oc) else {
                    return answer(ctx, cb, i18n::prediction_invalid(lang), true).await;
                };
                // `settle` requires a closed pool — close it now if still betting.
                if p.state == BetState::betting {
                    p.close();
                    if let Err(err) = db.save_prediction(p) {
                        eprintln!("save_prediction(menu close) error: {err}");
                    }
                }
                outcome
            };
            if !finish_settle(ctx, key, &outcome).await {
                return answer(ctx, cb, i18n::prediction_invalid(lang), true).await;
            }
            // Refresh the settle list in place — or drop to the home menu if nothing's left.
            let preds = predict::host_open_predictions(ctx, cb.from.id).await;
            if preds.is_empty() {
                return handle_menu_home(ctx, cb).await;
            }
            answer(ctx, cb, i18n::settle_success(lang), false).await?;
            let (body, rows) = settle_list_view(lang, &preds);
            let _ = tg::edit_with_buttons(ctx, mchat, mid, &body, &rows).await;
            return Ok(());
        }
        _ => {}
    }
    answer(ctx, cb, "", false).await
}

/// Trim a button label to at most `max` chars (char-safe), adding an ellipsis when cut.
fn truncate_label(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

/// This user's referral deep link, from the cached bot username.
fn referral_link_of(ctx: &Context, user_id: i64) -> String {
    let username = ctx
        .data
        .read()
        .get::<crate::bot::BotUsernameKey>()
        .cloned()
        .unwrap_or_default();
    menu::referral_link(&username, user_id)
}

/// `inv:link` — the referral link in a tap-to-copy `<code>` span, posted as a
/// **new** message so the format chooser stays put (the user can pick another
/// format) and the link stands on its own.
async fn handle_invite_link(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    answer(ctx, cb, "", false).await?;
    let link = referral_link_of(ctx, cb.from.id);
    let _ = tg::send_html(ctx, message.chat.get_id(), &i18n::invite_copy(lang, &link)).await;
    Ok(())
}

/// `inv:fwd` — a forward-safe message: the link is baked into the **text** (so a
/// forward keeps it), plus a `[🎮 Play now]` URL button for tapping in place
/// (inline keyboards are stripped on forward, hence the link also lives in text).
/// Posted as a **new** message — a clean standalone the user can forward, with
/// the chooser left intact above it.
async fn handle_invite_fwd(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    answer(ctx, cb, "", false).await?;
    let link = referral_link_of(ctx, cb.from.id);
    let rows = vec![vec![(i18n::btn_join(lang).to_string(), link.clone())]];
    let _ = tg::send_with_buttons(ctx, message.chat.get_id(), &i18n::invite_forward(lang, &link), &rows).await;
    Ok(())
}

/// `inv:qr` — a QR photo, caption = the bare link, no keyboard. The QR is
/// generated locally and its Telegram `file_id` cached per user, so repeat taps
/// re-send by id with no regeneration/upload.
async fn handle_invite_qr(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    answer(ctx, cb, "", false).await?;
    let chat_id = message.chat.get_id();
    let link = referral_link_of(ctx, cb.from.id);
    // Caption is just the link (count is shown on the chooser, not here); no
    // keyboard — the link rides in the caption and the QR image. The [Play]
    // button lives on the forwardable message instead.
    let text = link.clone();
    let rows: &[tg::Row] = &[];

    let token = bot_token(ctx);
    let cached = qr_cache().lock().get(&cb.from.id).cloned();
    let sent = if let Some(file_id) = cached {
        tg::send_photo_id(&token, chat_id, &file_id, &text, rows).await.is_ok()
    } else {
        false
    };
    if !sent {
        match qrcode_generator::to_png_to_vec(&link, qrcode_generator::QrCodeEcc::Medium, 512) {
            Ok(png) => match tg::send_photo_bytes(&token, chat_id, png, &text, rows).await {
                Ok(Some(file_id)) => {
                    qr_cache().lock().insert(cb.from.id, file_id);
                }
                Ok(None) => {} // sent, but couldn't read the id — re-upload next time
                Err(_) => {
                    crate::commands::util::send_text(ctx, chat_id, text.clone()).await?;
                }
            },
            Err(_) => {
                crate::commands::util::send_text(ctx, chat_id, text.clone()).await?;
            }
        }
    }
    Ok(())
}

/// Per-user cache of the referral QR's Telegram `file_id`. The QR is a function
/// of the user's (immutable) referral link, so the id stays valid indefinitely;
/// a process restart just costs one re-upload per user. In-memory on purpose.
fn qr_cache() -> &'static parking_lot::Mutex<HashMap<i64, String>> {
    static CACHE: std::sync::OnceLock<parking_lot::Mutex<HashMap<i64, String>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| parking_lot::Mutex::new(HashMap::new()))
}

/// `menu:balance` — edit the menu in place into the presser's balance + open
/// positions. Private-chat only (the button is hidden in groups), so the
/// balance is shown.
async fn handle_menu_balance(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    answer(ctx, cb, "", false).await?;
    let show_balance = !is_group_chat(message.chat.get_id());
    let text = assets::assets_text(ctx, lang, &cb.from, show_balance).await;
    let rows = vec![vec![(i18n::bet_btn_back(lang).to_string(), menu::MENU_HOME.to_string())]];
    let _ = tg::edit_with_buttons(ctx, message.chat.get_id(), message.message_id, &text, &rows).await;
    Ok(())
}

/// `menu:markets` — post the market brief as a fresh message, leaving the menu
/// in place.
async fn handle_menu_markets(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    answer(ctx, cb, "", false).await?;
    let chat = message.chat.get_id();
    let tz = if is_group_chat(chat) {
        0
    } else {
        db(ctx).get_tz(cb.from.id).ok().flatten().unwrap_or(0)
    };
    let fmt = db(ctx).get_odds_fmt(cb.from.id).unwrap_or_default();
    let (text, mut rows) = markets::brief(lang, tz, fmt).await;
    rows.push(vec![(i18n::bet_btn_back(lang).to_string(), menu::MENU_HOME.to_string())]);
    let _ = tg::edit_with_buttons(ctx, chat, message.message_id, &text, &rows).await;
    Ok(())
}

/// `menu:rule` — edit the menu in place into the "how to earn coins" rules.
async fn handle_menu_rule(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    use crate::commands::checkin::CHECKIN_REWARD;
    use crate::commands::referral::REFERRAL_REWARD;
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    answer(ctx, cb, "", false).await?;
    let text = i18n::rules_text(lang, &fmt_coins(CHECKIN_REWARD), &fmt_coins(REFERRAL_REWARD));
    let rows = vec![vec![(i18n::bet_btn_back(lang).to_string(), menu::MENU_HOME.to_string())]];
    let _ = tg::edit_with_buttons(ctx, message.chat.get_id(), message.message_id, &text, &rows).await;
    Ok(())
}

/// `menu:predict` — the group home page's "create prediction" button: open the
/// `/predict` builder for the presser (same flow as the `/predict` command), DM'd
/// to them, with a toast pointing there (or "DM me first" if they have no DM open).
async fn handle_menu_predict(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    if predict::open_draft(ctx, &cb.from, message.chat.get_id()).await {
        answer(ctx, cb, i18n::predict_check_dm(lang), false).await
    } else {
        answer(ctx, cb, i18n::bet_dm_first(lang), true).await
    }
}

/// `menu:repost` — the group-only `[♻️ Restore card]` button. Reposts the
/// presser's prediction card **into this group** (the chat the button was tapped
/// in is provably reachable — the bot just received this callback there), which
/// is exactly the recovery for a card deleted from the chat. Picks the host's
/// most-recent open prediction in this group; a toast if they host none here.
async fn handle_menu_repost(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    let chat = message.chat.get_id();
    let Some(key) = predict::host_prediction_in_chat(ctx, cb.from.id, chat).await else {
        return answer(ctx, cb, i18n::nothing_to_restore(lang), true).await;
    };
    if predict::repost_card(ctx, &key).await {
        answer(ctx, cb, i18n::card_reposted(lang), false).await
    } else {
        answer(ctx, cb, i18n::predict_post_failed(lang), true).await
    }
}

async fn handle_sell(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    let chat_id = message.chat.get_id();
    let msg_id = message.message_id;
    let lang = Lang::from_user(&cb.from);
    let db = db(ctx);
    let outcome = match db.consume_sell(chat_id, msg_id, cb.from.id) {
        Ok(o) => o,
        Err(_) => return answer(ctx, cb, i18n::db_error(lang), true).await,
    };
    match outcome {
        OfferOutcome::AlreadyTaken => answer(ctx, cb, i18n::someone_dealt(lang), false).await,
        OfferOutcome::SelfCancelled => {
            tg::delete_message(ctx, chat_id, msg_id).await.ok();
            answer(ctx, cb, i18n::withdrew_sell(lang), false).await
        }
        OfferOutcome::Filled { fruits, price } => {
            let _ = tg::edit_text_only(
                ctx,
                chat_id,
                msg_id,
                &i18n::bought_msg(lang, &cb.from.first_name, &fmt_coins(price), &fruits),
            )
            .await;
            answer(ctx, cb, &i18n::bought_toast(lang, &fruits), true).await
        }
        OfferOutcome::TakerNotEnoughBalance => {
            answer(ctx, cb, i18n::not_enough_money(lang), true).await
        }
        OfferOutcome::TakerFruitFull => answer(ctx, cb, i18n::too_many_fruits(lang), true).await,
        OfferOutcome::TakerMissingFruit(_) => answer(ctx, cb, i18n::system_error(lang), true).await,
    }
}

async fn handle_buy(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    let chat_id = message.chat.get_id();
    let msg_id = message.message_id;
    let lang = Lang::from_user(&cb.from);
    let db = db(ctx);
    let outcome = match db.consume_buy(chat_id, msg_id, cb.from.id) {
        Ok(o) => o,
        Err(_) => return answer(ctx, cb, i18n::db_error(lang), true).await,
    };
    match outcome {
        OfferOutcome::AlreadyTaken => answer(ctx, cb, i18n::someone_dealt(lang), false).await,
        OfferOutcome::SelfCancelled => {
            tg::delete_message(ctx, chat_id, msg_id).await.ok();
            answer(ctx, cb, i18n::withdrew_buy(lang), false).await
        }
        OfferOutcome::Filled { fruits, price } => {
            let _ = tg::edit_text_only(
                ctx,
                chat_id,
                msg_id,
                &i18n::sold_msg(lang, &cb.from.first_name, &fruits, &fmt_coins(price)),
            )
            .await;
            answer(ctx, cb, &i18n::sold_toast(lang, &fmt_coins(price)), true).await
        }
        OfferOutcome::TakerMissingFruit(ch) => {
            answer(ctx, cb, &i18n::you_dont_have(lang, &ch.to_string()), true).await
        }
        OfferOutcome::TakerFruitFull => answer(ctx, cb, i18n::buyer_fruit_full(lang), true).await,
        OfferOutcome::TakerNotEnoughBalance => {
            answer(ctx, cb, i18n::system_error(lang), true).await
        }
    }
}
