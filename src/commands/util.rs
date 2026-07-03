use crate::bot::{ConfigKey, Convo, ConvosKey, DbKey};
use crate::database::Database;
use std::collections::HashMap;
use std::sync::Arc;
use telexide::api::types::SendMessage;
use telexide::framework::CommandError;
use telexide::model::{CallbackQuery, Message, MessageContent, User};
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

/// The in-flight DM-conversation map (`/predict` + `/feedback` drafts), keyed by
/// user id. Shared by both flows — see [`crate::bot::Convo`].
pub fn convos(ctx: &Context) -> Arc<Mutex<HashMap<i64, Convo>>> {
    ctx.data
        .read()
        .get::<ConvosKey>()
        .expect("ConvosKey missing — bot::run did not init properly")
        .clone()
}

/// Process-wide HTTP client with bounded timeouts, reused across calls. The
/// timeouts are the point: `reqwest`'s default is *no* request timeout, so a
/// hung upstream (connection held open, no response) would park the calling
/// handler task forever. With these, a stall instead surfaces as an `Err` that
/// the markets/QR call sites already degrade gracefully on. Reusing one client
/// also avoids per-call connection-pool churn.
pub fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .connect_timeout(std::time::Duration::from_secs(4))
            .build()
            .unwrap_or_default()
    })
}

