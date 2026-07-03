//! `/claim` — collect your settled winnings. Settles the caller's positions in
//! every resolved/void event they hold (`Database::claim`): winning shares pay 1
//! coin each, losers nothing, a void refunds the cost basis. AMM (`/predict`)
//! events are resolved by their host; sourced (Polymarket) events are resolved by
//! the oracle (a public `/settle` sweep caches the result on the event row).

use crate::commands::markets;
use crate::commands::util::*;
use crate::core::i18n;
use crate::database::ClaimKind;
use telexide::prelude::*;

#[command(description = "claim your settled winnings")]
pub async fn claim(ctx: Context, message: Message) -> CommandResult {
    let Some((user, lang)) = begin(&ctx, &message).await? else {
        return Ok(());
    };
    // Detect Polymarket resolution for the caller's open sourced events and cache
    // the winner on the event row (so every later claim/settle reuses it, no
    // re-fetch). Best-effort: a feed error or still-open event just leaves it.
    let now = chrono::Utc::now().timestamp();
    for (event_id, slug, n_outcomes) in db(&ctx).user_open_sourced(user.id).unwrap_or_default() {
        if slug.is_empty() {
            continue;
        }
        if let Ok(Some(idx)) = markets::gamma_resolution(&slug, n_outcomes as usize).await {
            let _ = db(&ctx).resolve_event(event_id, idx, now);
        }
    }
    let payouts = match db(&ctx).claim(user.id) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("claim error (user {}): {e}", user.id);
            reply(&ctx, &message, i18n::db_error(lang)).await?;
            return Ok(());
        }
    };
    if payouts.is_empty() {
        reply(&ctx, &message, i18n::claim_nothing(lang)).await?;
        return Ok(());
    }
    let mut body = String::new();
    for p in &payouts {
        let line = match p.kind {
            ClaimKind::Won => i18n::claim_won(lang, &p.title, &fmt_coins(p.coins)),
            ClaimKind::Refunded => i18n::claim_refunded(lang, &p.title, &fmt_coins(p.coins)),
            ClaimKind::Lost => i18n::claim_lost(lang, &p.title),
        };
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(&line);
    }
    reply(&ctx, &message, body).await?;
    Ok(())
}
