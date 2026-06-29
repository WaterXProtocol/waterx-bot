use crate::commands::tg::Row;
use crate::commands::util::*;
use crate::core::i18n::{self, Lang};
use crate::core::types::OddsFormat;
use chrono::Utc;
use parking_lot::Mutex;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;
use telexide::prelude::*;

/// waterx prediction-market browse endpoint (the normalized match list:
/// teams, kickoff, sport detection, localized names).
const BROWSE_URL: &str = "https://api.waterx.app/predict/browse";
/// Polymarket Gamma event lookup (`…/events/slug/<ticker>`). Live odds are
/// re-priced straight from here and overlaid onto the waterx match list — see
/// the Gamma odds-overlay section below.
const GAMMA_EVENT_URL: &str = "https://gamma-api.polymarket.com/events/slug/";
/// How long a fetched feed is reused before re-hitting the API (per locale), to
/// stay under the upstream rate limit. Every real-money bet is **re-priced from
/// this cache at place time** (`betting::refetch_quote`), so a wager is booked at
/// odds at most `FEED_CACHE_TTL` old — never an older locked snapshot.
const FEED_CACHE_TTL: i64 = 30;
/// Cap so the brief stays well under Telegram's 4096-char message limit; the
/// overflow is summarised with a "…and N more" tail.
const MAX_MARKETS: usize = 8;

/// Callback-data prefix: tapping a market number opens the bet flow.
pub const BET: &str = "bet:";

/// A single sport match, distilled from the browse feed.
#[derive(Debug, Clone)]
pub struct MarketInfo {
    pub market_id: String,
    pub slug: String,
    pub team_a: String,
    pub team_b: String,
    /// YES odds in cents for each outcome (None if the side is missing).
    pub odds_a: Option<f64>,
    pub odds_draw: Option<f64>,
    pub odds_b: Option<f64>,
    pub starts_at: Option<i64>,
    pub ends_at: i64,
    pub live: bool,
}

impl MarketInfo {
    /// YES odds (cents) for one of `teamA`/`teamB`/`draw`.
    pub fn odds(&self, outcome: &str) -> Option<f64> {
        match outcome {
            "teamA" => self.odds_a,
            "teamB" => self.odds_b,
            "draw" => self.odds_draw,
            _ => None,
        }
    }
}

#[command(description = "browse live prediction events")]
pub async fn events(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let chat_id = message.chat.get_id();
    // Kickoff times render in the caller's saved timezone (private chats only —
    // a group brief is a shared message, so it stays UTC).
    let tz = if is_group_chat(chat_id) {
        0
    } else {
        message
            .from
            .as_ref()
            .and_then(|u| db(&ctx).get_tz(u.id).ok().flatten())
            .unwrap_or(0)
    };
    // Odds render in the caller's chosen format (not privacy-sensitive, so used
    // in groups too — the brief is rendered once by the invoker).
    let fmt = message
        .from
        .as_ref()
        .and_then(|u| db(&ctx).get_odds_fmt(u.id).ok())
        .unwrap_or_default();
    let (text, rows) = brief(lang_for_msg(&ctx, &message), tz, fmt).await;
    crate::commands::tg::send_with_buttons(&ctx, chat_id, &text, &rows).await?;
    Ok(())
}

