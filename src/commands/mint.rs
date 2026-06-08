use crate::commands::util::*;
use telexide::prelude::*;

#[command(description = "owner+dev only: grant balance or fruit to the owner")]
pub async fn mint(ctx: Context, message: Message) -> CommandResult {
    let cfg = config(&ctx);
    let Some(uid) = from_id(&message) else {
        return Ok(());
    };
    if !cfg.dev || cfg.owner != uid {
        return Ok(());
    }
    let database = db(&ctx);
    for token in args(&message) {
        if let Ok(amount) = token.parse::<i64>() {
            database.force_change(cfg.owner, amount)?;
        } else {
            database.fruit_change(cfg.owner, &token, true)?;
        }
    }
    let info = database.get_user_info(cfg.owner)?;
    reply(
        &ctx,
        &message,
        format!(
            "balance = {}\nfruit = {}",
            format_number(info.balance),
            info.fruit
        ),
    )
    .await?;
    Ok(())
}
