use crate::commands::util::*;
use std::collections::HashMap;
use telexide::prelude::*;

#[command(description = "owner-only: print active bet games to the owner's DM")]
pub async fn status(ctx: Context, message: Message) -> CommandResult {
    let Some(uid) = from_id(&message) else {
        return Ok(());
    };
    if !is_owner(&ctx, uid) {
        return Ok(());
    }
    let owner_id = config(&ctx).owner;
    let games_arc = games(&ctx);
    let snapshot = games_arc.lock().await.clone();
    let txt = build_status(&snapshot);
    drop(snapshot); // not strictly necessary, but explicit
    send_text(&ctx, owner_id, txt).await?;
    Ok(())
}

/// Used by both /status and /clear to render the bet-games summary.
pub fn build_status(games: &HashMap<String, crate::game::BetGame>) -> String {
    let mut txt = format!("bet_games: {}", games.len());
    for (gid, game) in games {
        txt.push_str(&format!("\n {gid} {}", game.state.as_str()));
    }
    txt
}
