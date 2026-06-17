use crate::commands::util::*;
use crate::i18n::{self, Lang};
use telexide::prelude::*;

#[command(description = "show the caller's fruit inventory")]
pub async fn fruit(ctx: Context, message: Message) -> CommandResult {
    let Some(user) = message.from.as_ref() else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let lang = Lang::from_user(user);
    let info = db(&ctx).get_user_info(user.id)?;
    let body = if info.fruit.is_empty() {
        i18n::want_fruit(lang, &full_name(user))
    } else {
        i18n::fruit_store(lang, &full_name(user), &info.fruit)
    };
    reply(&ctx, &message, body).await?;
    Ok(())
}
