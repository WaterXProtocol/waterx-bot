use crate::commands::{callbacks, *};
use crate::core::types::BotConfig;
use crate::database::{db_path, Database};
use std::{collections::HashMap, sync::Arc, time::Duration};
use telexide::{
    api::{types::GetUpdates, APIEndpoint, API},
    create_framework,
    prelude::*,
};
use tokio::sync::Mutex;
use typemap_rev::TypeMapKey;

/// Backoff after a failed getUpdates poll (network/HTTP/parse error) before
/// retrying — `robust_poll` keeps the bot alive across transient outages.
const POLL_RETRY_BACKOFF: Duration = Duration::from_secs(5);
/// Startup getMe retry: initial backoff, doubled on each failure up to the cap,
/// so a boot-time network blip doesn't crash-loop the process.
const STARTUP_BACKOFF_START: Duration = Duration::from_secs(1);
const STARTUP_BACKOFF_MAX: Duration = Duration::from_secs(30);

pub struct DbKey;
impl TypeMapKey for DbKey {
    type Value = Arc<Database>;
}

pub struct ConfigKey;
impl TypeMapKey for ConfigKey {
    type Value = Arc<BotConfig>;
}

pub struct BotIdKey;
impl TypeMapKey for BotIdKey {
    type Value = i64;
}

pub struct BotUsernameKey;
impl TypeMapKey for BotUsernameKey {
    type Value = String;
}

pub struct QuotesKey;
impl TypeMapKey for QuotesKey {
    type Value = Arc<parking_lot::Mutex<crate::commands::betting::QuoteStore>>;
}

/// In-flight DM conversation state, keyed by user id. At most one per user — a
/// newly-started flow overwrites any previous one, so `/predict` and `/feedback`
/// can never both be live for the same user (their DM listeners each match only
/// their own variant). Populated by the `/predict` and `/feedback` commands and
/// consumed by the `predict::on_message` / `feedback::on_message` listeners (+ the
/// `/predict` end-time callback). Those two listeners are **spawned concurrently**
/// per DM update (telexide `fire_handlers` runs each handler in its own task), so
/// safety rests on each matching only its own variant while holding the shared
/// `Mutex` — the map holds at most one variant per user, so the non-matching
/// listener is a clean no-op.
pub enum Convo {
    /// A `/predict` builder draft (question → options → end-time).
    Predict(crate::commands::predict::PredictDraft),
    /// Awaiting a `/feedback` message; `lang` is the composer's locale.
    Feedback { lang: crate::core::i18n::Lang },
}
pub struct ConvosKey;
impl TypeMapKey for ConvosKey {
    type Value = Arc<Mutex<HashMap<i64, Convo>>>;
}

