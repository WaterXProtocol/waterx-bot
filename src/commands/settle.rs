//! `/settle` — public **manual** settle of resolved markets, and the periodic
//! [`auto_settle`] task. Both detect Polymarket (Gamma) resolution for every open
//! **sourced** (`/events`) event, cache the winner on the row, then pay out
//! **every** resolved/void position — sourced (oracle-decided) *and* any
//! host-resolved AMM (`/predict`) event whose resolve-time settle didn't land.
//! Settlement is deterministic (the oracle / host already picked the winner), so
//! anyone can run `/settle`; it's the manual fallback if the 5-min auto-settle
//! task ever stalls. (There is no per-user `/claim` anymore — everything settles
//! automatically.)

use crate::commands::util::*;
use crate::commands::{markets, tg};
use crate::core::i18n::{self, Lang};
use crate::database::{ClaimKind, Database, Payout};
use std::collections::HashSet;
use telexide::prelude::*;

#[command(description = "settle resolved markets")]
pub async fn settle(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let lang = lang_for_msg(&ctx, &message);
    let database = db(&ctx);
    let payouts = match sweep(&database).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("settle sweep error: {e}");
            reply(&ctx, &message, i18n::db_error(lang)).await?;
            return Ok(());
        }
    };
    // DM each winner/refundee (same as the auto-settle task), then reply to the
    // caller with the sweep summary.
    notify_settled(&bot_token(&ctx), &database, &payouts).await;
    if payouts.is_empty() {
        reply(&ctx, &message, i18n::settle_nothing(lang)).await?;
        return Ok(());
    }
    let events: HashSet<i64> = payouts.iter().map(|p| p.event_id).collect();
    let paid: i64 = payouts.iter().map(|p| p.coins).sum();
    reply(
        &ctx,
        &message,
        i18n::settle_done(lang, &events.len().to_string(), &fmt_coins(paid)),
    )
    .await?;
    Ok(())
}

/// Detect Gamma resolution for every open sourced event (caching the winner on
/// the row), then settle **all** resolved/void positions — sourced and AMM.
/// Shared by the `/settle` command and the periodic [`auto_settle`] task.
pub async fn sweep(database: &Database) -> rusqlite::Result<Vec<Payout>> {
    let now = now();
    for (event_id, slug, n_outcomes) in database.all_open_sourced().unwrap_or_default() {
        if slug.is_empty() {
            continue;
        }
        if let Ok(Some(idx)) = markets::gamma_resolution(&slug, n_outcomes as usize).await {
            let _ = database.resolve_event(event_id, idx, now);
        }
    }
    database.settle_all_resolved()
}

/// The periodic auto-settle task body (every `bot::AUTO_SETTLE_INTERVAL`): run the
/// [`sweep`], then DM each credited user ([`notify_settled`]). Logs on sweep error.
/// The `token` is used to send DMs directly via the Bot API — the task runs outside
/// the telexide framework, so it has no `Context`.
pub async fn auto_settle(token: &str, database: &Database) {
    match sweep(database).await {
        Ok(payouts) => notify_settled(token, database, &payouts).await,
        Err(e) => eprintln!("[auto-settle] sweep error: {e}"),
    }
}

/// DM each user a settlement credited — winners (`Won`) and void refunds
/// (`Refunded`), i.e. every payout with `coins > 0`. Pure losers (`coins == 0`,
/// which is also all the sweep logs to `/history`) are not messaged. Each renders
/// in the user's saved locale (English if unset). Best-effort per user: a DM to
/// someone who bet in a group but never started the bot just fails and is skipped —
/// the coins already landed on their balance regardless. Shared by the auto-settle
/// task and the manual `/settle` (whichever path settles an event first sees its
/// payouts; the other gets an empty list, so a user is never double-notified).
pub(crate) async fn notify_settled(token: &str, database: &Database, payouts: &[Payout]) {
    for p in payouts.iter().filter(|p| p.coins > 0) {
        let lang = database.get_lang(p.user).ok().flatten().unwrap_or(Lang::En);
        let coins = fmt_coins(p.coins);
        let text = if p.kind == ClaimKind::Refunded {
            i18n::bet_refunded(lang, &p.title, &coins)
        } else {
            i18n::bet_won(lang, &p.title, &coins)
        };
        let _ = tg::send_text_token(token, p.user, &text).await;
    }
}
