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
use telexide::api::types::{InputFile, SendPhoto};
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
                    // A `data` that looks like a link becomes a URL button
                    // (these survive message forwarding; callback buttons don't).
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

/// Send a local image file as a photo with a caption — used for the referral
/// QR code. Uploaded via multipart, so nothing leaves the bot.
pub async fn send_photo_file(
    ctx: &Context,
    chat_id: i64,
    path: &str,
    caption: &str,
) -> Result<(), CommandError> {
    let photo = SendPhoto {
        chat_id: chat_id.into(),
        photo: InputFile::from_path(path)?,
        caption: (!caption.is_empty()).then(|| caption.to_string()),
        message_thread_id: None,
        caption_entities: None,
        parse_mode: None,
        has_spoiler: None,
        disable_notification: None,
        protect_content: None,
        reply_to_message_id: None,
        allow_sending_without_reply: None,
        reply_markup: None,
    };
    ctx.api.send_photo(photo).await?;
    Ok(())
}
