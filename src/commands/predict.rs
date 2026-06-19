use crate::commands::tg;
use crate::commands::util::*;
use crate::game::BetGame;
use crate::i18n;
use telexide::prelude::*;

#[command(description = "open a prediction game or show open ones")]
pub async fn predict(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let Some(host) = message.from.clone() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let lang = lang_for(&ctx, &host);
    // Raw argument text (everything after the command word), so the question can
    // contain spaces.
    let after = text_of(&message)
        .split_once(char::is_whitespace)
        .map_or("", |(_, rest)| rest.trim());

    // No args → list the caller's open games, or the usage hint if they have none.
    if after.is_empty() {
        let chunks: Vec<String> = {
            let games_arc = games(&ctx);
            let snapshot = games_arc.lock().await;
            snapshot
                .values()
                .map(|g| g.check(host.id))
                .filter(|e| !e.is_empty())
                .collect()
        };
        let body = if chunks.is_empty() {
            i18n::usage_predict(lang).to_string()
        } else {
            chunks.join("\n")
        };
        reply(&ctx, &message, body).await?;
        return Ok(());
    }

    // Parse "<question>? <opt1> <opt2> …": the question runs up to the first `?`
    // or full-width `？` (the mark is kept in the title), then space-separated
    // options.
    let Some(pos) = after.find(['?', '？']) else {
        reply(&ctx, &message, i18n::usage_predict(lang)).await?;
        return Ok(());
    };
    let mark_len = after[pos..].chars().next().map_or(1, char::len_utf8);
    let description = after[..pos + mark_len].trim().to_string();
    let option_strs: Vec<&str> = after[pos + mark_len..].split_whitespace().collect();
    if description.is_empty() || option_strs.len() < 2 {
        reply(&ctx, &message, i18n::usage_predict(lang)).await?;
        return Ok(());
    }

    let mut game = BetGame::new(host.id, lang, &description, &option_strs);

    let rows = game.get_buttons();
    let sent =
        tg::send_with_buttons(&ctx, message.chat.get_id(), &game.get_text(), &rows).await?;
    game.set_id(sent.chat.get_id(), sent.message_id);
    let key = format!("{}:{}", sent.chat.get_id(), sent.message_id);
    // Re-edit so the description shows the freshly-assigned id tail. Best-effort:
    // a failed cosmetic edit must NOT abort the command before the game is
    // registered below (that would orphan the posted board with live-but-unknown
    // buttons). `edit_with_buttons` already logs logical rejects.
    let new_rows = game.get_buttons();
    let _ = tg::edit_with_buttons(
        &ctx,
        sent.chat.get_id(),
        sent.message_id,
        &game.get_text(),
        &new_rows,
    )
    .await;

    if let Err(err) = db(&ctx).save_bet_game(&game) {
        eprintln!("save_bet_game error (continuing in-memory only): {err}");
    }
    let games_arc = games(&ctx);
    games_arc.lock().await.insert(key, game);
    Ok(())
}
