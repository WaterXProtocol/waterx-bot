use crate::commands::util::*;
use telexide::prelude::*;

#[command(description = "send your entire balance to the replied-to user")]
pub async fn allin(ctx: Context, message: Message) -> CommandResult {
    let Some(sender) = message.from.clone() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let Some(receiver) = message
        .reply_to_message
        .as_ref()
        .and_then(|r| r.from.clone())
    else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let database = db(&ctx);
    let info = database.get_user_info(sender.id)?;
    if info.balance <= 0 {
        reply(&ctx, &message, "錢不夠喔😶").await?;
        return Ok(());
    }
    if !database.balance_change(sender.id, -info.balance)? {
        reply(&ctx, &message, "錢不夠耶😶").await?;
        return Ok(());
    }
    database.force_change(receiver.id, info.balance)?;
    reply(
        &ctx,
        &message,
        format!(
            "{} 歐印 {}\n{} 顆 水幣",
            full_name(&sender),
            full_name(&receiver),
            format_number(info.balance),
        ),
    )
    .await?;
    Ok(())
}
