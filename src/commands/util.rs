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
    // Display rounds to at most 2 decimals (the ledger keeps full micro-coin
    // precision). Round half-up to the nearest 0.01 coin, then trim trailing
    // zeros so whole amounts read "42", not "42.00".
    let per_cent = coin / 100; // micro-coins in 0.01 coin
    let cents = (u + per_cent / 2) / per_cent; // hundredths of a coin
    let int_str = format_number(cents / 100);
    let frac = cents % 100;
    let body = if frac == 0 {
        int_str
    } else {
        let mut f = format!("{frac:02}");
        while f.ends_with('0') {
            f.pop();
        }
        format!("{int_str}.{f}")
    };
    if neg && cents != 0 {
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

/// The configured `BOT_OWNER` id (0 if config is somehow missing).
pub fn owner_id(ctx: &Context) -> i64 {
    ctx.data
        .read()
        .get::<ConfigKey>()
        .map(|cfg| cfg.owner)
        .unwrap_or(0)
}

/// DM the configured `BOT_OWNER` a technical-error alert, best-effort (a bounced
/// owner DM must never affect the user's flow). No-op if no owner is configured.
pub async fn notify_owner(ctx: &Context, text: &str) {
    let owner = owner_id(ctx);
    if owner != 0 {
        let _ = send_text(ctx, owner, format!("⚠️ {text}")).await;
    }
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
    // Fail **closed**: if the pause flag can't be read, treat the bot as paused
    // (a kill-switch that can't confirm "off" should stop, not pass through). The
    // owner is already let through above, so they can still recover.
    let paused = db(ctx).is_paused().unwrap_or_else(|e| {
        eprintln!("paused_block is_paused error (failing closed): {e}");
        true
    });
    if paused {
        reply(ctx, msg, crate::i18n::service_paused(lang_for_msg(ctx, msg))).await?;
        return Ok(true);
    }
    // Group referral: a brand-new member's first interaction binds them to the
    // bot-adder. Mirrors the button-press path in `callbacks::on_callback`, so a
    // text command counts too. Runs before the command body creates the row.
    if let Some(user) = msg.from.as_ref() {
        crate::commands::referral::maybe_bind_group(ctx, msg.chat.get_id(), user).await;
    }
    Ok(false)
}

pub const ERR_REPLY: &str = "🤯";
pub const ERR_NEG_REPLY: &str = "😐";

/// Largest whole-coin amount a user may name in a single operation. Kept far
/// below `i64::MAX / COIN` so the `* COIN` conversion can never overflow.
pub const MAX_COINS: i64 = 1_000_000_000;

/// Convert a user-typed whole-coin `amount` into micro-coins, rejecting
/// non-positive or implausibly large values. Returning `None` (rather than
/// letting `amount * COIN` silently wrap in release builds) keeps the ledger's
/// conservation invariant intact — without this guard a single large `/send`
/// could overflow `i64` and mint coins out of thin air.
pub fn to_micro(amount: i64) -> Option<i64> {
    if amount <= 0 || amount > MAX_COINS {
        return None;
    }
    amount.checked_mul(crate::database::COIN)
}

/// Consolation fruits used by `open_envelope` when amount ≤ 0 and the
/// fruit-set restriction enforced on `/buy`. Matches the original Python
/// `sorry_reply` array.
pub const SORRY_FRUITS: &[char] = &['🍑', '🍓', '🍎', '🍊', '🥭', '🍍', '🍅', '🍈', '🍋', '🍐'];

/// The signed offset part only: `0 → ""`, `480 → "+8"`, `330 → "+5:30"`,
/// `-300 → "-5"`.
fn tz_offset_str(minutes: i64) -> String {
    if minutes == 0 {
        return String::new();
    }
    let sign = if minutes > 0 { '+' } else { '-' };
    let (h, m) = (minutes.abs() / 60, minutes.abs() % 60);
    if m == 0 {
        format!("{sign}{h}")
    } else {
        format!("{sign}{h}:{m:02}")
    }
}

/// Full label for in-text use: `0 → "UTC"`, `480 → "UTC+8"`, `-300 → "UTC-5"`.
pub fn tz_label(minutes: i64) -> String {
    format!("UTC{}", tz_offset_str(minutes))
}

/// Short label for picker buttons (no "UTC" prefix to keep them compact):
/// `0 → "UTC"`, `480 → "+8"`, `330 → "+5:30"`, `-300 → "-5"`.
pub fn tz_button_label(minutes: i64) -> String {
    if minutes == 0 {
        "UTC".to_string()
    } else {
        tz_offset_str(minutes)
    }
}

/// Convert the feed's YES odds in cents to **decimal odds** for display
/// (e.g. 65¢ → 1.54). Shared by the `/markets` brief and the bet quote so
/// both render the same number. Display-only — payout uses
/// [`crate::database::decimal_payout`].
pub fn decimal_odds(cents: f64) -> f64 {
    100.0 / cents
}

/// Render a YES price (`odds_cents`) in the user's chosen [`OddsFormat`] — the
/// single place odds become a display string, so every surface is consistent.
/// A 65¢ price: Decimal `1.54`, American `-185`, Percent `65%`, Price `65¢`.
/// Callers only invoke this for priced outcomes (`cents > 0`); a non-positive or
/// degenerate (≥100%) price falls back gracefully rather than dividing by zero.
pub fn format_odds(odds_cents: f64, fmt: crate::types::OddsFormat) -> String {
    use crate::types::OddsFormat;
    if odds_cents <= 0.0 {
        return "—".to_string();
    }
    match fmt {
        OddsFormat::Decimal => format!("{:.2}", decimal_odds(odds_cents)),
        OddsFormat::American => {
            if odds_cents >= 100.0 {
                // ≥100% implied (no profit) — moneyline is undefined, show decimal.
                format!("{:.2}", decimal_odds(odds_cents))
            } else if odds_cents <= 50.0 {
                // Underdog (decimal ≥ 2.0) → positive moneyline.
                format!("+{}", (100.0 * (100.0 - odds_cents) / odds_cents).round() as i64)
            } else {
                // Favorite → negative moneyline.
                format!("{}", (-100.0 * odds_cents / (100.0 - odds_cents)).round() as i64)
            }
        }
        // Percent = implied probability ≈ the cents price; Price = the cents price.
        OddsFormat::Percent => format!("{}%", odds_cents.round() as i64),
        OddsFormat::Price => format!("{}¢", odds_cents.round() as i64),
    }
}

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

    #[test]
    fn fmt_coins_rounds_to_two_decimals() {
        // 5 coins @ 1.54 decimal odds = 7.692308… → display caps at 2 dp.
        assert_eq!(fmt_coins(7_692_308), "7.69");
        // Half-up rounding at the 3rd decimal.
        assert_eq!(fmt_coins(1_005_000), "1.01"); // 1.005 → 1.01
        // Sub-cent amounts round to 0 (no "-0").
        assert_eq!(fmt_coins(COIN / 1000), "0"); // 0.001 → 0
        assert_eq!(fmt_coins(-(COIN / 1000)), "0");
    }

    #[test]
    fn to_micro_rejects_overflow_and_nonpositive() {
        // Normal amounts convert to micro-coins.
        assert_eq!(to_micro(1), Some(COIN));
        assert_eq!(to_micro(1234), Some(1234 * COIN));
        assert_eq!(to_micro(MAX_COINS), Some(MAX_COINS * COIN));
        // Non-positive is rejected.
        assert_eq!(to_micro(0), None);
        assert_eq!(to_micro(-5), None);
        // Above the cap is rejected (would otherwise overflow i64 on * COIN).
        assert_eq!(to_micro(MAX_COINS + 1), None);
        // The concrete exploit input that wrapped i64 in release builds.
        assert_eq!(to_micro(9_300_000_000_000), None);
        assert_eq!(to_micro(i64::MAX), None);
    }

    #[test]
    fn tz_label_formats_offsets() {
        assert_eq!(tz_label(0), "UTC");
        assert_eq!(tz_label(480), "UTC+8");
        assert_eq!(tz_label(-300), "UTC-5");
        assert_eq!(tz_label(330), "UTC+5:30");
        assert_eq!(tz_label(-210), "UTC-3:30");
        // Picker buttons drop the "UTC" prefix (except for 0).
        assert_eq!(tz_button_label(0), "UTC");
        assert_eq!(tz_button_label(480), "+8");
        assert_eq!(tz_button_label(-300), "-5");
        assert_eq!(tz_button_label(330), "+5:30");
    }

    #[test]
    fn decimal_odds_converts_cents() {
        assert!((decimal_odds(65.0) - 1.538_461).abs() < 1e-6); // 65¢ → ~1.54
        assert!((decimal_odds(50.0) - 2.0).abs() < 1e-9);
        assert!((decimal_odds(100.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn format_odds_renders_each_format() {
        use crate::types::OddsFormat::{American, Decimal, Percent, Price};
        // Decimal = 100/cents.
        assert_eq!(format_odds(65.0, Decimal), "1.54");
        assert_eq!(format_odds(50.0, Decimal), "2.00");
        // American: even at 50¢; positive for underdogs (<50¢), negative for favorites.
        assert_eq!(format_odds(50.0, American), "+100");
        assert_eq!(format_odds(40.0, American), "+150");
        assert_eq!(format_odds(80.0, American), "-400");
        assert_eq!(format_odds(65.0, American), "-186");
        // Percent = implied probability; Price = the cents price.
        assert_eq!(format_odds(65.0, Percent), "65%");
        assert_eq!(format_odds(65.0, Price), "65¢");
        // Degenerate prices don't divide by zero.
        assert_eq!(format_odds(0.0, Decimal), "—");
    }
}
