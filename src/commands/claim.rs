//! `/claim` — collect your settled winnings. Settles the caller's positions in
//! every resolved/void event they hold (`Database::claim`): winning shares pay 1
//! coin each, losers nothing, a void refunds the cost basis. AMM (`/predict`)
//! events are resolved by their host; sourced (Polymarket) events are resolved by
//! the oracle (a public `/settle` sweep caches the result on the event row).

use crate::commands::util::*;
use crate::core::i18n;
use crate::database::ClaimKind;
use telexide::prelude::*;

#[command(description = "claim your settled winnings")]
pub async fn claim(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let Some(user) = message.from.as_ref() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let lang = lang_for(&ctx, user);
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
