use crate::commands::util::*;
use crate::utils::split_question;
use rand::seq::SliceRandom;
use telexide::prelude::*;

#[command(description = "pair items from the two halves of a `?` expression")]
pub async fn pair(ctx: Context, message: Message) -> CommandResult {
    let body = text_of(&message);
    let after_cmd = body.split_once(char::is_whitespace).map_or("", |(_, rest)| rest);
    let Some((left, right)) = split_question(after_cmd) else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let src: Vec<String> = left.split_whitespace().map(String::from).collect();
    let tgt: Vec<String> = right.split_whitespace().map(String::from).collect();
    if src.is_empty() || tgt.is_empty() {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    }

    let mut pool = tgt.clone();
    while pool.len() < src.len() {
        pool.extend(tgt.iter().cloned());
    }
    {
        let mut rng = rand::thread_rng();
        pool.shuffle(&mut rng);
    }

    let lines: Vec<String> = src
        .iter()
        .zip(pool.iter())
        .map(|(s, t)| format!("{s} - {t}"))
        .collect();
    reply(&ctx, &message, lines.join("\n")).await?;
    Ok(())
}
