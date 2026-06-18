# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A Telegram bot for a small private group, written in Rust on top of [`telexide`](https://docs.rs/telexide). The slash commands are localized utilities: balance/fruit ledger, coin/fruit transfers, host-run bet games, fruit trading, daily check-in, and a prediction-market brief. State persists in a SQLite file in the working directory, chosen by the `BOT_DEV` flag via `database::db_filename(dev)`: `waterx-dev.db` for development (the default) and `waterx.db` for production (`BOT_DEV=false`) — so a dev bot never clobbers live balances. Configuration comes from environment variables (loaded from `.env` if present): `BOT_TOKEN`, `BOT_OWNER` (numeric Telegram user id), and optional `BOT_DEV` (default `true`; set `false` for production).

The current command set is `start, status, send, predict, sell, buy, markets, checkin, language` — though only `start, status, send, predict, language` appear in the `/` menu (`sell`/`buy`/`markets`/`checkin` still work when typed but are hidden from `command_menu`). `/status` (renamed from `/balance`) shows the caller's coins **and** fruit (the standalone `/fruit` command was removed) **plus their open positions** in two sections, each omitted when empty:
1. **Match bets** (`i18n::positions_title`) — every unsettled real-money wager (`Database::list_open_wagers` → `database::Position`), rendered as `team_a vs. team_b / side · 🪙stake → 🏆potential-payout`.
2. **Self-host predictions** (`i18n::predictions_title`) — the caller's stakes in `/predict` games still in `betting`/`closed` state, read from the in-memory `GamesKey` map (`BetGame.options[*].detail[user]`). Game stakes are debited at bet time (`callbacks` `gamble:` → `balance_change(-stake*COIN)`), so they're committed coins not yet reconciled into the balance; `settled`/`draw` games are skipped because their payouts already are. Stakes are stored in **whole coins**, so rendered via `fmt_coins(stake * COIN)`.

