use crate::commands::menu;
use crate::commands::tg;
use crate::commands::util::*;
use crate::i18n::{self, Lang};
use telexide::prelude::*;

/// Water-coins paid to the referrer when a brand-new user joins via their link.
pub(crate) const REFERRAL_REWARD: i64 = 20;

#[command(description = "say hi and open the menu")]
pub async fn start(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let Some(user) = message.from.clone() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let uid = user.id;
    let database = db(&ctx);

    // Referral: a brand-new user opening `t.me/<bot>?start=<referrer_id>` sends
    // `/start <referrer_id>`. Record it once and pay the referrer.
    if let Some(referrer) = args(&message).first().and_then(|p| p.parse::<i64>().ok()) {
        if database.set_referrer_if_new(uid, referrer)? {
            database.force_change(referrer, REFERRAL_REWARD)?;
            let rlang = database.get_lang(referrer).ok().flatten().unwrap_or(Lang::En);
            let _ = send_text(
                &ctx,
                referrer,
                i18n::referral_bonus(rlang, &full_name(&user), &format_number(REFERRAL_REWARD)),
            )
            .await;
        }
    }
    database.balance_change(uid, 0)?; // ensure the user row exists

    let chat_id = message.chat.get_id();
    match database.get_lang(uid)? {
        // Language already chosen → straight to the Xaliah menu.
        Some(lang) => {
            // In a group the menu is shared, so always offer the button; in a
            // private chat hide it once the caller has already claimed today.
            let available =
                is_group_chat(chat_id) || database.checkin_available(uid).unwrap_or(true);
            tg::send_with_buttons(
                &ctx,
                chat_id,
                &menu::menu_text(&ctx, lang, uid),
                &menu::main_menu_rows(lang, available),
            )
            .await?;
        }
        // First time → make them pick a language; the menu opens from the
        // `setlang:` callback once they choose.
        None => {
            tg::send_with_buttons(
                &ctx,
                chat_id,
                i18n::CHOOSE_LANGUAGE,
                &menu::lang_picker_rows(),
            )
            .await?;
        }
    }
    Ok(())
}
