//! Referral helpers shared by the two entry points: the `/start` deep link
//! (`t.me/<bot>?start=<referrer_id>`) and the group bind (a brand-new user's
//! first interaction — any command or button tap — in a group the referrer
//! added the bot to).

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

/// In a group, bind the acting `user` to whoever added the bot (`chats.added_by`)
/// as their referrer — but only when `user` is **brand-new** (no `balance` row
/// yet, so `set_referrer_if_new`'s `INSERT OR IGNORE` actually inserts). Fires on
/// **any** interaction the bot sees in a group: button taps (`callbacks::on_callback`)
/// and text commands (`util::paused_block`). Both call sites run before the user's
/// row is created, preserving the brand-new check. No-op in private chats, for
/// existing users, when the adder is unknown (`added_by = 0`), or when the user is
/// the adder (`set_referrer_if_new` rejects `referrer == referee`). Pays both
/// sides once on a successful bind.
pub(crate) async fn maybe_bind_group(ctx: &Context, chat_id: i64, user: &User) {
    if !is_group_chat(chat_id) {
        return;
    }
    let database = db(ctx);
    // Fast path: only brand-new users can bind, so skip all work (and the
    // per-command write below) for anyone who already has a row — the common
    // case now that this runs on every group interaction. On a read error,
    // assume they exist (don't bind) — fail safe.
    if database.user_exists(user.id).unwrap_or(true) {
        return;
    }
    if let Ok(Some(adder)) = database.group_adder(chat_id) {
        database.force_change(adder, 0).ok(); // ensure the adder has a row to refer from
        if database.set_referrer_if_new(user.id, adder).unwrap_or(false) {
            pay_referral(ctx, adder, user).await;
        }
    }
}