/// Build the market brief (text) plus a numbered button per shown match
/// (`bet:<market_id>`). Kickoff times are shown in `tz_min` (minutes east of
/// UTC; 0 = UTC). On any fetch/parse failure returns the localized "unavailable"
/// line and no buttons.
pub(crate) async fn brief(lang: Lang, tz_min: i64, fmt: OddsFormat) -> (String, Vec<Row>) {
    let now = Utc::now().timestamp();
    let markets = match fetch_markets(lang).await {
        Ok(mut m) => {
            m.retain(|x| within_window(x, now));
            m
        }
        Err(err) => {
            eprintln!("[markets] fetch error: {err}");
            return (i18n::markets_unavailable(lang).to_string(), Vec::new());
        }
    };

    let mut out = format!(
        "{} — {}\n",
        i18n::markets_title(lang),
        fmt_local_date(now, tz_min)
    );
    if markets.is_empty() {
        out.push('\n');
        out.push_str(i18n::markets_empty(lang));
        return (out, Vec::new());
    }

    out.push('\n');
    out.push_str(i18n::markets_section(lang));
    out.push('\n');
    let shown = &markets[..markets.len().min(MAX_MARKETS)];
    for (idx, m) in shown.iter().enumerate() {
        out.push_str(&render_market(lang, idx + 1, m, tz_min, fmt));
    }
    if markets.len() > MAX_MARKETS {
        out.push_str(&i18n::markets_more(lang, &(markets.len() - MAX_MARKETS).to_string()));
        out.push('\n');
    }

    // Numbered bet buttons, four per row.
    let rows: Vec<Row> = shown
        .chunks(4)
        .enumerate()
        .map(|(row_idx, chunk)| {
            chunk
                .iter()
                .enumerate()
                .map(|(col, m)| {
                    let n = row_idx * 4 + col + 1;
                    (n.to_string(), format!("{BET}{}", m.market_id))
                })
                .collect()
        })
        .collect();

    (out, rows)
}

/// Re-fetch the feed and return the current snapshot for one match (fresh odds),
/// regardless of the display window — used when the user taps a bet button.
/// `Err` = feed fetch/parse failure (a tech error worth alerting on); `Ok(None)`
/// = fetched fine but the market is no longer listed (stale button, expected).
pub(crate) async fn fetch_one(
    lang: Lang,
    market_id: &str,
) -> Result<Option<MarketInfo>, reqwest::Error> {
    Ok(fetch_markets(lang)
        .await?
        .into_iter()
        .find(|m| m.market_id == market_id))
}

/// Like [`fetch_one`] but keyed by the match **slug** (what a sourced event
/// stores as its `source_ref`) — used by the sell flow to re-price a held
/// position at the current odds.
pub(crate) async fn fetch_one_by_slug(
    lang: Lang,
    slug: &str,
) -> Result<Option<MarketInfo>, reqwest::Error> {
    Ok(fetch_markets(lang).await?.into_iter().find(|m| m.slug == slug))
}

/// True when a match should appear in the brief: live, or kicking off within 24h.
fn within_window(m: &MarketInfo, now: i64) -> bool {
    m.live || m.starts_at.is_some_and(|t| t >= now && t <= now + 86_400)
}

/// Per-locale cache of the parsed feed: `locale → (fetched_at_unix, markets)`.
type FeedCache = HashMap<&'static str, (i64, Vec<MarketInfo>)>;

