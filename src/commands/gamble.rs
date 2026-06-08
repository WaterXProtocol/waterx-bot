use crate::commands::tg;
use crate::commands::util::*;
use crate::game::BetGame;
use telexide::prelude::*;

#[command(description = "open a betting game (host) or show open ones")]
pub async fn gamble(ctx: Context, message: Message) -> CommandResult {
    let Some(host) = message.from.clone() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let parts = args(&message);

    if parts.is_empty() {
        let games_arc = games(&ctx);
        let snapshot = games_arc.lock().await;
        let mut chunks: Vec<String> = Vec::new();
        for game in snapshot.values() {
            let entry = game.check(host.id);
            if !entry.is_empty() {
                chunks.push(entry);
            }
        }
        if chunks.is_empty() {
            reply(&ctx, &message, "沒有賭博記錄🤗").await?;
        } else {
            reply(&ctx, &message, chunks.join("\n")).await?;
        }
        return Ok(());
    }

    if parts.len() < 3 {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    }

    let description = parts[0].clone();
    let option_strs: Vec<&str> = parts[1..].iter().map(String::as_str).collect();
    let mut game = BetGame::new(host.id, &description, &option_strs);

    let rows = game.get_buttons();
    let sent =
        tg::send_with_buttons(&ctx, message.chat.get_id(), &game.get_text(), &rows).await?;
    game.set_id(sent.chat.get_id(), sent.message_id);
    let key = format!("{}:{}", sent.chat.get_id(), sent.message_id);
    // Re-edit so the description shows the freshly-assigned id tail.
    let new_rows = game.get_buttons();
    tg::edit_with_buttons(
        &ctx,
        sent.chat.get_id(),
        sent.message_id,
        &game.get_text(),
        &new_rows,
    )
    .await?;

    let games_arc = games(&ctx);
    games_arc.lock().await.insert(key, game);
    Ok(())
}
