use crate::commands::checkin::CHECKIN_REWARD;
use crate::commands::referral::REFERRAL_REWARD;
use crate::commands::util::*;
use crate::i18n;
use telexide::prelude::*;

#[command(description = "how to earn coins")]
pub async fn rule(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let lang = lang_for_msg(&ctx, &message);
    let text = i18n::rules_text(lang, &fmt_coins(CHECKIN_REWARD), &fmt_coins(REFERRAL_REWARD));
    reply(&ctx, &message, text).await?;
    Ok(())
}
