use crate::bot::{DbKey, GamesKey};
use crate::database::Database;
use crate::game::BetGame;
use std::collections::HashMap;
use std::sync::Arc;
use telexide::api::types::SendMessage;
use telexide::framework::CommandError;
use telexide::model::{Message, MessageContent, User};
use telexide::prelude::Context;
use tokio::sync::Mutex;

// These getters expect their TypeMap key was inserted in `bot::run` before
// the polling loop started. If init succeeded, the key is present; if init
// failed, run() returned Err and polling never began. So in practice the
// .expect()s are unreachable. If something exotic ever did make them panic,
// tokio::spawn's panic-catching contains it to the single handler invocation
// — the polling loop in `bot::robust_poll` stays alive.
pub fn db(ctx: &Context) -> Arc<Database> {
    ctx.data
        .read()
        .get::<DbKey>()
        .expect("DbKey missing — bot::run did not init properly")
        .clone()
}

pub fn games(ctx: &Context) -> Arc<Mutex<HashMap<String, BetGame>>> {
    ctx.data
        .read()
        .get::<GamesKey>()
        .expect("GamesKey missing — bot::run did not init properly")
        .clone()
}

pub async fn reply(
    ctx: &Context,
    msg: &Message,
    text: impl Into<String>,
) -> Result<(), CommandError> {
    let text = text.into();
    eprintln!("[out] {text}");
    let mut sm = SendMessage::new(msg.chat.get_id().into(), text);
    sm.reply_to_message_id = Some(msg.message_id);
    ctx.api.send_message(sm).await?;
    Ok(())
}

pub async fn send_text(
    ctx: &Context,
    chat_id: i64,
    text: impl Into<String>,
) -> Result<Message, CommandError> {
    let text = text.into();
    eprintln!("[out] {text}");
    Ok(ctx
        .api
        .send_message(SendMessage::new(chat_id.into(), text))
        .await?)
}

pub fn full_name(u: &User) -> String {
    match &u.last_name {
        Some(last) if !last.is_empty() => format!("{} {}", u.first_name, last),
        _ => u.first_name.clone(),
    }
}

pub fn text_of(msg: &Message) -> &str {
    if let MessageContent::Text { content, .. } = &msg.content {
        content
    } else {
        ""
    }
}

pub fn args(msg: &Message) -> Vec<String> {
    text_of(msg)
        .split_whitespace()
        .skip(1)
        .map(|s| s.to_string())
        .collect()
}

pub fn format_number(n: i64) -> String {
    let mut s = n.abs().to_string();
    if s.len() <= 3 {
        return s;
    }
    let mut i = s.len() as isize - 3;
    while i > 0 {
        s.insert(i as usize, ',');
        i -= 3;
    }
    s
}

pub fn from_id(msg: &Message) -> Option<i64> {
    msg.from.as_ref().map(|u| u.id)
}

/// Locale for the user who sent `msg`, from their Telegram `language_code`.
/// Falls back to English when there is no sender or no language tag.
pub fn lang_of(msg: &Message) -> crate::i18n::Lang {
    msg.from
        .as_ref()
        .map(crate::i18n::Lang::from_user)
        .unwrap_or(crate::i18n::Lang::En)
}

pub const ERR_REPLY: &str = "🤯";
pub const ERR_NEG_REPLY: &str = "😐";

/// Consolation fruits used by `open_envelope` when amount ≤ 0 and the
/// fruit-set restriction enforced on `/buy`. Matches the original Python
/// `sorry_reply` array.
pub const SORRY_FRUITS: &[char] = &['🍑', '🍓', '🍎', '🍊', '🥭', '🍍', '🍅', '🍈', '🍋', '🍐'];
