//! `/settle` — public global sweep of resolved **sourced** (Polymarket) events:
//! fetch each open sourced event's resolution from Gamma, cache the winner on the
//! event row, then pay out **every** position (`Database::settle_all_sourced`).
//! AMM (`/predict`) events are settled by their host + `/claim`, never here.
//! Anyone can run it — settlement is deterministic (the oracle picks the winner),
//! so it just flushes the pending payouts for everyone; `/claim` is the per-user
//! equivalent.

use crate::commands::markets;
use crate::commands::util::*;
use crate::core::i18n;
use std::collections::HashSet;
use telexide::prelude::*;

#[command(description = "settle resolved Polymarket events")]
pub async fn settle(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let lang = lang_for_msg(&ctx, &message);
    let database = db(&ctx);
    // Detect + cache Polymarket resolution for every open sourced event.
    let now = chrono::Utc::now().timestamp();
    for (event_id, slug, n_outcomes) in database.all_open_sourced().unwrap_or_default() {
        if slug.is_empty() {
            continue;
        }
        if let Ok(Some(idx)) = markets::gamma_resolution(&slug, n_outcomes as usize).await {
            let _ = database.resolve_event(event_id, idx, now);
        }
    }
    // Pay out every position in every resolved/void sourced event.
    let payouts = match database.settle_all_sourced() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("settle_all_sourced error: {e}");
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
    reply(&ctx, &message, i18n::settle_done(lang, &events.len().to_string(), &fmt_coins(paid)))
        .await?;
    Ok(())
}
