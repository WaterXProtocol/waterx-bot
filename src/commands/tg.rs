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
use telexide::api::types::AnswerCallbackQuery;
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
    let payload = json!({
        "chat_id": chat_id,
        "text": text,
        "reply_markup": build_keyboard(rows),
    });
    let resp = ctx
        .api
        .post(APIEndpoint::SendMessage, Some(payload))
        .await?;
    let msg: telexide::Result<Message> = resp.into();
    Ok(msg?)
}

/// Send a plain message as a **reply** to `reply_to` in `chat_id`. Used to pin a
/// placed-bet announcement under the group game card it belongs to. Falls back to
/// a normal message if that card is gone (`allow_sending_without_reply`).
pub async fn send_text_reply(
    ctx: &Context,
    chat_id: i64,
    reply_to: i64,
    text: &str,
) -> Result<(), CommandError> {
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

/// Send a plain message with `parse_mode: HTML` (used for the tap-to-copy
/// `<code>` invite link). No inline keyboard.
pub async fn send_html(ctx: &Context, chat_id: i64, text: &str) -> Result<(), CommandError> {
    let payload = json!({
        "chat_id": chat_id,
        "text": text,
        "parse_mode": "HTML",
    });
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
