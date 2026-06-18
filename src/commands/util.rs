use crate::bot::{ConfigKey, DbKey, GamesKey};
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

/// Format a micro-coin balance (6-decimal fixed-point; see `database::COIN`)
/// as coins, trailing zeros trimmed, thousands separators on the whole part.
/// `1_000_000 → "1"`, `1_500_000 → "1.5"`, `10_000 → "0.01"`, `-5_000_000 → "-5"`.
pub fn fmt_coins(units: i64) -> String {
    let coin = crate::database::COIN;
    let neg = units < 0;
    let u = units.abs();
    let int_str = format_number(u / coin);
    let frac = u % coin; // 0..COIN
    let body = if frac == 0 {
        int_str
    } else {
        // Six-digit fractional part, trailing zeros trimmed.
        let mut f = format!("{frac:06}");
        while f.ends_with('0') {
            f.pop();
        }
        format!("{int_str}.{f}")
    };
    if neg {
        format!("-{body}")
    } else {
        body
    }
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

/// Resolved locale for `user`: their `/start`-chosen language if saved,
/// otherwise the Telegram-reported one. Prefer this over [`lang_of`] in
/// command handlers so an explicit choice wins everywhere.
pub fn lang_for(ctx: &Context, user: &User) -> crate::i18n::Lang {
    db(ctx)
        .get_lang(user.id)
        .ok()
        .flatten()
        .unwrap_or_else(|| crate::i18n::Lang::from_user(user))
}

/// [`lang_for`] keyed off a message's sender; English if there is no sender.
pub fn lang_for_msg(ctx: &Context, msg: &Message) -> crate::i18n::Lang {
    msg.from
        .as_ref()
        .map(|u| lang_for(ctx, u))
        .unwrap_or(crate::i18n::Lang::En)
}

/// True for group / supergroup / channel chats. Telegram gives private chats a
/// positive id (== the user id) and everything else a negative id.
pub fn is_group_chat(chat_id: i64) -> bool {
    chat_id < 0
}

/// True when the bot is running in development mode (`BOT_DEV` unset/true).
pub fn is_dev(ctx: &Context) -> bool {
    ctx.data
        .read()
        .get::<ConfigKey>()
        .map(|cfg| cfg.dev)
        .unwrap_or(false)
}

/// The configured `BOT_TOKEN`, copied out so the (non-`Send`) read guard never
/// crosses an `.await`. Empty string if the config is somehow missing.
pub fn bot_token(ctx: &Context) -> String {
    ctx.data
        .read()
        .get::<ConfigKey>()
        .map(|cfg| cfg.token.clone())
        .unwrap_or_default()
}

/// True when `user_id` is the configured `BOT_OWNER`. The read guard is dropped
/// on the same statement (it is not `Send`, so it must not cross an `.await`).
pub fn is_owner(ctx: &Context, user_id: i64) -> bool {
    ctx.data
        .read()
        .get::<ConfigKey>()
        .map(|cfg| cfg.owner == user_id)
        .unwrap_or(false)
}

/// Gate for the admin pause kill-switch. Returns `true` (and tells the caller
/// the bot is paused) when the bot is paused and the actor is **not** the
/// owner; command handlers should early-return when it does. The owner is
/// always allowed through so they can still `/unpause` and operate.
pub async fn paused_block(ctx: &Context, msg: &Message) -> Result<bool, CommandError> {
    // Learn this chat (private or group) so `/broadcast` can reach it later.
    db(ctx).touch_chat(msg.chat.get_id()).ok();
    let uid = from_id(msg).unwrap_or(0);
    if is_owner(ctx, uid) {
        return Ok(false);
    }
    if db(ctx).is_paused().unwrap_or(false) {
        reply(ctx, msg, crate::i18n::service_paused(lang_for_msg(ctx, msg))).await?;
        return Ok(true);
    }
    Ok(false)
}

pub const ERR_REPLY: &str = "🤯";
pub const ERR_NEG_REPLY: &str = "😐";

/// Consolation fruits used by `open_envelope` when amount ≤ 0 and the
/// fruit-set restriction enforced on `/buy`. Matches the original Python
/// `sorry_reply` array.
pub const SORRY_FRUITS: &[char] = &['🍑', '🍓', '🍎', '🍊', '🥭', '🍍', '🍅', '🍈', '🍋', '🍐'];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::COIN;

    #[test]
    fn fmt_coins_formats_fixed_point() {
        assert_eq!(fmt_coins(0), "0");
        assert_eq!(fmt_coins(COIN), "1");
        assert_eq!(fmt_coins(COIN * 1234), "1,234");
        assert_eq!(fmt_coins(COIN / 2), "0.5");
        assert_eq!(fmt_coins(COIN / 10), "0.1");
        assert_eq!(fmt_coins(COIN / 100), "0.01");
        assert_eq!(fmt_coins(COIN + COIN / 100), "1.01");
        assert_eq!(fmt_coins(-(COIN * 5)), "-5");
    }
}
