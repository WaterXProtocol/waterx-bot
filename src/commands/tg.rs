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
use telexide::api::APIEndpoint;
use telexide::framework::CommandError;
use telexide::model::Message;
use telexide::prelude::Context;

/// One row of `(button_label, callback_data)` pairs. Both strings are owned
/// so callers don't have to fight lifetimes when building rows from owned data.
pub type Row = Vec<(String, String)>;

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
    let _: telexide::Result<serde_json::Value> = resp.into();
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
    let _: telexide::Result<serde_json::Value> = resp.into();
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

/// Shared `sendPhoto` form fields (everything except the `photo` itself).
fn photo_form(chat_id: i64, caption: &str, rows: &[Row]) -> reqwest::multipart::Form {
    reqwest::multipart::Form::new()
        .text("chat_id", chat_id.to_string())
        .text("caption", caption.to_string())
        .text("reply_markup", build_keyboard(rows).to_string())
}

/// POST a `sendPhoto` form and return the largest size's `file_id` from the reply.
async fn send_photo_form(
    token: &str,
    form: reqwest::multipart::Form,
) -> Result<Option<String>, CommandError> {
    let url = format!("https://api.telegram.org/bot{token}/sendPhoto");
    let resp = reqwest::Client::new()
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