pub async fn run() -> anyhow::Result<()> {
    let cfg = BotConfig::from_env()?;
    let bot_id: i64 = cfg
        .token
        .split(':')
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow::anyhow!("malformed bot token (expected `<id>:<secret>`)"))?;
    let db = Arc::new(Database::new(&db_path(cfg.dev), bot_id)?);
    let cfg_arc = Arc::new(cfg.clone());

    // Resolve the bot's real @username via getMe BEFORE building the framework
    // — telexide's command router compares group-chat command suffixes against
    // this string (e.g. `/predict@BotUsername`). Using anything other than the
    // actual username makes group commands silently no-op when Telegram
    // appends the @suffix. The call also serves as a token sanity check.
    let probe = telexide::api::APIClient::new(None, &cfg.token);
    // Resilient startup: a transient network blip during getMe must NOT crash
    // the process. A bare `?` here exits `run()`, and systemd would just restart
    // straight back into the same failure — a tight crash-loop that can trip the
    // start-limit and leave the bot down for good. Instead retry with capped
    // exponential backoff until Telegram answers (once polling, `robust_poll`
    // already tolerates network loss the same way). A genuinely bad/revoked token
    // simply loops here, logging each attempt, rather than thrashing systemd.
    let me = {
        let mut delay = STARTUP_BACKOFF_START;
        loop {
            match probe.get_me().await {
                Ok(me) => break me,
                Err(err) => {
                    eprintln!(
                        "getMe failed at startup (retrying in {}s): {err}",
                        delay.as_secs()
                    );
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(STARTUP_BACKOFF_MAX);
                }
            }
        }
    };
    if me.id != bot_id {
        return Err(anyhow::anyhow!(
            "bot id mismatch: token parses to {bot_id} but Telegram reports {}",
            me.id
        ));
    }
    let bot_username = me
        .username
        .clone()
        .ok_or_else(|| anyhow::anyhow!("bot has no @username (BotFather should always assign one)"))?;

    let mut builder = ClientBuilder::new();
    builder
        .set_token(&cfg.token)
        .set_framework(create_framework!(
            bot_username.as_str(),
            start,
            assets,
            balance,
            bets,
            history,
            send,
            predict,
            rule,
            feedback,
            sell,
            buy,
            events,
            claim,
            checkin,
            settings,
            timezone,
            onlyreplyhere,
            replyanywhere,
            mint,
            pause,
            unpause,
            broadcast,
            reset,
            settle,
            redeploy,
            dashboard,
            load,
            backup,
            profile,
            delete
        ))
        .add_handler_func(callbacks::on_callback)
        .add_handler_func(callbacks::on_my_chat_member)
        .add_handler_func(crate::commands::predict::on_message)
        .add_handler_func(crate::commands::feedback::on_message);
    let client = builder.build();
    {
        let mut data = client.data.write();
        data.insert::<DbKey>(db);
        data.insert::<ConfigKey>(cfg_arc);
        data.insert::<BotIdKey>(bot_id);
        data.insert::<BotUsernameKey>(bot_username.clone());
        data.insert::<QuotesKey>(Arc::new(parking_lot::Mutex::new(
            crate::commands::betting::QuoteStore::default(),
        )));
        data.insert::<ConvosKey>(Arc::new(Mutex::new(HashMap::new())));
    }

    // Eagerly set the user-facing command menu (the "/" autocomplete), once
    // per supported locale plus a default (English) menu for everyone else.
    // Telegram serves each user the menu matching their client language.
    {
        use crate::core::i18n::Lang;
        use telexide::api::types::SetMyCommands;
        use telexide::model::BotCommand;

        let menu_for = |lang: Lang| -> Vec<BotCommand> {
            crate::core::i18n::command_menu(lang)
                .iter()
                .map(|(name, desc)| BotCommand {
                    command: (*name).to_string(),
                    description: (*desc).to_string(),
                })
                .collect()
        };

        // Default menu (no language_code) — English.
        if let Err(err) = client.api_client.set_my_commands(menu_for(Lang::En).into()).await {
            eprintln!("setMyCommands (default) error (continuing): {err}");
        }

        // Per-language menus. Telegram only accepts ISO 639-1 codes here, so
        // the Hant/Hans split both register under their `zh-*` menu_code and a
        // failure for any one locale is logged and skipped.
        for lang in Lang::ALL {
            let mut req = SetMyCommands::new(menu_for(lang));
            req.language_code = Some(lang.menu_code().to_string());
            if let Err(err) = client.api_client.set_my_commands(req).await {
                eprintln!("setMyCommands ({}) error (continuing): {err}", lang.menu_code());
            }
        }
    }

    eprintln!(
        "waterx-bot ready: @{bot_username} (id {bot_id}), {} mode",
        if cfg.dev { "dev" } else { "production" }
    );

    // Post-redeploy notice: if `/redeploy` left a marker (the chat to notify),
    // confirm we're back online here, then remove it. (The process that ran
    // `/redeploy` died in the restart, so this is the only place to report it.)
    {
        use telexide::api::types::SendMessage;
        let marker = crate::commands::admin::REDEPLOY_MARKER;
        if let Ok(contents) = std::fs::read_to_string(marker) {
            if let Ok(chat) = contents.trim().parse::<i64>() {
                if let Err(err) = client
                    .api_client
                    .send_message(SendMessage::new(
                        chat.into(),
                        "✅ Redeploy complete — back online.".to_string(),
                    ))
                    .await
                {
                    eprintln!("redeploy notify error: {err}");
                }
            }
            let _ = std::fs::remove_file(marker);
        }
    }

    // Custom polling loop: robust to per-update deserialization errors and
    // transient HTTP errors. The default telexide stream bails the whole bot
    // on the first malformed update, which kills the process whenever
    // Telegram adds an unknown enum variant (the classic example is a new
    // sticker `type` field). Here we parse each update individually and just
    // skip the bad ones, always advancing the offset so we never re-fetch a
    // poisoned batch.
    robust_poll(&client).await
}

async fn robust_poll(client: &telexide::client::Client) -> anyhow::Result<()> {
    use serde_json::Value;
    let mut offset: i64 = 0;
    let allowed = client.allowed_updates.clone();
    loop {
        let req = GetUpdates {
            offset: Some(offset + 1),
            limit: Some(100),
            timeout: Some(5),
            allowed_updates: if allowed.is_empty() {
                None
            } else {
                Some(allowed.clone())
            },
        };
        let payload = match serde_json::to_value(&req) {
            Ok(v) => Some(v),
            Err(err) => {
                eprintln!("getUpdates serialize error (sleeping 5s): {err}");
                tokio::time::sleep(POLL_RETRY_BACKOFF).await;
                continue;
            }
        };
        let resp = match client.api_client.post(APIEndpoint::GetUpdates, payload).await {
            Ok(r) => r,
            Err(err) => {
                eprintln!("getUpdates http error (sleeping 5s): {err}");
                tokio::time::sleep(POLL_RETRY_BACKOFF).await;
                continue;
            }
        };
        if !resp.ok {
            eprintln!("getUpdates not-ok (sleeping 5s): {:?}", resp.description);
            tokio::time::sleep(POLL_RETRY_BACKOFF).await;
            continue;
        }
        let items = match resp.result {
            Some(Value::Array(items)) => items,
            _ => continue,
        };
        for item in items {
            // Always advance offset based on the raw update_id, even if full
            // deserialisation fails — otherwise a poisoned update would make
            // us re-fetch the same batch forever.
            let raw_id = item.get("update_id").and_then(Value::as_i64);
            if let Some(id) = raw_id {
                offset = offset.max(id);
            }
            match serde_json::from_value::<telexide::model::Update>(item) {
                Ok(update) => client.fire_handlers(update),
                Err(err) => eprintln!("update parse error (skipping update_id={raw_id:?}): {err}"),
            }
        }
    }
}
