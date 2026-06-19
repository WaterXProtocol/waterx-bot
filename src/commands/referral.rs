//! Referral helpers shared by the two entry points: the `/start` deep link
//! (`t.me/<bot>?start=<referrer_id>`) and the group check-in bind (a new user
//! tapping check-in in a group the referrer added the bot to).

use crate::commands::util::*;
use crate::i18n::{self, Lang};
use telexide::model::User;
use telexide::prelude::Context;

/// Micro-coins paid to **both** the referrer and the new referee when a
/// referral is newly recorded (10 coins each).
pub(crate) const REFERRAL_REWARD: i64 = 10 * crate::database::COIN;

/// Credit both sides of a freshly-recorded referral and DM the referrer. The
/// caller must have already confirmed the binding is new (e.g. via
/// `Database::set_referrer_if_new` returning `true`) so this pays out once.
pub(crate) async fn pay_referral(ctx: &Context, referrer: i64, referee: &User) {
    let database = db(ctx);
    // Pay both sides atomically (both or neither). Log a failure instead of
    // silently swallowing it — the binding is already committed, so a lost
    // payout must at least leave a trace.
    if let Err(e) = database.reward_referral(referrer, referee.id, REFERRAL_REWARD) {
        eprintln!("pay_referral credit failed (referrer {referrer}, referee {}): {e}", referee.id);
        return;
    }
    let rlang = database.get_lang(referrer).ok().flatten().unwrap_or(Lang::En);
    let _ = send_text(
        ctx,
        referrer,
        i18n::referral_bonus(rlang, &full_name(referee), &fmt_coins(REFERRAL_REWARD)),
    )
    .await;
}
