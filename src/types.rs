use std::env;

#[derive(Debug, Clone)]
pub struct BotConfig {
    pub token: String,
    pub owner: i64,
    pub name: String,
    pub dev: bool,
}

impl BotConfig {
    /// Load from environment variables. `dotenvy::dotenv()` should be called
    /// from `main` before this so a `.env` file is picked up.
    ///
    /// Required: `BOT_TOKEN`, `BOT_OWNER`, `BOT_NAME`.
    /// Optional: `BOT_DEV` (default `false`; truthy values: `true`, `1`).
    pub fn from_env() -> anyhow::Result<Self> {
        let token = env::var("BOT_TOKEN")
            .map_err(|_| anyhow::anyhow!("BOT_TOKEN not set"))?;
        let owner_raw = env::var("BOT_OWNER")
            .map_err(|_| anyhow::anyhow!("BOT_OWNER not set"))?;
        let owner: i64 = owner_raw
            .parse()
            .map_err(|e| anyhow::anyhow!("BOT_OWNER must be an integer: {e}"))?;
        let name = env::var("BOT_NAME")
            .map_err(|_| anyhow::anyhow!("BOT_NAME not set"))?;
        let dev = env::var("BOT_DEV")
            .ok()
            .map(|s| matches!(s.as_str(), "true" | "TRUE" | "True" | "1"))
            .unwrap_or(false);
        Ok(Self {
            token,
            owner,
            name,
            dev,
        })
    }
}

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, PartialEq, Eq)]
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
