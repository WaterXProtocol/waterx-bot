use crate::commands::util::*;
use crate::i18n;
use telexide::prelude::*;

/// Daily check-in reward, in water-coins. The claim window resets at 00:00 UTC
/// (see `Database::try_checkin`).
pub(crate) const CHECKIN_REWARD: i64 = 10;

/// Compact countdown to the next reset (next 00:00 UTC), e.g. `"5h 23m"`.
/// Shown instead of an absolute clock time because Telegram doesn't expose the
/// user's timezone — a duration is correct from anywhere.
pub(crate) fn time_until_reset() -> String {
    let secs = 86_400 - chrono::Utc::now().timestamp().rem_euclid(86_400);
    let (h, m) = (secs / 3600, (secs % 3600) / 60);
    if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

#[command(description = "claim your daily 10 water-coins")]
pub async fn checkin(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    let Some(uid) = from_id(&message) else {
        reply(&ctx, &message, ERR_REPLY).await?;
        return Ok(());
    };
    let lang = lang_for_msg(&ctx, &message);
    if db(&ctx).try_checkin(uid, CHECKIN_REWARD)? {
        let msg = i18n::checkin_done(lang, &format_number(CHECKIN_REWARD));
        reply(&ctx, &message, msg).await?;
    } else {
        reply(&ctx, &message, i18n::checkin_already(lang, &time_until_reset())).await?;
    }
    Ok(())
}
