use crate::commands::util::*;
use telexide::prelude::*;

#[command(description = "owner-only: reverse a settled bet game")]
pub async fn reverse(ctx: Context, message: Message) -> CommandResult {
    let Some(uid) = from_id(&message) else {
        return Ok(());
    };
    if !is_owner(&ctx, uid) {
        return Ok(());
    }
    let parts = args(&message);
    let Some(gid) = parts.first().cloned() else {
        reply(&ctx, &message, "用法: /reverse <chat_id:msg_id>").await?;
        return Ok(());
    };

    let games_arc = games(&ctx);
    let mut g = games_arc.lock().await;
    let Some(game) = g.remove(&gid) else {
        reply(&ctx, &message, "no such game").await?;
        return Ok(());
    };
    let Some(reverse_map) = game.reverse() else {
        // re-insert if state was wrong
        g.insert(gid.clone(), game);
        reply(&ctx, &message, "state error").await?;
        return Ok(());
    };
    drop(g);

    let database = db(&ctx);
    let mut lines = vec![format!("{gid}\n時光倒流")];
    for (user, delta) in &reverse_map {
        database.force_change(*user, *delta)?;
        let s = format!("{user:0>4}");
        let tail = &s[s.len().saturating_sub(4)..];
        let verb = if *delta > 0 { "收回" } else { "繳回" };
        lines.push(format!("***{tail} {verb}{}顆 水幣", delta.abs()));
    }
    reply(&ctx, &message, lines.join("\n")).await?;
    Ok(())
}
