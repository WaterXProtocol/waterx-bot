//! Raw-JSON wrappers around Telegram's sendMessage / editMessageText.
//!
//! telexide 0.1.17 ships an `InlineKeyboardButton` struct whose optional
//! fields lack `#[serde(skip_serializing_if = "Option::is_none")]`, so any
//! markup built with `InlineKeyboardButton::new` serialises with `"url":null`
//! and Telegram rejects the request:
//!
//!   Bad Request: can't parse inline keyboard button: Field "url" must be of
//!   type String
//!
//! Building the payload as a `serde_json::Value` and posting it via the
//! `API::post` low-level endpoint sidesteps the bug entirely. All
//! inline-keyboard code paths in the bot go through the helpers here.

use serde_json::{json, Value};
use telexide::api::types::{AnswerCallbackQuery, DeleteMessage};
use telexide::api::APIEndpoint;
use telexide::framework::CommandError;
use telexide::model::{CallbackQuery, Message};
use telexide::prelude::Context;

/// One row of `(button_label, callback_data)` pairs. Both strings are owned
/// so callers don't have to fight lifetimes when building rows from owned data.
pub type Row = Vec<(String, String)>;

/// Acknowledge a callback query. Empty `text` = a silent ack (no toast); a
/// non-empty `text` shows as a toast, or as a modal alert when `alert`. The one
/// home for what used to be `callbacks::answer` / `betting::answer` /
/// `predict::ack` / `admin::ack`.
pub async fn answer(
    ctx: &Context,
    cb: &CallbackQuery,
    text: &str,
    alert: bool,
) -> Result<(), telexide::Error> {
    let mut a = AnswerCallbackQuery::new(cb.id.clone());
    if !text.is_empty() {
        a.text = Some(text.to_string());
    }
    a.show_alert = Some(alert);
    ctx.api.answer_callback_query(a).await?;
    Ok(())
}

/// The forum topic `chat_id` is locked to (`/onlyreplyhere`), if any. Group
/// chats only; `None` for private chats and unlocked groups. Used by the send
/// helpers below to thread the bot's messages into the locked topic so it stays
/// confined there even for non-reply sends (which otherwise land in "General").
async fn locked_thread(ctx: &Context, chat_id: i64) -> Option<i64> {
    if chat_id >= 0 {
        return None; // private chats have no topics
    }
    crate::commands::util::db(ctx).reply_thread(chat_id).ok().flatten()
}

/// Attach `message_thread_id` to a **non-reply** `sendMessage` payload when
/// `chat_id` is a group locked to a topic, so the message lands in that topic.
///
/// Reply sends (`send_*_reply`) deliberately do **not** use this: a reply already
/// threads into its target's topic via `reply_parameters`, and forcing a
/// `message_thread_id` that disagrees with the replied-to message (e.g. an old
/// card still in "General") makes Telegram reject the send or thread it oddly.
async fn with_thread(ctx: &Context, chat_id: i64, mut payload: Value) -> Value {
    if let Some(thread) = locked_thread(ctx, chat_id).await {
        if let Value::Object(map) = &mut payload {
            map.insert("message_thread_id".to_string(), json!(thread));
        }
    }
    payload
}

/// Whether `user_id` is an administrator (or creator) of `chat_id`, via
/// `getChatMember`. Used to gate group-admin commands like `/onlyreplyhere`.
/// `false` on any API error or for a non-admin status.
pub async fn is_chat_admin(ctx: &Context, chat_id: i64, user_id: i64) -> bool {
    let payload = json!({ "chat_id": chat_id, "user_id": user_id });
    let resp = ctx
        .api
        .post(APIEndpoint::GetChatMember, Some(payload))
        .await
        .ok();
    let Some(resp) = resp else { return false };
    let Ok(v) = Into::<telexide::Result<Value>>::into(resp) else {
        return false;
    };
    matches!(
        v.get("status").and_then(Value::as_str),
        Some("creator") | Some("administrator")
    )
}

fn build_keyboard(rows: &[Row]) -> Value {
    let json_rows: Vec<Value> = rows
        .iter()
        .map(|row| {
            let cells: Vec<Value> = row
                .iter()
                .map(|(label, data)| {
                    // A `data` that looks like a link becomes a URL button;
                    // anything else is a callback button. (Telegram strips the
                    // whole inline keyboard on forward regardless of kind.)
                    if data.starts_with("https://")
                        || data.starts_with("http://")
                        || data.starts_with("tg://")
                    {
                        json!({ "text": label, "url": data })
                    } else {
                        json!({ "text": label, "callback_data": data })
                    }
                })
                .collect();
            Value::Array(cells)
        })
        .collect();
    json!({ "inline_keyboard": json_rows })
}

