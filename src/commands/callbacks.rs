use crate::commands::tg::answer;
use crate::commands::util::{bot_token, cb_lang, db, fmt_coins, is_group_chat, SORRY_FRUITS};
use crate::commands::{
    admin, assets, betting, history, markets, menu, predict, predmarket, referral, selling, tg,
};
use crate::core::i18n::{self, Lang};
use crate::database::OfferOutcome;
use rand::seq::SliceRandom;
use std::collections::HashMap;
use telexide::model::{CallbackQuery, ChatMember, MessageContent, UpdateContent};
use telexide::prelude::*;

/// The bot's own membership changed in a chat. When it's *added to a group*,
/// record the adder so new users who check in there bind to them as a referral.
#[prepare_listener]
pub async fn on_my_chat_member(ctx: Context, update: Update) {
    let UpdateContent::MyChatMember(upd) = update.content else {
        return;
    };
    let chat_id = upd.chat.get_id();
    if chat_id >= 0 {
        return; // groups / supergroups only
    }
    let now_in = matches!(
        upd.new_chat_member,
        ChatMember::Member(_) | ChatMember::Administrator(_) | ChatMember::Creator(_)
    );
    let was_out = matches!(upd.old_chat_member, ChatMember::Left(_) | ChatMember::Kicked(_));
    if now_in && was_out {
        let _ = db(&ctx).set_group_adder(chat_id, upd.from.id);
    }
}

/// A user added other members to a group (a "new member" service message). Record
/// who added whom (`group_adds`) so that when a brand-new member later interacts,
/// they bind to the person who actually added them — taking priority over the
/// bot-adder — with the group owner as the 0.5 co-referrer (see
/// `referral::maybe_bind_group`). The bot itself and self-joins (via invite link,
/// where `from` is the joining member) are skipped, so the bot-adder stays the
/// fallback for those. Groups only.
#[prepare_listener]
pub async fn on_new_members(ctx: Context, update: Update) {
    let UpdateContent::Message(message) = update.content else {
        return;
    };
    let MessageContent::NewChatMembers { content: members } = &message.content else {
        return;
    };
    let chat_id = message.chat.get_id();
    if chat_id >= 0 {
        return; // groups / supergroups only
    }
    let Some(adder) = message.from.as_ref() else {
        return;
    };
    if adder.is_bot {
        return; // bots don't refer
    }
    let bot_id = ctx
        .data
        .read()
        .get::<crate::bot::BotIdKey>()
        .copied()
        .unwrap_or(0);
    let database = db(&ctx);
    for member in members {
        // Skip the bot (added alongside members) and self-joins (from == the new
        // member) — those keep the bot-adder as the referrer.
        if member.is_bot || member.id == bot_id || member.id == adder.id {
            continue;
        }
        let _ = database.record_group_add(chat_id, member.id, adder.id);
    }
}