/// Process-wide [`FeedCache`]. Guards are never held across an `.await`.
fn feed_cache() -> &'static Mutex<FeedCache> {
    static CACHE: OnceLock<Mutex<FeedCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Per-ticker cache of distilled Gamma odds, **shared across locales** (the odds
/// are language-independent, so the `en` and `zh` feed refreshes shouldn't each
/// re-fetch the same events). Keyed by the de-prefixed Gamma ticker; entries live
/// for [`FEED_CACHE_TTL`]. (Worst case a wager books odds up to ~2×TTL old — a
/// gamma entry up to TTL old baked into a feed snapshot served for up to another
/// TTL — which is fine for play-money stakes.)
type GammaCache = HashMap<String, (i64, GammaSides)>;

/// Process-wide [`GammaCache`]. Guards are never held across an `.await`.
fn gamma_cache() -> &'static Mutex<GammaCache> {
    static CACHE: OnceLock<Mutex<GammaCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Pull and parse the feed into sport markets, live-first then soonest. Served
/// from a [`FEED_CACHE_TTL`]-second per-locale cache to avoid hammering the API.
async fn fetch_markets(lang: Lang) -> Result<Vec<MarketInfo>, reqwest::Error> {
    // Chinese users get Chinese team names; everyone else English.
    let api_locale = match lang {
        Lang::Hant | Lang::Hans => "zh",
        _ => "en",
    };
    let now = Utc::now().timestamp();

    // Cache hit: reuse a snapshot fetched within the TTL. (Clone out, then drop
    // the guard — it must not be held across the network await below.)
    if let Some((fetched_at, markets)) = feed_cache().lock().get(api_locale) {
        if now - *fetched_at <= FEED_CACHE_TTL {
            return Ok(markets.clone());
        }
    }

    let resp = http_client()
        .get(BROWSE_URL)
        .query(&[("locale", api_locale), ("limit", "200")])
        .header("user-agent", "waterx-bot/0.1")
        .send()
        .await?
        .error_for_status()?
        .json::<BrowseResp>()
        .await?;

    let mut markets: Vec<MarketInfo> = resp.data.items.iter().filter_map(to_match_info).collect();
    markets.sort_by_key(|m| (!m.live, m.starts_at.unwrap_or(i64::MAX)));
    // Replace the relayed waterx odds with live odds straight from Polymarket's
    // Gamma API (waterx odds kept as the per-side fallback). Only the matches
    // that actually get shown/bet are priced, to bound the call count. Done once
    // per cache refresh, so the cached snapshot — and thus bet-time re-pricing
    // via `fetch_one` — carries the Gamma numbers.
    overlay_gamma_odds(&mut markets, now).await;
    feed_cache().lock().insert(api_locale, (now, markets.clone()));
    Ok(markets)
}

// ---------------------------------------------------------------------------
// Gamma odds overlay — live odds straight from Polymarket
// ---------------------------------------------------------------------------
//
// waterx's `oddsCents` is just a relay of Polymarket's YES price, so for fresher
// numbers (and independence from waterx's odds cadence) we re-price each shown
// match directly against Polymarket's public Gamma API. The join key is the
// slug: waterx ships `sport-<ticker>` and Gamma's event slug is exactly
// `<ticker>`, so stripping the `sport-` prefix yields the event lookup. A 3-way
// match is a neg-risk event of separate Yes/No markets (one per team + draw);
// each is mapped back to teamA/teamB/draw and we read its YES price.

/// Overlay live Gamma odds onto the matches that will be shown/bet (the first
/// `MAX_MARKETS` within the display window). Best-effort: any per-match
/// fetch/parse failure leaves that match on its waterx odds, and a present Gamma
/// price overrides only its own side. Runs once per cache refresh.
async fn overlay_gamma_odds(markets: &mut [MarketInfo], now: i64) {
    let targets: Vec<(usize, String)> = markets
        .iter()
        .enumerate()
        .filter(|(_, m)| within_window(m, now))
        .take(MAX_MARKETS)
        .map(|(i, m)| (i, m.slug.clone()))
        .collect();
    if targets.is_empty() {
        return;
    }
    // One event fetch per match, concurrently — but each is served from the
    // locale-shared gamma cache when fresh, so the second locale to refresh reuses
    // the first's results instead of re-fetching.
    let mut set = tokio::task::JoinSet::new();
    for (i, slug) in targets {
        set.spawn(async move { (i, fetch_gamma_sides(&slug, now).await) });
    }
    while let Some(joined) = set.join_next().await {
        let Ok((i, Some(sides))) = joined else { continue };
        let m = &mut markets[i];
        if sides.a.is_some() {
            m.odds_a = sides.a;
        }
        if sides.draw.is_some() {
            m.odds_draw = sides.draw;
        }
        if sides.b.is_some() {
            m.odds_b = sides.b;
        }
    }
    // Drop entries no future refresh can reuse, so the cache stays bounded.
    gamma_cache()
        .lock()
        .retain(|_, (fetched_at, _)| now - *fetched_at <= FEED_CACHE_TTL);
}

/// Fetch one match's Gamma event by slug and distil its per-side YES odds (cents).
/// Served from the locale-shared [`gamma_cache`] when fresh, so `en`/`zh` refreshes
/// don't double-fetch the same event. `None` on any network/parse failure (caller
/// falls back to the waterx odds); only successful mappings are cached.
async fn fetch_gamma_sides(waterx_slug: &str, now: i64) -> Option<GammaSides> {
    let ticker = waterx_slug.strip_prefix("sport-").unwrap_or(waterx_slug);
    if ticker.is_empty() {
        return None;
    }
    // Cache hit shared across locales (clone out, drop the guard before the await).
    if let Some((fetched_at, sides)) = gamma_cache().lock().get(ticker) {
        if now - *fetched_at <= FEED_CACHE_TTL {
            return Some(*sides);
        }
    }
    let event = http_client()
        .get(format!("{GAMMA_EVENT_URL}{ticker}"))
        .header("user-agent", "waterx-bot/0.1")
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<GammaEvent>()
        .await
        .ok()?;
    let sides = map_sides(&event)?;
    gamma_cache().lock().insert(ticker.to_string(), (now, sides));
    Some(sides)
}

/// Map a Gamma event's per-outcome markets onto teamA/teamB/draw YES cents.
///
/// Draw is identified by its `groupItemTitle` ("Draw …") — locale-independent.
/// The two win markets are ordered by where their title appears in the event
/// title ("TeamA vs. TeamB"), the same slug-derived order waterx uses for
/// teamA/teamB, so the mapping holds in any display language. teamA/teamB odds
/// are only applied when **both** win markets resolve cleanly (else each side
/// keeps its waterx value); draw overlays independently.
fn map_sides(ev: &GammaEvent) -> Option<GammaSides> {
    let title = ev.title.as_deref().unwrap_or("");
    let mut draw = None;
    let mut wins: Vec<(usize, f64)> = Vec::new(); // (position in title, yes cents)
    for m in &ev.markets {
        let Some(cents) = m.yes_cents() else { continue };
        let git = m.group_item_title.trim();
        if git.is_empty() {
            continue;
        }
        if git.to_ascii_lowercase().starts_with("draw") {
            draw = Some(cents);
        } else if let Some(pos) = title.find(git) {
            wins.push((pos, cents));
        }
    }
    let (a, b) = if wins.len() == 2 {
        wins.sort_by_key(|(pos, _)| *pos);
        (Some(wins[0].1), Some(wins[1].1))
    } else {
        (None, None)
    };
    if a.is_none() && b.is_none() && draw.is_none() {
        return None;
    }
    Some(GammaSides { a, draw, b })
}

fn to_match_info(it: &Item) -> Option<MarketInfo> {
    let d = &it.market.display;
    if d.kind.as_deref() != Some("sport") {
        return None;
    }
    let a = d.team_a.as_ref()?;
    let b = d.team_b.as_ref()?;
    let r = it.next_round.as_ref()?;
    Some(MarketInfo {
        market_id: it.market.id.clone(),
        slug: it.market.slug.clone(),
        team_a: a.name.clone(),
        team_b: b.name.clone(),
        odds_a: odds(&r.sides, "teamA"),
        odds_draw: odds(&r.sides, "draw"),
        odds_b: odds(&r.sides, "teamB"),
        starts_at: r.starts_at,
        ends_at: r.ends_at.unwrap_or(0),
        live: r.phase.as_deref() == Some("live"),
    })
}

fn render_market(lang: Lang, n: usize, m: &MarketInfo, tz_min: i64, fmt: OddsFormat) -> String {
    let dot = if m.live { "🔴 " } else { "" };
    let mut s = format!("\n{n}) {dot}{} vs. {}\n", m.team_a, m.team_b);
    if let Some(t) = fmt_time(m.starts_at, tz_min) {
        s.push_str(&format!("├ {t}\n"));
    }
    // Home / draw / away.
    let rows: [(&str, Option<f64>); 3] = [
        (&m.team_a, m.odds_a),
        (i18n::draw_label(lang), m.odds_draw),
        (&m.team_b, m.odds_b),
    ];
    for (i, (label, yes)) in rows.iter().enumerate() {
        let branch = if i == rows.len() - 1 { "└" } else { "├" };
        match yes {
            // Rendered in the user's chosen odds format (Decimal/American/…).
            Some(y) if *y > 0.0 => {
                s.push_str(&format!("{branch} {label} — {}\n", format_odds(*y, fmt)));
            }
            _ => s.push_str(&format!("{branch} {label} — —\n")),
        }
    }
    s
}

/// YES odds (in cents) for the side keyed `key`, if present.
fn odds(sides: &[Side], key: &str) -> Option<f64> {
    sides.iter().find(|s| s.key == key).and_then(|s| s.odds_cents)
}

/// `startsAt` (unix seconds) → `"Jun 27 · 17:00 UTC+8"`, in `tz_min` (minutes
/// east of UTC; 0 = UTC). Thin wrapper over the shared `util::fmt_local_time`
/// that threads the optional timestamp; date-only uses `util::fmt_local_date`.
fn fmt_time(ts: Option<i64>, tz_min: i64) -> Option<String> {
    fmt_local_time(ts?, tz_min)
}

// ---------------------------------------------------------------------------
// Browse-endpoint response — only the fields the brief reads are modelled.
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct BrowseResp {
    data: BrowseData,
}

#[derive(Deserialize)]
struct BrowseData {
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    market: Market,
    #[serde(rename = "nextRound")]
    next_round: Option<Round>,
}

#[derive(Deserialize)]
struct Market {
    id: String,
    #[serde(default)]
    slug: String,
    display: Display,
}

#[derive(Deserialize)]
struct Display {
    kind: Option<String>,
    #[serde(rename = "teamA")]
    team_a: Option<Team>,
    #[serde(rename = "teamB")]
    team_b: Option<Team>,
}

#[derive(Deserialize)]
struct Team {
    name: String,
}

#[derive(Deserialize)]
struct Round {
    #[serde(rename = "startsAt")]
    starts_at: Option<i64>,
    #[serde(rename = "endsAt")]
    ends_at: Option<i64>,
    #[serde(default)]
    sides: Vec<Side>,
    phase: Option<String>,
}

#[derive(Deserialize)]
struct Side {
    key: String,
    // Usually whole cents, but crypto rounds can report fractional values
    // (e.g. 99.9), so this must be a float or the whole feed fails to parse.
    #[serde(rename = "oddsCents")]
    odds_cents: Option<f64>,
}

// ---------------------------------------------------------------------------
// Polymarket Gamma event — only the fields the odds overlay reads.
// ---------------------------------------------------------------------------

/// YES-price (cents) overlay for one match's three sides; `None` = keep waterx.
#[derive(Clone, Copy)]
struct GammaSides {
    a: Option<f64>,
    draw: Option<f64>,
    b: Option<f64>,
}

/// A Gamma event (`/events/slug/<ticker>`). The 3-way moneyline is a neg-risk
/// group of `markets`; `title` is "TeamA vs. TeamB" and pins the side order.
#[derive(Deserialize)]
struct GammaEvent {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    markets: Vec<GammaMarket>,
}

/// One outcome market inside a Gamma event. `outcomes`/`outcomePrices` arrive as
/// **JSON-encoded strings** (e.g. `"[\"Yes\", \"No\"]"`), not arrays — so they're
/// parsed lazily in [`GammaMarket::yes_cents`].
#[derive(Deserialize)]
struct GammaMarket {
    #[serde(rename = "groupItemTitle", default)]
    group_item_title: String,
    #[serde(default)]
    outcomes: Option<String>,
    #[serde(rename = "outcomePrices", default)]
    outcome_prices: Option<String>,
}

impl GammaMarket {
    /// This market's YES price as odds in cents (probability × 100), or `None`
    /// if it can't be parsed or is non-positive (treated as "no quote", so the
    /// waterx odds stand). Matches the bot's existing `oddsCents` convention.
    fn yes_cents(&self) -> Option<f64> {
        let outcomes: Vec<String> = serde_json::from_str(self.outcomes.as_deref()?).ok()?;
        let prices: Vec<String> = serde_json::from_str(self.outcome_prices.as_deref()?).ok()?;
        let yes = outcomes.iter().position(|o| o.eq_ignore_ascii_case("yes"))?;
        let p: f64 = prices.get(yes)?.parse().ok()?;
        (p > 0.0).then_some(p * 100.0)
    }
}
