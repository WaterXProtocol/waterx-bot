use crate::commands::util::*;
use crate::commands::{menu, predict, referral, tg};
use crate::core::i18n::{self, Lang};
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
    let saved = database.get_lang(uid)?;

    // In a group the menu is a single shared message, so don't push the language
    // picker — fall back to the sender's Telegram-reported language when they
    // haven't chosen one. In private chats a first-timer gets the picker so they
    // set a persistent locale. `/language` re-opens the picker anywhere.
    let lang = if is_group_chat(chat_id) {
        Some(saved.unwrap_or_else(|| Lang::from_user(&user)))
    } else {
        saved
    };

    match lang {
        Some(lang) => {
            let available =
                is_group_chat(chat_id) || database.checkin_available(uid).unwrap_or(true);
            let pending = predict::pending_settle_count(&ctx, uid).await;
            tg::send_with_buttons(
                &ctx,
                chat_id,
                &menu::menu_text(lang, &full_name(&user)),
                &menu::main_menu_rows(lang, available, is_group_chat(chat_id), pending, mini_app_url(&ctx).as_deref()),
            )
            .await?;
        }
        None => {
            tg::send_with_buttons(&ctx, chat_id, i18n::CHOOSE_LANGUAGE, &menu::lang_picker_rows(saved, false))
                .await?;
        }
    }
    Ok(())
}
