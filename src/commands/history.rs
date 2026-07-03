use crate::commands::util::*;
use crate::commands::{menu, tg};
use crate::core::i18n::{self, Lang};
use crate::database::{
    HistoryRow, HistoryTab, HK_BUY, HK_CHECKIN, HK_CLAIM, HK_LP_FUND, HK_LP_RETURN, HK_MINT, HK_REFERRAL,
    HK_REFUND, HK_SELL, HK_SEND_IN, HK_SEND_OUT,
};
use telexide::model::User;
use telexide::prelude::*;

/// How many recent actions each `/history` view shows (newest first).
const HISTORY_LIMIT: i64 = 20;

/// Callback prefix for the filter tabs — `hist:<mining|trading|transfer>`.
pub const HIST_TAB: &str = "hist:";

/// The three filter tabs, in display order, with their callback suffix.
const TABS: [(HistoryTab, &str); 3] = [
    (HistoryTab::Mining, "mining"),
    (HistoryTab::Trading, "trading"),
    (HistoryTab::Transfer, "transfer"),
];

/// `/history` — the caller's own recent money/position activity. In a **private**
/// chat it's a tabbed statement (Mining / Trading / Transfer) that filters in
/// place; in a **group** it's a static flat list (no tabs — a shared message must
/// not swap to another member's view on a tap). Group-safe either way: no
/// counterparty names are rendered.
#[command(description = "show your recent activity")]
pub async fn history(ctx: Context, message: Message) -> CommandResult {
    let Some((user, lang)) = begin(&ctx, &message).await? else {
        return Ok(());
    };
    if is_group_chat(message.chat.get_id()) {
        let body = flat_history_text(&ctx, lang, user).await;
        reply(&ctx, &message, body).await?;
    } else {
        let (text, rows) = tab_view(&ctx, lang, user, HistoryTab::Mining).await;
        tg::send_with_buttons(&ctx, message.chat.get_id(), &text, &rows).await?;
    }
    Ok(())
}

/// Parse a `hist:` callback suffix into its tab (`None` for an unknown suffix).
pub fn parse_tab(suffix: &str) -> Option<HistoryTab> {
    TABS.iter().find(|(_, s)| *s == suffix).map(|(t, _)| *t)
}

fn tab_label(lang: Lang, tab: HistoryTab) -> &'static str {
    match tab {
        HistoryTab::Mining => i18n::hist_tab_mining(lang),
        HistoryTab::Trading => i18n::hist_tab_trading(lang),
        HistoryTab::Transfer => i18n::hist_tab_transfer(lang),
    }
}

/// The filter-tab keyboard: one row of the three tabs + a back-to-home row.
fn tab_rows(lang: Lang) -> Vec<tg::Row> {
    let tabs: tg::Row = TABS
        .iter()
        .map(|(tab, suffix)| (tab_label(lang, *tab).to_string(), format!("{HIST_TAB}{suffix}")))
        .collect();
    vec![
        tabs,
        vec![(i18n::bet_btn_back(lang).to_string(), menu::MENU_HOME.to_string())],
    ]
}

/// The tabbed view for `tab`: the filtered statement + the tab keyboard. Shared by
/// the private `/history` command, the `menu:history` home button, and the `hist:`
/// tab switches. Renders for the acting user in their saved timezone (0 = UTC).
pub async fn tab_view(ctx: &Context, lang: Lang, user: &User, tab: HistoryTab) -> (String, Vec<tg::Row>) {
    let database = db(ctx);
    let header = format!("{} · {}", i18n::history_title(lang), tab_label(lang, tab));
    let body = match database.user_history_by(user.id, tab, HISTORY_LIMIT) {
        Ok(rows) if rows.is_empty() => format!("{header}\n\n{}", i18n::history_empty(lang)),
        Ok(rows) => {
            let tz = database.get_tz(user.id).ok().flatten().unwrap_or(0);
            format!("{header}\n\n{}", render_lines(lang, tz, &rows))
        }
        Err(e) => {
            eprintln!("user_history_by error (user {}): {e}", user.id);
            format!("{header}\n\n{}", i18n::db_error(lang))
        }
    };
    (body, tab_rows(lang))
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
