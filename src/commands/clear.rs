use crate::commands::status::build_status;
use crate::commands::util::*;
use crate::types::BetState;
use telexide::prelude::*;

#[command(description = "owner-only: drop all draw (流局) bet games and DM the new status")]
pub async fn clear(ctx: Context, message: Message) -> CommandResult {
    let Some(uid) = from_id(&message) else {
        return Ok(());
    };
    if !is_owner(&ctx, uid) {
        return Ok(());
    }
    let owner_id = config(&ctx).owner;
    let games_arc = games(&ctx);
    let database = db(&ctx);
    let (snapshot, dropped_ids) = {
        let mut g = games_arc.lock().await;
        let dropped: Vec<String> = g
            .iter()
            .filter(|(_, game)| game.state == BetState::draw)
            .map(|(k, _)| k.clone())
            .collect();
        g.retain(|_, game| game.state != BetState::draw);
        (g.clone(), dropped)
    };
    for id in &dropped_ids {
        if let Err(err) = database.delete_bet_game(id) {
            eprintln!("delete_bet_game({id}) error: {err}");
        }
    }
    send_text(&ctx, owner_id, build_status(&snapshot)).await?;
    Ok(())
}
