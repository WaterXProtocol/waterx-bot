//! Shared builders for the button-driven `/start` flow: the language picker and
//! the Xaliah main menu. Both the `/start` command (which *sends* a fresh
//! message) and the callback handlers (which *edit* the existing one) reuse
//! these so the keyboards stay in one place.

use crate::commands::tg::Row;
use crate::commands::util::{format_odds, tz_button_label};
use crate::core::i18n::{self, Lang};
use crate::core::types::OddsFormat;

/// Callback-data prefixes routed in `callbacks::on_callback`.
pub const SET_LANG: &str = "setlang:";
pub const SET_TZ: &str = "settz:";
/// `/settings` hub: three uniform click-in buttons, each opening its own picker.
pub const SET_FMT: &str = "setfmt:"; // setfmt:<store_code> — persist + back to hub
pub const CFG_LANG: &str = "cfg:lang"; // open the language picker
pub const CFG_TZ: &str = "cfg:tz"; // open the timezone picker
pub const CFG_ODDS: &str = "cfg:odds"; // open the odds-format picker
pub const CFG_HOME: &str = "cfg:home"; // back to the settings hub
/// Settings-variant pick callbacks: persist + return to the hub (vs the onboarding
/// `setlang:`/`settz:`, which chain lang→timezone→menu).
pub const SLANG: &str = "slang:"; // slang:<store_code>
pub const STZ: &str = "stz:"; // stz:<minutes>

/// `/settings` hub keyboard: three uniform buttons — `[🌐 Language]`,
/// `[🕐 Timezone]`, `[🎲 Format]` — each opening its own picker (where the
/// current choice is `✅`-marked). Built by `/settings` and the `cfg:home` /
/// `setfmt:` re-renders.
pub fn settings_rows(lang: Lang) -> Vec<Row> {
    vec![
        vec![(i18n::btn_language(lang).to_string(), CFG_LANG.to_string())],
        vec![(i18n::btn_timezone(lang).to_string(), CFG_TZ.to_string())],
        vec![(i18n::btn_odds(lang).to_string(), CFG_ODDS.to_string())],
        vec![(i18n::bet_btn_back(lang).to_string(), MENU_HOME.to_string())],
    ]
}

/// Odds-format picker (reached via `cfg:odds`): one button per format, labelled
/// with a **live example** for a 65¢ price (so the formats need no localized
/// names), `✅` on the current one; `setfmt:<code>` saves + returns to the hub.
/// Plus a `[⬅ Back]` to the hub.
pub fn odds_picker_rows(lang: Lang, current: OddsFormat) -> Vec<Row> {
    const REF_CENTS: f64 = 65.0;
    let mut rows: Vec<Row> = OddsFormat::ALL
        .chunks(2)
        .map(|chunk| {
            chunk
                .iter()
                .map(|&f| {
                    let mark = if f == current { "✅ " } else { "" };
                    (
                        format!("{mark}{}", format_odds(REF_CENTS, f)),
                        format!("{SET_FMT}{}", f.store_code()),
                    )
                })
                .collect()
        })
        .collect();
    rows.push(vec![(i18n::bet_btn_back(lang).to_string(), CFG_HOME.to_string())]);
    rows
}

/// Curated UTC offsets (minutes east) offered in the timezone picker — common
/// Every real-world UTC offset (minutes east of UTC), -12:00 … +14:00, including
/// the :30/:45 ones — so the picker lists all timezones.
const TZ_OFFSETS: &[i64] = &[
    -720, -660, -600, -570, -540, -480, -420, -360, -300, -240, -210, -180, -120, -60, 0, 60, 120,
    180, 210, 240, 270, 300, 330, 345, 360, 390, 420, 480, 525, 540, 570, 600, 630, 660, 720, 765,
    780, 840,
];

/// Timezone picker: one button per curated offset, four per row, labelled
/// `UTC±h[:mm]`. `current` (the user's saved offset, if any) is `✅`-marked.
/// `settings`: when opened from the `/settings` hub the pick emits `stz:` (persist
/// + back to hub); from onboarding it emits `settz:` (chain → main menu).
pub fn tz_picker_rows(current: Option<i64>, settings: bool) -> Vec<Row> {
    let prefix = if settings { STZ } else { SET_TZ };
    TZ_OFFSETS
        .chunks(6)
        .map(|chunk| {
            chunk
                .iter()
                .map(|&m| {
                    let mark = if current == Some(m) { "✅ " } else { "" };
                    (format!("{mark}{}", tz_button_label(m)), format!("{prefix}{m}"))
                })
                .collect()
        })
        .collect()
}
pub const MENU_CHECKIN: &str = "menu:checkin";
pub const MENU_HOME: &str = "menu:home";

