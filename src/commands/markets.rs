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
/// Polymarket Gamma event lookup (`…/events/slug/<ticker>`). Used only by
/// `gamma_resolution` to detect a settled match (display/bet odds come from the
/// waterx feed's relayed YES prices — there's no separate Gamma odds overlay).
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

/// One outcome of a sourced (Polymarket) event: a localized name + its YES odds.
#[derive(Debug, Clone)]
pub struct SourcedOutcome {
    pub name: String,
    /// YES odds in cents (None = no quote).
    pub yes_cents: Option<f64>,
}

/// A sourced (Polymarket) event from the waterx feed. Holds an ordered list of
/// outcomes (a 1X2 sport match → 3: `[teamA, draw, teamB]`), so the bet/sell flow
/// is outcome-index based. Supersedes the old sport-specific `MarketInfo`.
#[derive(Debug, Clone)]
pub struct SourcedEvent {
    /// The event's slug — carried in the `bet:`/`opt:` callbacks, used to re-find
    /// the event in the feed, and stored as the sourced event's `source_ref` (the
    /// join key for `gamma_resolution`).
    pub key: String,
    pub title: String,
    pub outcomes: Vec<SourcedOutcome>,
    pub starts_at: Option<i64>,
    pub ends_at: i64,
    pub live: bool,
}

/// Group the browse feed into sourced events. **Sport matches only**: the 1X2
/// moneyline → 3 outcomes in the fixed `[teamA, draw, teamB]` order (matching
/// `gamma_resolution`'s idx convention); `sport-award`/`crypto` are dropped. Odds
/// are the feed's relayed YES prices; `lang` localizes the Draw label.
fn group_events(items: &[Item], lang: Lang) -> Vec<SourcedEvent> {
    let mut events: Vec<SourcedEvent> = Vec::new();
    for it in items {
        let Some(r) = it.next_round.as_ref() else { continue };
        let d = &it.market.display;
        if d.kind.as_deref() != Some("sport") {
            continue;
        }
        let (Some(a), Some(b)) = (d.team_a.as_ref(), d.team_b.as_ref()) else { continue };
        let side = |key: &str, name: String| -> Option<SourcedOutcome> {
            let s = r.sides.iter().find(|s| s.key == key)?;
            Some(SourcedOutcome { name, yes_cents: s.odds_cents })
        };
        let outcomes: Vec<SourcedOutcome> = [
            side("teamA", a.name.clone()),
            side("draw", i18n::draw_label(lang).to_string()),
            side("teamB", b.name.clone()),
        ]
        .into_iter()
        .flatten()
        .collect();
        if outcomes.is_empty() {
            continue;
        }
        events.push(SourcedEvent {
            key: it.market.slug.clone(),
            title: format!("{} vs. {}", a.name, b.name),
            outcomes,
            starts_at: r.starts_at,
            ends_at: r.ends_at.unwrap_or(0),
            live: r.phase.as_deref() == Some("live"),
        });
    }
    events
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
/// (`bet:<key>`). Kickoff times are shown in `tz_min` (minutes east of
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
        out.push_str(&render_market(idx + 1, m, tz_min, fmt));
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
                    (n.to_string(), format!("{BET}{}", m.key))
                })
                .collect()
        })
        .collect();

    (out, rows)
}

/// Re-fetch the feed and return the current snapshot for one event (fresh odds) by
/// its `key`, regardless of the display window — used when a bet button is tapped
/// and by the sell flow to re-price a held position (a sourced event stores its
/// `key` as `source_ref`). `Err` = feed fetch/parse failure (worth alerting on);
/// `Ok(None)` = fetched fine but the event is no longer listed (stale button).
pub(crate) async fn fetch_one(
    lang: Lang,
    key: &str,
) -> Result<Option<SourcedEvent>, reqwest::Error> {
    Ok(fetch_markets(lang).await?.into_iter().find(|m| m.key == key))
}

/// True when an event should appear in the brief: live, or kicking off within 24h.
fn within_window(m: &SourcedEvent, now: i64) -> bool {
    m.live || m.starts_at.is_some_and(|t| t >= now && t <= now + 86_400)
}

