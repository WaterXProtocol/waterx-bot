# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A Telegram bot for a small private group, written in Rust on top of [`telexide`](https://docs.rs/telexide). The slash commands are localized utilities: balance/fruit ledger, coin/fruit transfers, host-run bet games, fruit trading, daily check-in, and a prediction-market brief. State persists in a SQLite file in the working directory, chosen by the `BOT_DEV` flag via `database::db_filename(dev)`: `waterx-dev.db` for development (the default) and `waterx.db` for production (`BOT_DEV=false`) — so a dev bot never clobbers live balances. Configuration comes from environment variables (loaded from `.env` if present): `BOT_TOKEN`, `BOT_OWNER` (numeric Telegram user id), and optional `BOT_DEV` (default `true`; set `false` for production).

The current command set is `start, assets, balance, bets, send, predict, rule, feedback, sell, buy, markets, checkin, settings, timezone` — though only `start, balance, bets, send, predict, rule, feedback, settings` appear in the `/` menu (`assets`/`sell`/`buy`/`markets`/`checkin`/`timezone` still work when typed but are hidden from `command_menu`). `/feedback` reaches the configured `BOT_OWNER` (`util::owner_id`) two ways: `/feedback <message>` (inline) DMs the owner immediately — message + sender (`name (@username, id …)`), best-effort — and confirms (`i18n::feedback_sent`); bare `/feedback` (no inline text) opens a **DM compose flow** like `/predict` — it DMs `i18n::feedback_ask`, registers a `Convo::Feedback` entry, and (in a group) replies `i18n::feedback_check_dm`; the user's next plain-text DM is forwarded by the `#[prepare_listener]` `feedback::on_message` (registered in `bot::run` next to `predict::on_message`), which then DMs `feedback_sent` and clears the draft. If the prompt DM bounces (the user never started the bot) → `i18n::feedback_dm_first`. The owner DM is sent via the shared `feedback::send_to_owner` helper used by both paths. `/balance` shows just the caller's coin balance and `/bets` just their open positions — **both work in DM *and* group** (a user opts into showing off by typing them, so unlike `/assets` neither hides anything in a group; `/bets` shows `i18n::no_open_bets` when there's nothing). `/assets` is the **combined** balance + positions view, **removed from the `/` menu** (still typeable) and kept for the `[💰 Check assets]` home button (`menu:balance`). Fruit display is **hidden until the fruit feature is designed** — the `fruit` column still exists and `/send`/`/sell`/`/buy` still move fruit, it's just not rendered. The shared renderers all live in `assets`: `balance_block` (name + balance line, or a db-error notice instead of a fake zero), `positions_block` (open match bets + self-host predictions, each section omitted when empty, `""` when none), and `assets_text(ctx, lang, user, show_balance)` which composes them. For the combined `/assets`, **`show_balance` is false in a group** (`util::is_group_chat`) so the balance stays private there — `/balance` is the explicit opt-in to expose it — and the `[💰 Check assets]` button is hidden in groups (its `menu:balance` path passes `true`, private-only). The two sections are:
1. **Match bets** (`i18n::positions_title`) — every unsettled real-money wager (`Database::list_open_wagers` → `database::Position`), rendered as `team_a vs. team_b / side · 🪙stake → 🏆potential-payout`.
2. **Self-host predictions** (`i18n::predictions_title`) — the caller's stakes in `/predict` games still in `betting`/`closed` state, read from the in-memory `PredictionsKey` map (`Prediction.options[*].detail[user]`). Game stakes are debited at bet time (`callbacks` `gamble:` → `balance_change(-stake*COIN)`), so they're committed coins not yet reconciled into the balance; `settled`/`draw` games are skipped because their payouts already are. Stakes are stored in **whole coins**, so rendered via `fmt_coins(stake * COIN)`.

The position lines are language-neutral (team/option names + numbers + symbols; the match side name is localized via the stored team names / `i18n::draw_label`). Plus owner-only admin commands `mint, pause, unpause, broadcast, reset, settle, redeploy, dashboard, load` (gated on `BOT_OWNER` via `util::is_owner`; non-owners are silently ignored and these are deliberately kept out of the `/` command menu). `/dashboard` replies with a plain-English bot-wide snapshot (an operator diagnostic, no i18n): user count + how many were referred, users who checked in today, group vs private chat counts, the **circulating coin supply** (sum of all `balance.balance`), real-money match-bet exposure (open count + coins staked, plus all-time count + volume), and open self-host `/predict` games (count + committed coins, aggregated straight from the normalized `games` table — `COUNT(*)` + `SUM(total)`, since that table holds only live games) and the pause status. It's **one** `Database::dashboard()` call (`database::dashboard::Dashboard`, a single lock + a handful of `COUNT`/`SUM` aggregates over `balance`/`chats`/`wagers`/`games`) — the self-host game figures come from the DB too now, so the handler no longer folds in the in-memory `PredictionsKey` map. `/redeploy` fire-and-forget triggers a **separate** systemd oneshot (`waterx-deploy.service` → `deploy/deploy.sh`: git pull → `cargo build --release` → `systemctl restart waterx-bot`) via `$REDEPLOY_CMD` (default `sudo systemctl start --no-block waterx-deploy.service`); it runs in its own unit so the restart can't kill the build, and is **inert until the operator installs the unit + sudoers** (see `DEPLOY.md`). On success `/redeploy` writes a marker file (`admin::REDEPLOY_MARKER` = `redeploy.notify`, holding the chat id); since the process that ran the command dies in the restart, the **freshly restarted** bot reads that marker in `bot::run` on startup, posts "✅ Redeploy complete — back online" to that chat, and deletes it. `/reset` (owner **and** dev-mode only — `util::is_dev`, so it can never fire on a production bot) posts a **multi-select picker** (`admin::handle_reset_cb`, callback prefix `rst:`): `[Matches]` / `[Predictions]` / `[Balances]` each toggle a `✅`/`⬜` mark (the selection rides as a bitmask in the callback data — `rst:t:<flags>` re-renders, `rst:go:<flags>` executes), and `[🧹 Submit]` runs the picked parts — **Matches** refunds every open wager's stake then wipes `wagers` (`Database::reset_wagers`), **Predictions** refunds every game stake (whole→micro) then wipes the game tables + clears the in-memory `PredictionsKey` map (`Database::reset_predictions`), and **Everything** does the **full** maintenance flow: (1) return all open bets (both refunds above, so the snapshot captures those coins), (2) **snapshot** every non-zero balance to a timestamped `balances-<ts>.json` file (`admin::backup_balances` → `Database::export_balances`), then (3) wipe every table (`Database::reset_all` + clears `PredictionsKey`). If the backup **can't be written, the wipe is aborted** (balances are never lost). Everything **subsumes** the granular parts and short-circuits them. Because it clears `balance` the same Telegram users become brand-new — referral binding gates on a row's *existence*, so they can be **re-referred** — and because it clears `chats` (the group adder), re-test the group-add referral by **kicking + re-adding the bot** (`on_my_chat_member` re-records `chats.added_by` on the Left/Kicked→Member transition), then members re-bind on their next interaction. `/load` (owner-only) **lists** the `balances-*.json` snapshots newest-first, or **restores** one — `/load <file>` upserts each user's coin balance from the file (`Database::import_balances`); the filename is validated (`admin::valid_backup_name`: `balances-*.json`, no path separators) so a load can't traverse out of the working dir. `/mint <amt>` credits whole coins (positive only — no debt) to the **replied-to user**, or to the **owner themselves** when sent without a reply; `/pause`/`/unpause` toggle a persisted kill-switch (`meta` table) that `util::paused_block` enforces at the top of every non-admin command and `callbacks::on_callback` enforces for button presses — the owner always passes through; `/broadcast <msg>` posts to every chat the bot has seen — private DMs **and** groups (`Database::all_chat_ids`), skipping failures. Chats are recorded into the `chats` table via `Database::touch_chat`, called from `util::paused_block` (top of every non-admin command) and `callbacks::on_callback`; group/channel ids are negative, private-chat ids positive (== user id). `/start` is the button-driven entry point: in a **private chat** a user picking a language is **always** then shown the **timezone picker**, and only after that the Xaliah main menu (so the language flow doubles as "set timezone too"); a first-time user gets the language picker first (chaining language→timezone→menu); a returning user with a saved locale skips straight to the menu. In a **group** both pickers are skipped (it's a shared message) — the menu opens immediately in the sender's saved locale, or their Telegram-reported language (`Lang::from_user`) if unset. `/settings` (renamed from `/language`) opens a **hub** (`menu::settings_rows`) of three uniform click-in buttons — `[🌐 Language]` (`cfg:lang`), `[🕐 Timezone]` (`cfg:tz`), `[🎲 Format]` (`cfg:odds`) — each editing the message in place into its own picker that **`✅`-marks the current choice** (`menu::lang_picker_rows`/`tz_picker_rows`/`odds_picker_rows` each take the current value). The odds picker labels every format with a live 65¢ example (Decimal `1.54`, American `-185`, Percent `65%`, Price `65¢`); `setfmt:<code>` → `handle_set_fmt` persists `balance.odds_fmt` and **re-renders the hub in place** (so the odds pick returns to the hub, uniform with language/timezone); the odds picker's `[⬅ Back]` (`cfg:home` → `handle_cfg_home`) also returns to the hub if you don't pick. The language/timezone pickers opened **from the hub** emit **settings-variant** callbacks — `slang:<store_code>` / `stz:<minutes>` (via the `settings` flag on `menu::{lang_picker_rows, tz_picker_rows}`) — handled by `handle_settings_lang`/`handle_settings_tz`, which persist the choice and **re-render the hub in place** (the language one in the newly-chosen locale). So all three settings land back on the hub after a pick. The **onboarding** `setlang:`/`settz:` flow (used only by first-run `/start` and the standalone `/timezone`) is unchanged: it still chains language→timezone→menu. `/timezone` re-opens the timezone picker (a curated set of UTC offsets, `menu::tz_picker_rows`, callback `settz:<minutes>`, persisted as **minutes east of UTC** in `balance.tz_offset`). The timezone is used to render **all absolute times** in the caller's local time in private chats — currently the `/markets` kickoff times (`markets::fmt_time`) and the brief's date header (`markets::fmt_date`); a group `/markets` is a shared message so it stays UTC. (The daily check-in stays on a fixed **00:00 UTC** boundary — never local-midnight — so a user can't farm it by hopping timezones; its "come back in 5h 23m" countdown is a duration, which is timezone-independent and so needs no localization.) `/timezone` works when typed but is hidden from the `/` menu (`/settings` is in it). The Xaliah menu (`menu::menu_text`) is just an intro that greets the caller by name (`i18n::intro(lang, name)` — name = `util::full_name`, threaded in from each call site); the balance is **not** shown here (the `[💰 Check assets]` button covers it). It has inline buttons (one per row, `menu::main_menu_rows(lang, checkin_available, is_group)`) firing the `setlang:` / `menu:` callbacks. **Private chats** show `[💰 Check assets]` (`menu:balance`, **edits the message in place** into the full `/assets` view incl. open positions via the shared `assets::assets_text`, + `[⬅ Back]` → `menu:home`), `[today's matches]` (`menu:markets`, also **edits in place** to the match brief + `[⬅ Back]` → `menu:home`), `[📜 How to earn coins]` (`menu:rule`, edits in place into the `i18n::rules_text` brief + `[⬅ Back]` → `menu:home`), `[🔗 Invite friends]`, and `[daily check-in]` on top when claimable (`Database::checkin_available`). **Group chats** (negative chat id, `util::is_group_chat`) get a single shared message, so only the **shared** buttons show — `[daily check-in]` (always shown, never dropped after a claim, so every member can claim — per-user gating still happens in `try_checkin`), `[today's matches]`, and `[🎲 Create prediction]` (`menu:predict` → `handle_menu_predict`, which opens the `/predict` builder exactly like the command via the shared `predict::open_draft`); the per-user `[Check assets]` / `[Invite friends]` and the `[How to earn coins]` buttons are **hidden** in groups (rule/invite/balance are private-only). In private chats the check-in button drops off once the caller has claimed. The home page deliberately carries **no** referral deep-link button (it'd be a private-info surface). Instead, `[🔗 Invite friends]` (`menu:invite`, private chats only) **edits the current message in place** into a **chooser** showing the caller's **referral count** (`i18n::invite_count` via `Database::count_referrals`) above three format buttons (`i18n::invite_how`) plus a `[⬅ Back]` (`menu:home` → re-renders the main menu via `handle_menu_home`). Menu navigation is **edit-in-place** (one message morphs, not a pile of new ones): the artefact is generated **only when the user picks one** (in `callbacks`):
The three invite artefacts are each posted as a **new message** (the chooser is left in place, so the user can pick more than one format) — only the chooser/menu navigation itself is edit-in-place:
- **`inv:link` — copyable link**: `handle_invite_link` sends a **new** message with the referral link in a tap-to-copy `<code>` span (`tg::send_html`, HTML `parse_mode`), no keyboard.
- **`inv:fwd` — forwardable message**: `handle_invite_fwd` sends a **new** message — a friendly one-liner (`i18n::invite_forward`) with the link in **plain text** (so it survives a forward — inline keyboards don't) **plus the `[🎮 Play now]` URL deep-link button** (`menu::referral_link`, needs `BotUsernameKey`), via `tg::send_with_buttons`.
- **`inv:qr` — QR code** (the one exception — Telegram can't edit a text message into a photo): `handle_invite_qr` sends a **single new photo** — a QR of the link (generated **locally** via `qrcode_generator::to_png_to_vec`, no third-party service), caption = the **bare link** (count is on the chooser, not here), and **no keyboard** (the link rides in the caption/QR image). The QR is identical every tap (the link is immutable), so Telegram's returned **`file_id`** is cached per user (`callbacks::qr_cache`, in-memory `OnceLock<parking_lot::Mutex<HashMap<i64,String>>>`) and re-sent via `tg::send_photo_id` with no regeneration/upload on later taps; only the first tap (or first after a restart) uploads via `tg::send_photo_bytes`, which returns the `file_id`. (A bot can't read arbitrary DM media, but it can re-send its own uploads by `file_id`.) `tg::send_photo_bytes` posts the multipart `sendPhoto` directly via `reqwest` (token from `util::bot_token`) rather than telexide's `send_photo` — **telexide's file-upload path is broken**: it serialises the `photo` field as `attach://qr.png` but names the multipart part `qr` (truncated at the first `.`), so Telegram never matches the attachment and the upload silently fails (this also lets us attach a `reply_markup`, which telexide's button struct can't — see `tg.rs`'s header). On forwarding, Telegram **strips the entire inline keyboard** (callback *and* `url` buttons) — which is exactly why the `[Play]` button lives on the forwardable text message, not here; a forwarded QR relies on the **caption link text** and the **QR image** (link baked in). The QR path is best-effort: on any failure it falls back to a plain `util::send_text` (link + count). **Referral system** (three surfaces, one binding/payout path — shared payout in `referral::pay_referral`, which credits **both** sides `referral::REFERRAL_REWARD` = 10 and DMs the referrer via `i18n::referral_bonus`):
1. **Deep link** — a brand-new user opening someone's link sends `/start <referrer_id>`; `start` records it once via `Database::set_referrer_if_new` (referrer must already exist, must differ, referee must be a new row).
2. **Group add** — when the bot is added to a group, the `callbacks::on_my_chat_member` listener (registered via a second `add_handler_func`; fires on `UpdateContent::MyChatMember` when status goes Left/Kicked → Member/Admin/Creator) records the adder in `chats.added_by` (first adder wins, `Database::set_group_adder`). When a brand-new user then has **any interaction** with the bot in that group — a **button tap** (`callbacks::on_callback`, before dispatch) **or a text command** (`util::paused_block`, top of every non-admin command) — the shared `referral::maybe_bind_group(ctx, chat_id, user)` binds them. **The adder and the group _owner_ (Telegram creator) are co-referrers**: the owner is resolved lazily (`referral::resolve_group_owner` → `tg::chat_creator` via `getChatAdministrators`, cached in `chats.owner`, distinct from `added_by`) and, **when different from the adder**, stored as the referee's `balance.co_referrer` (`Database::bind_group_referral`, after `force_change` on both so they have rows). The signup bonus is then **split 50/50** between adder and owner (`Database::reward_group_signup`; referee still gets the full `REFERRAL_REWARD`), each DM'd their share (`referral::pay_group_referral`); when adder == owner or the owner is unknown, the adder is the sole referrer (full reward, as before). It short-circuits on a `Database::user_exists` fast-path so existing members do a single read and bail (no per-interaction write); it's a no-op outside groups, for existing users (the `INSERT OR IGNORE` only inserts a fresh row), or when the adder is unknown (`added_by = 0`). Both call sites run **before** the command/handler creates the user's row, preserving the brand-new check; binding before dispatch means it's in place for whatever follows (e.g. check-in's upline cascade). Plain group chatter is **not** a trigger — Telegram privacy mode only delivers commands and button callbacks to the bot.

Already-existing users earn nothing on either path (no farming). On top of the one-time signup reward, **every successful check-in pays a referral cascade up the chain** inside `Database::try_checkin`: the direct referrer +1 coin, the referrer-of-referrer +0.1, and one level above +0.01 (`CHECKIN_UPLINE`). When the referee has a **co-referrer** (a group-add bind where the owner ≠ adder), the **direct (level-1) reward is split 50/50** — adder +0.5, owner +0.5 — while the deeper 0.1 / 0.01 levels follow the **adder's** chain only (the owner is a level-1 co-credit, not a chain node).

**Money model:** balances are stored as integer **micro-coins** (6-decimal fixed-point — `database::COIN = 1_000_000` units = 1 coin), kept as `i64` (not `u64`: SQLite integers are signed, ledger deltas are signed, and the non-negative invariant is enforced by `balance_change`'s guard, not the type). User-typed whole-coin amounts (send/sell/buy/mint/stake) are multiplied by `COIN` at the ledger boundary; balances are displayed with `util::fmt_coins`, which rounds to **at most 2 decimals** (half-up, trailing zeros trimmed — so "42", "0.5", "7.69") for display only; the ledger keeps full micro-coin precision. Balances are stored directly in micro-coins with **no startup rescale** — an earlier `×COIN` legacy migration was removed because it double-scaled balances whenever `/reset` wiped its `meta` guard flag. There is no debt path, so the old `debt_coins` message was removed and `/assets` always renders "has". **Atomicity invariant:** `balance_change` is a single conditional `UPDATE … WHERE balance + ?1 >= 0` (atomic, no read-then-write TOCTOU), and every multi-statement money/state mutation runs inside one rusqlite transaction — `Database::transfer` (direct `/send`), `place_wager` (debit + wager insert), `settle_market` (whole market), `reward_referral` / `reward_group_signup` (referral legs — the latter splits the referrer share 50/50 with the group owner), `try_checkin` (reward + the upline cascade, incl. the level-1 co-referrer split), `fruit_transfer` (debit + credit), `claim_envelope` (delete-then-credit, exactly-once under concurrent taps), the startup `refund_and_prune_old_buffer` (refund + delete), and the `buffer` escrow/fill/refund paths (`open_*_offer`, `consume_*`). So a mid-sequence failure can never leave money half-moved, and concurrent debits can't overdraw. Whole-coin amounts from callback data are converted with `util::to_micro` (caps at `MAX_COINS`, `checked_mul`) before any `× COIN`, so a crafted stake can't overflow `i64`. DB read failures **fail safe**: `assets`/`/settle`/`/broadcast` surface the error instead of masking it as a zero balance / empty list, and the pause kill-switch **fails closed** (an unreadable `is_paused` flag blocks non-owners — the owner is checked first, so they can still `/unpause`). **Odds display format** is a per-user preference (`types::OddsFormat` = Decimal/American/Percent/Price, default Decimal) set via the `/settings` hub and persisted in `balance.odds_fmt` (`Database::{get_odds_fmt, set_odds_fmt}`). The single formatter `util::format_odds(cents, fmt)` turns a YES price into a display string (Decimal `1.54`, American `-185`, Percent `65%`, Price `65¢`) and is used at every odds-rendering site: the `/markets` brief (`render_market`/`brief`, caller's format), the match card (`quote_text`/`option_rows`, the **creator's** format pinned in the `opt:<lang>:<fmt>:<market_id>:<outcome>` button so a shared card never flips per tapper), the DM stake builder + `bet_placed`/`bet_announce` (the bettor's format). The DB schema is `balance(user, balance, fruit, last_checkin, lang, referrer, co_referrer, tz_offset, odds_fmt)` (`referrer`/`co_referrer` = the two group-add co-referrers, 0 = none; `tz_offset` = minutes east of UTC, **nullable** — NULL = not yet picked, `Database::{get_tz, set_tz}`; `odds_fmt` = empty → Decimal) + `buffer` + the **normalized** self-host prediction tables `games(id, host, lang, description, state, total, ends_at, odds_fmt, tz_offset, created_at)` (`tz_offset` = the host's timezone pinned at creation, for rendering the deadline in their local time) + `game_options(game_id, idx, name, bet)` + `game_stakes(game_id, option_name, user, amount, bettor_name)` (open `/predict` games only — terminal settled/draw games are **dropped** on settle, so the tables never grow unbounded; `save_prediction` deletes on a terminal state, `load_all_predictions` rebuilds each `Prediction` and recomputes `inputs`; a one-time `migrate_blob_games` folds the legacy JSON-blob `bet_games` table in and drops it) + `meta` (key/value bot-wide flags, currently the `paused` kill-switch via `Database::{is_paused, set_paused}`) + `chats(chat, seen_at, added_by, owner)` (every chat the bot has seen, for `/broadcast`; `added_by` = who added the bot to that group, `owner` = the group's Telegram creator cached lazily — both feed the group-add co-referral) + `wagers` (real-money match bets — see the betting section). `balance.referrer` is the inviter's user id (0 = none); `Database::count_referrals` counts a user's referees. `/checkin` grants 10 coins once per UTC day — `last_checkin` stores the last claimed UTC day index (`unix_secs / 86400`), so the window resets exactly at 00:00 UTC (see `Database::try_checkin`). A vestigial `cloth` column was dropped (a startup migration `ALTER TABLE balance DROP COLUMN cloth` cleans up old data files).

## Commands

```bash
cargo build              # main gate
cargo build --release
cargo run                # reads ./.env (or env vars set externally)
cargo check
```

There are no tests yet — `cargo test` is a no-op.

## Architecture

### Framework version

`Cargo.toml` pins `telexide = "0.1.17"`. **There is no `0.31`** — an earlier commit pinned that and it doesn't resolve. If you see references to a newer telexide version anywhere, treat them as suspect and check `https://crates.io/crates/telexide` before bumping.

### One file per command

`src/commands/<name>.rs` holds exactly one `#[command]`-annotated async fn. `commands/mod.rs` re-exports each with `pub use <name>::*;` so the proc-macro-generated `<name>_COMMAND` static is also re-exported. `src/bot.rs` does `use crate::commands::*;` and lists every command identifier in `create_framework!` — adding a command means:

1. New file under `src/commands/`
2. `pub mod foo;` + `pub use foo::*;` in `commands/mod.rs`
3. Append `foo` to the `create_framework!` ident list in `bot.rs`

### Shared state via `TypeMapKey`

`bot.rs` defines four keys: `DbKey`, `PredictionsKey`, `ConfigKey`, `BotIdKey`. Handlers reach state through helpers in `src/commands/util.rs`:

- `db(&ctx) -> Arc<Database>`
- `games(&ctx) -> Arc<tokio::sync::Mutex<HashMap<String, Prediction>>>`

`ConfigKey` and `BotIdKey` are read directly via `ctx.data.read().get::<…>()` at the few sites that need them.

These helpers also encapsulate the **Send-across-await gotcha**: `ctx.data.read()` returns a `parking_lot::RwLockReadGuard` that is **not `Send`**, so it cannot be held across `.await`. The helpers grab the value and let the guard drop on the same statement — always go through them rather than calling `ctx.data.read()` inline.

### Callback queries are routed separately

Telegram callback queries (inline-button presses) are *not* commands. They come through `src/commands/callbacks.rs::on_callback`, a `#[prepare_listener]` registered via `add_handler_func` in `bot.rs`. It matches `UpdateContent::CallbackQuery` and dispatches on the `cb.data` string prefix: `envelope:`, `gamble:` (incl. `gamble:pick:<idx>`) + the self-host stake-builder `gsz:`/`gsc:`/`gsp:`, `sell:`, `buy:`, the match-bet `bet:`/`opt:`/`sz:`/`szc:`/`szp:`, the shared in-group-board `bx:` (Dismiss), the onboarding `settz:`/`setlang:` and the `/settings`-hub `stz:`/`slang:` (+ `setfmt:`, `cfg:lang`/`cfg:tz`/`cfg:odds`/`cfg:home`), `menu:checkin`/`menu:home`/`menu:balance`/`menu:markets`/`menu:rule`/`menu:predict`/`menu:invite`, `inv:link`/`inv:fwd`/`inv:qr` (invite-format chooser outputs), `stl:` (owner-only button settle flow → `admin::handle_settle_cb`), `rst:` (owner+dev selective-`/reset` picker → `admin::handle_reset_cb`). The `menu:*`/`setlang:` ones drive the `/start` menu (defined in `src/commands/menu.rs`): `setlang:<store_code>` saves the locale and edits the picker into the main menu in place; `menu:checkin` grants the daily reward as an alert and (in private chats) refreshes the menu to drop the now-spent button; `menu:balance` **edits the menu in place** into the caller's balance + open positions (the shared `assets::assets_text`) + `[⬅ Back]` → `menu:home`; `menu:markets` **edits in place** into the match brief (via the shared `markets::brief`) + `[⬅ Back]` → `menu:home`; `menu:home` re-renders the main menu (`handle_menu_home`); `menu:rule` edits in place into the "how to earn coins" brief (private-only button); `menu:predict` (group-only button) opens the `/predict` builder for the presser via the shared `predict::open_draft` and toasts them to their DM (or "DM me first" if it bounces) — identical to running `/predict` in the group. `menu:invite` posts the invite-format chooser, and `inv:link`/`inv:fwd`/`inv:qr` generate the picked format on demand (copyable `<code>` link / forwardable text / cached-`file_id` QR photo — see the `/start` menu section). A second `#[prepare_listener]`, `callbacks::on_my_chat_member` (also registered via `add_handler_func`), watches `UpdateContent::MyChatMember` to record who added the bot to a group (`chats.added_by`) for the group-add referral path.

Logging is error-only on stderr via `eprintln!` (DB/save failures, getUpdates/parse errors, setMyCommands failures, markets fetch failures), with **one** exception: a single startup line (`waterx-bot ready: @<user> (id <n>), <dev|production> mode`) printed by `bot::run` right before the poll loop. Keep new logging to error paths only (plus that one ready line).

This matters because `/sell` and `/buy` slash commands **don't transact** — they just post an inline keyboard with a `sell:<seller>:<fruits>:<price>` or `buy:<buyer>:<fruits>:<price>` payload. The actual fruit/coin exchange happens when the counterparty taps the button and the callback fires.

The `envelope:` callback prefix is still routed even though the `/envelope` command was removed: `/send <amount>` replying to a real user is a **direct transfer**; with **no reply target** (or replying to the bot) it posts a red-envelope-style claim button, and that share path uses the same callback.

### Bet games

`/predict` (fn `predict` in `src/commands/predict.rs`) opens a **stateful DM builder wizard** — the bot's only multi-step conversational flow. It records a per-host `PredictDraft` wrapped in a `Convo::Predict` entry of the shared `bot::ConvosKey` map (keyed by user id — one in-flight DM flow per user, so starting `/predict` overwrites any `/feedback` compose draft and vice versa) holding the **origin chat** (where the finished card posts), DMs the host the first prompt, and (in a group) replies `predict_check_dm`. The host's next plain-text DMs are routed by the `#[prepare_listener]` `predict::on_message` (registered in `bot::run` alongside `on_callback`): first reply → question, second → options (`parse_options`: one-per-line when multi-line, else whitespace-split; need ≥2). It then DMs end-time **preset buttons** (`gend:<minutes>`, 0 = no deadline) plus a `[⌨️ Custom]` button (`gend:custom`) and a `[♾️ No deadline]` button. The shared `finalize` helper builds the `Prediction` (host's locale, `ends_at` = now + minutes·60, or 0), posts the card to the origin chat, and **only on a successful post** registers it (`set_id` + `save_prediction` + `PredictionsKey` under `{chat}:{msg}`); the button path then edits the DM to `predict_created`. **`[⌨️ Custom]`** instead flips `PredictDraft.awaiting_custom` and re-prompts (`predict_ask_custom`); the host's next DM is parsed by `parse_duration` (bare number = minutes, or `<n>d`/`<n>h`/`<n>m` combos like `1d12h`, case/space-insensitive, capped at `MAX_PREDICT_MINUTES` = 30d), then `finalize`d — bad input replies `predict_bad_duration` and stays in the flow. The draft is `remove`d **before** posting so a double-tap can't post twice; `on_message` no-ops for group chats, command text, and DMs with no draft (so it never hijacks ordinary DMs), and fails closed on pause for non-owners. The prediction's `ends_at` (shown on the board in the **host's** timezone — pinned at creation in `Prediction.tz_offset` from `db.get_tz(host)`, like `lang`/`odds_fmt`, and rendered with an explicit `UTC±N` label via `util::tz_label`; `Prediction.ends_at`, 0 = none) is enforced **lazily** — no scheduler — via `Prediction::ended(now)` at all three bet entry points (`gamble:pick`, the `gsz`/`gsc` builder via `game_option`, and the `gsp` place step, which refunds); the host still closes/settles manually — but **can't close early**: the `[close]` handler gates on `Prediction::can_close(now)` (`ends_at == 0 || ended(now)`), so a prediction with a deadline stays open until that time (`close_before_deadline` toast, showing the deadline in the host's tz, otherwise). All bet activity flows through `gamble:` callbacks. The shared group board carries **all options on one row** (`gamble:pick:<idx>` — option by index so its text can't break the callback data, e.g. `[optA] [optB]`) with a host `[close]` (`gamble:` empty) on its **own row below**; once closed, the settle buttons follow the same layout (all `gamble:<outcome>` on one row, `[draw]` on the next; outcome by text matched against `option_order`). Placing a bet uses the **same in-group stake-board as match betting** (the option label carries its current pari-mutuel odds in the **bettor's** format): tapping an option posts the tapper their **own stake board** — a new message **replied to the game card** (`tg::send_with_buttons_reply`, `callbacks::game_builder_rows`) with whole-coin preset buttons (`crate::game::STAKE_AMOUNTS = [1, 5, 10, 50]`) and a `[✖ Dismiss]` row (`bx:<owner>`) that deletes it. In a group the board is headed by the bettor's `👤 name` (`util::board_header`, rendered from `cb.from` since the board is owner-locked) so members can tell whose board is whose. The board is **owner-locked** (only the tapper can use it; others get `i18n::not_your_bet`), so the shared game card stays a card every member can tap for their own board. The board draft is **stateless** — owner + total are encoded in the callback data itself: `gsz:<chat>:<msg>:<idx>:<owner>:<total>` accumulates presets into the running `total` (re-rendered via `i18n::game_build`), `gsc:` shows a confirm screen (`game_confirm`, `[place]`→`gsp:` / `[back]`→`gsz:…:0`), and `gsp:` is the only step that moves money — it debits `total × COIN`, calls `game.stake` (which also records the bettor's display **name** in `Prediction.names` for the settlement readout), edits the **group** board (looked up by the `chat:msg` key) with the new pool/odds, **deletes the personal board**, and posts `i18n::game_announce` to the origin group **as a reply to the board card** (`tg::send_text_reply` to the board's `msg`, `allow_sending_without_reply` so it falls back to a loose message if the card is gone) — mirroring how match betting replies to its game card. Every `gsz`/`gsc`/`gsp` step re-checks `owner == presser`. Nothing is debited until `gsp:`, and there's no in-memory draft to lose on restart. (The shared `bx:<owner>` Dismiss callback — `callbacks::handle_board_dismiss`, owner-locked — deletes a personal board for both match and self-host.) Settlement writes balances to `Database` from the callback handler, not from the game struct itself. `Prediction::settle` returns only the **header** (`result_header`, in the host's locale); the body is built separately by `Prediction::winners_readout(10)` — the **top 10 net winners** by amount won (`top_winners_block`: ranked, names via `Prediction.names`/masked `***<id-tail>`, `more_winners` "…and N more" tail) — so a busy prediction can't blow past Telegram's message-size limit. **One-sided pools** are handled in `settle`: if the winning option drew **no stakes** (everyone bet a different side) it **voids → refunds all stakes** (`no_winners_refund`) rather than burning the pool; if everyone bet the **winning** side (ratio 1.0, nobody nets a profit) `winners_readout` shows an `all_broke_even` note instead of an empty list; truly-no-bets is the existing `no_one_bet_suffix` void. The handler **strips the card's now-dead settle buttons** (`tg::clear_buttons` → `editMessageReplyMarkup`, keyboard only — the card **text** is left as-is) and posts `header + winners_readout` as a **new message replied to the card** (`tg::send_text_reply`). The board (`Prediction::get_text`) is a **shared, live pari-mutuel display** re-rendered on every bet: `🎲 <question> · <state emoji+label>`, one numbered line per option (`① name   <pool> 🪙 → <odds>`, `×—` until it has bets), and a localized pool footer (`i18n::board_footer_open`/`board_footer_closed`). **Odds-format preference applies here too** (`Prediction::option_odds(opt, fmt)`): the natural form is the decimal **multiplier** `pool ÷ option-stake` (shown `×2.50`); other formats convert via `cents = 100/multiplier` through `util::format_odds`. The **shared board** renders in the **host's** format, pinned at creation in `Prediction.odds_fmt` (`#[serde(default)]` Decimal; set from `db.get_odds_fmt(host)` in `handle_predict_endtime`, exactly like `lang`) so it never flips per viewer; the per-bettor **in-group stake board/confirm** instead shows the option in the **bettor's** format — `game_option(ctx, lang, key, idx, fmt)` returns `(name, odds)` and `opt_label` renders `name (odds)` into `i18n::game_build`/`game_confirm` (fmt = the tapper's `get_odds_fmt`). The id-tail `set_id` prepends to `description` is hidden from the board (shown only via `get_header` in the per-user `check()` view). Odds are **never stored** — `OptionData` holds only `detail`/`bet` (the pools), and `option_odds` derives the displayed odds on demand, so `stake`/load just move pools.

### Internationalisation (`src/core/i18n.rs`)

Every user-facing string is localized into 18 locales (English + Traditional &
Simplified Chinese + Japanese, Korean, Russian, French, Spanish, German,
Vietnamese, Indonesian, Filipino, Thai, Dutch, Turkish, Português, हिन्दी,
العربية). **Owner-only/admin replies are exempt** — they use plain English
literals in the handler (the owner is one known person), *not* `tr!` functions;
i18n is reserved for genuinely user-facing strings. (Caveat: a string is only
owner-facing if no non-owner ever sees it — e.g. `service_paused`/`im_back` stay
localized because `paused_block` shows them to regular users; the owner's
pause/unpause confirmation just reuses those.) The module is dependency-free: a `Lang` enum, a `tr!` macro that picks
one of 18 literal arms in a fixed order (`en, hant, hans, ja, ko, ru, fr, es, de,
vi, id, fil, th, nl, tr, pt, hi, ar`), and one `pub fn` per message so all locales
for a message sit together. Parameterised messages leave `{token}` placeholders
in every arm and substitute with `.replace(...)` (real `format!` can't take a
runtime format string). The `tr!` match is exhaustive over all 18 variants, so a
new message that omits an arm fails to compile. The picker display order is driven
by `Lang::ALL` (chunked two-per-row in `menu::lang_picker_rows`), *not* the
`tr!`/`native_label` arm order.

Adding a message: add a `pub fn` with all 18 `tr!` arms in the order above; the
`no_unfilled_placeholders_in_any_locale` test catches any arm that drops a token.

Language resolution is **explicit-choice-wins, auto-detect-fallback**. A user's
`/start`-chosen locale is persisted in `balance.lang` (stable `Lang::store_code`,
e.g. `hant`/`hans`, round-tripped via `Lang::from_store_code`; empty = not yet
chosen). When unset, the bot falls back to Telegram's `User.language_code` via
`Lang::from_user`: unknown / unsupported tags → English; bare `zh` → Simplified,
`zh-Hant`/`zh-TW`/`zh-HK`/`zh-MO` → Traditional. Command handlers resolve the
acting user through `commands::util::lang_for(&ctx, &user)` /
`lang_for_msg(&ctx, &msg)` (DB-then-detect); callbacks use the equivalent
`cb_lang`. The detect-only `lang_of(&Message)` remains for sites without DB
access. `Database::{get_lang, set_lang}` are the persistence accessors.

- **Per-user messages** (direct replies, callback toasts) render in the *acting*
  user's locale.
- **Shared/edited messages** (the bet-game board, sell/buy listings, the
  envelope/settlement edits) render in the **creator's** locale. `Prediction`
  therefore stores a `lang: Lang` field (`#[serde(default)]` so games persisted
  before i18n load as English) set from the host at creation; `Prediction::new` now
  takes `lang` as its 2nd arg. `BetState::label(lang)` replaced the old
  `as_str()`.
- **Command menu**: `bot::run` registers a localized `setMyCommands` per locale
  plus a default (English) menu. Telegram only accepts ISO 639-1 codes there, so
  both Chinese scripts collapse to one `zh` menu (`Lang::menu_code`) and Filipino
  best-efforts under `tl`; per-locale failures are logged and skipped.

### `/markets` — read-only external feed

`src/commands/markets.rs` is the one command that talks to an outside service
rather than the local DB. It `GET`s `https://api.waterx.app/predict/browse`
(via `reqwest` with `rustls-tls`, reusing the rustls already in the tree) and
renders a Jupiter-style "market brief": sport matches only, **kicking off
within the next 24h or already live** (team vs team, kickoff time, and
per-outcome **decimal odds** = `100/oddsCents`, e.g. 65¢ → 1.54) — the feed's
crypto up/down pools are filtered out. It never touches the
coin ledger. Two gotchas baked into the structs: the response models only
the fields the brief reads (serde ignores the rest), and `oddsCents` **must** be
`f64` — some rounds report fractional cents (e.g. `99.9`), which would make an
`i64` field fail to deserialize the entire feed. The fetch sends `locale=zh` for
Chinese users (Hant/Hans → Chinese team names) and `locale=en` for everyone
else; the brief's chrome is localized separately via `render(lang, …)`. (Note:
only the `?locale=` query form works — a `/en/predict/browse` path 404s.)
`fetch_markets` is served from a process-wide **30-second per-locale cache**
(`FEED_CACHE_TTL` = 30s, `feed_cache()` = `OnceLock<Mutex<FeedCache>>`, keyed by
the `en`/`zh` API locale) so repeated `/markets`, `menu:markets`, and `bet:` taps
don't hammer the rate-limited API — the lock is never held across the network
`.await`. Every real-money placement **re-prices from this cache at place time**
(`betting::refetch_quote`, see the betting section), so a wager is always booked
at odds **at most `FEED_CACHE_TTL` (30s) old** — never the older snapshot the
quote was locked to. The displayed quote keeps its own 60s `QUOTE_TTL_SECS` purely
for the build/confirm UI (auto-renewed past that). Concurrent cold-cache misses
aren't coalesced, so a startup burst may fetch a couple of times before the cache
warms.

`/markets` renders the brief with a **numbered button per
shown match** (`bet:<market_id>`); `markets::brief` returns `(text, button rows)`
and `fetch_one(market_id)` re-fetches a single match's fresh odds at bet time.
`MarketInfo` is the distilled per-match struct shared by both.

### Match betting (real balance)

`src/commands/betting.rs` drives a callback-only bet flow funded by the
coin balance:

1. Tapping a match number (`bet:`) re-fetches that match's **current** odds and
   **replaces the brief in place** with the match card (`team_a vs team_b` +
   per-side odds buttons) in **both** groups and private chats, so the chat
   converges on **one focal message** (the multi-match list is consumed — re-open
   `/markets`/`menu:markets` to pick another). The card is **stateless**: each
   side button is `opt:<lang>:<market_id>:<outcome>` (`option_rows`), carrying the
   **market id** — so a tap re-prices on demand and never depends on a stored
   quote surviving (eviction / restart) — **and the locale the card was created
   in**, so a shared group card always re-renders in that one language (it can't
   flip per tapper). Nothing is written to `QuoteStore` at this step. Entry
   error/closed states (`handle_bet`): round already ended (`now ≥ ends_at`) →
   `bet_closed` (⏱️); fetched fine but not listed → `bet_unavailable`; feed
   fetch/parse **failure** → `bet_unavailable` **and** an owner DM
   (`util::notify_owner`) + `eprintln`. `fetch_one` returns
   `Result<Option<MarketInfo>, _>` so callers tell "not listed" (`Ok(None)`) from
   "feed error" (`Err`).
2. Picking a side (`opt:<lang>:<market_id>:<outcome>`, `handle_opt`) **always
   re-prices** via `markets::fetch_one` (cache-served, ≤30s) — self-healing, so
   it works even after the prior quote was evicted or the bot restarted. In a
   **group** the shared card is refreshed to the current odds **in its creator's
   locale** (`card_lang` from the button — no language flip; and a **no-op when
   odds are unchanged**, since the content is byte-identical → Telegram "not
   modified" → no flicker, concurrent taps idempotent) and the tapper's stake
   builder is posted as their **own board** — a new message **replied to the
   game card** (`tg::send_with_buttons_reply`) in the *tapper's* locale, so the
   shared card stays a card (everyone can tap it for their own board) and isn't
   clobbered; in a **private** chat the card itself becomes the builder in place.
   The board is **owner-locked** (see step 2's tail) and, in a group, headed by
   the bettor's `👤 name` (`util::board_header`). A gone/ended
   match (`Ok(None)` or past `ends_at`) → `finish_card` (🏁 match finished, no
   buttons); a transient feed `Err` → card kept + owner DM + retry toast. The
   quote (carrying the group card's `origin_msg` so the placed bet is announced as
   a reply to it; `origin_msg` 0 in a private DM, so no announcement) is minted
   **here**, not at step 1. *(The old manual `[🔄 Refresh]` button —
   `betref:`/`handle_betref`/`bet_stale`/`btn_refresh` — is gone; the card
   refreshes itself on every tap.)* The builder (shared `builder_text_rows`) is
   whole-coin preset buttons that **accumulate** (`sz:<qid>:<outcome>:<total>` —
   each preset re-renders at `total + preset`; `Clear` → `…:0`), plus a
   `[✅ Confirm] [🗑 Clear]` row and, on a group board, a `[✖ Dismiss]` row
   (`bx:<owner>`) that deletes it. The running total rides in the callback data,
   so there is **no server-side per-user stake state**. The board is
   **owner-locked**: the `Quote` (keyed by `qid`) stores `owner` = the tapper, and
   every `sz`/`szc`/`szp` step rejects a presser who isn't the owner with
   `i18n::not_your_bet` (`betting::quote_owner_ok`) — so a shared in-group board
   only its opener can use. When the side was tapped on a **group** card the board
   is a **new in-group message replied to the card**; in a private chat it edits
   in place (owner trivially the only user).
3. `Confirm` (`szc:<qid>:<outcome>:<total>`) shows a **confirmation screen** (the
   "modal": `[✅ Place bet] [⬅ Back]`). Only `Place` (`szp:<qid>:<outcome>:<total>`)
   moves money — it **always re-prices** via `refetch_quote` (re-fetches the
   match's current odds from the ≤30s feed cache regardless of quote age, or
   `expire` if the match is gone/ended), so every wager books at the **current**
   odds, not the snapshot the user was viewing. The stake is converted to
   micro-coins via `util::to_micro` (caps at `MAX_COINS`, rejects overflow), then
   `Database::place_wager` **atomically debits the stake and inserts the wager in
   one transaction** — returning `Ok(false)` on insufficient funds (nothing
   written) and `Ok(true)` on success, with those **just-fetched** `odds_cents`;
   there is no separate debit to roll back. Payout on a win is
   `stake × 100 / odds_cents` (= stake × the decimal odds in `bet_placed`). After
   placing, in a **group** it **deletes the personal board** and posts a
   third-person announcement (`i18n::bet_announce` — "🎟️ Name bet N on Side @
   odds") **as a reply to the game card** (`tg::send_text_reply` to `origin_msg`,
   falling back to a loose `send_text` when there's no card id) so the group sees
   the action under the game it belongs to; in a **private** `/markets` the board
   edits in place into `bet_placed` (no card to reply to). Self-host
   `/predict` betting uses the **same in-group stake-board** (`gsz:`/`gsc:`/
   `gsp:`, `i18n::game_*`, announces via `game_announce`); its option label shows
   the live pari-mutuel odds in the **bettor's** format (`game.option_odds`), and
   its draft rides entirely in the callback data (`chat:msg:idx:owner:total`),
   since the game already lives in the `PredictionsKey` map; see the bet-games section.

Settlement is **manual** (no results endpoint exists on the API — browse only
lists scheduled/live matches, and resolution is on-chain Polymarket), owner-only,
and runs through one shared `admin::run_settle` (calls `Database::settle_market`,
pays winners, marks each wager won/lost, DMs every bettor
`i18n::bet_won`/`bet_lost`, returns a one-line summary). Two front-ends:
- **Buttons (default)** — `/settle` with no args posts a **picker**: one button per
  open market, **labelled with the human-readable title** (`team_a vs team_b`, via
  `admin::market_label`; the `market_id` rides only in the callback data, never
  shown). The owner-only `stl:` callback flow (`admin::handle_settle_cb`, routed in
  `callbacks::on_callback`) then edits the same message through: pick market
  (`stl:p:<id>`) → pick outcome (`stl:o:<id>:<a|b|d>`) → **confirm 1/2**
  (`stl:1:…`) → **confirm 2/2** (`stl:2:…`) → settle. Every step re-reads
  `list_open_markets` and matches by full `market_id`, so a stale/duplicate press
  fails safe instead of settling the wrong market; non-owner presses get a silent ack.
- **Text (power-user)** — `/settle <market_id|slug> <a|b|draw>` settles directly
  via the same `run_settle`.
`wagers(id, user, market_id, slug, team_a, team_b, outcome, stake, odds_cents,
placed_at, ends_at, status, settled_at)` — `stake` micro-coins, `odds_cents` the
locked YES odds, `status` open|won|lost.

### Database

`src/database/` wraps a single `rusqlite::Connection` in a `parking_lot::Mutex` so `Database` is `Send + Sync` and can sit behind an `Arc`. Two tables: `balance(user, balance, fruit)` and `buffer(chat, msg)` — the latter tracks live envelope/sell/buy messages so a callback can detect "someone already took this." The module is split by concern: `mod.rs` (struct + schema + migrations + `COIN` + `reset_all` + `ensure_row`), `user.rs` (balance/`UserRow` in micro-coins, check-in + cascade), `fruit.rs`, `buffer.rs`, `meta.rs` (pause flag), `chats.rs` (seen chats + group adder), `referral.rs` (`set_referrer_if_new`/`count_referrals`), `predictions.rs` (self-host `/predict` games: **owns** its normalized `games`/`game_options`/`game_stakes` schema via `create_game_tables`, the `save_prediction`/`load_all_predictions`/`delete_prediction` codecs, and the one-time blob→normalized `migrate_blob_games`). All sub-files add methods to the same `impl Database` block. Balances are `i64` micro-coins (see the money-model note above).

## Non-obvious gotchas

- **`InlineKeyboardButton::new(text, pay)`** takes two args. The second is `pay: bool` (a Telegram payment-button flag) — telexide's `#[build_struct]` macro promoted it to mandatory because it isn't `Option<T>`. Pass `false` unless you're actually building a payment button.
- **`rand::thread_rng()`** returns a non-`Send` `ThreadRng` — scope it in a block that ends before any `.await`, or the `#[command]` future fails the `Send` bound.
- **The `_COMMAND` statics are pub-glob-reexported** (`pub use start::*`) so `create_framework!(name, start, assets, ...)` can resolve them at the bot.rs call site. Don't move command fns into private modules or the macro expansion will fail to resolve `<name>_COMMAND`.

## Configuration

Env vars consumed by `BotConfig::from_env` (see `src/core/types.rs`): `BOT_TOKEN` / `BOT_OWNER` (required) and optional `BOT_DEV` (default `true`). `BOT_DEV` also selects the SQLite data file via `database::db_filename` — `waterx-dev.db` (dev, default) vs `waterx.db` (production). Both match the `*.db` gitignore rule. `bot.rs::run` calls `BotConfig::from_env`. `main` picks the dotenv file via the `ENV_FILE` var: unset → default `.env` lookup (production); set → that file (`ENV_FILE=.env.dev cargo run` for the dev bot). The repo ships two gitignored configs — `.env` (production, `BOT_DEV=false`) and `.env.dev` (development, `BOT_DEV=true`) — with `.env.example` / `.env.dev.example` as their tracked templates. Dev and production **must** use different bot tokens: one Telegram token can't be polled by two running bots at once (getUpdates conflicts).
