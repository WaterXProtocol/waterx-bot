use crate::bot::{DbKey, GamesKey};
use crate::commands::util::{
    bot_token, fmt_coins, format_number, full_name, is_group_chat, SORRY_FRUITS,
};
use crate::database::COIN;
use crate::commands::{balance, betting, markets, menu, referral, tg};
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
    } else if let Some(rest) = data.strip_prefix(markets::BET) {
        betting::handle_bet(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(betting::OPT) {
        betting::handle_opt(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(betting::SIZE) {
        betting::handle_size(&ctx, &cb, rest).await
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
    let lang = Lang::from_user(&cb.from);

    let games = games_arc(ctx);
    let db = db_arc(ctx);

    let parts: Vec<&str> = if rest.is_empty() {
        Vec::new()
    } else {
        rest.split(':').collect()
    };

    // label noop: gamble:label:<option>  (option-header buttons in the
    // betting keyboard; tap just dismisses the spinner with no state change)
    if parts.len() >= 2 && parts[0] == "label" {
        let opt = parts[1..].join(":");
        return answer(ctx, cb, i18n::bets_for_option(lang, &opt), false).await;
    }

    // close: gamble:
    if parts.is_empty() {
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

    // bet: gamble:<option>:<stake>
    if parts.len() == 2 {
        let option = parts[0];
        let Ok(stake) = parts[1].parse::<i64>() else {
            return answer(ctx, cb, i18n::bad_stake(lang), true).await;
        };
        if stake <= 0 {
            return answer(ctx, cb, i18n::bad_stake(lang), true).await;
        }
        if !db.balance_change(cb.from.id, -stake * COIN).unwrap_or(false) {
            return answer(ctx, cb, i18n::not_enough_money(lang), true).await;
        }
        let (text, rows) = {
            let mut g = games.lock().await;
            let Some(game) = g.get_mut(&key) else {
                db.force_change(cb.from.id, stake * COIN).ok();
                return answer(ctx, cb, i18n::game_invalid(lang), true).await;
            };
            if !game.stake(cb.from.id, option, stake) {
                db.force_change(cb.from.id, stake * COIN).ok();
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

    // settle: gamble:<outcome>
    if parts.len() == 1 {
        let outcome = parts[0];
        // The settled board is rendered in the host's language (game.lang),
        // not the tapper's — it's a single shared message.
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
        return answer(ctx, cb, i18n::settle_success(lang), false).await;
    }

    answer(ctx, cb, i18n::system_error(lang), true).await
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
            &menu::menu_text(ctx, lang, cb.from.id, &full_name(&cb.from)),
            &menu::main_menu_rows(ctx, lang, cb.from.id, available),
        )
        .await;
    }
    answer(ctx, cb, "", false).await
}

/// `menu:checkin` — grant the daily reward; result shown as an alert. In a
/// private chat the menu refreshes so the now-spent button drops off; in a group
/// the menu is shared, so the button stays for everyone else to claim.
async fn handle_menu_checkin(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let db = db_arc(ctx);

    // Group referral bind: a brand-new user checking in inside a group binds to
    // whoever added the bot there. Must run before try_checkin (which would
    // create the user's row). pays out once; existing users bind to nothing.
    if let Some(message) = &cb.message {
        let chat_id = message.chat.get_id();
        if is_group_chat(chat_id) {
            if let Ok(Some(adder)) = db.group_adder(chat_id) {
                db.force_change(adder, 0).ok(); // ensure the adder has a row
                if db.set_referrer_if_new(cb.from.id, adder).unwrap_or(false) {
                    referral::pay_referral(ctx, adder, &cb.from).await;
                }
            }
        }
    }

    match db.try_checkin(cb.from.id, crate::commands::checkin::CHECKIN_REWARD) {
        Ok(true) => {
            if let Some(message) = cb.message.clone() {
                if !is_group_chat(message.chat.get_id()) {
                    let _ = tg::edit_with_buttons(
                        ctx,
                        message.chat.get_id(),
                        message.message_id,
                        &menu::menu_text(ctx, lang, cb.from.id, &full_name(&cb.from)),
                        &menu::main_menu_rows(ctx, lang, cb.from.id, false),
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

/// `menu:invite` — post the presser's personal referral link and their current
/// referral count as a fresh message.
async fn handle_menu_invite(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    let username = ctx
        .data
        .read()
        .get::<crate::bot::BotUsernameKey>()
        .cloned()
        .unwrap_or_default();
    let link = menu::referral_link(&username, cb.from.id);
    let count = db_arc(ctx).count_referrals(cb.from.id).unwrap_or(0);
    answer(ctx, cb, "", false).await?;
    let chat_id = message.chat.get_id();
    let text = i18n::invite_text(lang, &link, &count.to_string());

    // The invite output (unlike the home page) carries no private balance/fruit,
    // so it's the share-safe surface for the referral link. The `[Play]` URL
    // deep-link button lives here.
    let rows = vec![vec![(i18n::btn_join(lang).to_string(), link.clone())]];

    // Render the link to a QR locally (it never leaves the bot — no third-party
    // QR service) and post it as a photo whose caption is the invite text and
    // keyboard is the deep-link button — all in one message. The QR is the
    // forward-safe carrier: a forward strips the inline keyboard, but the link is
    // baked into the QR image. Best-effort: if QR/upload fails, fall back to a
    // plain text message with the same link + button.
    let qr = qrcode_generator::to_png_to_vec(&link, qrcode_generator::QrCodeEcc::Medium, 512);
    let sent = match qr {
        Ok(png) => tg::send_photo_bytes(&bot_token(ctx), chat_id, png, &text, &rows)
            .await
            .is_ok(),
        Err(_) => false,
    };
    if !sent {
        tg::send_with_buttons(ctx, chat_id, &text, &rows).await?;
    }
    Ok(())
}

/// `menu:balance` — post the presser's balance + open positions as a fresh
/// message, leaving the menu in place.
async fn handle_menu_balance(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    answer(ctx, cb, "", false).await?;
    let text = balance::balance_text(ctx, lang, &cb.from).await;
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