/// Per-locale cache of the parsed feed: `locale → (fetched_at_unix, events)`.
type FeedCache = HashMap<&'static str, (i64, Vec<SourcedEvent>)>;

/// Process-wide [`FeedCache`]. Guards are never held across an `.await`.
fn feed_cache() -> &'static Mutex<FeedCache> {
    static CACHE: OnceLock<Mutex<FeedCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Pull and parse the feed into sourced events, live-first then soonest. Served
/// from a [`FEED_CACHE_TTL`]-second per-locale cache to avoid hammering the API.
/// Odds are the feed's **relayed Polymarket YES prices** (no separate Gamma odds
/// fetch); every bet re-prices from this cache, so a wager is booked at odds at
/// most `FEED_CACHE_TTL` old. Resolution still goes to Gamma (`gamma_resolution`).
async fn fetch_markets(lang: Lang) -> Result<Vec<SourcedEvent>, reqwest::Error> {
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

    let mut markets = group_events(&resp.data.items, lang);
    markets.sort_by_key(|m| (!m.live, m.starts_at.unwrap_or(i64::MAX)));
    feed_cache().lock().insert(api_locale, (now, markets.clone()));
    Ok(markets)
}

/// Detect a sourced match's resolution from Polymarket: fetch the Gamma event by
/// its `sport-<ticker>` slug and, if it's `closed`, return the winning outcome's
/// index in the sourced event's `[teamA, draw, teamB]` order — the winner is the
/// only market whose YES price resolved to ~1 (losers → 0). `Ok(None)` when not
/// yet resolved, the slug is empty, or there's no clear winner (e.g. a void);
/// `Err` on a feed/parse failure.
pub(crate) async fn gamma_resolution(slug: &str) -> Result<Option<i64>, reqwest::Error> {
    let ticker = slug.strip_prefix("sport-").unwrap_or(slug);
    if ticker.is_empty() {
        return Ok(None);
    }
    let event = http_client()
        .get(format!("{GAMMA_EVENT_URL}{ticker}"))
        .header("user-agent", "waterx-bot/0.1")
        .send()
        .await?
        .error_for_status()?
        .json::<GammaEvent>()
        .await?;
    if !event.closed {
        return Ok(None);
    }
    let title = event.title.as_deref().unwrap_or("");
    let winner = event
        .markets
        .iter()
        .find(|m| m.yes_cents().is_some_and(|c| c >= 99.0))
        .map(|m| m.group_item_title.trim().to_string());
    let Some(winner) = winner else {
        return Ok(None); // closed but no clear winner
    };
    if winner.to_ascii_lowercase().starts_with("draw") {
        return Ok(Some(1)); // draw is the sourced event's middle outcome
    }
    // teamA/teamB by their order in the "TeamA vs. TeamB" title.
    let mut teams: Vec<(usize, &str)> = event
        .markets
        .iter()
        .map(|m| m.group_item_title.trim())
        .filter(|g| !g.to_ascii_lowercase().starts_with("draw"))
        .filter_map(|g| title.find(g).map(|p| (p, g)))
        .collect();
    teams.sort_by_key(|(p, _)| *p);
    match teams.iter().position(|(_, g)| *g == winner) {
        Some(0) => Ok(Some(0)),
        Some(_) => Ok(Some(2)),
        None => Ok(None),
    }
}

