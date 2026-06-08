use crate::commands::util::*;
use telexide::prelude::*;

#[command(description = "show fruit stains on the target's clothes")]
pub async fn cloth(ctx: Context, message: Message) -> CommandResult {
    let target = message
        .reply_to_message
        .as_ref()
        .and_then(|r| r.from.clone())
        .or_else(|| message.from.clone());
    let Some(target) = target else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let info = db(&ctx).get_user_info(target.id)?;
    let body = if info.cloth.is_empty() {
        format!("{}\n品行優良😌", full_name(&target))
    } else {
        format!("{}\n衣服上的水果:\n{}", full_name(&target), info.cloth)
    };
    reply(&ctx, &message, body).await?;
    Ok(())
}
