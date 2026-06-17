use crate::bot::{DbKey, GamesKey};
use crate::commands::tg;
use crate::commands::util::{format_number, SORRY_FRUITS};
use crate::database::OfferOutcome;
use crate::game::BetGame;
use crate::i18n::{self, Lang};
use crate::types::BetState;
use rand::seq::SliceRandom;
use std::collections::HashMap;
use std::sync::Arc;
use telexide::api::types::{AnswerCallbackQuery, DeleteMessage};
use telexide::model::{CallbackQuery, UpdateContent};
use telexide::prelude::*;
use tokio::sync::Mutex;

#[prepare_listener]
pub async fn on_callback(ctx: Context, update: Update) {
    let UpdateContent::CallbackQuery(cb) = update.content else {
        return;
    };
    let Some(data) = cb.data.clone() else {
        return;
    };
    eprintln!("[cb] {}: {data}", cb.from.first_name);
    let result = if let Some(rest) = data.strip_prefix("envelope:") {
        handle_envelope(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix("gamble:") {
        handle_gamble(&ctx, &cb, rest).await
    } else if data.starts_with("sell:") {
        handle_sell(&ctx, &cb).await
    } else if data.starts_with("buy:") {
        handle_buy(&ctx, &cb).await
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
    if db.balance_change(cb.from.id, amount).is_err() {
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
        if !db.balance_change(cb.from.id, -stake).unwrap_or(false) {
            return answer(ctx, cb, i18n::not_enough_money(lang), true).await;
        }
        let (text, rows) = {
            let mut g = games.lock().await;
            let Some(game) = g.get_mut(&key) else {
                db.force_change(cb.from.id, stake).ok();
                return answer(ctx, cb, i18n::game_invalid(lang), true).await;
            };
            if !game.stake(cb.from.id, option, stake) {
                db.force_change(cb.from.id, stake).ok();
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
                db.force_change(*user, *win).ok();
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
                &i18n::bought_msg(lang, &cb.from.first_name, &format_number(price), &fruits),
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
                &i18n::sold_msg(lang, &cb.from.first_name, &fruits, &format_number(price)),
            )
            .await;
            answer(ctx, cb, i18n::sold_toast(lang, &format_number(price)), true).await
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
