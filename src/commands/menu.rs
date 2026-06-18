//! Shared builders for the button-driven `/start` flow: the language picker and
//! the Xaliah main menu. Both the `/start` command (which *sends* a fresh
//! message) and the callback handlers (which *edit* the existing one) reuse
//! these so the keyboards stay in one place.

use crate::commands::tg::Row;
use crate::i18n::{self, Lang};
use telexide::prelude::Context;

/// Callback-data prefixes routed in `callbacks::on_callback`.
pub const SET_LANG: &str = "setlang:";
pub const MENU_CHECKIN: &str = "menu:checkin";
pub const MENU_BALANCE: &str = "menu:balance";
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
/// balance. (Fruit is hidden until the fruit feature is designed.)
pub fn menu_text(lang: Lang, name: &str) -> String {
    // Balance is intentionally not shown here — the home page has a
    // [Check assets] button for that. The menu body is just the greeting.
    i18n::intro(lang, name)
}

/// The Xaliah main-menu keyboard: today's matches, the daily check-in button
/// (only when claimable), and the invite button. The home page shows the user's
/// balance/fruit, so it deliberately carries **no** referral deep-link button —
/// the `[Play]` URL button lives only in the `menu:invite` output, which has no
/// private info and is meant to be shared.
pub fn main_menu_rows(_ctx: &Context, lang: Lang, _user_id: i64, checkin_available: bool) -> Vec<Row> {
    // One button per row (vertical stack), check-in on top only when claimable.
    let mut rows: Vec<Row> = Vec::new();
    if checkin_available {
        rows.push(vec![(
            i18n::btn_checkin(lang).to_string(),
            MENU_CHECKIN.to_string(),
        )]);
    }
    rows.push(vec![(
        i18n::btn_balance(lang).to_string(),
        MENU_BALANCE.to_string(),
    )]);
    rows.push(vec![(
        i18n::btn_matches(lang).to_string(),
        MENU_MATCHES.to_string(),
    )]);
    rows.push(vec![(
        i18n::btn_invite(lang).to_string(),
        MENU_INVITE.to_string(),
    )]);
    rows
}
