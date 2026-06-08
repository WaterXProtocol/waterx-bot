use crate::bot::{ConfigKey, DbKey, ParamsKey};
use crate::commands::tg;
use std::sync::Arc;
use telexide::model::{Chat, UpdateContent};
use telexide::prelude::*;

#[prepare_listener]
pub async fn on_message(ctx: Context, update: Update) {
    let UpdateContent::Message(message) = update.content else {
        return;
    };

    if let telexide::model::MessageContent::Text { content, .. } = &message.content {
        let who = message
            .from
            .as_ref()
            .map(|u| u.first_name.as_str())
            .unwrap_or("?");
        eprintln!("[in] {who}: {content}");
    }

    let chat_id = message.chat.get_id();
    let is_private = matches!(message.chat, Chat::Private(_));
    maybe_spawn_envelope(&ctx, chat_id, is_private).await;
}

/// Probability-rolls the envelope drop and posts one if the dice say so.
/// Pulled out of `on_message` so the polling loop can still invoke it for
/// `Update`s whose full deserialisation failed (e.g. unknown StickerType
/// variants) — we only need the chat id and chat type to decide whether to
/// drop an envelope, both of which are extractable from the raw JSON.
pub async fn maybe_spawn_envelope(ctx: &Context, chat_id: i64, is_private: bool) {
    // Match the original Python's `_valid_type`: normal mode allows only
    // non-private chats; dev mode is the inverse.
    let dev = ctx
        .data
        .read()
        .get::<ConfigKey>()
        .expect("ConfigKey missing — bot::run did not init properly")
        .dev;
    if dev != is_private {
        return;
    }

    let amount = {
        use rand::Rng;
        use rand_distr::{Distribution, Normal};
        let params = ctx
            .data
            .read()
            .get::<ParamsKey>()
            .expect("ParamsKey missing — bot::run did not init properly")
            .clone();
        let (possi, mean, std) = {
            let p = params.read();
            (p.p_possi, p.p_mean, p.p_std)
        };
        if possi == 0 {
            return;
        }
        let mut rng = rand::thread_rng();
        if rng.gen_range(0..possi) != 0 {
            return;
        }
        let normal = match Normal::new(mean, std) {
            Ok(n) => n,
            Err(err) => {
                eprintln!("envelope drop: invalid p_mean/p_std ({mean}, {std}): {err}");
                return;
            }
        };
        normal.sample(&mut rng).round() as i64
    };

    let rows = vec![vec![(
        "領取🧧".to_string(),
        format!("envelope:{amount}"),
    )]];
    let sent = match tg::send_with_buttons(ctx, chat_id, "搶紅包囉！", &rows).await {
        Ok(m) => m,
        Err(err) => {
            eprintln!("envelope spawn send error: {err:?}");
            return;
        }
    };

    let db: Arc<crate::database::Database> = ctx
        .data
        .read()
        .get::<DbKey>()
        .expect("DbKey missing — bot::run did not init properly")
        .clone();
    if let Err(err) = db.insert_buffer(sent.chat.get_id(), sent.message_id) {
        eprintln!("envelope buffer insert error: {err}");
    }
}