The position lines are language-neutral (team/option names + numbers + symbols; the match side name is localized via the stored team names / `i18n::draw_label`). Plus owner-only admin commands `mint, pause, unpause, broadcast, reset, settle` (gated on `BOT_OWNER` via `util::is_owner`; non-owners are silently ignored and these are deliberately kept out of the `/` command menu). `/reset` additionally requires dev mode (`util::is_dev`) and wipes every table (`Database::reset_all`) plus the in-memory bet games — it can never fire on a production bot. `/mint <amt>` credits the replied-to user whole coins (positive only — no debt); `/pause`/`/unpause` toggle a persisted kill-switch (`meta` table) that `util::paused_block` enforces at the top of every non-admin command and `callbacks::on_callback` enforces for button presses — the owner always passes through; `/broadcast <msg>` posts to every chat the bot has seen — private DMs **and** groups (`Database::all_chat_ids`), skipping failures. Chats are recorded into the `chats` table via `Database::touch_chat`, called from `util::paused_block` (top of every non-admin command) and `callbacks::on_callback`; group/channel ids are negative, private-chat ids positive (== user id). `/start` is the button-driven entry point: in a **private chat** a first-time user is shown a language picker, and once a locale is chosen (persisted to `balance.lang`) the bot opens the Xaliah main menu; in a **group** the picker is skipped entirely (it's a shared message) — the menu opens immediately in the sender's saved locale, or their Telegram-reported language (`Lang::from_user`) if unset. `/language` re-opens the picker anywhere to change the saved locale. The Xaliah menu shows an intro line, the caller's current balance + fruit inventory (`menu::menu_text`), plus inline buttons that fire the `setlang:` / `menu:` callbacks: always `[today's matches]`, and `[daily check-in]` shown when claimable (`Database::checkin_available`) — but in **group chats** (negative chat id, `util::is_group_chat`) the menu is a single shared message, so the check-in button is **always shown** and never dropped after a claim, letting every member claim (per-user gating still happens in `try_checkin`). In private chats the button drops off once the caller has claimed. The menu also has a `[🔗 Invite friends]` button (`menu:invite`). The home page deliberately carries **no** referral deep-link button — it renders the caller's balance/fruit, which is private and shouldn't be forwarded. Instead, tapping `[🔗 Invite friends]` posts the **share-safe invite surface** (in `callbacks::handle_menu_invite`) as a **single photo message**: a **QR code** of the referral link (generated **locally** as in-memory PNG bytes via `qrcode_generator::to_png_to_vec`, so the link never leaves the bot to a third-party QR service), with the referral link + referee count as the **caption** and a `[🎮 Play now]` **URL deep-link** button (`https://t.me/<bot>?start=<user_id>`, via `menu::referral_link`, needs the `BotUsernameKey` typemap entry) as the **keyboard**. It's sent by `tg::send_photo_bytes`, which posts the multipart `sendPhoto` directly via `reqwest` (token from `util::bot_token`) rather than telexide's `send_photo` — **telexide's file-upload path is broken**: it serialises the `photo` field as `attach://qr.png` but names the multipart part `qr` (filename truncated at the first `.`), so Telegram can never match the attachment and the upload silently fails (this also lets us attach a proper `reply_markup`, which telexide's button struct can't — see `tg.rs`'s header). On forwarding, Telegram **strips the entire inline keyboard** (both callback *and* `url` buttons), so forward-referral relies on the surviving **caption link text** and the **QR image** (the link is baked into the picture), not the button — the button is for tapping in place. The whole photo path is best-effort: if QR generation or the upload fails, it falls back to `tg::send_with_buttons` (the link + count text with the same `[Play]` button). **Referral system** (three surfaces, one binding/payout path — shared payout in `referral::pay_referral`, which credits **both** sides `referral::REFERRAL_REWARD` = 10 and DMs the referrer via `i18n::referral_bonus`):
1. **Deep link** — a brand-new user opening someone's link sends `/start <referrer_id>`; `start` records it once via `Database::set_referrer_if_new` (referrer must already exist, must differ, referee must be a new row).
2. **Group add** — when the bot is added to a group, the `callbacks::on_my_chat_member` listener (registered via a second `add_handler_func`; fires on `UpdateContent::MyChatMember` when status goes Left/Kicked → Member/Admin/Creator) records the adder in `chats.added_by` (first adder wins, `Database::set_group_adder`). When a brand-new user then taps the check-in button in that group, `handle_menu_checkin` binds them to the adder via `set_referrer_if_new` (after `force_change(adder, 0)` to ensure the adder has a row) and pays both — before `try_checkin` runs.

Already-existing users earn nothing on either path (no farming). On top of the one-time signup reward, **every successful check-in pays a referral cascade up the chain** inside `Database::try_checkin`: the direct referrer +1 coin, the referrer-of-referrer +0.1, and one level above +0.01 (`CHECKIN_UPLINE`).

