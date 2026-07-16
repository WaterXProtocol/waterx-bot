use crate::commands::markets;
use crate::commands::util::*;
use crate::core::types::LeagueFilter;
use crate::database::Database;
use telexide::prelude::*;

/// Usage footer shown on the listing / bad input (owner-only, plain English).
const USAGE: &str = "\nCommands:\n\
     • /leagues add <type> [league] [tournament…]\n\
     • /leagues remove <n>\n\
     • /leagues reset   (built-in defaults)\n\
     • /leagues clear   (surface nothing)\n\
     type = sport | esports | crypto | … ; league = fifa_wc | lol | …\n\
     e.g. /leagues add esports lol Esports World Cup";

/// `/leagues` — owner-only runtime config for which competitions `/events` shows.
/// Each entry is a [`LeagueFilter`] (`type` + optional `league` + optional
/// tournament markers) fetched as its own narrowed browse stream. Persisted in the
/// `meta` table and mirrored into the live fetch path via `markets::set_active_leagues`.
/// Plain English (an operator surface, no i18n), silent for non-owners.
#[command(description = "owner: configure /events leagues")]
pub async fn leagues(ctx: Context, message: Message) -> CommandResult {
    let Some(user) = message.from.clone() else {
        return Ok(());
    };
    if !is_owner(&ctx, user.id) {
        return Ok(()); // silently ignored for non-owners, like other admin commands
    }
    let db = db(&ctx);
    let a = args(&message);
    match a.first().map(|s| s.to_ascii_lowercase()).as_deref() {
        None | Some("list") => {
            reply(&ctx, &message, list_text(&db)).await?;
        }
        Some("add") => {
            let Some(api_type) = a.get(1).map(|s| s.to_string()) else {
                reply(
                    &ctx,
                    &message,
                    format!("usage: /leagues add <type> [league] [tournament…]{USAGE}"),
                )
                .await?;
                return Ok(());
            };
            let league = a.get(2).map(|s| s.trim_matches('"').to_string());
            let tournament = (a.len() > 3).then(|| a[3..].join(" ").trim_matches('"').to_string());
            add_filter(&ctx, &message, &db, api_type, league, tournament).await?;
        }
        Some("remove" | "rm" | "del") => match a.get(1).and_then(|s| s.parse::<usize>().ok()) {
            Some(n) => remove_filter(&ctx, &message, &db, n).await?,
            None => {
                reply(
                    &ctx,
                    &message,
                    "usage: /leagues remove <n>  (n from /leagues list)",
                )
                .await?;
            }
        },
        Some("reset") => match db.clear_allowed_leagues() {
            Ok(()) => {
                markets::set_active_leagues(LeagueFilter::defaults());
                reply(
                    &ctx,
                    &message,
                    format!("Reset to built-in defaults.\n\n{}", list_text(&db)),
                )
                .await?;
            }
            Err(e) => {
                alert_owner(&ctx, &format!("[leagues] reset failed: {e}")).await;
                reply(&ctx, &message, "DB error resetting leagues.").await?;
            }
        },
        Some("clear") => match db.set_allowed_leagues(&[]) {
            Ok(()) => {
                markets::set_active_leagues(vec![]);
                reply(
                    &ctx,
                    &message,
                    "Cleared — /events now surfaces nothing until you add a league (or /leagues reset).",
                )
                .await?;
            }
            Err(e) => {
                alert_owner(&ctx, &format!("[leagues] clear failed: {e}")).await;
                reply(&ctx, &message, "DB error clearing leagues.").await?;
            }
        },
        Some(other) => {
            reply(&ctx, &message, format!("unknown subcommand '{other}'.{USAGE}")).await?;
        }
    }
    Ok(())
}

/// The effective allowlist: the owner's stored list, or the built-in defaults when
/// unset (or on a DB read error — never blanks the surface).
fn current(db: &Database) -> Vec<LeagueFilter> {
    match db.get_allowed_leagues() {
        Ok(Some(v)) => v,
        _ => LeagueFilter::defaults(),
    }
}

/// The numbered listing + usage footer (owner-only, plain English).
fn list_text(db: &Database) -> String {
    let list = current(db);
    let mut s = String::from("Leagues surfaced by /events:\n");
    if list.is_empty() {
        s.push_str("  (none — /events shows nothing)\n");
    } else {
        for (i, f) in list.iter().enumerate() {
            s.push_str(&format!("  {}. {}\n", i + 1, f.label()));
        }
    }
    s.push_str(USAGE);
    s
}

/// Validate a new/merged filter with a live test-fetch, then persist it. An
/// existing `(type, league)` entry gains the tournament marker; otherwise a new
/// entry is pushed. An invalid `type`/`league` (or a feed outage) fails the probe
/// and nothing is stored.
async fn add_filter(
    ctx: &Context,
    message: &Message,
    db: &Database,
    api_type: String,
    league: Option<String>,
    tournament: Option<String>,
) -> CommandResult {
    let mut list = current(db);
    let pos = list
        .iter()
        .position(|f| f.api_type == api_type && f.league == league);
    let mut candidate = match pos {
        Some(i) => list[i].clone(),
        None => LeagueFilter {
            api_type: api_type.clone(),
            league: league.clone(),
            tournaments: vec![],
        },
    };
    if let Some(t) = &tournament {
        if !candidate.tournaments.iter().any(|x| x.eq_ignore_ascii_case(t)) {
            candidate.tournaments.push(t.clone());
        }
    }

    // Live test-fetch: an invalid type/league can't be fetched (the feed rejects
    // it), so a probe error means "don't store it".
    let events = match markets::probe_filter(&candidate).await {
        Ok(ev) => ev,
        Err(e) => {
            let which = league.as_deref().map(|l| format!("/{l}")).unwrap_or_default();
            reply(ctx, message, format!("Couldn't fetch `{api_type}{which}` — check the type/league values (the feed rejects invalid ones), or retry if it's down. Not added.\n({e})")).await?;
            return Ok(());
        }
    };

    match pos {
        Some(i) => list[i] = candidate.clone(),
        None => list.push(candidate.clone()),
    }
    if let Err(e) = db.set_allowed_leagues(&list) {
        alert_owner(ctx, &format!("[leagues] save failed: {e}")).await;
        reply(ctx, message, "DB error saving. Not added.").await?;
        return Ok(());
    }
    markets::set_active_leagues(list);
    reply(
        ctx,
        message,
        format!(
            "Added: {}\n({} match(es) surfaced right now)\n\n{}",
            candidate.label(),
            events.len(),
            list_text(db)
        ),
    )
    .await?;
    Ok(())
}

/// Drop entry `n` (1-based, as shown by `/leagues list`) and persist.
async fn remove_filter(ctx: &Context, message: &Message, db: &Database, n: usize) -> CommandResult {
    let mut list = current(db);
    if n == 0 || n > list.len() {
        reply(ctx, message, format!("no entry #{n}. See /leagues list.")).await?;
        return Ok(());
    }
    let removed = list.remove(n - 1);
    if let Err(e) = db.set_allowed_leagues(&list) {
        alert_owner(ctx, &format!("[leagues] save failed: {e}")).await;
        reply(ctx, message, "DB error saving. Not removed.").await?;
        return Ok(());
    }
    markets::set_active_leagues(list);
    reply(
        ctx,
        message,
        format!("Removed: {}\n\n{}", removed.label(), list_text(db)),
    )
    .await?;
    Ok(())
}
