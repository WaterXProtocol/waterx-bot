use crate::commands::util::*;
use crate::i18n;
use telexide::prelude::*;

#[command(description = "send feedback to the bot team")]
pub async fn feedback(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let Some(user) = message.from.as_ref() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let lang = lang_for(&ctx, user);

    // Everything after the command word is the feedback body (may contain spaces).
    let body = text_of(&message)
        .split_once(char::is_whitespace)
        .map_or("", |(_, rest)| rest.trim());
    if body.is_empty() {
        reply(&ctx, &message, i18n::usage_feedback(lang)).await?;
        return Ok(());
    }

    // DM the owner (best-effort — don't fail the user if the owner DM bounces).
    let owner = owner_id(&ctx);
    if owner != 0 {
        let who = match &user.username {
            Some(u) if !u.is_empty() => format!("{} (@{u}, id {})", full_name(user), user.id),
            _ => format!("{} (id {})", full_name(user), user.id),
        };
        // Best-effort, but log a bounce so feedback isn't lost without a trace
        // (e.g. the owner never opened a DM with the bot).
        if let Err(e) = send_text(&ctx, owner, format!("📣 Feedback from {who}:\n\n{body}")).await {
            eprintln!("feedback owner DM failed (owner {owner}): {e:?}");
        }
    }

    reply(&ctx, &message, i18n::feedback_sent(lang)).await?;
    Ok(())
}
