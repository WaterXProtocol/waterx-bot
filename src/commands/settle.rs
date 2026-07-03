//! `/settle` — public **manual** settle of resolved markets, and the periodic
//! [`auto_settle`] task. Both detect Polymarket (Gamma) resolution for every open
//! **sourced** (`/events`) event, cache the winner on the row, then pay out
//! **every** resolved/void position — sourced (oracle-decided) *and* any
//! host-resolved AMM (`/predict`) event whose resolve-time settle didn't land.
//! Settlement is deterministic (the oracle / host already picked the winner), so
//! anyone can run `/settle`; it's the manual fallback if the 5-min auto-settle
//! task ever stalls. (There is no per-user `/claim` anymore — everything settles
//! automatically.)

use crate::commands::markets;
use crate::commands::util::*;
use crate::core::i18n;
use crate::database::{Database, Payout};
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
/// [`sweep`], logging on error. Silent on success — each winner's credited balance
/// and their `/history` `claim`/`refund` row are the record; no per-user DM.
pub async fn auto_settle(database: &Database) {
    if let Err(e) = sweep(database).await {
        eprintln!("[auto-settle] sweep error: {e}");
    }
}
