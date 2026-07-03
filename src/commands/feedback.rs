//! `/feedback` — two ways to reach the bot team:
//!
//! - `/feedback <text>` (inline) sends immediately, from any chat.
//! - `/feedback` with no text opens a **DM compose flow** (like `/predict`): the
//!   bot DMs a prompt and the user's next plain-text DM is forwarded to the owner
//!   by the [`on_message`] listener. The in-flight state is a `Convo::Feedback`
//!   entry in the shared `bot::ConvosKey` map (so starting `/feedback` cancels any
//!   half-built `/predict`, and vice versa — at most one DM flow per user).

use crate::bot::Convo;
use crate::commands::tg;
use crate::commands::util::*;
use crate::core::i18n;
use telexide::model::{UpdateContent, User};
use telexide::prelude::*;

/// DM the owner a feedback body tagged with the sender (best-effort — a bounce is
/// logged, never surfaced to the user). Sent as HTML so the sender's numeric id
/// rides in a tap-to-copy `<code>` span (handy for `/mint`, `/profile`, etc.);
/// every dynamic part is escaped so HTML in a name/body can't break the message.
async fn send_to_owner(ctx: &Context, user: &User, body: &str) {
    let owner = owner_id(ctx);
    if owner == 0 {
        return;
    }
    let name = tg::escape(&full_name(user));
    let who = match &user.username {
        Some(u) if !u.is_empty() => {
            format!("{name} (@{}, id <code>{}</code>)", tg::escape(u), user.id)
        }
        _ => format!("{name} (id <code>{}</code>)", user.id),
    };
    let text = format!("📣 Feedback from {who}:\n\n{}", tg::escape(body));
    if let Err(e) = tg::send_html(ctx, owner, &text).await {
        eprintln!("feedback owner DM failed (owner {owner}): {e:?}");
    }
}

#[command(description = "send feedback to the bot team")]
pub async fn feedback(ctx: Context, message: Message) -> CommandResult {
    let Some((user, lang)) = begin(&ctx, &message).await? else {
        return Ok(());
    };

    // Inline fast-path: anything after the command word is the feedback body, sent
    // straight away (handy in DMs; in a group the "thanks" reply leaks nothing).
    let body = text_of(&message)
        .split_once(char::is_whitespace)
        .map_or("", |(_, rest)| rest.trim())
        .to_string();
    if !body.is_empty() {
        send_to_owner(&ctx, user, &body).await;
        reply(&ctx, &message, i18n::feedback_sent(lang)).await?;
        return Ok(());
    }

    // No inline text → open the DM compose flow. DM the prompt first; only register
    // the draft once we know the DM lands (a user who never started the bot can't
    // use the flow). Inserting overwrites any in-flight `/predict` draft.
    let landed = send_text(&ctx, user.id, i18n::feedback_ask(lang)).await.is_ok();
    if landed {
        convos(&ctx)
            .lock()
            .await
            .insert(user.id, Convo::Feedback { lang });
    }
    dm_pointer(
        &ctx,
        &message,
        landed,
        i18n::feedback_check_dm(lang),
        i18n::feedback_dm_first(lang),
    )
    .await
}

/// DM message listener: forwards a user's plain-text reply to the owner when they
/// have an active `Convo::Feedback`. No-op for groups, command text, DMs with no
/// feedback draft (so it never hijacks ordinary DMs), and fails closed on pause.
#[prepare_listener]
pub async fn on_message(ctx: Context, update: Update) {
    let UpdateContent::Message(message) = update.content else {
        return;
    };
    if is_group_chat(message.chat.get_id()) {
        return;
    }
    let Some(user) = message.from.clone() else {
        return;
    };
    let text = text_of(&message).trim().to_string();
    if text.is_empty() || text.starts_with('/') {
        return;
    }

    let drafts = convos(&ctx);
    let lang = {
        let mut guard = drafts.lock().await;
        let Some(Convo::Feedback { lang }) = guard.get(&user.id) else {
            return; // not in the feedback flow — ordinary DM or another flow
        };
        let lang = *lang;
        // A paused bot shouldn't forward a non-owner's message (fail closed); leave
        // the draft in place so they can retry once it's back.
        if !is_owner(&ctx, user.id) && db(&ctx).is_paused().unwrap_or(true) {
            return;
        }
        // Consume the draft now so a second message becomes an ordinary DM.
        guard.remove(&user.id);
        lang
    };

    send_to_owner(&ctx, &user, &text).await;
    let _ = send_text(&ctx, user.id, i18n::feedback_sent(lang)).await;
}