#[prepare_listener]
pub async fn on_callback(ctx: Context, update: Update) {
    let UpdateContent::CallbackQuery(cb) = update.content else {
        return;
    };
    let Some(data) = cb.data.clone() else {
        return;
    };
    // Learn this chat so /broadcast can reach it later.
    if let Some(m) = &cb.message {
        let _ = db(&ctx).touch_chat(m.chat.get_id());
        // `/onlyreplyhere`: ignore button presses outside the locked topic (silent
        // ack, no edit) so the bot stays confined to its topic. This runs before the
        // owner bypass below, so it gates **everyone including the owner** — an owner
        // driving a button flow (`stl:`/`rst:`) from another topic gets a silent
        // no-op. Their admin *commands* skip this (they don't call `paused_block`),
        // and the picker those buttons live on is itself posted into the locked
        // topic, so in practice the owner taps in-topic anyway.
        if crate::commands::util::out_of_locked_topic(&ctx, m.chat.get_id(), m.message_thread_id) {
            let _ = answer(&ctx, &cb, "", false).await;
            return;
        }
    }
    // Admin pause kill-switch: block every non-owner button press while paused.
    // Fail **closed** — if we can't read the flag, treat the bot as paused (a
    // kill-switch that can't confirm "off" should stop, not pass through).
    if !crate::commands::util::is_owner(&ctx, cb.from.id) {
        let paused = db(&ctx).is_paused().unwrap_or_else(|e| {
            eprintln!("on_callback is_paused error (failing closed): {e}");
            true
        });
        if paused {
            let _ = answer(&ctx, &cb, i18n::service_paused(cb_lang(&ctx, &cb)), false).await;
            return;
        }
    }
    // Group-add referral: a brand-new user's first interaction in a group binds
    // them to whoever added the bot. Runs before dispatch so the row is created
    // here (not by the handler that follows). The text-command path is mirrored
    // in `util::paused_block`.
    if let Some(m) = &cb.message {
        referral::maybe_bind_group(&ctx, m.chat.get_id(), &cb.from).await;
    }
    let result = if let Some(rest) = data.strip_prefix("envelope:") {
        handle_envelope(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix("bx:") {
        handle_board_dismiss(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(predict::PREDICT_END) {
        predict::handle_predict_endtime(&ctx, &cb, rest).await
    } else if data.starts_with("sell:") {
        handle_sell(&ctx, &cb).await
    } else if data.starts_with("buy:") {
        handle_buy(&ctx, &cb).await
    } else if let Some(rest) = data.strip_prefix(menu::STZ) {
        handle_settings_tz(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(menu::SET_TZ) {
        handle_set_tz(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(menu::SET_FMT) {
        handle_set_fmt(&ctx, &cb, rest).await
    } else if data == menu::CFG_LANG {
        handle_cfg_lang(&ctx, &cb).await
    } else if data == menu::CFG_TZ {
        handle_cfg_tz(&ctx, &cb).await
    } else if data == menu::CFG_ODDS {
        handle_cfg_odds(&ctx, &cb).await
    } else if data == menu::CFG_HOME {
        handle_cfg_home(&ctx, &cb).await
    } else if let Some(rest) = data.strip_prefix(menu::SLANG) {
        handle_settings_lang(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(menu::SET_LANG) {
        handle_set_lang(&ctx, &cb, rest).await
    } else if data == menu::MENU_CHECKIN {
        handle_menu_checkin(&ctx, &cb).await
    } else if data == menu::MENU_HOME {
        handle_menu_home(&ctx, &cb).await
    } else if data == menu::MENU_BALANCE {
        handle_menu_balance(&ctx, &cb).await
    } else if data == menu::MENU_BETS {
        handle_menu_bets(&ctx, &cb).await
    } else if data == menu::MENU_HISTORY {
        handle_menu_history(&ctx, &cb).await
    } else if let Some(rest) = data.strip_prefix(history::HIST_TAB) {
        handle_history_tab(&ctx, &cb, rest).await
    } else if data == menu::MENU_MARKETS {
        handle_menu_markets(&ctx, &cb).await
    } else if data == menu::MENU_RULE {
        handle_menu_rule(&ctx, &cb).await
    } else if data == menu::MENU_PREDICT {
        handle_menu_predict(&ctx, &cb).await
    } else if data == menu::MENU_INVITE {
        handle_menu_invite(&ctx, &cb).await
    } else if data == menu::INVITE_LINK {
        handle_invite_link(&ctx, &cb).await
    } else if data == menu::INVITE_FWD {
        handle_invite_fwd(&ctx, &cb).await
    } else if data == menu::INVITE_QR {
        handle_invite_qr(&ctx, &cb).await
    } else if let Some(rest) = data.strip_prefix(markets::PAGE) {
        handle_events_page(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(markets::BET) {
        betting::handle_bet(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(betting::OPT) {
        betting::handle_opt(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(betting::SIZE_PLACE) {
        betting::handle_size_place(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(betting::SIZE) {
        betting::handle_size(&ctx, &cb, rest).await
    } else if data == selling::SELL_PICK {
        selling::handle_sell_pick(&ctx, &cb).await
    } else if let Some(rest) = data.strip_prefix(selling::SELL_PLACE) {
        selling::handle_sell_place(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(selling::SELL_BUILD) {
        selling::handle_sell_build(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(predict::PREDICT_FEE) {
        predict::handle_predict_fee(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(predict::PREDICT_FUND) {
        predict::handle_predict_funding(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(predmarket::PM_FPLACE) {
        predmarket::handle_fund_place(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(predmarket::PM_FSIZE) {
        predmarket::handle_fund_size(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(predmarket::PM_FUND) {
        predmarket::handle_fund(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(predmarket::PM_PLACE) {
        predmarket::handle_place(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(predmarket::PM_SIZE) {
        predmarket::handle_size(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(predmarket::PM_BUY) {
        predmarket::handle_buy(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(predmarket::PM_RESOLVE) {
        predmarket::handle_resolve(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(predmarket::PM_WIN) {
        predmarket::handle_win(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(predmarket::PM_VOID) {
        predmarket::handle_void(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(predmarket::PM_BACK) {
        predmarket::handle_back(&ctx, &cb, rest).await
    } else if let Some(rest) = data.strip_prefix(admin::RESET_CB) {
        admin::handle_reset_cb(&ctx, &cb, rest).await
    } else if data == admin::DASH_REFRESH {
        handle_dashboard_refresh(&ctx, &cb).await
    } else {
        Ok(())
    };
    if let Err(err) = result {
        eprintln!("callback handler error: {err}");
    }
}

async fn handle_envelope(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    let chat_id = message.chat.get_id();
    let msg_id = message.message_id;
    let lang = Lang::from_user(&cb.from);
    let db = db(ctx);
    if !db.has_buffer(chat_id, msg_id).unwrap_or(false) {
        return answer(ctx, cb, i18n::someone_took_it(lang), false).await;
    }
    // The amount in the callback data is **untrusted** (a crafted callback could
    // claim to be any value). It only selects the branch; the actual coin credit
    // comes from the escrow stored in the buffer row (see `claim_envelope`).
    let amount: i64 = rest.parse().unwrap_or(0);
    if amount <= 0 {
        // A forged non-positive claim: hand out a consolation fruit and **refund**
        // the envelope's escrow to its owner (so the coins are neither minted nor
        // stranded), cancelling the envelope.
        let fruit = {
            let mut rng = rand::thread_rng();
            // SORRY_FRUITS is a non-empty const, so `choose` is always `Some`;
            // fall back to a fixed fruit rather than `unwrap` to keep this path
            // panic-free regardless.
            SORRY_FRUITS.choose(&mut rng).copied().unwrap_or('🍑')
        };
        let s = fruit.to_string();
        match db.fruit_change(cb.from.id, &s, true) {
            Ok(true) => {
                db.refund_envelope(chat_id, msg_id).ok();
                let _ = tg::edit_text_only(
                    ctx,
                    chat_id,
                    msg_id,
                    &i18n::received_fruit(lang, &cb.from.first_name, &s),
                )
                .await;
                return answer(ctx, cb, i18n::grabbed_it(lang), false).await;
            }
            Ok(false) => return answer(ctx, cb, i18n::too_many_fruits(lang), false).await,
            Err(_) => return answer(ctx, cb, i18n::db_error(lang), true).await,
        }
    }
    // Atomically claim: the buffer row is deleted and the **escrowed** amount
    // credited only if THIS tap won the delete, so a concurrent double-tap can't
    // double-credit and a crafted callback can't change the credited amount.
    let credited = match db.claim_envelope(chat_id, msg_id, cb.from.id) {
        Ok(Some(units)) => units,
        Ok(None) => return answer(ctx, cb, i18n::someone_took_it(lang), false).await,
        Err(e) => {
            eprintln!("claim_envelope error (chat {chat_id}, msg {msg_id}): {e}");
            return answer(ctx, cb, i18n::db_error(lang), true).await;
        }
    };
    let _ = tg::edit_text_only(
        ctx,
        chat_id,
        msg_id,
        &i18n::received_coins(lang, &cb.from.first_name, &fmt_coins(credited)),
    )
    .await;
    answer(ctx, cb, i18n::grabbed_it(lang), false).await
}

/// `bx:<owner>` — dismiss an in-group personal stake board by deleting it.
/// Owner-locked: only the user the board was opened for can delete it (anyone
/// else sees a toast). Shared by match-bet and self-host `/predict` boards.
async fn handle_board_dismiss(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Ok(owner) = rest.parse::<i64>() else {
        return answer(ctx, cb, "", false).await;
    };
    if owner != cb.from.id {
        return answer(ctx, cb, i18n::not_your_bet(lang), true).await;
    }
    if let Some(m) = &cb.message {
        let _ = tg::delete_message(ctx, m.chat.get_id(), m.message_id).await;
    }
    answer(ctx, cb, "", false).await
}

/// `setlang:<store_code>` — persist the chosen locale and swap the picker for
/// the Wixy main menu in place.
async fn handle_set_lang(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let Some(lang) = Lang::from_store_code(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    let db = db(ctx);
    if db.set_lang(cb.from.id, lang).is_err() {
        return answer(ctx, cb, i18n::db_error(lang), true).await;
    }
    if let Some(message) = cb.message.clone() {
        let chat = message.chat.get_id();
        let in_group = is_group_chat(chat);
        // Private chats then pick a timezone — always, so `/language` doubles as
        // "change my timezone too" (first-timers and returning users alike).
        // Groups skip it (shared message) and go straight to the menu.
        if in_group {
            let available = true;
            let _ = tg::edit_with_buttons(
                ctx,
                chat,
                message.message_id,
                &menu::menu_text(lang),
                &menu::main_menu_rows(lang, available, in_group),
            )
            .await;
        } else {
            let _ = tg::edit_with_buttons(
                ctx,
                chat,
                message.message_id,
                i18n::choose_timezone(lang),
                &menu::tz_picker_rows(db.get_tz(cb.from.id).ok().flatten(), false),
            )
            .await;
        }
    }
    answer(ctx, cb, "", false).await
}

/// `settz:<minutes>` — save the picked UTC offset and open the main menu.
async fn handle_set_tz(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let Ok(minutes) = rest.parse::<i64>() else {
        return answer(ctx, cb, "", false).await;
    };
    let db = db(ctx);
    let lang = cb_lang(ctx, cb);
    if db.set_tz(cb.from.id, minutes).is_err() {
        return answer(ctx, cb, i18n::db_error(lang), true).await;
    }
    if let Some(message) = cb.message.clone() {
        let chat = message.chat.get_id();
        let in_group = is_group_chat(chat);
        let available = in_group || db.checkin_available(cb.from.id).unwrap_or(true);
        let _ = tg::edit_with_buttons(
            ctx,
            chat,
            message.message_id,
            &menu::menu_text(lang),
            &menu::main_menu_rows(lang, available, in_group),
        )
        .await;
    }
    answer(ctx, cb, "", false).await
}

/// Re-render the `/settings` hub in place on the callback's message, then ack.
/// The shared tail of every settings-pick handler (`setfmt:`/`cfg:home`/`slang:`/
/// `stz:`), rendered in `lang` (the *new* locale for a language pick).
async fn rerender_settings_hub(ctx: &Context, cb: &CallbackQuery, lang: Lang) -> Result<(), telexide::Error> {
    tg::edit_cb(ctx, cb, i18n::settings_title(lang), &menu::settings_rows(lang)).await;
    answer(ctx, cb, "", false).await
}

/// `setfmt:<code>` — settings-variant odds pick: persist the chosen format and
/// return to the `/settings` hub in place (uniform with `slang:`/`stz:`).
async fn handle_set_fmt(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let fmt = crate::core::types::OddsFormat::from_store_code(rest);
    let db = db(ctx);
    let lang = cb_lang(ctx, cb);
    if db.set_odds_fmt(cb.from.id, fmt).is_err() {
        return answer(ctx, cb, i18n::db_error(lang), true).await;
    }
    rerender_settings_hub(ctx, cb, lang).await
}

/// `cfg:home` — re-render the `/settings` hub in place (from a sub-picker's Back).
async fn handle_cfg_home(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    rerender_settings_hub(ctx, cb, lang).await
}

/// `cfg:lang` — open the language picker from the `/settings` hub, ✅-marking the
/// current locale. The picker's `slang:` buttons persist + return to the hub.
async fn handle_cfg_lang(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    tg::edit_cb(
        ctx,
        cb,
        i18n::CHOOSE_LANGUAGE,
        &menu::lang_picker_rows(Some(lang), true),
    )
    .await;
    answer(ctx, cb, "", false).await
}

/// `cfg:tz` — open the timezone picker from the `/settings` hub, ✅-marking the
/// current offset. The picker's `stz:` buttons persist + return to the hub.
async fn handle_cfg_tz(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let current = db(ctx).get_tz(cb.from.id).ok().flatten();
    tg::edit_cb(
        ctx,
        cb,
        i18n::choose_timezone(lang),
        &menu::tz_picker_rows(current, true),
    )
    .await;
    answer(ctx, cb, "", false).await
}

/// `slang:<store_code>` — settings-variant language pick: persist the locale and
/// return to the `/settings` hub in place (re-rendered in the *new* locale).
async fn handle_settings_lang(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let Some(lang) = Lang::from_store_code(rest) else {
        return answer(ctx, cb, "", false).await;
    };
    let db = db(ctx);
    if db.set_lang(cb.from.id, lang).is_err() {
        return answer(ctx, cb, i18n::db_error(lang), true).await;
    }
    rerender_settings_hub(ctx, cb, lang).await
}

/// `stz:<minutes>` — settings-variant timezone pick: persist the UTC offset and
/// return to the `/settings` hub in place.
async fn handle_settings_tz(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let Ok(minutes) = rest.parse::<i64>() else {
        return answer(ctx, cb, "", false).await;
    };
    let db = db(ctx);
    let lang = cb_lang(ctx, cb);
    if db.set_tz(cb.from.id, minutes).is_err() {
        return answer(ctx, cb, i18n::db_error(lang), true).await;
    }
    rerender_settings_hub(ctx, cb, lang).await
}

/// `cfg:odds` — open the odds-format picker from the `/settings` hub, ✅-marking
/// the current format.
async fn handle_cfg_odds(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let current = db(ctx).get_odds_fmt(cb.from.id).unwrap_or_default();
    if let Some(message) = cb.message.clone() {
        let _ = tg::edit_with_buttons(
            ctx,
            message.chat.get_id(),
            message.message_id,
            i18n::btn_odds(lang),
            &menu::odds_picker_rows(lang, current),
        )
        .await;
    }
    answer(ctx, cb, "", false).await
}

/// `menu:checkin` — grant the daily reward; result shown as an alert. In a
/// private chat the menu refreshes so the now-spent button drops off; in a group
/// the menu is shared, so the button stays for everyone else to claim. (Group-add
/// referral binding happens up in `on_callback` before dispatch — see
/// `referral::maybe_bind_group`.)
async fn handle_menu_checkin(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let db = db(ctx);

    // (Group-add referral binding now happens for ANY button press in a group,
    // up in `on_callback` before dispatch — see `maybe_bind_group_referral`.)
    match db.try_checkin(cb.from.id, crate::commands::checkin::CHECKIN_REWARD) {
        Ok(true) => {
            if let Some(message) = cb.message.clone() {
                if !is_group_chat(message.chat.get_id()) {
                    let _ = tg::edit_with_buttons(
                        ctx,
                        message.chat.get_id(),
                        message.message_id,
                        &menu::menu_text(lang),
                        &menu::main_menu_rows(lang, false, false),
                    )
                    .await;
                }
            }
            let amt = fmt_coins(crate::commands::checkin::CHECKIN_REWARD);
            answer(ctx, cb, &i18n::checkin_done(lang, &amt), true).await
        }
        Ok(false) => {
            let t = crate::commands::checkin::time_until_reset();
            answer(ctx, cb, &i18n::checkin_already(lang, &t), true).await
        }
        Err(_) => answer(ctx, cb, i18n::db_error(lang), true).await,
    }
}

/// `menu:invite` — show a chooser: copyable link / forwardable message / QR.
/// The actual artefact is generated only when the user picks one.
async fn handle_menu_invite(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    answer(ctx, cb, "", false).await?;
    // Show the caller's referral count above the format chooser — edited in
    // place over the current message (home menu, or a previous invite result).
    let count = db(ctx).count_referrals(cb.from.id).unwrap_or(0);
    let text = format!(
        "{}\n\n{}",
        i18n::invite_count(lang, &count.to_string()),
        i18n::invite_how(lang)
    );
    let rows = vec![
        vec![(
            i18n::btn_invite_link(lang).to_string(),
            menu::INVITE_LINK.to_string(),
        )],
        vec![(
            i18n::btn_invite_fwd(lang).to_string(),
            menu::INVITE_FWD.to_string(),
        )],
        vec![(i18n::btn_invite_qr(lang).to_string(), menu::INVITE_QR.to_string())],
        vec![(i18n::bet_btn_back(lang).to_string(), menu::MENU_HOME.to_string())],
    ];
    let _ = tg::edit_with_buttons(ctx, message.chat.get_id(), message.message_id, &text, &rows).await;
    Ok(())
}

/// `menu:home` — re-render the main menu in place (used by the invite chooser's
/// Back button). Private-chat flow, so `is_group` is false.
async fn handle_menu_home(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    answer(ctx, cb, "", false).await?;
    let chat = message.chat.get_id();
    let in_group = is_group_chat(chat);
    let available = in_group || db(ctx).checkin_available(cb.from.id).unwrap_or(true);
    let _ = tg::edit_with_buttons(
        ctx,
        chat,
        message.message_id,
        &menu::menu_text(lang),
        &menu::main_menu_rows(lang, available, in_group),
    )
    .await;
    Ok(())
}

/// This user's referral deep link, from the cached bot username.
fn referral_link_of(ctx: &Context, user_id: i64) -> String {
    let username = ctx
        .data
        .read()
        .get::<crate::bot::BotUsernameKey>()
        .cloned()
        .unwrap_or_default();
    menu::referral_link(&username, user_id)
}

/// `inv:link` — the referral link in a tap-to-copy `<code>` span, posted as a
/// **new** message so the format chooser stays put (the user can pick another
/// format) and the link stands on its own.
async fn handle_invite_link(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    answer(ctx, cb, "", false).await?;
    let link = referral_link_of(ctx, cb.from.id);
    let _ = tg::send_html(ctx, message.chat.get_id(), &i18n::invite_copy(lang, &link)).await;
    Ok(())
}

/// `inv:fwd` — a forward-safe message: the link is baked into the **text** (so a
/// forward keeps it), plus a `[🎮 Play now]` URL button for tapping in place
/// (inline keyboards are stripped on forward, hence the link also lives in text).
/// Posted as a **new** message — a clean standalone the user can forward, with
/// the chooser left intact above it.
async fn handle_invite_fwd(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    answer(ctx, cb, "", false).await?;
    let link = referral_link_of(ctx, cb.from.id);
    let rows = vec![vec![(i18n::btn_join(lang).to_string(), link.clone())]];
    let _ = tg::send_with_buttons(
        ctx,
        message.chat.get_id(),
        &i18n::invite_forward(lang, &link),
        &rows,
    )
    .await;
    Ok(())
}

/// `inv:qr` — a QR photo, caption = the bare link, no keyboard. The QR is
/// generated locally and its Telegram `file_id` cached per user, so repeat taps
/// re-send by id with no regeneration/upload.
async fn handle_invite_qr(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    answer(ctx, cb, "", false).await?;
    let chat_id = message.chat.get_id();
    let link = referral_link_of(ctx, cb.from.id);
    // Caption is just the link (count is shown on the chooser, not here); no
    // keyboard — the link rides in the caption and the QR image. The [Play]
    // button lives on the forwardable message instead.
    let text = link.clone();
    let rows: &[tg::Row] = &[];

    let token = bot_token(ctx);
    let cached = qr_cache().lock().get(&cb.from.id).cloned();
    let sent = if let Some(file_id) = cached {
        tg::send_photo_id(&token, chat_id, &file_id, &text, rows)
            .await
            .is_ok()
    } else {
        false
    };
    if !sent {
        match tg::qr_png(&link, 512) {
            Ok(png) => match tg::send_photo_bytes(&token, chat_id, png, &text, rows).await {
                Ok(Some(file_id)) => {
                    qr_cache().lock().insert(cb.from.id, file_id);
                }
                Ok(None) => {} // sent, but couldn't read the id — re-upload next time
                Err(_) => {
                    crate::commands::util::send_text(ctx, chat_id, text.clone()).await?;
                }
            },
            Err(_) => {
                crate::commands::util::send_text(ctx, chat_id, text.clone()).await?;
            }
        }
    }
    Ok(())
}

/// Per-user cache of the referral QR's Telegram `file_id`. The QR is a function
/// of the user's (immutable) referral link, so the id stays valid indefinitely;
/// a process restart just costs one re-upload per user. In-memory on purpose.
fn qr_cache() -> &'static parking_lot::Mutex<HashMap<i64, String>> {
    static CACHE: std::sync::OnceLock<parking_lot::Mutex<HashMap<i64, String>>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| parking_lot::Mutex::new(HashMap::new()))
}

/// `menu:balance` — edit the menu in place into the presser's balance + open
/// positions. Private-chat only (the button is hidden in groups), so the
/// balance is shown.
async fn handle_menu_balance(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    answer(ctx, cb, "", false).await?;
    let show_balance = !is_group_chat(message.chat.get_id());
    let text = assets::assets_text(ctx, lang, &cb.from, show_balance).await;
    let mut rows = Vec::new();
    // Offer [💸 Sell] when the caller holds open positions (this view is private).
    if db(ctx)
        .user_positions(cb.from.id)
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        rows.push(vec![(
            i18n::btn_sell(lang).to_string(),
            selling::SELL_PICK.to_string(),
        )]);
    }
    rows.push(vec![(
        i18n::bet_btn_back(lang).to_string(),
        menu::MENU_HOME.to_string(),
    )]);
    let _ = tg::edit_with_buttons(ctx, message.chat.get_id(), message.message_id, &text, &rows).await;
    Ok(())
}

/// `menu:bets` — edit the message into the caller's open-positions view (the
/// `/bets` surface: positions only, no balance), with a `[💸 Sell]` button (when
/// holding) + back-to-home. This is the sell flow's back target.
async fn handle_menu_bets(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    answer(ctx, cb, "", false).await?;
    let (body, holds) = assets::bets_body(ctx, lang, &cb.from).await;
    let mut rows = Vec::new();
    if holds {
        rows.push(vec![(
            i18n::btn_sell(lang).to_string(),
            selling::SELL_PICK.to_string(),
        )]);
    }
    rows.push(vec![(
        i18n::bet_btn_back(lang).to_string(),
        menu::MENU_HOME.to_string(),
    )]);
    tg::edit_cb(ctx, cb, &body, &rows).await;
    Ok(())
}

/// `menu:history` — edit the message into the caller's tabbed activity statement
/// (default the Mining tab) with the filter tabs + back-to-home.
async fn handle_menu_history(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    answer(ctx, cb, "", false).await?;
    let (text, rows) = history::tab_view(ctx, lang, &cb.from, crate::database::HistoryTab::Mining).await;
    tg::edit_cb(ctx, cb, &text, &rows).await;
    Ok(())
}

/// `hist:<tab>` — switch the caller's history filter tab (Mining/Trading/Transfer),
/// edit-in-place. Renders the *tapper's* own history, so it's safe if a group's
/// shared `/history` somehow carried tabs (it doesn't — groups get the flat view).
async fn handle_history_tab(ctx: &Context, cb: &CallbackQuery, suffix: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(tab) = history::parse_tab(suffix) else {
        return answer(ctx, cb, "", false).await;
    };
    answer(ctx, cb, "", false).await?;
    let (text, rows) = history::tab_view(ctx, lang, &cb.from, tab).await;
    tg::edit_cb(ctx, cb, &text, &rows).await;
    Ok(())
}

/// `dash:refresh` — owner-only: rebuild the `/dashboard` snapshot and edit it in
/// place. The button is only posted to the owner, but gate anyway (the snapshot
/// exposes bot-wide totals). No i18n — the dashboard is a plain-English diagnostic.
async fn handle_dashboard_refresh(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    if !crate::commands::util::is_owner(ctx, cb.from.id) {
        return answer(ctx, cb, "", false).await;
    }
    let (text, rows) = admin::dashboard_view(ctx);
    tg::edit_cb(ctx, cb, &text, &rows).await;
    answer(ctx, cb, "Refreshed", false).await
}

/// `menu:markets` — post the market brief as a fresh message, leaving the menu
/// in place.
async fn handle_menu_markets(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    answer(ctx, cb, "", false).await?;
    let chat = tg::cb_coords(cb).0;
    let tz = if is_group_chat(chat) {
        0
    } else {
        db(ctx).get_tz(cb.from.id).ok().flatten().unwrap_or(0)
    };
    let fmt = db(ctx).get_odds_fmt(cb.from.id).unwrap_or_default();
    let (text, rows) = markets::brief(lang, tz, fmt, 0, true).await;
    tg::edit_cb(ctx, cb, &text, &rows).await;
    Ok(())
}

/// `evpage:<m|s>:<page>` — re-render the market brief at the requested page in
/// place (edits the current message). The `m`/`s` flag (menu vs standalone) and
/// the page number ride in the callback, so no server-side state is kept.
async fn handle_events_page(ctx: &Context, cb: &CallbackQuery, rest: &str) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    answer(ctx, cb, "", false).await?;
    let (flag, page_str) = rest.split_once(':').unwrap_or(("s", rest));
    let page: usize = page_str.parse().unwrap_or(0);
    let chat = tg::cb_coords(cb).0;
    let tz = if is_group_chat(chat) {
        0
    } else {
        db(ctx).get_tz(cb.from.id).ok().flatten().unwrap_or(0)
    };
    let fmt = db(ctx).get_odds_fmt(cb.from.id).unwrap_or_default();
    let (text, rows) = markets::brief(lang, tz, fmt, page, flag == "m").await;
    tg::edit_cb(ctx, cb, &text, &rows).await;
    Ok(())
}

/// `menu:rule` — edit the menu in place into the "how to earn coins" rules.
async fn handle_menu_rule(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    use crate::commands::checkin::CHECKIN_REWARD;
    use crate::commands::referral::REFERRAL_REWARD;
    let lang = cb_lang(ctx, cb);
    answer(ctx, cb, "", false).await?;
    let text = i18n::rules_text(lang, &fmt_coins(CHECKIN_REWARD), &fmt_coins(REFERRAL_REWARD));
    let rows = vec![vec![(
        i18n::bet_btn_back(lang).to_string(),
        menu::MENU_HOME.to_string(),
    )]];
    tg::edit_cb(ctx, cb, &text, &rows).await;
    Ok(())
}

/// `menu:predict` — the group home page's "create prediction" button: open the
/// `/predict` builder for the presser (same flow as the `/predict` command), DM'd
/// to them, with a toast pointing there (or "DM me first" if they have no DM open).
async fn handle_menu_predict(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let lang = cb_lang(ctx, cb);
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    if predict::open_draft(ctx, &cb.from, message.chat.get_id()).await {
        answer(ctx, cb, i18n::predict_check_dm(lang), false).await
    } else {
        answer(ctx, cb, i18n::bet_dm_first(lang), true).await
    }
}

async fn handle_sell(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    let chat_id = message.chat.get_id();
    let msg_id = message.message_id;
    let lang = Lang::from_user(&cb.from);
    let db = db(ctx);
    let outcome = match db.consume_sell(chat_id, msg_id, cb.from.id) {
        Ok(o) => o,
        Err(_) => return answer(ctx, cb, i18n::db_error(lang), true).await,
    };
    match outcome {
        OfferOutcome::AlreadyTaken => answer(ctx, cb, i18n::someone_dealt(lang), false).await,
        OfferOutcome::SelfCancelled => {
            tg::delete_message(ctx, chat_id, msg_id).await.ok();
            answer(ctx, cb, i18n::withdrew_sell(lang), false).await
        }
        OfferOutcome::Filled { fruits, price } => {
            let _ = tg::edit_text_only(
                ctx,
                chat_id,
                msg_id,
                &i18n::bought_msg(lang, &cb.from.first_name, &fmt_coins(price), &fruits),
            )
            .await;
            answer(ctx, cb, &i18n::bought_toast(lang, &fruits), true).await
        }
        OfferOutcome::TakerNotEnoughBalance => answer(ctx, cb, i18n::not_enough_money(lang), true).await,
        OfferOutcome::TakerFruitFull => answer(ctx, cb, i18n::too_many_fruits(lang), true).await,
        OfferOutcome::TakerMissingFruit(_) => answer(ctx, cb, i18n::system_error(lang), true).await,
    }
}

async fn handle_buy(ctx: &Context, cb: &CallbackQuery) -> Result<(), telexide::Error> {
    let Some(message) = cb.message.clone() else {
        return Ok(());
    };
    let chat_id = message.chat.get_id();
    let msg_id = message.message_id;
    let lang = Lang::from_user(&cb.from);
    let db = db(ctx);
    let outcome = match db.consume_buy(chat_id, msg_id, cb.from.id) {
        Ok(o) => o,
        Err(_) => return answer(ctx, cb, i18n::db_error(lang), true).await,
    };
    match outcome {
        OfferOutcome::AlreadyTaken => answer(ctx, cb, i18n::someone_dealt(lang), false).await,
        OfferOutcome::SelfCancelled => {
            tg::delete_message(ctx, chat_id, msg_id).await.ok();
            answer(ctx, cb, i18n::withdrew_buy(lang), false).await
        }
        OfferOutcome::Filled { fruits, price } => {
            let _ = tg::edit_text_only(
                ctx,
                chat_id,
                msg_id,
                &i18n::sold_msg(lang, &cb.from.first_name, &fruits, &fmt_coins(price)),
            )
            .await;
            answer(ctx, cb, &i18n::sold_toast(lang, &fmt_coins(price)), true).await
        }
        OfferOutcome::TakerMissingFruit(ch) => {
            answer(ctx, cb, &i18n::you_dont_have(lang, &ch.to_string()), true).await
        }
        OfferOutcome::TakerFruitFull => answer(ctx, cb, i18n::buyer_fruit_full(lang), true).await,
        OfferOutcome::TakerNotEnoughBalance => answer(ctx, cb, i18n::system_error(lang), true).await,
    }
}
