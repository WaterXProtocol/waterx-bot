//! Shared builders for the button-driven `/start` flow: the language picker and
//! the Xaliah main menu. Both the `/start` command (which *sends* a fresh
//! message) and the callback handlers (which *edit* the existing one) reuse
//! these so the keyboards stay in one place.

use crate::bot::BotUsernameKey;
use crate::commands::tg::Row;
use crate::commands::util::{db, fmt_coins};
use crate::i18n::{self, Lang};
use telexide::prelude::Context;

/// Callback-data prefixes routed in `callbacks::on_callback`.
pub const SET_LANG: &str = "setlang:";
pub const MENU_CHECKIN: &str = "menu:checkin";
pub const MENU_MATCHES: &str = "menu:matches";
pub const MENU_INVITE: &str = "menu:invite";

/// A user's personal referral deep link: opening it sends `/start <user_id>`.
pub fn referral_link(bot_username: &str, user_id: i64) -> String {
    format!("https://t.me/{bot_username}?start={user_id}")
}

/// Language-picker keyboard: every supported locale, two per row, labelled with
/// its flag + endonym. Payload is `setlang:<store_code>`.
pub fn lang_picker_rows() -> Vec<Row> {
    Lang::ALL
        .chunks(2)
        .map(|pair| {
            pair.iter()
                .map(|l| {
                    (
                        l.native_label().to_string(),
                        format!("{}{}", SET_LANG, l.store_code()),
                    )
                })
                .collect()
        })
        .collect()
}

/// The Xaliah main-menu body: intro line followed by the user's current
/// balance and fruit inventory.
pub fn menu_text(ctx: &Context, lang: Lang, user_id: i64) -> String {
    let info = db(ctx).get_user_info(user_id).unwrap_or_default();
    let fruits = if info.fruit.is_empty() {
        "—".to_string()
    } else {
        info.fruit
    };
    format!(
        "{}\n\n{}\n\n{}",
        i18n::intro(lang),
        i18n::menu_status(lang, &fmt_coins(info.balance), &fruits),
        i18n::menu_prompt(lang)
    )
}

/// The Xaliah main-menu keyboard: today's matches, the daily check-in button
/// (only when claimable), the invite button, and a forward-safe `[Play]`
/// deep-link button. Telegram strips callback buttons from a forwarded message
/// but keeps URL buttons — so the `[Play]` link is how a forwarded home page
/// still earns `user_id` a referral (recipient taps it → `/start <user_id>`).
pub fn main_menu_rows(ctx: &Context, lang: Lang, user_id: i64, checkin_available: bool) -> Vec<Row> {
    let mut row: Row = Vec::new();
    if checkin_available {
        row.push((i18n::btn_checkin(lang).to_string(), MENU_CHECKIN.to_string()));
    }
    row.push((i18n::btn_matches(lang).to_string(), MENU_MATCHES.to_string()));

    let username = ctx
        .data
        .read()
        .get::<BotUsernameKey>()
        .cloned()
        .unwrap_or_default();
    vec![
        row,
        vec![(i18n::btn_invite(lang).to_string(), MENU_INVITE.to_string())],
        vec![(i18n::btn_join(lang).to_string(), referral_link(&username, user_id))],
    ]
}
