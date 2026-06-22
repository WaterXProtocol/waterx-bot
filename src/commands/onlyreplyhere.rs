use crate::commands::tg;
use crate::commands::util::*;
use crate::core::i18n;
use telexide::prelude::*;

/// `/onlyreplyhere` — a group admin runs this **inside a forum topic** to confine
/// the bot to that topic: it records the topic's `message_thread_id` for this
/// group (`Database::set_reply_thread`). Afterwards the bot ignores commands and
/// button presses from every other topic (the gate in `util::out_of_locked_topic`,
/// checked in `paused_block` and `on_callback`) and threads its own messages into
/// the locked topic. Cleared by `/replyanywhere`.
///
/// Deliberately does **not** call `paused_block` — that's where the topic gate
/// lives, and running it here would let a stale lock prevent an admin from moving
/// the lock to a new topic. Admin-gated via `getChatMember`, group-only, and kept
/// out of the public `/` menu (it's a group-admin utility, not an everyone command).
#[command(description = "lock the bot to this topic (group admins)")]
pub async fn onlyreplyhere(ctx: Context, message: Message) -> CommandResult {
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
    // Only a genuine forum topic can be locked to. `message_thread_id` is also set
    // for reply-threads in non-forum groups, so require `is_topic_message` too —
    // threading a message into a non-existent topic would be rejected by Telegram.
    let thread = match message.message_thread_id {
        Some(t) if message.is_topic_message => t,
        _ => {
            reply(&ctx, &message, i18n::onlyreply_need_topic(lang)).await?;
            return Ok(());
        }
    };
    if db(&ctx).set_reply_thread(chat_id, thread).is_err() {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    }
    reply(&ctx, &message, i18n::onlyreply_set(lang)).await?;
    Ok(())
}
