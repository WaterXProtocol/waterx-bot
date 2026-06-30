use crate::commands::util::*;
use crate::core::i18n;
use crate::core::i18n::Lang;
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

/// The caller's open holdings — share positions (`positions_title`, from the
/// unified engine) + liquidity-provider stakes (`liquidity_title`, host AMM pools
/// they've funded) — each section prefixed with "\n\n" and omitted when empty.
/// Returns "" when the user has nothing open. Shared by `/bets` and the combined
/// view. Lines are language-neutral (titles + numbers + symbols).
pub(crate) async fn positions_block(ctx: &Context, lang: Lang, user: &User) -> String {
    let database = db(ctx);
    let mut body = String::new();

    // Open share positions (match events + host AMM predictions), from the unified
    // engine. Each line is cost basis paid → potential payout (1 share settles to
    // 1 coin, so the payout is the share count) — mirroring the old stake→payout
    // format, language-neutral (title + outcome name + numbers + symbols).
    let positions = database.user_positions(user.id).unwrap_or_else(|e| {
        eprintln!("user_positions error (user {}): {e}", user.id);
        Vec::new()
    });
    if !positions.is_empty() {
        body.push_str(&format!("\n\n{}", i18n::positions_title(lang)));
        for p in &positions {
            body.push_str(&format!(
                "\n• {}\n  {} · 🪙{} → 🏆{}",
                p.event_title,
                p.outcome,
                fmt_coins(p.cost),
                fmt_coins(p.shares),
            ));
        }
    }

    // Open liquidity-provider stakes in host AMM events (funding-stage seed or a
    // live pool). Committed capital that's repaid pro-rata at resolution, so it's
    // shown separately from tradeable positions. A 🌱/🟢 tag marks funding vs live.
    let lps = database.user_liquidity(user.id).unwrap_or_else(|e| {
        eprintln!("user_liquidity error (user {}): {e}", user.id);
        Vec::new()
    });
    if !lps.is_empty() {
        body.push_str(&format!("\n\n{}", i18n::liquidity_title(lang)));
        for lp in &lps {
            let tag = match lp.state.as_str() {
                "funding" => " 🌱",
                "open" => " 🟢",
                _ => "",
            };
            body.push_str(&format!("\n• {}\n  🪙{}{tag}", lp.event_title, fmt_coins(lp.contributed)));
        }
    }

    body
}