pub async fn send_with_buttons(
    ctx: &Context,
    chat_id: i64,
    text: &str,
    rows: &[Row],
) -> Result<Message, CommandError> {
    let payload = with_thread(ctx, chat_id, json!({
        "chat_id": chat_id,
        "text": text,
        "reply_markup": build_keyboard(rows),
    })).await;
    let resp = ctx
        .api
        .post(APIEndpoint::SendMessage, Some(payload))
        .await?;
    let msg: telexide::Result<Message> = resp.into();
    Ok(msg?)
}

/// Send a message **with an inline keyboard** as a **reply** to `reply_to`. Used
/// to post a per-user stake board threaded under the group prediction card it belongs
/// to. Falls back to a loose message if that card is gone
/// (`allow_sending_without_reply`). Returns the sent `Message` so the caller can
/// track the board's id.
pub async fn send_with_buttons_reply(
    ctx: &Context,
    chat_id: i64,
    reply_to: i64,
    text: &str,
    rows: &[Row],
) -> Result<Message, CommandError> {
    // No `with_thread`: a reply threads into its target's topic via
    // `reply_parameters` on its own — see `with_thread`'s doc for why forcing a
    // `message_thread_id` here would conflict.
    let payload = json!({
        "chat_id": chat_id,
        "text": text,
        "reply_markup": build_keyboard(rows),
        "reply_parameters": { "message_id": reply_to, "allow_sending_without_reply": true },
    });
    let resp = ctx
        .api
        .post(APIEndpoint::SendMessage, Some(payload))
        .await?;
    let msg: telexide::Result<Message> = resp.into();
    Ok(msg?)
}

/// Delete a message (best-effort). Used to remove a per-user stake board once the
/// bet is placed or dismissed. A logical failure (already gone) converts to an
/// `Err` the caller can ignore.
pub async fn delete_message(
    ctx: &Context,
    chat_id: i64,
    message_id: i64,
) -> Result<(), CommandError> {
    ctx.api
        .delete_message(DeleteMessage::new(chat_id.into(), message_id))
        .await?;
    Ok(())
}

/// The group's **creator** (owner) via `getChatAdministrators`, found by the
/// member whose `status` is `creator`. `None` on any failure (API error, no
/// creator, anonymous owner). Posted at the low level (like the other helpers
/// here) so we can read the raw JSON without depending on telexide's typed
/// `ChatMember` deserialization.
pub async fn chat_creator(ctx: &Context, chat_id: i64) -> Option<i64> {
    let payload = json!({ "chat_id": chat_id });
    let resp = ctx
        .api
        .post(APIEndpoint::GetChatAdministrators, Some(payload))
        .await
        .ok()?;
    let result: Value = Into::<telexide::Result<Value>>::into(resp).ok()?;
    for m in result.as_array()? {
        if m.get("status").and_then(Value::as_str) == Some("creator") {
            return m.get("user").and_then(|u| u.get("id")).and_then(Value::as_i64);
        }
    }
    None
}

/// Send a plain message as a **reply** to `reply_to` in `chat_id`. Used to pin a
/// placed-bet announcement under the group prediction card it belongs to. Falls back to
/// a normal message if that card is gone (`allow_sending_without_reply`).
pub async fn send_text_reply(
    ctx: &Context,
    chat_id: i64,
    reply_to: i64,
    text: &str,
) -> Result<(), CommandError> {
    // No `with_thread`: the reply threads via `reply_parameters` (see that helper's
    // doc) — forcing a possibly-conflicting `message_thread_id` would break it.
    let payload = json!({
        "chat_id": chat_id,
        "text": text,
        "reply_parameters": { "message_id": reply_to, "allow_sending_without_reply": true },
    });
    let resp = ctx
        .api
        .post(APIEndpoint::SendMessage, Some(payload))
        .await?;
    let msg: telexide::Result<Message> = resp.into();
    msg?;
    Ok(())
}