pub async fn reply(ctx: &Context, msg: &Message, text: impl Into<String>) -> Result<(), CommandError> {
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
    use telexide::api::APIEndpoint;
    let text = text.into();
    // Posted via the raw endpoint (not telexide's typed `SendMessage`, which has
    // no `message_thread_id` field) so a group locked to a topic via
    // `/onlyreplyhere` gets its message threaded there rather than into "General".
    let mut payload = serde_json::json!({ "chat_id": chat_id, "text": text });
    if is_group_chat(chat_id) {
        if let Some(thread) = db(ctx).reply_thread(chat_id).ok().flatten() {
            payload["message_thread_id"] = serde_json::json!(thread);
        }
    }
    let resp = ctx.api.post(APIEndpoint::SendMessage, Some(payload)).await?;
    let msg: telexide::Result<Message> = resp.into();
    Ok(msg?)
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
/// Like [`fmt_coins`] but with an explicit leading sign for non-negative
/// amounts (`+5`, `-3`, `0`) — for showing signed deltas like realized P&L.
pub fn fmt_signed_coins(units: i64) -> String {
    if units > 0 {
        format!("+{}", fmt_coins(units))
    } else {
        // fmt_coins already prefixes a real loss with "-"; zero stays "0".
        fmt_coins(units)
    }
}

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
pub fn lang_of(msg: &Message) -> crate::core::i18n::Lang {
    msg.from
        .as_ref()
        .map(crate::core::i18n::Lang::from_user)
        .unwrap_or(crate::core::i18n::Lang::En)
}

/// Resolved locale for `user`: their `/start`-chosen language if saved,
/// otherwise the Telegram-reported one. Prefer this over [`lang_of`] in
/// command handlers so an explicit choice wins everywhere.
pub fn lang_for(ctx: &Context, user: &User) -> crate::core::i18n::Lang {
    db(ctx)
        .get_lang(user.id)
        .ok()
        .flatten()
        .unwrap_or_else(|| crate::core::i18n::Lang::from_user(user))
}

/// [`lang_for`] keyed off a message's sender; English if there is no sender.
pub fn lang_for_msg(ctx: &Context, msg: &Message) -> crate::core::i18n::Lang {
    msg.from
        .as_ref()
        .map(|u| lang_for(ctx, u))
        .unwrap_or(crate::core::i18n::Lang::En)
}

/// [`lang_for`] keyed off a callback query's presser. The single home for what
/// used to be a per-module `cb_lang` copy in every button-flow file.
pub fn cb_lang(ctx: &Context, cb: &CallbackQuery) -> crate::core::i18n::Lang {
    lang_for(ctx, &cb.from)
}

/// True for group / supergroup / channel chats. Telegram gives private chats a
/// positive id (== the user id) and everything else a negative id.
pub fn is_group_chat(chat_id: i64) -> bool {
    chat_id < 0
}

/// Current wall-clock time as unix seconds. The single home for what used to be
/// a per-module `now()` copy in `betting`/`predmarket`/`predict`.
pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// The user's time-of-day bucket for the home-menu greeting, from their saved UTC
/// offset (minutes east; `None` = not picked → UTC). Bands: 05:00–11:59 morning,
/// 12:00–17:59 afternoon, else evening/night. Pure over `now()` + the offset.
pub fn day_part(tz_offset: Option<i64>) -> crate::core::i18n::DayPart {
    day_part_at(now(), tz_offset)
}

/// [`day_part`] at an explicit unix time (for tests).
fn day_part_at(now_unix: i64, tz_offset: Option<i64>) -> crate::core::i18n::DayPart {
    use crate::core::i18n::DayPart;
    let local = now_unix + tz_offset.unwrap_or(0) * 60;
    let hour = local.rem_euclid(86_400) / 3_600; // 0..=23, safe for negative offsets
    match hour {
        5..=11 => DayPart::Morning,
        12..=17 => DayPart::Afternoon,
        _ => DayPart::Evening,
    }
}

/// Parse a colon-separated run of `N` integers out of callback data (e.g.
/// `"12:3:0"` → `[12, 3, 0]`). Returns `None` if any of the first `N` fields is
/// missing or non-numeric; trailing fields are ignored. Every field parses as
/// `i64` — call sites cast to `u64`/`usize` as needed. Replaces the hand-rolled
/// `parse2`/`parse3`/`parse4`/`parse_qid_outcome_total` copies in the button
/// flows (all fields there are numeric and colon-free).
pub fn parse_ints<const N: usize>(rest: &str) -> Option<[i64; N]> {
    let mut it = rest.split(':');
    let mut out = [0i64; N];
    for slot in out.iter_mut() {
        *slot = it.next()?.parse().ok()?;
    }
    Some(out)
}

/// Whole-coin amount presets offered by the buy / spend / fund builders — each
/// button *adds* this much to the running total.
pub const WHOLE_COIN_PRESETS: [i64; 4] = [1, 5, 10, 50];

/// Prefix an **in-group** stake board with a `👤 name` header so members can tell
/// whose owner-locked board is whose. In a private chat (the board's only user is
/// the caller) the body is returned unchanged. The name is plain message text —
/// no parse mode — so it needs no escaping.
pub fn board_header(chat_id: i64, name: &str, body: &str) -> String {
    if is_group_chat(chat_id) {
        format!("👤 {name}\n{body}")
    } else {
        body.to_string()
    }
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

/// Pure decision behind [`out_of_locked_topic`]: given a group's `lock` (the
/// topic it's pinned to, `None` = unlocked) and an incoming message's `thread`
/// (`message_thread_id`), is the interaction outside the locked topic? An unlocked
/// group is never blocked; a locked one blocks everything whose thread isn't
/// exactly the lock — including the General topic (`thread = None`).
fn topic_blocked(lock: Option<i64>, thread: Option<i64>) -> bool {
    match lock {
        Some(t) => thread != Some(t),
        None => false,
    }
}

/// `/onlyreplyhere`: when a group is locked to a forum topic, this returns `true`
/// for any interaction that originates **outside** that topic — the bot should
/// stay silent there. `false` for private chats, unlocked groups, and the locked
/// topic itself. `thread` is the incoming message's `message_thread_id`.
pub fn out_of_locked_topic(ctx: &Context, chat_id: i64, thread: Option<i64>) -> bool {
    if !is_group_chat(chat_id) {
        return false;
    }
    topic_blocked(db(ctx).reply_thread(chat_id).ok().flatten(), thread)
}

/// Gate for the admin pause kill-switch. Returns `true` (and tells the caller
/// the bot is paused) when the bot is paused and the actor is **not** the
/// owner; command handlers should early-return when it does. The owner is
/// always allowed through so they can still `/unpause` and operate.
pub async fn paused_block(ctx: &Context, msg: &Message) -> Result<bool, CommandError> {
    // Learn this chat (private or group) so `/broadcast` can reach it later.
    db(ctx).touch_chat(msg.chat.get_id()).ok();
    // `/onlyreplyhere`: outside the locked topic the bot stays completely silent
    // (no reply), so report "blocked" without sending anything. The lock commands
    // themselves don't call this gate, so an admin can always re-lock/unlock.
    if out_of_locked_topic(ctx, msg.chat.get_id(), msg.message_thread_id) {
        return Ok(true);
    }
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
        reply(
            ctx,
            msg,
            crate::core::i18n::service_paused(lang_for_msg(ctx, msg)),
        )
        .await?;
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

/// Standard command entry guard: enforce the pause kill-switch, then resolve the
/// sender and their locale. Returns `None` — meaning the command body should
/// early-return `Ok(())` — when the bot is paused for this user, or the message
/// has no sender (a bare [`ERR_REPLY`] is sent in that case). Otherwise
/// `Some((sender, locale))`. Mirrors `admin::owner_guard`'s style for the
/// non-owner path and collapses the ~7-line preamble repeated across every text
/// command into `let Some((user, lang)) = begin(&ctx, &message).await? else { … };`.
pub async fn begin<'a>(
    ctx: &Context,
    msg: &'a Message,
) -> Result<Option<(&'a User, crate::core::i18n::Lang)>, CommandError> {
    if paused_block(ctx, msg).await? {
        return Ok(None);
    }
    let Some(user) = msg.from.as_ref() else {
        reply(ctx, msg, ERR_REPLY).await?;
        return Ok(None);
    };
    let lang = lang_for(ctx, user);
    Ok(Some((user, lang)))
}

/// After trying to DM a user to open a private flow (`/predict`, `/feedback`, the
/// group `/settings` hub), point them at the outcome: when the DM **landed**, a
/// group caller gets `check_dm` ("look in your DMs") and a private caller nothing
/// (they're already there); when it **bounced** (they never started the bot),
/// `dm_first` ("message me first"). Collapses the identical branch these flows
/// each hand-rolled. Best-effort on the group "check DM" ack (the real surface —
/// the DM — already landed).
pub async fn dm_pointer(
    ctx: &Context,
    msg: &Message,
    landed: bool,
    check_dm: &str,
    dm_first: &str,
) -> Result<(), CommandError> {
    if landed {
        if is_group_chat(msg.chat.get_id()) {
            let _ = reply(ctx, msg, check_dm).await;
        }
    } else {
        reply(ctx, msg, dm_first).await?;
    }
    Ok(())
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

/// `ts` (unix seconds) in `tz_min` (minutes east of UTC) → `"Jun 27 · 17:00
/// UTC+8"` (local time + offset label). `None` only on an out-of-range
/// offset/timestamp. Shared by the `/markets` brief and the `/predict` deadline.
pub fn fmt_local_time(ts: i64, tz_min: i64) -> Option<String> {
    use chrono::{FixedOffset, TimeZone};
    let dt = FixedOffset::east_opt((tz_min * 60) as i32)?
        .timestamp_opt(ts, 0)
        .single()?;
    Some(format!("{} {}", dt.format("%b %-d · %H:%M"), tz_label(tz_min)))
}

/// Date-only in `tz_min`, e.g. `"Jun 27"` (no time, no label).
pub fn fmt_local_date(ts: i64, tz_min: i64) -> String {
    use chrono::{FixedOffset, TimeZone};
    FixedOffset::east_opt((tz_min * 60) as i32)
        .and_then(|o| o.timestamp_opt(ts, 0).single())
        .map(|d| d.format("%b %-d").to_string())
        .unwrap_or_default()
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
pub fn format_odds(odds_cents: f64, fmt: crate::core::types::OddsFormat) -> String {
    use crate::core::types::OddsFormat;
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
    use crate::core::i18n::DayPart;
    use crate::database::COIN;

    #[test]
    fn day_part_bands_by_local_hour() {
        // Anchor at a known UTC midnight; add hours to probe each band. tz = None → UTC.
        let midnight = 1_704_067_200; // 2024-01-01 00:00:00 UTC
        let at = |h: i64| midnight + h * 3600;
        assert_eq!(day_part_at(at(0), None), DayPart::Evening); // 00:00 → night
        assert_eq!(day_part_at(at(4), None), DayPart::Evening);
        assert_eq!(day_part_at(at(5), None), DayPart::Morning); // band starts
        assert_eq!(day_part_at(at(11), None), DayPart::Morning);
        assert_eq!(day_part_at(at(12), None), DayPart::Afternoon);
        assert_eq!(day_part_at(at(17), None), DayPart::Afternoon);
        assert_eq!(day_part_at(at(18), None), DayPart::Evening);
        assert_eq!(day_part_at(at(23), None), DayPart::Evening);
        // Timezone offset shifts the local hour: 00:00 UTC + 9h (JST) = 09:00 → morning.
        assert_eq!(day_part_at(at(0), Some(9 * 60)), DayPart::Morning);
        // Negative offset wraps correctly: 00:00 UTC − 5h = 19:00 prev day → evening.
        assert_eq!(day_part_at(at(0), Some(-5 * 60)), DayPart::Evening);
    }

    #[test]
    fn topic_blocked_truth_table() {
        // Unlocked group: never blocked, whatever topic the message is in.
        assert!(!topic_blocked(None, None));
        assert!(!topic_blocked(None, Some(42)));
        // Locked to topic 42: only topic 42 passes.
        assert!(!topic_blocked(Some(42), Some(42))); // the locked topic → allowed
        assert!(topic_blocked(Some(42), None)); // General (no thread) → blocked
        assert!(topic_blocked(Some(42), Some(7))); // a different topic → blocked
    }

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
        use crate::core::types::OddsFormat::{American, Decimal, Percent, Price};
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