**Money model:** balances are stored as integer **micro-coins** (6-decimal fixed-point — `database::COIN = 1_000_000` units = 1 coin), kept as `i64` (not `u64`: SQLite integers are signed, ledger deltas are signed, and the non-negative invariant is enforced by `balance_change`'s guard, not the type). User-typed whole-coin amounts (send/sell/buy/mint/stake) are multiplied by `COIN` at the ledger boundary; balances are displayed with `util::fmt_coins` (trailing zeros trimmed). Balances are stored directly in micro-coins with **no startup rescale** — an earlier `×COIN` legacy migration was removed because it double-scaled balances whenever `/reset` wiped its `meta` guard flag. There is no debt path, so the old `debt_coins` message was removed and `/status` always renders "has". The DB schema is `balance(user, balance, fruit, last_checkin, lang, referrer)` + `buffer` + `bet_games` + `meta` (key/value bot-wide flags, currently the `paused` kill-switch via `Database::{is_paused, set_paused}`) + `chats(chat, seen_at, added_by)` (every chat the bot has seen, for `/broadcast`; `added_by` = who added the bot to that group, for referrals) + `wagers` (real-money match bets — see the betting section). `balance.referrer` is the inviter's user id (0 = none); `Database::count_referrals` counts a user's referees. `/checkin` grants 10 coins once per UTC day — `last_checkin` stores the last claimed UTC day index (`unix_secs / 86400`), so the window resets exactly at 00:00 UTC (see `Database::try_checkin`). A vestigial `cloth` column was dropped (a startup migration `ALTER TABLE balance DROP COLUMN cloth` cleans up old data files); a `fruit_pop` helper from the prior larger command set still lingers.

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

`bot.rs` defines four keys: `DbKey`, `GamesKey`, `ConfigKey`, `BotIdKey`. Handlers reach state through helpers in `src/commands/util.rs`:

- `db(&ctx) -> Arc<Database>`
- `games(&ctx) -> Arc<tokio::sync::Mutex<HashMap<String, BetGame>>>`

`ConfigKey` and `BotIdKey` are read directly via `ctx.data.read().get::<…>()` at the few sites that need them.

These helpers also encapsulate the **Send-across-await gotcha**: `ctx.data.read()` returns a `parking_lot::RwLockReadGuard` that is **not `Send`**, so it cannot be held across `.await`. The helpers grab the value and let the guard drop on the same statement — always go through them rather than calling `ctx.data.read()` inline.

### Callback queries are routed separately

Telegram callback queries (inline-button presses) are *not* commands. They come through `src/commands/callbacks.rs::on_callback`, a `#[prepare_listener]` registered via `add_handler_func` in `bot.rs`. It matches `UpdateContent::CallbackQuery` and dispatches on the `cb.data` string prefix: `envelope:`, `gamble:`, `sell:`, `buy:`, `setlang:`, `menu:checkin`, `menu:matches`, `menu:invite`. The `menu:*`/`setlang:` ones drive the `/start` menu (defined in `src/commands/menu.rs`): `setlang:<store_code>` saves the locale and edits the picker into the main menu in place; `menu:checkin` grants the daily reward as an alert and (in private chats) refreshes the menu to drop the now-spent button; `menu:matches` posts the match brief (via the shared `markets::brief`) as a fresh message; `menu:invite` posts the presser's referral QR as a single photo message (local `qrcode-generator` PNG via `tg::send_photo_bytes`, which posts `sendPhoto` over `reqwest`) — caption = link + count, keyboard = `[🎮 Play now]` URL deep-link button (see the `/start` menu section for the telexide-upload-bug and forward-safety rationale). A second `#[prepare_listener]`, `callbacks::on_my_chat_member` (also registered via `add_handler_func`), watches `UpdateContent::MyChatMember` to record who added the bot to a group (`chats.added_by`) for the group-add referral path.

There is no trace/info logging — only genuine errors are written to stderr via `eprintln!` (DB/save failures, getUpdates/parse errors, setMyCommands failures, markets fetch failures). Keep new logging to error paths only.

This matters because `/sell` and `/buy` slash commands **don't transact** — they just post an inline keyboard with a `sell:<seller>:<fruits>:<price>` or `buy:<buyer>:<fruits>:<price>` payload. The actual fruit/coin exchange happens when the counterparty taps the button and the callback fires.

The `envelope:` callback prefix is still routed even though the `/envelope` command was removed: `/send <amount>` with no reply target (or replying to the bot) posts a red-envelope-style claim button, and that share path uses the same callback.

### Bet games

`/predict <desc> <opt1> <opt2> ...` (fn `predict` in `src/commands/predict.rs`) creates a `BetGame`, stores it under `{chat_id}:{message_id}`, and posts an inline keyboard. All bet activity (place, close, settle, draw) flows through `gamble:` callbacks (the internal callback-data prefix kept its old name; only the user-facing command was renamed). Settlement writes balances to `Database` from the callback handler, not from the game struct itself.

### Internationalisation (`src/i18n.rs`)

Every user-facing string is localized into 18 locales (English + Traditional &
Simplified Chinese + Japanese, Korean, Russian, French, Spanish, German,
Vietnamese, Indonesian, Filipino, Thai, Dutch, Turkish, Português, हिन्दी,
العربية). The module is dependency-free: a `Lang` enum, a `tr!` macro that picks
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
  envelope/settlement edits) render in the **creator's** locale. `BetGame`
  therefore stores a `lang: Lang` field (`#[serde(default)]` so games persisted
  before i18n load as English) set from the host at creation; `BetGame::new` now
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

`/markets` renders the brief with a **numbered button per
shown match** (`bet:<market_id>`); `markets::brief` returns `(text, button rows)`
and `fetch_one(market_id)` re-fetches a single match's fresh odds at bet time.
`MatchInfo` is the distilled per-match struct shared by both.

