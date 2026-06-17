use crate::commands::util::*;
use crate::commands::{menu, referral, tg};
use crate::i18n;
use telexide::prelude::*;

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
            referral::pay_referral(&ctx, referrer, &user).await;
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