/// Render one event in the brief: title + kickoff time + a branch line per outcome
/// (`name — odds` in the caller's odds format, `—` when unpriced). Outcome names
/// are already localized by the feed (teams) / `group_events` (Draw), so no `lang`.
fn render_market(n: usize, ev: &SourcedEvent, tz_min: i64, fmt: OddsFormat) -> String {
    let dot = if ev.live { "🔴 " } else { "" };
    let mut s = format!("\n{n}) {dot}{}\n", ev.title);
    if let Some(t) = fmt_time(ev.starts_at, tz_min) {
        s.push_str(&format!("├ {t}\n"));
    }
    let last = ev.outcomes.len().saturating_sub(1);
    for (i, o) in ev.outcomes.iter().enumerate() {
        let branch = if i == last { "└" } else { "├" };
        match o.yes_cents {
            // Rendered in the user's chosen odds format (Decimal/American/…).
            Some(y) if y > 0.0 => {
                s.push_str(&format!("{branch} {} — {}\n", o.name, format_odds(y, fmt)));
            }
            _ => s.push_str(&format!("{branch} {} — —\n", o.name)),
        }
    }
    s
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
// Polymarket Gamma event — only the fields `gamma_resolution` reads.
// ---------------------------------------------------------------------------

/// A Gamma event (`/events/slug/<ticker>`). The 3-way moneyline is a neg-risk
/// group of `markets`; `title` is "TeamA vs. TeamB" and pins the side order.
#[derive(Deserialize)]
struct GammaEvent {
    #[serde(default)]
    title: Option<String>,
    /// True once Polymarket has resolved the event (the winner's YES price → 1).
    #[serde(default)]
    closed: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{"data":{"items":[
        {"market":{"id":"m1","slug":"sport-fra-swe","display":{"kind":"sport","teamA":{"name":"France"},"teamB":{"name":"Sweden"}}},
         "nextRound":{"startsAt":100,"endsAt":200,"phase":"open","sides":[
            {"key":"teamA","oddsCents":78.1,"trade":{"marketId":"0xA"}},
            {"key":"draw","oddsCents":16.8,"trade":{"marketId":"0xD"}},
            {"key":"teamB","oddsCents":8.4,"trade":{"marketId":"0xB"}}]}},
        {"market":{"id":"m2","slug":"award-x-usa","display":{"kind":"sport-award","award":"WC Winner","candidate":{"name":"USA"}}},
         "nextRound":{"startsAt":100,"endsAt":300,"phase":"open","sides":[
            {"key":"up","oddsCents":2.8,"trade":{"marketId":"0xUSA"}},
            {"key":"down","oddsCents":97.3,"trade":{"marketId":"0xUSA"}}]}},
        {"market":{"id":"m3","slug":"award-x-mexico","display":{"kind":"sport-award","award":"WC Winner","candidate":{"name":"Mexico"}}},
         "nextRound":{"startsAt":100,"endsAt":300,"phase":"live","sides":[
            {"key":"up","oddsCents":6.0,"trade":{"marketId":"0xMEX"}}]}},
        {"market":{"id":"m4","slug":"prop-trump","display":{"kind":"sport-award","award":"Trump to attend?","candidate":{"name":"Trump to attend?"}}},
         "nextRound":{"startsAt":100,"endsAt":300,"phase":"open","sides":[
            {"key":"up","oddsCents":40.0,"trade":{"marketId":"0xTRU"}},
            {"key":"down","oddsCents":60.0,"trade":{"marketId":"0xTRU"}}]}},
        {"market":{"id":"m5","slug":"crypto-btc","display":{"kind":"crypto"}},
         "nextRound":{"phase":"open","sides":[]}}
    ]}}"#;

    fn grouped() -> Vec<SourcedEvent> {
        let resp: BrowseResp = serde_json::from_str(FIXTURE).unwrap();
        group_events(&resp.data.items, Lang::En)
    }

    #[test]
    fn sport_becomes_a_three_outcome_event() {
        let evs = grouped();
        let s = evs.iter().find(|e| e.key == "sport-fra-swe").unwrap();
        assert_eq!(s.title, "France vs. Sweden");
        assert_eq!(s.outcomes.len(), 3);
        let names: Vec<&str> = s.outcomes.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["France", "Draw", "Sweden"]);
        assert_eq!(s.outcomes[0].yes_cents, Some(78.1));
        assert_eq!(s.outcomes[2].yes_cents, Some(8.4));
    }

    #[test]
    fn only_sport_kept_other_kinds_dropped() {
        // The fixture also has sport-award candidates, a prop, and crypto; only the
        // sport match survives grouping (sourced `/events` is sport-only).
        let evs = grouped();
        assert_eq!(evs.len(), 1, "only the sport match is kept");
        assert_eq!(evs[0].key, "sport-fra-swe");
    }
}
