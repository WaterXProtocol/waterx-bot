use crate::commands::menu;
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
/// Matches shown per page (keeps each brief well under Telegram's 4096-char
/// message limit); the rest are reachable via the `[next page]` button.
const PAGE_SIZE: usize = 8;
/// The waterx feed tags every `kind == "sport"` match with a `league`. That
/// bucket carries the World Cup (`FIFA_WC`) plus several esports (LOL/CS2/
/// VALORANT/DOTA2), so `/events` keeps only this **curated allowlist** — the
/// World Cup and League of Legends. Add a league here to surface it.
const ALLOWED_LEAGUES: &[&str] = &["FIFA_WC", "LOL"];

/// Callback-data prefix: tapping a market number opens the bet flow.
pub const BET: &str = "bet:";
/// Callback-data prefix for brief pagination: `evpage:<m|s>:<page>` re-renders the
/// brief at `page` in place (`m` = embedded in the /start menu — keep the
/// back-to-home button; `s` = standalone `/events`).
pub const PAGE: &str = "evpage:";

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

/// Group the browse feed into sourced events. **Allowlisted leagues only** (the
/// World Cup + League of Legends — `ALLOWED_LEAGUES`): a `kind == "sport"` match
/// → its outcomes in the fixed `[teamA, draw, teamB]` order (matching
/// `gamma_resolution`'s idx convention; an esports match with no `draw` side
/// collapses to `[teamA, teamB]`). Other `sport` leagues (CS2/VALORANT/DOTA2) and
/// the feed's `sport-award`/`crypto` kinds are dropped. Odds are the feed's
/// relayed YES prices; `lang` localizes the Draw label.
fn group_events(items: &[Item], lang: Lang) -> Vec<SourcedEvent> {
    let mut events: Vec<SourcedEvent> = Vec::new();
    for it in items {
        let Some(r) = it.next_round.as_ref() else { continue };
        let d = &it.market.display;
        let allowed = d.league.as_deref().is_some_and(|l| ALLOWED_LEAGUES.contains(&l));
        if d.kind.as_deref() != Some("sport") || !allowed {
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
    let (text, rows) = brief(lang_for_msg(&ctx, &message), tz, fmt, 0, false).await;
    crate::commands::tg::send_with_buttons(&ctx, chat_id, &text, &rows).await?;
    Ok(())
}

/// Build the market brief (text) for `page` plus a numbered button per shown
/// match (`bet:<key>`) and a prev/next navigation row. Kickoff times are shown in
/// `tz_min` (minutes east of UTC; 0 = UTC). `menu_mode` = the brief is embedded in
/// the `/start` menu, so a `[⬅ Back]` → home button is appended and the nav
/// callbacks carry the `m` flag. On any fetch/parse failure returns the localized
/// "unavailable" line (plus the back button in menu mode).
pub(crate) async fn brief(
    lang: Lang,
    tz_min: i64,
    fmt: OddsFormat,
    page: usize,
    menu_mode: bool,
) -> (String, Vec<Row>) {
    let now = Utc::now().timestamp();
    let markets = match fetch_markets(lang).await {
        Ok(mut m) => {
            m.retain(|x| within_window(x, now));
            m
        }
        Err(err) => {
            eprintln!("[markets] fetch error: {err}");
            return (
                i18n::markets_unavailable(lang).to_string(),
                with_back(Vec::new(), lang, menu_mode),
            );
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
        return (out, with_back(Vec::new(), lang, menu_mode));
    }

    // Clamp the requested page into range (the feed may have shrunk since the
    // button was rendered), then slice this page out.
    let total_pages = markets.len().div_ceil(PAGE_SIZE).max(1);
    let page = page.min(total_pages - 1);
    let start = page * PAGE_SIZE;
    let end = (start + PAGE_SIZE).min(markets.len());
    let shown = &markets[start..end];

    out.push('\n');
    out.push_str(i18n::markets_section(lang));
    out.push('\n');
    for (i, m) in shown.iter().enumerate() {
        out.push_str(&render_market(start + i + 1, m, tz_min, fmt));
    }
    if total_pages > 1 {
        out.push('\n');
        out.push_str(&i18n::markets_page(
            lang,
            &(page + 1).to_string(),
            &total_pages.to_string(),
        ));
        out.push('\n');
    }

    // Numbered bet buttons (absolute index across pages), four per row.
    let mut rows: Vec<Row> = shown
        .chunks(4)
        .enumerate()
        .map(|(row_idx, chunk)| {
            chunk
                .iter()
                .enumerate()
                .map(|(col, m)| {
                    let n = start + row_idx * 4 + col + 1;
                    (n.to_string(), format!("{BET}{}", m.key))
                })
                .collect()
        })
        .collect();

    // Prev / next navigation row (the mode flag rides in the callback so the
    // re-render keeps or drops the menu back button).
    let flag = if menu_mode { "m" } else { "s" };
    let mut nav: Row = Vec::new();
    if page > 0 {
        nav.push((
            i18n::markets_prev(lang).to_string(),
            format!("{PAGE}{flag}:{}", page - 1),
        ));
    }
    if page + 1 < total_pages {
        nav.push((
            i18n::markets_next(lang).to_string(),
            format!("{PAGE}{flag}:{}", page + 1),
        ));
    }
    if !nav.is_empty() {
        rows.push(nav);
    }

    (out, with_back(rows, lang, menu_mode))
}

/// Append the `[⬅ Back]` → home button when the brief is embedded in the
/// `/start` menu; a standalone `/events` brief gets no extra chrome.
fn with_back(mut rows: Vec<Row>, lang: Lang, menu_mode: bool) -> Vec<Row> {
    if menu_mode {
        rows.push(vec![(
            i18n::bet_btn_back(lang).to_string(),
            menu::MENU_HOME.to_string(),
        )]);
    }
    rows
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
/// index in the bot's stored `[teamA, draw?, teamB]` order (the event has
/// `n_outcomes` outcomes — 3 for a 1X2 sport match, 2 for a draw-less esports
/// match). `Ok(None)` when not yet resolved, the slug is empty, or there's no
/// clear winner (e.g. a void); `Err` on a feed/parse failure. The pure mapping
/// lives in [`GammaEvent::winning_idx`] (unit-tested for both Gamma shapes).
pub(crate) async fn gamma_resolution(
    slug: &str,
    n_outcomes: usize,
) -> Result<Option<i64>, reqwest::Error> {
    let ticker = slug.strip_prefix("sport-").unwrap_or(slug);
    if ticker.is_empty() || n_outcomes == 0 {
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
    Ok(event.winning_idx(n_outcomes))
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
    /// Competition tag (e.g. `FIFA_WC`, `LOL`); `/events` keeps only the
    /// `ALLOWED_LEAGUES`.
    league: Option<String>,
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

    /// This market's `outcomes` JSON array as owned strings (`[]` on parse fail).
    fn outcome_names(&self) -> Vec<String> {
        self.outcomes
            .as_deref()
            .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
            .unwrap_or_default()
    }

    /// True if this is a Yes/No market (a per-team moneyline leg or a prop), as
    /// opposed to a multi-outcome market like an esports "Match Winner".
    fn is_yes_no(&self) -> bool {
        self.outcome_names().iter().any(|o| o.eq_ignore_ascii_case("yes"))
    }

    /// The name of the outcome whose resolved price is ≥ `threshold` cents
    /// (price × 100) — the winner of a multi-outcome market (e.g. an esports
    /// "Match Winner" with team-name outcomes). `None` if unparsable / no winner.
    fn winning_outcome(&self, threshold_cents: f64) -> Option<String> {
        let outcomes: Vec<String> = serde_json::from_str(self.outcomes.as_deref()?).ok()?;
        let prices: Vec<String> = serde_json::from_str(self.outcome_prices.as_deref()?).ok()?;
        outcomes
            .iter()
            .zip(prices.iter())
            .find(|(_, p)| p.parse::<f64>().is_ok_and(|v| v * 100.0 >= threshold_cents))
            .map(|(o, _)| o.trim().to_string())
    }
}

/// A Gamma `groupItemTitle` / outcome name that denotes the match draw.
fn is_draw(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("draw")
}

/// The winning side of a resolved Gamma event.
enum WinSide {
    Draw,
    Team(String),
}

impl GammaEvent {
    /// Map this **resolved** event to the winning outcome's index in the bot's
    /// stored `[teamA, draw?, teamB]` order (length `n`). Locale-independent: it
    /// reads the winner from Gamma's English data and maps it positionally —
    /// `teamA` (named first in the title) → `0`, the draw → `1`, `teamB` → the
    /// last stored outcome `n − 1`. That single rule covers both the 3-way sport
    /// shape (`n = 3`) and the draw-less esports shape (`n = 2`). `None` = no clear
    /// winner (void / still settling / unrecognised structure).
    fn winning_idx(&self, n: usize) -> Option<i64> {
        let title = self.title.as_deref().unwrap_or("");
        match self.winning_side(title)? {
            WinSide::Draw => Some(1),
            WinSide::Team(name) => {
                let mut teams: Vec<(usize, String)> = self
                    .team_candidates(title)
                    .into_iter()
                    .filter_map(|g| title.find(&g).map(|p| (p, g)))
                    .collect();
                teams.sort_by_key(|(p, _)| *p);
                match teams.iter().position(|(_, g)| g.eq_ignore_ascii_case(&name)) {
                    Some(0) => Some(0),
                    Some(_) => Some(n as i64 - 1),
                    None => None,
                }
            }
        }
    }

    /// The winning side, read from whichever Gamma shape carries the moneyline:
    /// the per-team Yes/No markets (1X2 sport — one resolved to YES ≈ 1) or the
    /// single multi-outcome "Match Winner" market (esports — one team-name outcome
    /// priced ≈ 1). Prop markets (also Yes/No) are skipped by requiring the YES
    /// market's title to be the draw or a team named in the event title.
    fn winning_side(&self, title: &str) -> Option<WinSide> {
        let yesno = self
            .markets
            .iter()
            .filter(|m| m.yes_cents().is_some_and(|c| c >= 99.0))
            .map(|m| m.group_item_title.trim().to_string())
            .find(|g| is_draw(g) || title.contains(g.as_str()));
        if let Some(g) = yesno {
            return Some(if is_draw(&g) { WinSide::Draw } else { WinSide::Team(g) });
        }
        self.match_winner_market()
            .and_then(|m| m.winning_outcome(99.0))
            .map(WinSide::Team)
    }

    /// The non-draw team names of the moneyline, from whichever shape applies:
    /// the in-title per-team Yes/No market titles (sport), else the "Match Winner"
    /// market's outcomes (esports).
    fn team_candidates(&self, title: &str) -> Vec<String> {
        // All per-team Yes/No legs named in the title (price-independent, so the
        // losing teams stay in the list to fix the title ordering), minus draw.
        let per_team: Vec<String> = self
            .markets
            .iter()
            .filter(|m| m.is_yes_no())
            .map(|m| m.group_item_title.trim().to_string())
            .filter(|g| !is_draw(g) && title.contains(g.as_str()))
            .collect();
        if !per_team.is_empty() {
            return per_team;
        }
        self.match_winner_market()
            .map(|m| m.outcome_names())
            .unwrap_or_default()
    }

    /// The single "Match Winner" market (the esports moneyline), if present.
    fn match_winner_market(&self) -> Option<&GammaMarket> {
        self.markets
            .iter()
            .find(|m| m.group_item_title.trim().eq_ignore_ascii_case("match winner"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{"data":{"items":[
        {"market":{"id":"m1","slug":"sport-fifwc-fra-swe","display":{"kind":"sport","league":"FIFA_WC","teamA":{"name":"France"},"teamB":{"name":"Sweden"}}},
         "nextRound":{"startsAt":100,"endsAt":200,"phase":"open","sides":[
            {"key":"teamA","oddsCents":78.1,"trade":{"marketId":"0xA"}},
            {"key":"draw","oddsCents":16.8,"trade":{"marketId":"0xD"}},
            {"key":"teamB","oddsCents":8.4,"trade":{"marketId":"0xB"}}]}},
        {"market":{"id":"m1b","slug":"sport-lol-t1-geng","display":{"kind":"sport","league":"LOL","teamA":{"name":"T1"},"teamB":{"name":"Gen.G"}}},
         "nextRound":{"startsAt":100,"endsAt":200,"phase":"open","sides":[
            {"key":"teamA","oddsCents":55.0,"trade":{"marketId":"0xL1"}},
            {"key":"teamB","oddsCents":45.0,"trade":{"marketId":"0xL2"}}]}},
        {"market":{"id":"m1c","slug":"sport-cs2-navi-faze","display":{"kind":"sport","league":"CS2","teamA":{"name":"NAVI"},"teamB":{"name":"FaZe"}}},
         "nextRound":{"startsAt":100,"endsAt":200,"phase":"open","sides":[
            {"key":"teamA","oddsCents":50.0,"trade":{"marketId":"0xC1"}},
            {"key":"teamB","oddsCents":50.0,"trade":{"marketId":"0xC2"}}]}},
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
        let s = evs.iter().find(|e| e.key == "sport-fifwc-fra-swe").unwrap();
        assert_eq!(s.title, "France vs. Sweden");
        assert_eq!(s.outcomes.len(), 3);
        let names: Vec<&str> = s.outcomes.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["France", "Draw", "Sweden"]);
        assert_eq!(s.outcomes[0].yes_cents, Some(78.1));
        assert_eq!(s.outcomes[2].yes_cents, Some(8.4));
    }

    #[test]
    fn only_allowlisted_leagues_kept() {
        // The fixture has FIFA_WC + LOL (allowlisted) plus a CS2 esports match,
        // sport-award candidates, a prop, and crypto. Only the World Cup and LoL
        // matches survive grouping; CS2 and the other kinds are dropped.
        let evs = grouped();
        let keys: Vec<&str> = evs.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(evs.len(), 2, "World Cup + LoL kept, CS2/awards/crypto dropped");
        assert!(keys.contains(&"sport-fifwc-fra-swe"));
        assert!(keys.contains(&"sport-lol-t1-geng"));
        assert!(!keys.contains(&"sport-cs2-navi-faze"));
    }

    #[test]
    fn esports_match_with_no_draw_is_two_outcomes() {
        // A LoL match has only teamA/teamB sides (no draw), so it collapses to a
        // 2-outcome event rather than the 1X2 sport shape.
        let evs = grouped();
        let lol = evs.iter().find(|e| e.key == "sport-lol-t1-geng").unwrap();
        let names: Vec<&str> = lol.outcomes.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["T1", "Gen.G"]);
    }

    // --- Gamma resolution mapping (`GammaEvent::winning_idx`) ------------------

    fn gamma(json: &str) -> GammaEvent {
        serde_json::from_str(json).unwrap()
    }

    /// 1X2 sport: one Yes/No market per team + draw; the winner's YES resolves ~1.
    fn wc_event(fra: &str, draw: &str, swe: &str) -> GammaEvent {
        gamma(&format!(
            r#"{{"title":"France vs. Sweden","closed":true,"markets":[
                {{"groupItemTitle":"France","outcomes":"[\"Yes\",\"No\"]","outcomePrices":"[\"{fra}\",\"0\"]"}},
                {{"groupItemTitle":"Draw (France vs. Sweden)","outcomes":"[\"Yes\",\"No\"]","outcomePrices":"[\"{draw}\",\"0\"]"}},
                {{"groupItemTitle":"Sweden","outcomes":"[\"Yes\",\"No\"]","outcomePrices":"[\"{swe}\",\"0\"]"}}]}}"#
        ))
    }

    #[test]
    fn wc_maps_team_draw_team_to_0_1_2() {
        assert_eq!(wc_event("1", "0", "0").winning_idx(3), Some(0)); // France (teamA)
        assert_eq!(wc_event("0", "1", "0").winning_idx(3), Some(1)); // Draw (middle)
        assert_eq!(wc_event("0", "0", "1").winning_idx(3), Some(2)); // Sweden (teamB)
        assert_eq!(wc_event("0", "0", "0").winning_idx(3), None); // unresolved / void
    }

    /// Esports: a single multi-outcome "Match Winner" market (team-name outcomes),
    /// plus a resolved per-game market and a Yes/No prop that must NOT be mistaken
    /// for the moneyline.
    fn lol_event(t1: &str, tl: &str) -> GammaEvent {
        gamma(&format!(
            r#"{{"title":"LoL: T1 vs Team Liquid (BO5) - MSI","closed":true,"markets":[
                {{"groupItemTitle":"Game 1 Winner","outcomes":"[\"T1\",\"Team Liquid\"]","outcomePrices":"[\"1\",\"0\"]"}},
                {{"groupItemTitle":"Match Winner","outcomes":"[\"T1\",\"Team Liquid\"]","outcomePrices":"[\"{t1}\",\"{tl}\"]"}},
                {{"groupItemTitle":"First Blood in Game 2?","outcomes":"[\"Yes\",\"No\"]","outcomePrices":"[\"1\",\"0\"]"}}]}}"#
        ))
    }

    #[test]
    fn esports_maps_match_winner_to_0_or_last() {
        // teamB (Team Liquid) wins → the last stored outcome (idx 1 for n = 2).
        assert_eq!(lol_event("0", "1").winning_idx(2), Some(1));
        // teamA (T1) wins → idx 0. The resolved Yes/No prop is ignored.
        assert_eq!(lol_event("1", "0").winning_idx(2), Some(0));
        // No outcome resolved → no winner.
        assert_eq!(lol_event("0", "0").winning_idx(2), None);
    }
}
