use crate::bot::{DbKey, GamesKey};
use crate::commands::tg;
use crate::commands::util::{format_number, SORRY_FRUITS};
use crate::database::OfferOutcome;
use crate::game::BetGame;
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
    let db = db_arc(ctx);
    if !db.has_buffer(chat_id, msg_id).unwrap_or(false) {
        return answer(ctx, cb, "別人領走了🙁", false).await;
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
                    &format!("{} 收到一顆 {fruit}", cb.from.first_name),
                )
                .await;
                return answer(ctx, cb, "搶到啦😁", false).await;
            }
            Ok(false) => return answer(ctx, cb, "水果太多囉😶", false).await,
            Err(_) => return answer(ctx, cb, "資料庫錯誤", true).await,
        }
    }
    if db.balance_change(cb.from.id, amount).is_err() {
        return answer(ctx, cb, "資料庫錯誤", true).await;
    }
    db.delete_buffer(chat_id, msg_id).ok();
    let _ = tg::edit_text_only(
        ctx,
        chat_id,
        msg_id,
        &format!(
            "{} 收到 {} 顆 水幣",
            cb.from.first_name,
            format_number(amount)
        ),
    )
    .await;
    answer(ctx, cb, "搶到啦😁", false).await
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
        return answer(ctx, cb, format!("{opt} 的押注"), false).await;
    }

    // close: gamble:
    if parts.is_empty() {
        let (text, rows) = {
            let mut g = games.lock().await;
            let Some(game) = g.get_mut(&key) else {
                return answer(ctx, cb, "賭局已失效", true).await;
            };
            if game.host != cb.from.id {
                return answer(ctx, cb, "你不是莊家😶", true).await;
            }
            if !game.close() {
                return answer(ctx, cb, "已收盤", true).await;
            }
            (game.get_text(), game.get_buttons())
        };
        let _ = tg::edit_with_buttons(ctx, chat_id, msg_id, &text, &rows).await;
        return answer(ctx, cb, "關閉賭局", false).await;
    }

    // bet: gamble:<option>:<stake>
    if parts.len() == 2 {
        let option = parts[0];
        let Ok(stake) = parts[1].parse::<i64>() else {
            return answer(ctx, cb, "下注額錯誤", true).await;
        };
        if stake <= 0 {
            return answer(ctx, cb, "下注額錯誤", true).await;
        }
        if !db.balance_change(cb.from.id, -stake).unwrap_or(false) {
            return answer(ctx, cb, "錢不夠耶😶", true).await;
        }
        let (text, rows) = {
            let mut g = games.lock().await;
            let Some(game) = g.get_mut(&key) else {
                db.force_change(cb.from.id, stake).ok();
                return answer(ctx, cb, "賭局已失效", true).await;
            };
            if !game.stake(cb.from.id, option, stake) {
                db.force_change(cb.from.id, stake).ok();
                return answer(ctx, cb, "下注失敗", true).await;
            }
            (game.get_text(), game.get_buttons())
        };
        let _ = tg::edit_with_buttons(ctx, chat_id, msg_id, &text, &rows).await;
        return answer(ctx, cb, "下注成功", false).await;
    }

    // settle: gamble:<outcome>
    if parts.len() == 1 {
        let outcome = parts[0];
        let (outputs, display, state) = {
            let mut g = games.lock().await;
            let Some(game) = g.get_mut(&key) else {
                return answer(ctx, cb, "賭局已失效", true).await;
            };
            if game.host != cb.from.id {
                return answer(ctx, cb, "你不是莊家😶", true).await;
            }
            if game.state != BetState::closed {
                return answer(ctx, cb, "尚未收盤", true).await;
            }
            let (outputs, display) = game.settle(outcome);
            (outputs, display, game.state.clone())
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
            &format!("{}\n---{}\n", display, state.as_str()),
        )
        .await;
        return answer(ctx, cb, "結算成功", false).await;
    }

    answer(ctx, cb, "系統錯誤", true).await
}

async fn handle_sell(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    let chat_id = message.chat.get_id();
    let msg_id = message.message_id;
    let db = db_arc(ctx);
    let outcome = match db.consume_sell(chat_id, msg_id, cb.from.id) {
        Ok(o) => o,
        Err(_) => return answer(ctx, cb, "資料庫錯誤", true).await,
    };
    match outcome {
        OfferOutcome::AlreadyTaken => answer(ctx, cb, "別人成交了🙁", false).await,
        OfferOutcome::SelfCancelled => {
            delete_msg(ctx, chat_id, msg_id).await.ok();
            answer(ctx, cb, "已撤回賣單", false).await
        }
        OfferOutcome::Filled { fruits, price } => {
            let _ = tg::edit_text_only(
                ctx,
                chat_id,
                msg_id,
                &format!(
                    "{} 花 {} 水幣\n買了 {fruits}",
                    cb.from.first_name,
                    format_number(price)
                ),
            )
            .await;
            answer(ctx, cb, format!("買了 {fruits}🥳"), true).await
        }
        OfferOutcome::TakerNotEnoughBalance => answer(ctx, cb, "錢不夠耶😶", true).await,
        OfferOutcome::TakerFruitFull => answer(ctx, cb, "水果太多囉😶", true).await,
        OfferOutcome::TakerMissingFruit(_) => answer(ctx, cb, "系統錯誤", true).await,
    }
}

async fn handle_buy(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    let chat_id = message.chat.get_id();
    let msg_id = message.message_id;
    let db = db_arc(ctx);
    let outcome = match db.consume_buy(chat_id, msg_id, cb.from.id) {
        Ok(o) => o,
        Err(_) => return answer(ctx, cb, "資料庫錯誤", true).await,
    };
    match outcome {
        OfferOutcome::AlreadyTaken => answer(ctx, cb, "別人成交了🙁", false).await,
        OfferOutcome::SelfCancelled => {
            delete_msg(ctx, chat_id, msg_id).await.ok();
            answer(ctx, cb, "已撤回買單", false).await
        }
        OfferOutcome::Filled { fruits, price } => {
            let _ = tg::edit_text_only(
                ctx,
                chat_id,
                msg_id,
                &format!(
                    "{} 賣出 {fruits}\n賺了 {} 水幣",
                    cb.from.first_name,
                    format_number(price)
                ),
            )
            .await;
            answer(ctx, cb, format!("賺取 {} 水幣🥳", format_number(price)), true).await
        }
        OfferOutcome::TakerMissingFruit(ch) => {
            answer(ctx, cb, format!("你沒有 {ch} 喔😶"), true).await
        }
        OfferOutcome::TakerFruitFull => answer(ctx, cb, "買家水果太多囉😶", true).await,
        OfferOutcome::TakerNotEnoughBalance => answer(ctx, cb, "系統錯誤", true).await,
    }
}
