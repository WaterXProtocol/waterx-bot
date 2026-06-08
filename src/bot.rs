use crate::commands::{callbacks, messages, *};
use crate::database::Database;
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

#[derive(Debug, Clone)]
pub struct RuntimeParams {
    /// Inverse probability of an envelope spawn per message — 1-in-N.
    pub p_possi: u32,
    /// Mean of the normal distribution that picks the envelope amount.
    pub p_mean: f64,
    /// Standard deviation of the same distribution.
    pub p_std: f64,
}

impl Default for RuntimeParams {
    fn default() -> Self {
        Self {
            p_possi: 5,
            p_mean: 4.0,
            p_std: 3.0,
        }
    }
}

pub struct ParamsKey;
impl TypeMapKey for ParamsKey {
    type Value = Arc<parking_lot::RwLock<RuntimeParams>>;
}

pub async fn run() -> anyhow::Result<()> {
    let cfg = BotConfig::from_env()?;
    let bot_id: i64 = cfg
        .token
        .split(':')
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow::anyhow!("malformed bot token (expected `<id>:<secret>`)"))?;
    let db = Arc::new(Database::new(&cfg.name, bot_id)?);
    let games: Arc<Mutex<HashMap<String, BetGame>>> = Arc::new(Mutex::new(HashMap::new()));
    let cfg_arc = Arc::new(cfg.clone());

    // Resolve the bot's real @username via getMe BEFORE building the framework
    // — telexide's command router compares group-chat command suffixes against
    // this string (e.g. `/gamble@BotUsername`). Using anything other than the
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
    eprintln!(
        "[{}] starting as @{bot_username} (id={})",
        cfg.name, me.id
    );

    let mut builder = ClientBuilder::new();
    builder
        .set_token(&cfg.token)
        .set_framework(create_framework!(
            bot_username.as_str(),
            start, choose, random, tell, tells, shuffle, pair, wolfram,
            balance, send, allin, dice, gamble, fruit, cloth, throw,
            sell, buy, envelope,
            sleep, status, clear, param, reverse, mint
        ))
        .add_handler_func(callbacks::on_callback)
        .add_handler_func(messages::on_message);
    let client = builder.build();
    {
        let mut data = client.data.write();
        data.insert::<DbKey>(db);
        data.insert::<GamesKey>(games);
        data.insert::<ConfigKey>(cfg_arc);
        data.insert::<BotIdKey>(bot_id);
        data.insert::<ParamsKey>(Arc::new(parking_lot::RwLock::new(RuntimeParams::default())));
    }

    // Eagerly set the user-facing command menu (the "/" autocomplete). Other
    // commands stay registered with the framework so they still dispatch when
    // typed; they just don't appear in the menu. Owner-only tools (/sleep
    // /status /clear /param /reverse /mint) and string-utility commands
    // (/choose /tell /tells /shuffle /pair /wolfram) are hidden.
    {
        use telexide::model::BotCommand;
        const VISIBLE: &[(&str, &str)] = &[
            ("start", "嗨？"),
            ("random", "從參數中隨機挑一個"),
            ("balance", "查看水幣餘額"),
            ("fruit", "查看水果"),
            ("cloth", "查看衣服上的水果"),
            ("send", "回覆訊息以送出水幣或水果"),
            ("allin", "回覆訊息以歐印"),
            ("throw", "回覆訊息以隨機丟一顆水果"),
            ("dice", "/dice <猜1-6> <下注> 中6倍"),
            ("gamble", "開賭局或查看自己押注"),
            ("sell", "/sell <水果> <價格>"),
            ("buy", "/buy <水果> <價格>"),
            ("envelope", "/envelope <金額> 發紅包"),
        ];
        let cmds: Vec<BotCommand> = VISIBLE
            .iter()
            .map(|(name, desc)| BotCommand {
                command: (*name).to_string(),
                description: (*desc).to_string(),
            })
            .collect();
        if let Err(err) = client.api_client.set_my_commands(cmds.into()).await {
            eprintln!("setMyCommands error (continuing): {err}");
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
            eprintln!(
                "getUpdates not-ok (sleeping 5s): {:?}",
                resp.description
            );
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
            match serde_json::from_value::<telexide::model::Update>(item.clone()) {
                Ok(update) => client.fire_handlers(update),
                Err(err) => {
                    eprintln!(
                        "update parse error (salvaging update_id={raw_id:?}): {err}"
                    );
                    // Salvage path: even if telexide can't model the message
                    // content (typical cause: Telegram added a new sticker /
                    // entity variant), we can still roll an envelope drop
                    // from chat_id + chat type alone. Spawn so we don't
                    // block the polling loop on send_message.
                    if let Some((chat_id, is_private)) = salvage_chat(&item) {
                        let ctx = telexide::client::Context::new(
                            client.api_client.clone(),
                            client.data.clone(),
                        );
                        tokio::spawn(async move {
                            crate::commands::messages::maybe_spawn_envelope(
                                &ctx, chat_id, is_private,
                            )
                            .await;
                        });
                    }
                }
            }
        }
    }
}

/// Pull `(chat_id, is_private)` out of a raw Update JSON object when its
/// full deserialisation failed. Returns None if it doesn't look like a
/// `message`-bearing update.
fn salvage_chat(item: &serde_json::Value) -> Option<(i64, bool)> {
    let chat = item
        .get("message")
        .or_else(|| item.get("edited_message"))
        .or_else(|| item.get("channel_post"))
        .or_else(|| item.get("edited_channel_post"))?
        .get("chat")?;
    let chat_id = chat.get("id")?.as_i64()?;
    let is_private = chat.get("type").and_then(serde_json::Value::as_str) == Some("private");
    Some((chat_id, is_private))
}
