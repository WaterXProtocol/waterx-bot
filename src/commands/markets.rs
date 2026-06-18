use crate::commands::tg::Row;
use crate::commands::util::*;
use crate::i18n::{self, Lang};
use chrono::{TimeZone, Utc};
use serde::Deserialize;
use telexide::prelude::*;

/// waterx prediction-market browse endpoint.
const BROWSE_URL: &str = "https://api.waterx.app/predict/browse";
/// Cap so the brief stays well under Telegram's 4096-char message limit; the
/// overflow is summarised with a "…and N more" tail.
const MAX_MATCHES: usize = 8;

/// Callback-data prefix: tapping a match number opens the bet flow.
pub const BET: &str = "bet:";

/// A single sport match, distilled from the browse feed.
#[derive(Debug, Clone)]
pub struct MatchInfo {
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

impl MatchInfo {
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

#[command(description = "browse live prediction markets")]
pub async fn markets(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let (text, rows) = brief(lang_for_msg(&ctx, &message)).await;
    crate::commands::tg::send_with_buttons(&ctx, message.chat.get_id(), &text, &rows).await?;
    Ok(())
}

/// Build the match brief (text) plus a numbered button per shown match
/// (`bet:<market_id>`). On any fetch/parse failure returns the localized
/// "unavailable" line and no buttons.
pub(crate) async fn brief(lang: Lang) -> (String, Vec<Row>) {
    let now = Utc::now().timestamp();
    let matches = match fetch_matches(lang).await {
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
        Utc::now().format("%b %-d")
    );
    if matches.is_empty() {
        out.push('\n');
        out.push_str(i18n::markets_empty(lang));
        return (out, Vec::new());
    }

    out.push('\n');
    out.push_str(i18n::markets_matches(lang));
    out.push('\n');
    let shown = &matches[..matches.len().min(MAX_MATCHES)];
    for (idx, m) in shown.iter().enumerate() {
        out.push_str(&render_match(lang, idx + 1, m));
    }
    if matches.len() > MAX_MATCHES {
        out.push_str(&i18n::markets_more(lang, &(matches.len() - MAX_MATCHES).to_string()));
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
pub(crate) async fn fetch_one(lang: Lang, market_id: &str) -> Option<MatchInfo> {
    fetch_matches(lang)
        .await
        .ok()?
        .into_iter()
        .find(|m| m.market_id == market_id)
}

/// True when a match should appear in the brief: live, or kicking off within 24h.
fn within_window(m: &MatchInfo, now: i64) -> bool {
    m.live || m.starts_at.is_some_and(|t| t >= now && t <= now + 86_400)
}

/// Pull and parse the feed into sport matches, live-first then soonest.
async fn fetch_matches(lang: Lang) -> Result<Vec<MatchInfo>, reqwest::Error> {
    // Chinese users get Chinese team names; everyone else English.
    let api_locale = match lang {
        Lang::Hant | Lang::Hans => "zh",
        _ => "en",
    };
    let resp = reqwest::Client::new()
        .get(BROWSE_URL)
        .query(&[("locale", api_locale), ("limit", "200")])
        .header("user-agent", "waterx-bot/0.1")
        .send()
        .await?
        .error_for_status()?
        .json::<BrowseResp>()
        .await?;

    let mut matches: Vec<MatchInfo> = resp.data.items.iter().filter_map(to_match_info).collect();
    matches.sort_by_key(|m| (!m.live, m.starts_at.unwrap_or(i64::MAX)));
    Ok(matches)
}

fn to_match_info(it: &Item) -> Option<MatchInfo> {
    let d = &it.market.display;
    if d.kind.as_deref() != Some("sport") {
        return None;
    }
    let a = d.team_a.as_ref()?;
    let b = d.team_b.as_ref()?;
    let r = it.next_round.as_ref()?;
    Some(MatchInfo {
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

fn render_match(lang: Lang, n: usize, m: &MatchInfo) -> String {
    let dot = if m.live { "🔴 " } else { "" };
    let mut s = format!("\n{n}) {dot}{} vs. {}\n", m.team_a, m.team_b);
    if let Some(t) = fmt_time(m.starts_at) {
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
            // Decimal odds = 100/cents (e.g. 65¢ → 1.54).
            Some(y) if *y > 0.0 => {
                s.push_str(&format!("{branch} {label} — {:.2}\n", decimal_odds(*y)));
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

/// `startsAt` (unix seconds) → `"Jun 27 · 17:00 UTC"`.
fn fmt_time(ts: Option<i64>) -> Option<String> {
    let dt = Utc.timestamp_opt(ts?, 0).single()?;
    Some(dt.format("%b %-d · %H:%M UTC").to_string())
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
