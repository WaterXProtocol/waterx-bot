use crate::commands::menu;
use crate::commands::tg;
use crate::commands::util::*;
use crate::i18n;
use telexide::prelude::*;

#[command(description = "say hi and open the menu")]
pub async fn start(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let Some(uid) = from_id(&message) else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let database = db(&ctx);
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
