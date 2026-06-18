use crate::bot::{DbKey, GamesKey};
use crate::commands::util::{
    bot_token, fmt_coins, format_number, full_name, is_group_chat, SORRY_FRUITS,
};
use crate::database::COIN;
use crate::commands::{admin, assets, betting, markets, menu, referral, tg};
use crate::database::OfferOutcome;
use crate::game::BetGame;
use crate::i18n::{self, Lang};
use crate::types::BetState;
use rand::seq::SliceRandom;
use std::collections::HashMap;
use std::sync::Arc;
use telexide::api::types::{AnswerCallbackQuery, DeleteMessage};
use telexide::model::{CallbackQuery, ChatMember, UpdateContent};
use telexide::prelude::*;
use tokio::sync::Mutex;

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
        let _ = db_arc(&ctx).set_group_adder(chat_id, upd.from.id);
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
        let _ = db_arc(&ctx).touch_chat(m.chat.get_id());
    }
    // Admin pause kill-switch: block every non-owner button press while paused.
    if !crate::commands::util::is_owner(&ctx, cb.from.id)
        && db_arc(&ctx).is_paused().unwrap_or(false)
    {
        let _ = answer(&ctx, &cb, i18n::service_paused(cb_lang(&ctx, &cb)), false).await;
        return;
    }
    // Group-add referral: the first button a brand-new user taps in a group binds
    // them to whoever added the bot. Runs before dispatch so the row is created
    // here (not by the handler that follows).
    maybe_bind_group_referral(&ctx, &cb).await;
    let result = if let Some(rest) = data.strip_prefix("envelope:") {
        handle_envelope(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix("gamble:") {
        handle_gamble(&ctx, &cb, rest).await
    } else if data.starts_with("sell:") {
        handle_sell(&ctx, &cb).await
    } else if data.starts_with("buy:") {
        handle_buy(&ctx, &cb).await
    } else if let Some(rest) = data.strip_prefix(menu::SET_LANG) {
        handle_set_lang(&ctx, &cb, rest).await
    } else if data == menu::MENU_CHECKIN {
        handle_menu_checkin(&ctx, &cb).await
    } else if data == menu::MENU_BALANCE {
        handle_menu_balance(&ctx, &cb).await
    } else if data == menu::MENU_MATCHES {
        handle_menu_matches(&ctx, &cb).await
    } else if data == menu::MENU_INVITE {
        handle_menu_invite(&ctx, &cb).await
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
    } else if let Some(rest) = data.strip_prefix(betting::SIZE_CONFIRM) {
        betting::handle_size_confirm(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(betting::SIZE_PLACE) {
        betting::handle_size_place(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(betting::SIZE) {
        betting::handle_size(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(admin::SETTLE_CB) {
        admin::handle_settle_cb(&ctx, &cb, rest).await
    } else {
        Ok(())
    };
    if let Err(err) = result {
        eprintln!("callback handler error: {err}");
    }
}

fn db_arc(ctx: &Context) -> Arc<crate::database::Database> {
    ctx.data
        .read()
        .get::<DbKey>()
        .expect("DbKey missing")
        .clone()
}

fn games_arc(ctx: &Context) -> Arc<Mutex<HashMap<String, BetGame>>> {
    ctx.data
        .read()
        .get::<GamesKey>()
        .expect("GamesKey missing")
        .clone()
}

async fn answer(
    ctx: &Context,
    cb: &CallbackQuery,
    text: impl Into<String>,
    alert: bool,
) -> Result<(), telexide::Error> {
    let mut a = AnswerCallbackQuery::new(cb.id.clone());
    a.text = Some(text.into());
    a.show_alert = Some(alert);
    ctx.api.answer_callback_query(a).await?;
    Ok(())
}

async fn delete_msg(
    ctx: &Context,
    chat_id: i64,
    message_id: i64,
) -> Result<(), telexide::Error> {
    ctx.api
        .delete_message(DeleteMessage::new(chat_id.into(), message_id))
        .await?;
    Ok(())
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
    let db = db_arc(ctx);
    if !db.has_buffer(chat_id, msg_id).unwrap_or(false) {
        return answer(ctx, cb, i18n::someone_took_it(lang), false).await;
    }
    let amount: i64 = rest.parse().unwrap_or(0);
    if amount <= 0 {
        let fruit = {
            let mut rng = rand::thread_rng();
            *SORRY_FRUITS.choose(&mut rng).unwrap()
        };
        let s = fruit.to_string();
        match db.fruit_change(cb.from.id, &s, true) {
            Ok(true) => {
                db.delete_buffer(chat_id, msg_id).ok();
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
    if db.balance_change(cb.from.id, amount * COIN).is_err() {
        return answer(ctx, cb, i18n::db_error(lang), true).await;
    }
    db.delete_buffer(chat_id, msg_id).ok();
    let _ = tg::edit_text_only(
        ctx,
        chat_id,
        msg_id,
        &i18n::received_coins(lang, &cb.from.first_name, &format_number(amount)),
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
    // `game.lang`.
    let lang = cb_lang(ctx, cb);
    let games = games_arc(ctx);
    let db = db_arc(ctx);

    // close: gamble:  (host only)
    if rest.is_empty() {
        let (text, rows) = {
            let mut g = games.lock().await;
            let Some(game) = g.get_mut(&key) else {
                return answer(ctx, cb, i18n::game_invalid(lang), true).await;
            };
            if game.host != cb.from.id {
                return answer(ctx, cb, i18n::not_host(lang), true).await;
            }
            if !game.close() {
                return answer(ctx, cb, i18n::already_closed(lang), true).await;
            }
            if let Err(err) = db.save_bet_game(game) {
                eprintln!("save_bet_game(close) error: {err}");
            }
            (game.get_text(), game.get_buttons())
        };
        let _ = tg::edit_with_buttons(ctx, chat_id, msg_id, &text, &rows).await;
        return answer(ctx, cb, i18n::close_game_toast(lang), false).await;
    }

    // accumulate: gamble:add:<idx>:<amt> — add to the tapper's pending stake.
    // Nothing is debited; feedback is a private toast (only the tapper sees it),
    // so the shared board isn't edited. Pending is per (game, user).
    if let Some(spec) = rest.strip_prefix("add:") {
        let parts: Vec<&str> = spec.split(':').collect();
        let [idx_s, amt_s] = parts.as_slice() else {
            return answer(ctx, cb, "", false).await;
        };
        let (Ok(idx), Ok(amt)) = (idx_s.parse::<usize>(), amt_s.parse::<i64>()) else {
            return answer(ctx, cb, "", false).await;
        };
        let opt = {
            let g = games.lock().await;
            let Some(game) = g.get(&key) else {
                return answer(ctx, cb, i18n::game_invalid(lang), true).await;
            };
            if game.state != BetState::betting {
                return answer(ctx, cb, i18n::already_closed(lang), true).await;
            }
            match game.option_order.get(idx) {
                Some(o) => o.clone(),
                None => return answer(ctx, cb, "", false).await,
            }
        };
        let pending = {
            let mut drafts = bet_drafts().lock();
            let entry = drafts.entry((key.clone(), cb.from.id)).or_insert((idx, 0));
            if entry.0 != idx {
                *entry = (idx, 0); // switched option → start fresh on the new one
            }
            entry.1 += amt;
            entry.1
        };
        return answer(ctx, cb, &i18n::bet_pending(lang, &pending.to_string(), &opt), false).await;
    }

    // clear: gamble:clear — drop the tapper's pending stake
    if rest == "clear" {
        bet_drafts().lock().remove(&(key.clone(), cb.from.id));
        return answer(ctx, cb, i18n::bet_cleared(lang), false).await;
    }

    // confirm: gamble:confirm — commit the tapper's pending stake (the only step
    // that moves money) and edit the shared board with the new pool/odds.
    if rest == "confirm" {
        let Some((idx, amount)) = bet_drafts().lock().remove(&(key.clone(), cb.from.id)) else {
            return answer(ctx, cb, i18n::bad_stake(lang), true).await;
        };
        if amount <= 0 {
            return answer(ctx, cb, i18n::bad_stake(lang), true).await;
        }
        if !db.balance_change(cb.from.id, -amount * COIN).unwrap_or(false) {
            // Keep the draft so the user can adjust or clear it.
            bet_drafts().lock().insert((key.clone(), cb.from.id), (idx, amount));
            return answer(ctx, cb, i18n::not_enough_money(lang), true).await;
        }
        let (text, rows) = {
            let mut g = games.lock().await;
            let Some(game) = g.get_mut(&key) else {
                db.force_change(cb.from.id, amount * COIN).ok();
                return answer(ctx, cb, i18n::game_invalid(lang), true).await;
            };
            let Some(option) = game.option_order.get(idx).cloned() else {
                db.force_change(cb.from.id, amount * COIN).ok();
                return answer(ctx, cb, i18n::bet_failed(lang), true).await;
            };
            if !game.stake(cb.from.id, &option, amount) {
                db.force_change(cb.from.id, amount * COIN).ok();
                return answer(ctx, cb, i18n::bet_failed(lang), true).await;
            }
            if let Err(err) = db.save_bet_game(game) {
                eprintln!("save_bet_game(stake) error: {err}");
            }
            (game.get_text(), game.get_buttons())
        };
        let _ = tg::edit_with_buttons(ctx, chat_id, msg_id, &text, &rows).await;
        return answer(ctx, cb, i18n::bet_success(lang), false).await;
    }

    // settle: gamble:<outcome>  (host only, after close). The settled board is
    // rendered in the host's language (game.lang) — it's a shared message.
    let outcome = rest;
    let (outputs, display, state, game_lang) = {
        let mut g = games.lock().await;
        let Some(game) = g.get_mut(&key) else {
            return answer(ctx, cb, i18n::game_invalid(lang), true).await;
        };
        if game.host != cb.from.id {
            return answer(ctx, cb, i18n::not_host(lang), true).await;
        }
        if game.state != BetState::closed {
            return answer(ctx, cb, i18n::not_closed_yet(lang), true).await;
        }
        let (outputs, display) = game.settle(outcome);
        if let Err(err) = db.save_bet_game(game) {
            eprintln!("save_bet_game(settle) error: {err}");
        }
        (outputs, display, game.state.clone(), game.lang)
    };
    for (user, win) in &outputs {
        if *win > 0 {
            db.force_change(*user, *win * COIN).ok();
        }
    }
    let _ = tg::edit_text_only(
        ctx,
        chat_id,
        msg_id,
        &format!("{}\n---{}\n", display, state.label(game_lang)),
    )
    .await;
    answer(ctx, cb, i18n::settle_success(lang), false).await
}

/// `(game_key, user_id) → (option_index, accumulated_whole_coins)`.
type BetDrafts = HashMap<(String, i64), (usize, i64)>;

/// Per-(game, user) pending stake for the shared `/predict` board. In memory —
/// nothing is debited until the user taps Confirm, so a restart just drops
/// uncommitted drafts. parking_lot Mutex; never locked across an `.await`.
fn bet_drafts() -> &'static parking_lot::Mutex<BetDrafts> {
    static D: std::sync::OnceLock<parking_lot::Mutex<BetDrafts>> = std::sync::OnceLock::new();
    D.get_or_init(|| parking_lot::Mutex::new(BetDrafts::new()))
}

/// Resolve the locale for a callback presser: their saved choice if any, else
/// the Telegram-reported language.
fn cb_lang(ctx: &Context, cb: &CallbackQuery) -> Lang {
    db_arc(ctx)
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
    let db = db_arc(ctx);
    if db.set_lang(cb.from.id, lang).is_err() {
        return answer(ctx, cb, i18n::db_error(lang), true).await;
    }
    if let Some(message) = cb.message.clone() {
        let available = is_group_chat(message.chat.get_id())
            || db.checkin_available(cb.from.id).unwrap_or(true);
        let _ = tg::edit_with_buttons(
            ctx,
            message.chat.get_id(),
            message.message_id,
            &menu::menu_text(lang, &full_name(&cb.from)),
            &menu::main_menu_rows(lang, available, is_group_chat(message.chat.get_id())),
        )
        .await;
    }
    answer(ctx, cb, "", false).await
}

/// Group-add referral bind: the first time a **brand-new** user taps *any* button
/// inside a group, bind them to whoever added the bot there and pay both sides.
/// No-op outside groups, when there's no recorded adder, or for existing users
/// (`set_referrer_if_new` only inserts a fresh row — so no farming).
async fn maybe_bind_group_referral(ctx: &Context, cb: &CallbackQuery) {
    let Some(message) = &cb.message else {
        return;
    };
    let chat_id = message.chat.get_id();
    if !is_group_chat(chat_id) {
        return;
    }
    let db = db_arc(ctx);
    if let Ok(Some(adder)) = db.group_adder(chat_id) {
        db.force_change(adder, 0).ok(); // ensure the adder has a row to refer from
        if db.set_referrer_if_new(cb.from.id, adder).unwrap_or(false) {
            referral::pay_referral(ctx, adder, &cb.from).await;
        }
    }
}

/// `menu:checkin` — grant the daily reward; result shown as an alert. In a
/// private chat the menu refreshes so the now-spent button drops off; in a group
/// the menu is shared, so the button stays for everyone else to claim.
async fn handle_menu_checkin(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let db = db_arc(ctx);

    // (Group-add referral binding now happens for ANY button press in a group,
    // up in `on_callback` before dispatch — see `maybe_bind_group_referral`.)
    match db.try_checkin(cb.from.id, crate::commands::checkin::CHECKIN_REWARD) {
        Ok(true) => {
            if let Some(message) = cb.message.clone() {
                if !is_group_chat(message.chat.get_id()) {
                    let _ = tg::edit_with_buttons(
                        ctx,
                        message.chat.get_id(),
                        message.message_id,
                        &menu::menu_text(lang, &full_name(&cb.from)),
                        &menu::main_menu_rows(lang, false, false),
                    )
                    .await;
                }
            }
            let amt = fmt_coins(crate::commands::checkin::CHECKIN_REWARD);
            answer(ctx, cb, i18n::checkin_done(lang, &amt), true).await
        }
        Ok(false) => {
            let t = crate::commands::checkin::time_until_reset();
            answer(ctx, cb, i18n::checkin_already(lang, &t), true).await
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
    // Show the caller's referral count above the format chooser.
    let count = db_arc(ctx).count_referrals(cb.from.id).unwrap_or(0);
    let text = format!(
        "{}\n\n{}",
        i18n::invite_count(lang, &count.to_string()),
        i18n::invite_how(lang)
    );
    let rows = vec![
        vec![(i18n::btn_invite_link(lang).to_string(), menu::INVITE_LINK.to_string())],
        vec![(i18n::btn_invite_fwd(lang).to_string(), menu::INVITE_FWD.to_string())],
        vec![(i18n::btn_invite_qr(lang).to_string(), menu::INVITE_QR.to_string())],
    ];
    tg::send_with_buttons(ctx, message.chat.get_id(), &text, &rows).await?;
    Ok(())
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

/// `inv:link` — the referral link in a tap-to-copy `<code>` span.
async fn handle_invite_link(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    answer(ctx, cb, "", false).await?;
    let link = referral_link_of(ctx, cb.from.id);
    tg::send_html(ctx, message.chat.get_id(), &i18n::invite_copy(lang, &link)).await?;
    Ok(())
}

/// `inv:fwd` — a forward-safe message: the link is baked into the **text** (so a
/// forward keeps it), plus a `[🎮 Play now]` URL button for tapping in place
/// (inline keyboards are stripped on forward, hence the link also lives in text).
async fn handle_invite_fwd(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    answer(ctx, cb, "", false).await?;
    let link = referral_link_of(ctx, cb.from.id);
    let rows = vec![vec![(i18n::btn_join(lang).to_string(), link.clone())]];
    tg::send_with_buttons(ctx, message.chat.get_id(), &i18n::invite_forward(lang, &link), &rows)
        .await?;
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

/// `menu:balance` — post the presser's balance + open positions as a fresh
/// message, leaving the menu in place.
async fn handle_menu_balance(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    answer(ctx, cb, "", false).await?;
    let text = assets::assets_text(ctx, lang, &cb.from).await;
    crate::commands::util::send_text(ctx, message.chat.get_id(), text).await?;
    Ok(())
}

/// `menu:matches` — post the match brief as a fresh message, leaving the menu
/// in place.
async fn handle_menu_matches(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    answer(ctx, cb, "", false).await?;
    let (text, rows) = markets::brief(lang).await;
    tg::send_with_buttons(ctx, message.chat.get_id(), &text, &rows).await?;
    Ok(())
}

async fn handle_sell(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    let chat_id = message.chat.get_id();
    let msg_id = message.message_id;
    let lang = Lang::from_user(&cb.from);
    let db = db_arc(ctx);
    let outcome = match db.consume_sell(chat_id, msg_id, cb.from.id) {
        Ok(o) => o,
        Err(_) => return answer(ctx, cb, i18n::db_error(lang), true).await,
    };
    match outcome {
        OfferOutcome::AlreadyTaken => answer(ctx, cb, i18n::someone_dealt(lang), false).await,
        OfferOutcome::SelfCancelled => {
            delete_msg(ctx, chat_id, msg_id).await.ok();
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
            answer(ctx, cb, i18n::bought_toast(lang, &fruits), true).await
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
    let db = db_arc(ctx);
    let outcome = match db.consume_buy(chat_id, msg_id, cb.from.id) {
        Ok(o) => o,
        Err(_) => return answer(ctx, cb, i18n::db_error(lang), true).await,
    };
    match outcome {
        OfferOutcome::AlreadyTaken => answer(ctx, cb, i18n::someone_dealt(lang), false).await,
        OfferOutcome::SelfCancelled => {
            delete_msg(ctx, chat_id, msg_id).await.ok();
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
            answer(ctx, cb, i18n::sold_toast(lang, &fmt_coins(price)), true).await
        }
        OfferOutcome::TakerMissingFruit(ch) => {
            answer(ctx, cb, i18n::you_dont_have(lang, &ch.to_string()), true).await
        }
        OfferOutcome::TakerFruitFull => answer(ctx, cb, i18n::buyer_fruit_full(lang), true).await,
        OfferOutcome::TakerNotEnoughBalance => {
            answer(ctx, cb, i18n::system_error(lang), true).await
        }
    }
}