/// A single `[🏠 Home page]` row → the `/start` menu (`MENU_HOME`), for terminal
/// flow nodes (placed bet, funded stake, expired board, created prediction) that
/// would otherwise dead-end with no way back.
pub fn home_row(lang: Lang) -> Vec<Row> {
    vec![vec![(i18n::home_btn(lang), MENU_HOME.to_string())]]
}
pub const MENU_BALANCE: &str = "menu:balance";
/// Edit-in-place `/bets` view (open positions only) — the sell flow's back target.
pub const MENU_BETS: &str = "menu:bets";
pub const MENU_MARKETS: &str = "menu:markets";
pub const MENU_RULE: &str = "menu:rule";
pub const MENU_INVITE: &str = "menu:invite";
/// Group-only home-page button: open the `/predict` builder (handled like the
/// `/predict` command).
pub const MENU_PREDICT: &str = "menu:predict";
/// Invite-format chooser (shown after `menu:invite`).
pub const INVITE_LINK: &str = "inv:link";
pub const INVITE_FWD: &str = "inv:fwd";
pub const INVITE_QR: &str = "inv:qr";

/// A user's personal referral deep link: opening it sends `/start <user_id>`.
pub fn referral_link(bot_username: &str, user_id: i64) -> String {
    format!("https://t.me/{bot_username}?start={user_id}")
}

/// Language-picker keyboard: every supported locale, two per row, labelled with
/// its flag + endonym. `current` (the user's saved locale, if any) is `✅`-marked.
/// `settings`: when opened from the `/settings` hub the pick emits `slang:`
/// (persist + back to hub); from onboarding it emits `setlang:` (chain →
/// timezone → main menu).
pub fn lang_picker_rows(current: Option<Lang>, settings: bool) -> Vec<Row> {
    let prefix = if settings { SLANG } else { SET_LANG };
    Lang::ALL
        .chunks(2)
        .map(|pair| {
            pair.iter()
                .map(|l| {
                    let mark = if current == Some(*l) { "✅ " } else { "" };
                    (
                        format!("{mark}{}", l.native_label()),
                        format!("{prefix}{}", l.store_code()),
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

/// The Xaliah main-menu keyboard: today's markets, the daily check-in button
/// (only when claimable), and the invite button. The home page shows the user's
/// balance/fruit, so it deliberately carries **no** referral deep-link button —
/// the `[Play]` URL button lives only in the `menu:invite` output, which has no
/// private info and is meant to be shared.
pub fn main_menu_rows(lang: Lang, checkin_available: bool, is_group: bool) -> Vec<Row> {
    // One button per row (vertical stack), check-in on top only when claimable.
    // In a **group** the menu is a single shared message, so the per-user actions
    // ([Check assets], [Invite friends]) are hidden — only the shared check-in
    // and today's-markets buttons show.
    let mut rows: Vec<Row> = Vec::new();
    if checkin_available {
        rows.push(vec![(
            i18n::btn_checkin(lang).to_string(),
            MENU_CHECKIN.to_string(),
        )]);
    }
    if !is_group {
        rows.push(vec![(
            i18n::btn_balance(lang).to_string(),
            MENU_BALANCE.to_string(),
        )]);
    }
    rows.push(vec![(
        i18n::btn_markets(lang).to_string(),
        MENU_MARKETS.to_string(),
    )]);
    if is_group {
        // Group-only: kick off a shared prediction (handled like `/predict`), and
        // restore a prediction card that was deleted from this group.
        rows.push(vec![(
            i18n::btn_predict(lang).to_string(),
            MENU_PREDICT.to_string(),
        )]);
    } else {
        // Private-only: the rules brief and the personal invite chooser.
        rows.push(vec![(i18n::btn_rule(lang).to_string(), MENU_RULE.to_string())]);
        rows.push(vec![(
            i18n::btn_invite(lang).to_string(),
            MENU_INVITE.to_string(),
        )]);
    }
    rows
}
