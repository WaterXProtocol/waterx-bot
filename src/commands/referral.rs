//! Referral helpers shared by the two entry points: the `/start` deep link
//! (`t.me/<bot>?start=<referrer_id>`) and the group bind (a referrer-less user's
//! interaction — any command or button tap — in a group the referrer added the
//! bot to). Both bind a user who has **no referrer yet**: a brand-new row, or an
//! existing one whose `referrer` is still empty (the friendly late-bind).

use crate::commands::tg;
use crate::commands::util::*;
use crate::core::i18n::{self, Lang};
use telexide::model::User;
use telexide::prelude::Context;

/// Micro-coins paid to **both** the referrer and the new referee when a
/// referral is newly recorded (10 coins each).
pub(crate) const REFERRAL_REWARD: i64 = 10 * crate::database::COIN;

/// Credit both sides of a freshly-recorded referral and DM the referrer. The
/// caller must have already confirmed the binding is new (e.g. via
/// `Database::set_referrer_if_unset` returning `true`) so this pays out once.
pub(crate) async fn pay_referral(ctx: &Context, referrer: i64, referee: &User) {
    let database = db(ctx);
    // Pay both sides atomically (both or neither). Log a failure instead of
    // silently swallowing it — the binding is already committed, so a lost
    // payout must at least leave a trace.
    if let Err(e) = database.reward_referral(referrer, referee.id, REFERRAL_REWARD) {
        alert_owner(
            ctx,
            &format!(
                "pay_referral credit failed (referrer {referrer}, referee {}): {e}",
                referee.id
            ),
        )
        .await;
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

/// In a group, bind the acting `user` to an **adder** **and** the group **owner**
/// (Telegram creator) as **co-referrers** — but only when `user` has **no referrer
/// yet** (a brand-new row, or an existing one whose `referrer` is still empty: the
/// friendly late-bind for members who were already around). The adder is whoever
/// actually added *this member* (`group_adds`, recorded from a "new member" service
/// message) if known, else whoever added the **bot** (`chats.added_by`) as the
/// fallback. Fires on **any** interaction the bot sees in a group: button taps
/// (`callbacks::on_callback`) and text commands (`util::paused_block`). No-op in
/// private chats, for already-referred users, or when no adder is known. When the
/// owner is distinct from the adder, the signup bonus is split 50/50 and the
/// check-in level-1 reward will be too (see `Database::try_checkin`); otherwise the
/// adder is the sole referrer.
pub(crate) async fn maybe_bind_group(ctx: &Context, chat_id: i64, user: &User) {
    if !is_group_chat(chat_id) {
        return;
    }
    let database = db(ctx);
    // Fast path: only referrer-less users can bind, so skip all work (and the
    // per-command write below) for anyone already referred — the common case now
    // that this runs on every group interaction. On a read error, assume they're
    // referred (don't bind) — fail safe. `bind_group_referral`'s atomic
    // `0 → set` guard is the real once-only backstop; this just avoids the work.
    if database.has_referrer(user.id).unwrap_or(true) {
        return;
    }
    // Prefer whoever actually added *this member* (a recorded "new member" service
    // message) over whoever added the bot to the group; either becomes the
    // referrer, with the group owner as the 0.5 co-referrer.
    let adder = database
        .group_add_referrer(chat_id, user.id)
        .ok()
        .flatten()
        .or_else(|| database.group_adder(chat_id).ok().flatten());
    if let Some(adder) = adder {
        database.force_change(adder, 0).ok(); // ensure the adder has a row to refer from
                                              // The group owner co-refers when distinct from the adder.
        let owner = resolve_group_owner(ctx, chat_id).await.filter(|o| *o != adder);
        if let Some(o) = owner {
            database.force_change(o, 0).ok();
        }
        if database
            .bind_group_referral(user.id, adder, owner.unwrap_or(0))
            .unwrap_or(false)
        {
            pay_group_referral(ctx, adder, owner, user).await;
        }
    }
}

/// The group's owner (Telegram creator), cached on the chat row. On a miss, fetch
/// it once via `getChatAdministrators` and cache it; a failure isn't cached, so a
/// later interaction retries. `None` when it can't be determined.
async fn resolve_group_owner(ctx: &Context, chat_id: i64) -> Option<i64> {
    let database = db(ctx);
    if let Ok(Some(owner)) = database.group_owner(chat_id) {
        return Some(owner);
    }
    let owner = tg::chat_creator(ctx, chat_id).await?;
    let _ = database.set_group_owner(chat_id, owner);
    Some(owner)
}

/// Pay a freshly-bound group referral's signup bonus and DM each co-referrer their
/// share. With a distinct `owner`, the referrer-side [`REFERRAL_REWARD`] is split
/// 50/50 (adder + owner); otherwise the adder gets it all. The referee always gets
/// the full reward. Mirrors [`pay_referral`] for the deep-link path.
async fn pay_group_referral(ctx: &Context, adder: i64, owner: Option<i64>, referee: &User) {
    let database = db(ctx);
    let co = owner.unwrap_or(0);
    if let Err(e) = database.reward_group_signup(adder, co, referee.id, REFERRAL_REWARD) {
        alert_owner(
            ctx,
            &format!(
                "pay_group_referral credit failed (adder {adder}, owner {co}, referee {}): {e}",
                referee.id
            ),
        )
        .await;
        return;
    }
    let split = co > 0 && co != adder;
    let half = REFERRAL_REWARD / 2;
    let adder_amt = if split { half } else { REFERRAL_REWARD };
    let alang = database.get_lang(adder).ok().flatten().unwrap_or(Lang::En);
    let _ = send_text(
        ctx,
        adder,
        i18n::referral_bonus(alang, &full_name(referee), &fmt_coins(adder_amt)),
    )
    .await;
    if split {
        let olang = database.get_lang(co).ok().flatten().unwrap_or(Lang::En);
        let _ = send_text(
            ctx,
            co,
            i18n::referral_bonus(olang, &full_name(referee), &fmt_coins(REFERRAL_REWARD - half)),
        )
        .await;
    }
}
