use crate::commands::util::*;
use crate::commands::{menu, tg};
use crate::core::i18n::{self, Lang};
use crate::database::{
    HistoryRow, HistoryTab, HK_BUY, HK_CHECKIN, HK_CLAIM, HK_LP_FUND, HK_LP_RETURN, HK_MINT, HK_REFERRAL,
    HK_REFUND, HK_SELL, HK_SEND_IN, HK_SEND_OUT,
};
use telexide::model::User;
use telexide::prelude::*;

/// Rows per page in a category's paginated `/history` view.
const HISTORY_PAGE: i64 = 10;
/// Rows in the flat **group** `/history` view (no pagination there).
const HISTORY_LIMIT: i64 = 20;

/// Callback prefix for a history category page — `hist:<tab>:<page>`.
pub const HIST_TAB: &str = "hist:";

/// The three categories, in display order, with their callback suffix.
const TABS: [(HistoryTab, &str); 3] = [
    (HistoryTab::Mining, "mining"),
    (HistoryTab::Trading, "trading"),
    (HistoryTab::Transfer, "transfer"),
];

/// `/history` — the caller's own recent money/position activity. In a **private**
/// chat it opens a **category menu** (Mining / Trading / Transfer); picking one
/// shows that category's **paginated** statement. In a **group** it's a static
/// flat list (no menu — a shared message must not swap to another member's view on
/// a tap). Group-safe either way: no counterparty names are rendered.
#[command(description = "show your recent activity")]
pub async fn history(ctx: Context, message: Message) -> CommandResult {
    let Some((user, lang)) = begin(&ctx, &message).await? else {
        return Ok(());
    };
    if is_group_chat(message.chat.get_id()) {
        let body = flat_history_text(&ctx, lang, user).await;
        reply(&ctx, &message, body).await?;
    } else {
        let (text, rows) = picker(lang);
        tg::send_with_buttons(&ctx, message.chat.get_id(), &text, &rows).await?;
    }
    Ok(())
}

/// Screen 1 — the category menu: one button per category (each opens its page 0) +
/// a back-to-home row. Shared by the `/history` command and the `menu:history`
/// home button (which edits it in place).
pub fn picker(lang: Lang) -> (String, Vec<tg::Row>) {
    let mut rows: Vec<tg::Row> = TABS
        .iter()
        .map(|(tab, suffix)| vec![(tab_label(lang, *tab).to_string(), format!("{HIST_TAB}{suffix}:0"))])
        .collect();
    rows.push(vec![(
        i18n::bet_btn_back(lang).to_string(),
        menu::MENU_HOME.to_string(),
    )]);
    (i18n::history_title(lang).to_string(), rows)
}

/// Parse a `hist:` callback suffix `<tab>:<page>` into `(tab, page)`.
pub fn parse_tab_page(suffix: &str) -> Option<(HistoryTab, i64)> {
    let (tab_s, page_s) = suffix.split_once(':')?;
    Some((parse_tab(tab_s)?, page_s.parse().ok()?))
}

fn parse_tab(s: &str) -> Option<HistoryTab> {
    TABS.iter().find(|(_, suffix)| *suffix == s).map(|(t, _)| *t)
}

fn suffix_of(tab: HistoryTab) -> &'static str {
    TABS.iter()
        .find(|(t, _)| *t == tab)
        .map(|(_, s)| *s)
        .unwrap_or("mining")
}

fn tab_label(lang: Lang, tab: HistoryTab) -> &'static str {
    match tab {
        HistoryTab::Mining => i18n::hist_tab_mining(lang),
        HistoryTab::Trading => i18n::hist_tab_trading(lang),
        HistoryTab::Transfer => i18n::hist_tab_transfer(lang),
    }
}