### Match betting (real balance)

`src/commands/betting.rs` drives a callback-only bet flow funded by the
coin balance:

1. Tapping a match number (`bet:`) re-fetches that match's **current** odds and
   DMs the user a quote with one button per priced outcome (`opt:<qid>:<outcome>`).
   The quote is stored in-memory (`bot::QuotesKey` → `QuoteStore`) under a short
   id and is valid for `QUOTE_TTL_SECS` (60s) — odds move, so it must be fresh.
   If the user has no DM open, a toast tells them to start the bot privately.
2. Picking a side shows whole-coin **preset stake buttons** (`sz:<qid>:<outcome>:<n>`).
3. Confirming a stake re-checks the quote is still fresh (else "open /matches
   again"), debits `stake × COIN` via `balance_change`, and records the wager
   (`Database::place_wager`) with the **locked** `odds_cents`. Payout on a win is
   `stake × 100 / odds_cents` (= stake × the decimal odds shown).

Settlement is **manual** (no results endpoint exists on the API — browse only
lists scheduled/live matches, and resolution is on-chain Polymarket). The owner
runs `/settle` (no args → lists markets with open wagers + their ids;
`/settle <market_id|slug> <a|b|draw>` → `Database::settle_market` pays winners,
marks each wager won/lost, and DMs every bettor `i18n::bet_won`/`bet_lost`).
`wagers(id, user, market_id, slug, team_a, team_b, outcome, stake, odds_cents,
placed_at, ends_at, status, settled_at)` — `stake` micro-coins, `odds_cents` the
locked YES odds, `status` open|won|lost.

### Database

`src/database/` wraps a single `rusqlite::Connection` in a `parking_lot::Mutex` so `Database` is `Send + Sync` and can sit behind an `Arc`. Two tables: `balance(user, balance, fruit)` and `buffer(chat, msg)` — the latter tracks live envelope/sell/buy messages so a callback can detect "someone already took this." The module is split by concern: `mod.rs` (struct + schema + migrations + `COIN` + `reset_all` + `ensure_row`), `user.rs` (balance/`UserRow` in micro-coins, check-in + cascade), `fruit.rs`, `buffer.rs`, `meta.rs` (pause flag), `chats.rs` (seen chats + group adder), `referral.rs` (`set_referrer_if_new`/`count_referrals`), `games.rs`. All sub-files add methods to the same `impl Database` block. Balances are `i64` micro-coins (see the money-model note above).

## Non-obvious gotchas

- **`InlineKeyboardButton::new(text, pay)`** takes two args. The second is `pay: bool` (a Telegram payment-button flag) — telexide's `#[build_struct]` macro promoted it to mandatory because it isn't `Option<T>`. Pass `false` unless you're actually building a payment button.
- **`rand::thread_rng()`** returns a non-`Send` `ThreadRng` — scope it in a block that ends before any `.await`, or the `#[command]` future fails the `Send` bound.
- **The `_COMMAND` statics are pub-glob-reexported** (`pub use start::*`) so `create_framework!(name, start, status, ...)` can resolve them at the bot.rs call site. Don't move command fns into private modules or the macro expansion will fail to resolve `<name>_COMMAND`.

## Configuration

Env vars consumed by `BotConfig::from_env` (see `src/types.rs`): `BOT_TOKEN` / `BOT_OWNER` (required) and optional `BOT_DEV` (default `true`). `BOT_DEV` also selects the SQLite data file via `database::db_filename` — `waterx-dev.db` (dev, default) vs `waterx.db` (production). Both match the `*.db` gitignore rule. `bot.rs::run` calls `BotConfig::from_env`. `main` picks the dotenv file via the `ENV_FILE` var: unset → default `.env` lookup (production); set → that file (`ENV_FILE=.env.dev cargo run` for the dev bot). The repo ships two gitignored configs — `.env` (production, `BOT_DEV=false`) and `.env.dev` (development, `BOT_DEV=true`) — with `.env.example` / `.env.dev.example` as their tracked templates. Dev and production **must** use different bot tokens: one Telegram token can't be polled by two running bots at once (getUpdates conflicts).
