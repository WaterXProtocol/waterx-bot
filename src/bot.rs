use crate::commands::{callbacks, *};
use crate::database::{db_filename, Database};
use crate::game::BetGame;
use crate::types::BotConfig;
use std::{collections::HashMap, sync::Arc, time::Duration};
use telexide::{
    api::{types::GetUpdates, APIEndpoint, API},
    create_framework,
    prelude::*,
};
use tokio::sync::Mutex;
use typemap_rev::TypeMapKey;

pub struct DbKey;
impl TypeMapKey for DbKey {
    type Value = Arc<Database>;
}

pub struct GamesKey;
impl TypeMapKey for GamesKey {
    type Value = Arc<Mutex<HashMap<String, BetGame>>>;
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

pub async fn run() -> anyhow::Result<()> {
    let cfg = BotConfig::from_env()?;
    let bot_id: i64 = cfg
        .token
        .split(':')
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow::anyhow!("malformed bot token (expected `<id>:<secret>`)"))?;
    let db = Arc::new(Database::new(db_filename(cfg.dev), bot_id)?);
    let games_map: HashMap<String, BetGame> = match db.load_all_bet_games() {
        Ok(rows) => rows.into_iter().collect(),
        Err(err) => {
            eprintln!("bet_games load error (starting with empty map): {err}");
            HashMap::new()
        }
    };
    let games: Arc<Mutex<HashMap<String, BetGame>>> = Arc::new(Mutex::new(games_map));
    let cfg_arc = Arc::new(cfg.clone());

    // Resolve the bot's real @username via getMe BEFORE building the framework
    // — telexide's command router compares group-chat command suffixes against
    // this string (e.g. `/host@BotUsername`). Using anything other than the
    // actual username makes group commands silently no-op when Telegram
    // appends the @suffix. The call also serves as a token sanity check.
    let probe = telexide::api::APIClient::new(None, &cfg.token);
    let me = probe.get_me().await?;
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
            start, balance, fruit, send, host, sell, buy, markets, checkin,
            mint, pause, unpause, broadcast
        ))
        .add_handler_func(callbacks::on_callback)
        .add_handler_func(callbacks::on_my_chat_member);
    let client = builder.build();
    {
        let mut data = client.data.write();
        data.insert::<DbKey>(db);
        data.insert::<GamesKey>(games);
        data.insert::<ConfigKey>(cfg_arc);
        data.insert::<BotIdKey>(bot_id);
        data.insert::<BotUsernameKey>(bot_username.clone());
    }

    // Eagerly set the user-facing command menu (the "/" autocomplete), once
    // per supported locale plus a default (English) menu for everyone else.
    // Telegram serves each user the menu matching their client language.
    {
        use crate::i18n::Lang;
        use telexide::api::types::SetMyCommands;
        use telexide::model::BotCommand;

        let menu_for = |lang: Lang| -> Vec<BotCommand> {
            crate::i18n::command_menu(lang)
                .iter()
                .map(|(name, desc)| BotCommand {
                    command: (*name).to_string(),
                    description: (*desc).to_string(),
                })
                .collect()
        };

        // Default menu (no language_code) — English.
        if let Err(err) = client
            .api_client
            .set_my_commands(menu_for(Lang::En).into())
            .await
        {
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
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        let resp = match client.api_client.post(APIEndpoint::GetUpdates, payload).await {
            Ok(r) => r,
            Err(err) => {
                eprintln!("getUpdates http error (sleeping 5s): {err}");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        if !resp.ok {
            eprintln!("getUpdates not-ok (sleeping 5s): {:?}", resp.description);
            tokio::time::sleep(Duration::from_secs(5)).await;
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
