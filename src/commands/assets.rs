use crate::commands::util::*;
use crate::database::COIN;
use crate::core::i18n;
use crate::core::i18n::Lang;
use crate::core::types::BetState;
use telexide::model::User;
use telexide::prelude::*;

#[command(description = "show the caller's balance and open positions")]
pub async fn assets(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let Some(user) = message.from.as_ref() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let lang = lang_for(&ctx, user);
    // The coin balance is private: don't expose it in a group (everyone would
    // see it) — show only the open positions there. In a private chat it's the
    // caller's own DM, so the balance is fine.
    let show_balance = !is_group_chat(message.chat.get_id());
    let body = assets_text(&ctx, lang, user, show_balance).await;
    reply(&ctx, &message, body).await?;
    Ok(())
}

/// Build the combined `/assets` body for `user`: name (+ balance when
/// `show_balance`), then any open match bets and self-host predictions. Shared by
/// the `/assets` command and the `menu:balance` home button. `show_balance` is
/// false in a group, where the combined view keeps the balance private.
pub async fn assets_text(ctx: &Context, lang: Lang, user: &User, show_balance: bool) -> String {
    let head = if show_balance {
        balance_block(ctx, lang, user).await
    } else {
        full_name(user)
    };
    // `positions_block` already prefixes each section with "\n\n", so it appends
    // cleanly after the header (and contributes nothing when there are none).
    format!("{head}{}", positions_block(ctx, lang, user).await)
}

/// The caller's name + coin balance line (or a db-error notice instead of a fake
/// zero). Shared by `/balance` and the combined `/assets` / home view.
pub(crate) async fn balance_block(ctx: &Context, lang: Lang, user: &User) -> String {
    match db(ctx).get_user_info(user.id) {
        Ok(info) => format!(
            "{}\n{}",
            full_name(user),
            i18n::menu_status(lang, &fmt_coins(info.balance))
        ),
        Err(e) => {
            eprintln!("balance get_user_info error (user {}): {e}", user.id);
            format!("{}\n{}", full_name(user), i18n::db_error(lang))
        }
    }
}

/// The caller's open positions — match bets (`positions_title`) + self-host
/// predictions (`predictions_title`) — each section prefixed with "\n\n" and
/// omitted when empty. Returns "" when the user has no open positions at all.
/// Shared by `/bets` and the combined view. Lines are language-neutral (teams +
/// numbers + symbols; the side name is already localized).
pub(crate) async fn positions_block(ctx: &Context, lang: Lang, user: &User) -> String {
    let database = db(ctx);
    let mut body = String::new();

    // Open (unsettled) match bets.
    let positions = database.list_open_wagers(user.id).unwrap_or_else(|e| {
        eprintln!("bets list_open_wagers error (user {}): {e}", user.id);
        Vec::new()
    });
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
