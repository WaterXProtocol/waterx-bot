use crate::commands::util::*;
use crate::core::i18n::{self, Lang};
use crate::database::{
    HK_BUY, HK_CHECKIN, HK_CLAIM, HK_LP_FUND, HK_LP_RETURN, HK_MINT, HK_REFERRAL, HK_REFUND, HK_SELL,
    HK_SEND_IN, HK_SEND_OUT,
};
use telexide::model::User;
use telexide::prelude::*;

/// How many recent actions `/history` shows (newest first).
const HISTORY_LIMIT: i64 = 20;

/// `/history` — the caller's own recent money/position activity, a bank-statement
/// view. Works in **both** DM and group (an opt-in show-off, like `/balance` and
/// `/bets`); no counterparty names are rendered, so it's group-safe.
#[command(description = "show your recent activity")]
pub async fn history(ctx: Context, message: Message) -> CommandResult {
    let Some((user, lang)) = begin(&ctx, &message).await? else {
        return Ok(());
    };
    let body = history_text(&ctx, lang, user).await;
    reply(&ctx, &message, body).await?;
    Ok(())
}

/// Render the caller's activity statement — shared by the `/history` command and
/// the `menu:history` home-page button (which edits it in place). Times render in
/// the user's saved timezone (0 = UTC when unset). Falls back to a db-error notice
/// rather than a misleading empty statement.
pub async fn history_text(ctx: &Context, lang: Lang, user: &User) -> String {
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
    let lines: Vec<String> = rows
        .iter()
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
        .collect();
    format!(
        "{}\n{}\n\n{}",
        i18n::history_title(lang),
        full_name(user),
        lines.join("\n")
    )
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
