use crate::commands::util::*;
use crate::utils::{determine_all, split_question};
use telexide::prelude::*;

#[command(description = "answer a question by ranking every option")]
pub async fn tells(ctx: Context, message: Message) -> CommandResult {
    let body = text_of(&message);
    let after_cmd = body.split_once(char::is_whitespace).map_or("", |(_, rest)| rest);
    let Some((question, opts_str)) = split_question(after_cmd) else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let opts: Vec<String> = opts_str.split_whitespace().map(String::from).collect();
    let ranked = determine_all(&opts, &question);
    if ranked.is_empty() {
        reply(&ctx, &message, ERR_REPLY).await?;
    } else {
        reply(&ctx, &message, ranked.join("\n")).await?;
    }
    Ok(())
}
