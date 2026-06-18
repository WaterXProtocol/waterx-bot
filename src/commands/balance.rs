use crate::commands::util::*;
use crate::database::COIN;
use crate::i18n;
use crate::i18n::Lang;
use crate::types::BetState;
use telexide::model::User;
use telexide::prelude::*;

#[command(description = "show the caller's balance and open positions")]
pub async fn balance(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let Some(user) = message.from.as_ref() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let lang = lang_for(&ctx, user);
    let body = balance_text(&ctx, lang, user).await;
    reply(&ctx, &message, body).await?;
    Ok(())
}

/// Build the `/balance` body for `user`: name + balance, then any open match
/// bets and self-host predictions (each section with its staked total).
/// Shared by the `/balance` command and the `menu:balance` button.
pub async fn balance_text(ctx: &Context, lang: Lang, user: &User) -> String {
    let database = db(ctx);
    let info = database.get_user_info(user.id).unwrap_or_default();

    let mut body = format!(
        "{}\n{}",
        full_name(user),
        i18n::menu_status(lang, &fmt_coins(info.balance))
    );

    // Open (unsettled) match bets, if any, with the section's staked total in
    // Lines are language-neutral — the side name is already localized, the rest
    // is teams + numbers + symbols.
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

    // Stakes locked in still-open self-host (`/predict`) games. The stake was
    // debited at bet time, so these coins are committed positions — not yet
    // reconciled into the balance (settled/draw games already are, so skip them).
    // Game stakes are stored in whole coins; render via fmt_coins(× COIN).
    let mut game_lines = String::new();
    {
        let games = games(ctx);
        let guard = games.lock().await;
        for g in guard.values() {
            if !matches!(g.state, BetState::betting | BetState::closed) {
                continue;
            }
            let staked: Vec<(&String, i64)> = g
                .option_order
                .iter()
                .filter_map(|opt| {
                    g.options
                        .get(opt)
                        .and_then(|d| d.detail.get(&user.id).copied())
                        .filter(|&s| s > 0)
                        .map(|s| (opt, s))
                })
                .collect();
            if staked.is_empty() {
                continue;
            }
            // Description is "<id-tail>\n<host's text>"; show the host's text.
            let desc = g
                .description
                .split_once('\n')
                .map_or(g.description.as_str(), |(_, rest)| rest);
            game_lines.push_str(&format!("\n🎲 {desc}"));
            for (opt, stake) in staked {
                game_lines.push_str(&format!("\n  {} · 🪙{}", opt, fmt_coins(stake * COIN)));
            }
        }
    }
    if !game_lines.is_empty() {
        body.push_str(&format!("\n\n{}{}", i18n::predictions_title(lang), game_lines));
    }

    body
}
