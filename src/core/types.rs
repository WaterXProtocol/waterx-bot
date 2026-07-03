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
        let token = env::var("BOT_TOKEN").map_err(|_| anyhow::anyhow!("BOT_TOKEN not set"))?;
        let owner_raw = env::var("BOT_OWNER").map_err(|_| anyhow::anyhow!("BOT_OWNER not set"))?;
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

/// User-selectable display format for odds (a 65¢ YES price shown in each):
/// `Decimal` → 1.54, `American` → -185, `Percent` → 65%, `Price` → 65¢. Persisted
/// per user in `balance.odds_fmt` (a stable store code); `Decimal` is the default
/// and the fallback for any unknown/legacy value. The conversion lives in
/// `util::format_odds`; the localized picker labels in `i18n::odds_fmt_label`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum OddsFormat {
    #[default]
    Decimal,
    American,
    Percent,
    Price,
}

impl OddsFormat {
    /// Stable code persisted in the DB and carried in card callback data.
    pub fn store_code(self) -> &'static str {
        match self {
            OddsFormat::Decimal => "dec",
            OddsFormat::American => "us",
            OddsFormat::Percent => "pct",
            OddsFormat::Price => "cents",
        }
    }

    /// Parse a stored code; anything unknown/empty falls back to `Decimal`.
    pub fn from_store_code(code: &str) -> OddsFormat {
        match code {
            "us" => OddsFormat::American,
            "pct" => OddsFormat::Percent,
            "cents" => OddsFormat::Price,
            _ => OddsFormat::Decimal,
        }
    }

    /// All formats, in picker display order.
    pub const ALL: [OddsFormat; 4] = [
        OddsFormat::Decimal,
        OddsFormat::American,
        OddsFormat::Percent,
        OddsFormat::Price,
    ];
}
