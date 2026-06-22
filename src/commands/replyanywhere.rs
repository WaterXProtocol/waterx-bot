use crate::commands::tg;
use crate::commands::util::*;
use crate::core::i18n;
use telexide::prelude::*;

/// `/replyanywhere` — undo `/onlyreplyhere`: clear this group's topic lock so the
/// bot replies in any topic again (`Database::clear_reply_thread`). Admin-gated,
/// group-only. Like `/onlyreplyhere` it skips `paused_block` so an admin can lift
/// the lock from anywhere (otherwise the lock would gate its own removal).
#[command(description = "let the bot reply in any topic again (group admins)")]
pub async fn replyanywhere(ctx: Context, message: Message) -> CommandResult {
    let Some(user) = message.from.clone() else {
        return Ok(());
    };
    let lang = lang_for(&ctx, &user);
    let chat_id = message.chat.get_id();
    db(&ctx).touch_chat(chat_id).ok();

    if !is_group_chat(chat_id) {
        reply(&ctx, &message, i18n::onlyreply_group_only(lang)).await?;
        return Ok(());
    }
    if !tg::is_chat_admin(&ctx, chat_id, user.id).await {
        reply(&ctx, &message, i18n::onlyreply_admin_only(lang)).await?;
        return Ok(());
    }
    if db(&ctx).clear_reply_thread(chat_id).is_err() {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    }
    reply(&ctx, &message, i18n::onlyreply_cleared(lang)).await?;
    Ok(())
}
