use std::env;

#[derive(Debug, Clone)]
pub struct BotConfig {
    pub token: String,
    pub owner: i64,
    pub dev: bool,
}

impl BotConfig {
    /// Load from environment variables. `dotenvy::dotenv()` should be called
    /// from `main` before this so a `.env` file is picked up.
    ///
    /// Required: `BOT_TOKEN`, `BOT_OWNER`.
    /// Optional: `BOT_DEV` (default `true`; falsy values: `false`, `0`).
    pub fn from_env() -> anyhow::Result<Self> {
        let token = env::var("BOT_TOKEN")
            .map_err(|_| anyhow::anyhow!("BOT_TOKEN not set"))?;
        let owner_raw = env::var("BOT_OWNER")
            .map_err(|_| anyhow::anyhow!("BOT_OWNER not set"))?;
        let owner: i64 = owner_raw
            .parse()
            .map_err(|e| anyhow::anyhow!("BOT_OWNER must be an integer: {e}"))?;
        // Dev defaults to true so a fresh `.env` works in any chat type
        // without thinking about the envelope-drop chat-type rule. Set
        // `BOT_DEV=false` only when running a production bot that should
        // restrict envelope drops to non-private chats.
        let dev = env::var("BOT_DEV")
            .ok()
            .map(|s| !matches!(s.as_str(), "false" | "FALSE" | "False" | "0"))
            .unwrap_or(true);
        Ok(Self { token, owner, dev })
    }
}

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BetState {
    betting,
    closed,
    settled,
    draw,
}

impl BetState {
    pub fn as_str(&self) -> &'static str {
        match self {
            BetState::betting => "下注中",
            BetState::closed => "已收盤",
            BetState::settled => "已結算",
            BetState::draw => "流局",
        }
    }
}
