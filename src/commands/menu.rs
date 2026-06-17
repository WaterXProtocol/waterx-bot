//! Shared builders for the button-driven `/start` flow: the language picker and
//! the Xaliah main menu. Both the `/start` command (which *sends* a fresh
//! message) and the callback handlers (which *edit* the existing one) reuse
//! these so the keyboards stay in one place.

use crate::commands::tg::Row;
use crate::i18n::{self, Lang};

/// Callback-data prefixes routed in `callbacks::on_callback`.
pub const SET_LANG: &str = "setlang:";
pub const MENU_CHECKIN: &str = "menu:checkin";
pub const MENU_MATCHES: &str = "menu:matches";

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

/// The Xaliah main-menu keyboard: today's matches, plus the daily check-in
/// button only when it's actually claimable (`checkin_available`).
pub fn main_menu_rows(lang: Lang, checkin_available: bool) -> Vec<Row> {
    let mut row: Row = Vec::new();
    if checkin_available {
        row.push((i18n::btn_checkin(lang).to_string(), MENU_CHECKIN.to_string()));
    }
    row.push((i18n::btn_matches(lang).to_string(), MENU_MATCHES.to_string()));
    vec![row]
}
