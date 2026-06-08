use crate::commands::util::*;
use telexide::prelude::*;

#[command(description = "show the caller's island-coin balance")]
pub async fn balance(ctx: Context, message: Message) -> CommandResult {
    let Some(user) = message.from.as_ref() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let info = db(&ctx).get_user_info(user.id)?;
    let body = if info.balance >= 0 {
        format!(
            "{}\n擁有 {} 顆 水幣",
            full_name(user),
            format_number(info.balance)
        )
    } else {
        format!(
            "{}\n欠債 {} 顆 水幣",
            full_name(user),
            format_number(info.balance)
        )
    };
    reply(&ctx, &message, body).await?;
    Ok(())
}
