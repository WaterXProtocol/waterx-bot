use crate::commands::util::*;
use crate::i18n;
use telexide::prelude::*;

#[command(description = "show the caller's balance and open positions")]
pub async fn status(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let Some(user) = message.from.as_ref() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let lang = lang_for(&ctx, user);
    let database = db(&ctx);
    let info = database.get_user_info(user.id)?;
    let fruits = if info.fruit.is_empty() {
        "—".to_string()
    } else {
        info.fruit
    };

    let mut body = format!(
        "{}\n{}",
        full_name(user),
        i18n::menu_status(lang, &fmt_coins(info.balance), &fruits)
    );

    // Open (unsettled) bets, if any. Lines are language-neutral — the side name
    // is already localized, the rest is teams + numbers + symbols.
    let positions = database.list_open_wagers(user.id).unwrap_or_default();
    if !positions.is_empty() {
        body.push_str(&format!("\n\n{}", i18n::positions_title(lang)));
        for p in &positions {
            let side = match p.outcome.as_str() {
                "teamA" => p.team_a.clone(),
                "teamB" => p.team_b.clone(),
                _ => i18n::draw_label(lang).to_string(),
            };
            body.push_str(&format!(
                "\n• {} vs. {}\n  {} · 🪙{} → 🏆{}",
                p.team_a,
                p.team_b,
                side,
                fmt_coins(p.stake),
                fmt_coins(p.potential_payout()),
            ));
        }
    }

    reply(&ctx, &message, body).await?;
    Ok(())
}
