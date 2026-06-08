use crate::commands::util::*;
use crate::utils::{determine, split_question};
use telexide::prelude::*;

#[command(description = "answer a question by picking the best-matching option")]
pub async fn tell(ctx: Context, message: Message) -> CommandResult {
    let body = text_of(&message);
    let after_cmd = body.splitn(2, char::is_whitespace).nth(1).unwrap_or("");
    let Some((question, opts_str)) = split_question(after_cmd) else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let opts: Vec<String> = opts_str.split_whitespace().map(String::from).collect();
    match determine(&opts, &question) {
        Some(pick) => reply(&ctx, &message, pick).await?,
        None => reply(&ctx, &message, ERR_REPLY).await?,
    }
    Ok(())
}
