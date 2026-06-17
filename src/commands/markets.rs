use crate::commands::util::*;
use crate::i18n::{self, Lang};
use chrono::{TimeZone, Utc};
use serde::Deserialize;
use telexide::prelude::*;

/// waterx prediction-market browse endpoint. The bot only reads it; nothing
/// here touches the local water-coin ledger — `/markets` is purely a "what's
/// trading right now" brief in the style of the Jupiter market bot.
const BROWSE_URL: &str = "https://api.waterx.app/predict/browse";
/// Cap so the brief stays well under Telegram's 4096-char message limit; the
/// overflow is summarised with a "…and N more" tail.
const MAX_MATCHES: usize = 8;

#[command(description = "browse live prediction markets")]
pub async fn markets(ctx: Context, message: Message) -> CommandResult {
    let lang = lang_of(&message);

    let resp = match fetch(lang).await {
        Ok(r) => r,
        Err(err) => {
            eprintln!("[markets] fetch error: {err}");
            reply(&ctx, &message, i18n::markets_unavailable(lang)).await?;
            return Ok(());
        }
    };

    reply(&ctx, &message, render(lang, &resp.data.items)).await?;
    Ok(())
}

/// Pull the browse feed, asking the API for the caller's locale. Unsupported
/// locales fall back to English server-side, so any `Lang` is safe to send.
async fn fetch(lang: Lang) -> Result<BrowseResp, reqwest::Error> {
    reqwest::Client::new()
        .get(BROWSE_URL)
        .query(&[("locale", lang.menu_code()), ("limit", "200")])
        .header("user-agent", "waterx-bot/0.1")
        .send()
        .await?
        .error_for_status()?
        .json::<BrowseResp>()
        .await
}

fn render(lang: Lang, items: &[Item]) -> String {
    // Sport matches with both teams and an open round, soonest (and live) first.
    let mut matches: Vec<&Item> = items
        .iter()
        .filter(|i| {
            i.market.display.kind.as_deref() == Some("sport")
                && i.market.display.team_a.is_some()
                && i.market.display.team_b.is_some()
                && i.next_round.is_some()
        })
        .collect();
    matches.sort_by_key(|i| {
        let r = i.next_round.as_ref().unwrap();
        (r.phase.as_deref() != Some("live"), r.starts_at.unwrap_or(i64::MAX))
    });

    let mut out = format!(
        "{} — {}\n",
        i18n::markets_title(lang),
        Utc::now().format("%b %-d")
    );

    if matches.is_empty() {
        out.push('\n');
        out.push_str(i18n::markets_empty(lang));
        return out;
    }

    out.push('\n');
    out.push_str(i18n::markets_matches(lang));
    out.push('\n');
    for (idx, it) in matches.iter().take(MAX_MATCHES).enumerate() {
        out.push_str(&render_match(lang, idx + 1, it));
    }
    push_more(&mut out, lang, matches.len(), MAX_MATCHES);

    out
}

fn push_more(out: &mut String, lang: Lang, total: usize, shown: usize) {
    if total > shown {
        out.push_str(&i18n::markets_more(lang, &(total - shown).to_string()));
        out.push('\n');
    }
}

fn render_match(lang: Lang, n: usize, it: &Item) -> String {
    let d = &it.market.display;
    let r = it.next_round.as_ref().unwrap();
    let a = d.team_a.as_ref().unwrap();
    let b = d.team_b.as_ref().unwrap();
    let dot = if r.phase.as_deref() == Some("live") { "🔴 " } else { "" };

    let mut s = format!("\n{n}) {dot}{} vs. {}\n", a.name, b.name);
    if let Some(t) = fmt_time(r.starts_at) {
        s.push_str(&format!("├ {t}\n"));
    }
    // Home / draw / away — the order the source feed and the brief expect.
    let rows: [(&str, Option<f64>); 3] = [
        (&a.name, odds(&r.sides, "teamA")),
        (i18n::draw_label(lang), odds(&r.sides, "draw")),
        (&b.name, odds(&r.sides, "teamB")),
    ];
    for (i, (label, yes)) in rows.iter().enumerate() {
        let branch = if i == rows.len() - 1 { "└" } else { "├" };
        match yes {
            // Cents are the implied probability ×100, so decimal odds are
            // 1 / (cents/100) = 100/cents (e.g. 65¢ → 1.54).
            Some(y) if *y > 0.0 => s.push_str(&format!("{branch} {label} — {:.2}\n", 100.0 / *y)),
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
// Browse-endpoint response — only the fields the brief reads are modelled;
// serde ignores everything else.
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