/// Screen 2 — one category's paginated statement (`page` is 0-based, clamped into
/// range so a stale button can't overshoot): the page's rows + a `[◂ Prev][Next ▸]`
/// row (each side shown only when it exists) + a back-to-**menu** row. Renders for
/// the acting user in their saved timezone (0 = UTC).
pub async fn page_view(
    ctx: &Context,
    lang: Lang,
    user: &User,
    tab: HistoryTab,
    page: i64,
) -> (String, Vec<tg::Row>) {
    let database = db(ctx);
    let total = database.user_history_count_by(user.id, tab).unwrap_or(0);
    // usize::div_ceil is stable (i64's isn't); total is a non-negative COUNT.
    let pages = (total.max(0) as usize).div_ceil(HISTORY_PAGE as usize).max(1) as i64;
    let page = page.clamp(0, pages - 1);
    let mut head = format!("{} · {}", i18n::history_title(lang), tab_label(lang, tab));
    if pages > 1 {
        head.push('\n');
        head.push_str(&i18n::markets_page(
            lang,
            &(page + 1).to_string(),
            &pages.to_string(),
        ));
    }
    let body = match database.user_history_by(user.id, tab, HISTORY_PAGE, page * HISTORY_PAGE) {
        Ok(rows) if rows.is_empty() => format!("{head}\n\n{}", i18n::history_empty(lang)),
        Ok(rows) => {
            let tz = database.get_tz(user.id).ok().flatten().unwrap_or(0);
            format!("{head}\n\n{}", render_lines(lang, tz, &rows))
        }
        Err(e) => {
            eprintln!("user_history_by error (user {}): {e}", user.id);
            format!("{head}\n\n{}", i18n::db_error(lang))
        }
    };
    let suffix = suffix_of(tab);
    let mut nav: tg::Row = Vec::new();
    if page > 0 {
        nav.push((
            i18n::markets_prev(lang).to_string(),
            format!("{HIST_TAB}{suffix}:{}", page - 1),
        ));
    }
    if page + 1 < pages {
        nav.push((
            i18n::markets_next(lang).to_string(),
            format!("{HIST_TAB}{suffix}:{}", page + 1),
        ));
    }
    let mut rows = Vec::new();
    if !nav.is_empty() {
        rows.push(nav);
    }
    // Back returns to the category menu (Screen 1), not straight home.
    rows.push(vec![(
        i18n::bet_btn_back(lang).to_string(),
        menu::MENU_HISTORY.to_string(),
    )]);
    (body, rows)
}

/// The **group** `/history` view: a flat, all-categories statement (no tabs).
/// Falls back to a db-error notice rather than a misleading empty statement.
pub async fn flat_history_text(ctx: &Context, lang: Lang, user: &User) -> String {
    let database = db(ctx);
    let rows = match database.user_history(user.id, HISTORY_LIMIT) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("user_history error (user {}): {e}", user.id);
            return format!("{}\n{}", full_name(user), i18n::db_error(lang));
        }
    };
    if rows.is_empty() {
        return format!("{}\n{}", full_name(user), i18n::history_empty(lang));
    }
    let tz = database.get_tz(user.id).ok().flatten().unwrap_or(0);
    format!(
        "{}\n{}\n\n{}",
        i18n::history_title(lang),
        full_name(user),
        render_lines(lang, tz, &rows)
    )
}

/// Render history rows to newest-first statement lines (shared by both views):
/// `<local time> — <emoji> <label>[ · <event title>]  <±coins>🪙`.
fn render_lines(lang: Lang, tz: i64, rows: &[HistoryRow]) -> String {
    rows.iter()
        .map(|h| {
            let when = fmt_local_time(h.at, tz).unwrap_or_default();
            let (emoji, label) = kind_label(lang, &h.kind);
            let ctx_str = match h.event_title.as_deref() {
                Some(t) if !t.is_empty() => format!(" · {t}"),
                _ => String::new(),
            };
            format!(
                "{when} — {emoji} {label}{ctx_str}  {}🪙",
                fmt_signed_coins(h.delta)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Map an action tag to its display emoji + localized label. The tags come from
/// the DB's `HK_*` consts, so this can't drift from what the record sites write.
fn kind_label(l: Lang, kind: &str) -> (&'static str, &'static str) {
    match kind {
        k if k == HK_BUY => ("🛒", i18n::hist_buy(l)),
        k if k == HK_SELL => ("💰", i18n::hist_sell(l)),
        k if k == HK_SEND_OUT => ("💸", i18n::hist_send_out(l)),
        k if k == HK_SEND_IN => ("🎁", i18n::hist_send_in(l)),
        k if k == HK_CHECKIN => ("📅", i18n::hist_checkin(l)),
        k if k == HK_REFERRAL => ("🤝", i18n::hist_referral(l)),
        k if k == HK_CLAIM => ("🏆", i18n::hist_claim(l)),
        k if k == HK_REFUND => ("↩️", i18n::hist_refund(l)),
        k if k == HK_MINT => ("🪄", i18n::hist_mint(l)),
        k if k == HK_LP_FUND => ("💧", i18n::hist_lp_fund(l)),
        k if k == HK_LP_RETURN => ("🌊", i18n::hist_lp_return(l)),
        _ => ("•", ""),
    }
}