/// Escape text for Telegram's HTML `parse_mode` (`&`, `<`, `>` only — sufficient
/// for message-body text, the only place we use HTML). Lets dynamic content
/// (names, team names, feedback bodies) sit safely next to a `<code>` span.
pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Send a plain message with `parse_mode: HTML` (used for the tap-to-copy
/// `<code>` invite link). No inline keyboard.
pub async fn send_html(ctx: &Context, chat_id: i64, text: &str) -> Result<(), CommandError> {
    let payload = with_thread(ctx, chat_id, json!({
        "chat_id": chat_id,
        "text": text,
        "parse_mode": "HTML",
    })).await;
    let resp = ctx
        .api
        .post(APIEndpoint::SendMessage, Some(payload))
        .await?;
    let msg: telexide::Result<Message> = resp.into();
    msg?;
    Ok(())
}

pub async fn edit_with_buttons(
    ctx: &Context,
    chat_id: i64,
    message_id: i64,
    text: &str,
    rows: &[Row],
) -> Result<(), CommandError> {
    let payload = json!({
        "chat_id": chat_id,
        "message_id": message_id,
        "text": text,
        "reply_markup": build_keyboard(rows),
    });
    let resp = ctx
        .api
        .post(APIEndpoint::EditMessageText, Some(payload))
        .await?;
    // Telegram returns HTTP 200 with `{"ok":false}` for logical rejections
    // (e.g. message too long, BUTTON_DATA_INVALID) — surface those instead of
    // silently leaving the message unchanged. The one exception is "message is not
    // modified": that's the expected no-op when re-rendering a card with unchanged
    // content (odds still in the cache window), so don't log it as an error.
    if let Err(e) = Into::<telexide::Result<serde_json::Value>>::into(resp) {
        let msg = e.to_string();
        if !msg.contains("not modified") {
            eprintln!("[tg] editMessageText failed (chat {chat_id}, msg {message_id}): {msg}");
        }
    }
    Ok(())
}

pub async fn edit_text_only(
    ctx: &Context,
    chat_id: i64,
    message_id: i64,
    text: &str,
) -> Result<(), CommandError> {
    let payload = json!({
        "chat_id": chat_id,
        "message_id": message_id,
        "text": text,
    });
    let resp = ctx
        .api
        .post(APIEndpoint::EditMessageText, Some(payload))
        .await?;
    // Surface logical rejections (HTTP 200 + `{"ok":false}`, e.g. message too
    // long) instead of discarding them silently — matches `edit_with_buttons`,
    // including ignoring the expected "message is not modified" no-op.
    if let Err(e) = Into::<telexide::Result<serde_json::Value>>::into(resp) {
        let msg = e.to_string();
        if !msg.contains("not modified") {
            eprintln!("[tg] editMessageText (text) failed (chat {chat_id}, msg {message_id}): {msg}");
        }
    }
    Ok(())
}

/// Remove a message's inline keyboard **without touching its text** (Telegram
/// `editMessageReplyMarkup` with no markup). Used after a prediction settles to
/// strip the now-dead settle buttons while leaving the card itself as-is.
pub async fn clear_buttons(ctx: &Context, chat_id: i64, message_id: i64) -> Result<(), CommandError> {
    let payload = json!({
        "chat_id": chat_id,
        "message_id": message_id,
    });
    let resp = ctx
        .api
        .post(APIEndpoint::EditMessageReplyMarkup, Some(payload))
        .await?;
    if let Err(e) = Into::<telexide::Result<serde_json::Value>>::into(resp) {
        let msg = e.to_string();
        if !msg.contains("not modified") {
            eprintln!("[tg] editMessageReplyMarkup failed (chat {chat_id}, msg {message_id}): {msg}");
        }
    }
    Ok(())
}

/// Pin a message in `chat_id` (best-effort). `disable_notification: true` keeps it
/// quiet — no "pinned a message" alert blasted at every member. Used to pin a fresh
/// `/predict` card so members can find it at the top of the group; requires the bot
/// to hold the `can_pin_messages` admin right, so any failure (not admin, private
/// chat) is the caller's to ignore.
pub async fn pin_message(ctx: &Context, chat_id: i64, message_id: i64) -> Result<(), CommandError> {
    let payload = json!({
        "chat_id": chat_id,
        "message_id": message_id,
        "disable_notification": true,
    });
    let resp = ctx
        .api
        .post(APIEndpoint::PinChatMessage, Some(payload))
        .await?;
    let v: telexide::Result<serde_json::Value> = resp.into();
    v?;
    Ok(())
}

/// Unpin a message in `chat_id` (best-effort). Used to undo a `/predict` card's pin
/// once it settles. Same admin-right caveat as [`pin_message`]; a failure (already
/// unpinned, lost rights) is harmless to ignore.
pub async fn unpin_message(ctx: &Context, chat_id: i64, message_id: i64) -> Result<(), CommandError> {
    let payload = json!({
        "chat_id": chat_id,
        "message_id": message_id,
    });
    let resp = ctx
        .api
        .post(APIEndpoint::UnpinChatMessage, Some(payload))
        .await?;
    let v: telexide::Result<serde_json::Value> = resp.into();
    v?;
    Ok(())
}

/// Upload in-memory PNG bytes as a photo (caption + inline keyboard), in a single
/// message. Returns the Telegram **`file_id`** of the sent photo so the caller can
/// cache it and later re-send via [`send_photo_id`] without re-uploading.
///
/// This bypasses telexide's `send_photo`: that path serialises the `photo` field
/// as `attach://<full-filename>` but names the multipart part with the filename
/// truncated at the first `.` (`qr.png` → part name `qr`), so Telegram can never
/// match the attachment and the upload silently fails. We post the multipart
/// `sendPhoto` ourselves via `reqwest` (already a dependency), which also lets us
/// attach a proper `reply_markup` keyboard (telexide's button struct serialises
/// `null` optional fields, which Telegram rejects — see this module's header).
pub async fn send_photo_bytes(
    token: &str,
    chat_id: i64,
    png: Vec<u8>,
    caption: &str,
    rows: &[Row],
) -> Result<Option<String>, CommandError> {
    let part = reqwest::multipart::Part::bytes(png)
        .file_name("qr.png")
        .mime_str("image/png")
        .map_err(|e| CommandError(e.to_string()))?;
    let form = photo_form(chat_id, caption, rows).part("photo", part);
    send_photo_form(token, form).await
}

/// Re-send an already-uploaded photo by its Telegram `file_id` (no upload). Used
/// to reuse a cached referral QR instead of regenerating it every time.
pub async fn send_photo_id(
    token: &str,
    chat_id: i64,
    file_id: &str,
    caption: &str,
    rows: &[Row],
) -> Result<(), CommandError> {
    let form = photo_form(chat_id, caption, rows).text("photo", file_id.to_string());
    send_photo_form(token, form).await.map(|_| ())
}

/// Shared `sendPhoto` form fields (everything except the `photo` itself). When
/// `rows` is empty no `reply_markup` is attached (Telegram dislikes an empty
/// keyboard).
fn photo_form(chat_id: i64, caption: &str, rows: &[Row]) -> reqwest::multipart::Form {
    let mut form = reqwest::multipart::Form::new()
        .text("chat_id", chat_id.to_string())
        .text("caption", caption.to_string());
    if !rows.is_empty() {
        form = form.text("reply_markup", build_keyboard(rows).to_string());
    }
    form
}

/// POST a `sendPhoto` form and return the largest size's `file_id` from the reply.
async fn send_photo_form(
    token: &str,
    form: reqwest::multipart::Form,
) -> Result<Option<String>, CommandError> {
    let url = format!("https://api.telegram.org/bot{token}/sendPhoto");
    let resp = crate::commands::util::http_client()
        .post(url)
        .multipart(form)
        .send()
        .await
        .map_err(|e| CommandError(e.to_string()))?;
    let ok = resp.status().is_success();
    let body = resp.text().await.unwrap_or_default();
    if !ok {
        return Err(CommandError(format!("sendPhoto failed: {body}")));
    }
    // result.photo is an array of sizes; the last is the largest. Any parse miss
    // just means "no cached id" — harmless, we'll re-upload next time.
    let file_id = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| {
            v.get("result")?
                .get("photo")?
                .as_array()?
                .last()?
                .get("file_id")?
                .as_str()
                .map(str::to_string)
        });
    Ok(file_id)
}

#[cfg(test)]
mod qr_tests {
    #[test]
    fn qr_produces_valid_png() {
        let png = qrcode_generator::to_png_to_vec(
            "https://t.me/foo?start=123",
            qrcode_generator::QrCodeEcc::Medium,
            512,
        )
        .expect("qr gen");
        assert!(png.len() > 100);
        assert_eq!(&png[1..4], b"PNG");
    }
}
