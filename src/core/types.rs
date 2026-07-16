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

/// One competition the `/events` feed surfaces, and how to fetch just it from the
/// waterx browse endpoint. The feed is a huge cursor-paginated mix (crypto/binary
/// and many leagues) with no "only these" filter, but its `type` + `league` query
/// params narrow a request to one competition (e.g. `type=esports&league=lol`), so
/// the bot pulls one stream per filter and skips the bulk. Owner-editable at
/// runtime via `/leagues` (persisted as JSON in the `meta` table); only
/// team-vs-team `sport`/`esports` matches render + settle, so other types yield
/// nothing even if fetched. Serialized field name is `type` for a compact store.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LeagueFilter {
    /// Browse `type` query value — the broad category (`sport`, `esports`, …).
    #[serde(rename = "type")]
    pub api_type: String,
    /// Browse `league` query value (lowercase, e.g. `fifa_wc`, `lol`). `None` =
    /// the whole type (every league in that category).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub league: Option<String>,
    /// Tournament markers matched (case-insensitively) in the always-English
    /// market `description`; empty = keep the whole league. Lets a league that
    /// mixes several tournaments (e.g. LoL: MSI + Esports World Cup + regional
    /// leagues) be narrowed to just the wanted ones.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tournaments: Vec<String>,
}

impl LeagueFilter {
    /// The built-in allowlist used until the owner customises it (and restored by
    /// `/leagues reset`): the World Cup + League of Legends' MSI & Esports World Cup.
    pub fn defaults() -> Vec<LeagueFilter> {
        vec![
            LeagueFilter {
                api_type: "sport".into(),
                league: Some("fifa_wc".into()),
                tournaments: vec![],
            },
            LeagueFilter {
                api_type: "esports".into(),
                league: Some("lol".into()),
                tournaments: vec!["Mid-Season Invitational".into(), "Esports World Cup".into()],
            },
        ]
    }

    /// A stable one-line human label, e.g. `esports / lol [Mid-Season Invitational,
    /// Esports World Cup]` — used by the `/leagues` listing (owner-only, plain text).
    pub fn label(&self) -> String {
        let mut s = self.api_type.clone();
        if let Some(l) = &self.league {
            s.push_str(" / ");
            s.push_str(l);
        }
        if !self.tournaments.is_empty() {
            s.push_str(" [");
            s.push_str(&self.tournaments.join(", "));
            s.push(']');
        }
        s
    }
}
