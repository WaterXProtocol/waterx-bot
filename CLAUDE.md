# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A Telegram bot for a small private group, written in Rust on top of [`telexide`](https://docs.rs/telexide). The slash commands are localized utilities: balance/fruit ledger, coin/fruit transfers, host-run bet games, fruit trading, daily check-in, and a prediction-market brief. State persists in a SQLite file in the working directory, chosen by the `BOT_DEV` flag via `database::db_filename(dev)`: `waterx-dev.db` for development (the default) and `waterx.db` for production (`BOT_DEV=false`) — so a dev bot never clobbers live balances. Configuration comes from environment variables (loaded from `.env` if present): `BOT_TOKEN`, `BOT_OWNER` (numeric Telegram user id), and optional `BOT_DEV` (default `true`; set `false` for production).

The current public command set is `start, balance, fruit, send, host, sell, buy, markets, checkin`, plus owner-only admin commands `mint, pause, unpause, broadcast` (gated on `BOT_OWNER` via `util::is_owner`; non-owners are silently ignored and these are deliberately kept out of the `/` command menu). `/mint <amt>` credits the replied-to user (negative burns); `/pause`/`/unpause` toggle a persisted kill-switch (`meta` table) that `util::paused_block` enforces at the top of every non-admin command and `callbacks::on_callback` enforces for button presses — the owner always passes through; `/broadcast <msg>` posts to every chat the bot has seen — private DMs **and** groups (`Database::all_chat_ids`), skipping failures. Chats are recorded into the `chats` table via `Database::touch_chat`, called from `util::paused_block` (top of every non-admin command) and `callbacks::on_callback`; group/channel ids are negative, private-chat ids positive (== user id). `/start` is the button-driven entry point: a first-time user is shown a language picker, and once a locale is chosen (persisted to `balance.lang`) the bot opens the Xaliah main menu — an intro line, the caller's current balance + fruit inventory (`menu::menu_text`), plus inline buttons that fire the `setlang:` / `menu:` callbacks: always `[today's matches]`, and `[daily check-in]` **only when the reward is currently claimable** (`Database::checkin_available`; the button is dropped after a claim). The DB schema is `balance(user, balance, fruit, last_checkin, lang)` + `buffer` + `bet_games` + `meta` (key/value bot-wide flags, currently the `paused` kill-switch via `Database::{is_paused, set_paused}`) + `chats` (every chat the bot has seen, for `/broadcast`). `/checkin` grants 10 water-coins once per UTC day — `last_checkin` stores the last claimed UTC day index (`unix_secs / 86400`), so the window resets exactly at 00:00 UTC (see `Database::try_checkin`). A vestigial `cloth` column was dropped (a startup migration `ALTER TABLE balance DROP COLUMN cloth` cleans up old data files); a `fruit_pop` helper from the prior larger command set still lingers.

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

Telegram callback queries (inline-button presses) are *not* commands. They come through `src/commands/callbacks.rs::on_callback`, a `#[prepare_listener]` registered via `add_handler_func` in `bot.rs`. It matches `UpdateContent::CallbackQuery` and dispatches on the `cb.data` string prefix: `envelope:`, `gamble:`, `sell:`, `buy:`, `setlang:`, `menu:checkin`, `menu:matches`. The last three drive the `/start` menu (defined in `src/commands/menu.rs`): `setlang:<store_code>` saves the locale and edits the picker into the main menu in place; `menu:checkin` grants the daily reward as an alert and refreshes the menu to drop the now-spent button; `menu:matches` posts the match brief (via the shared `markets::brief`) as a fresh message.

There is no trace/info logging — only genuine errors are written to stderr via `eprintln!` (DB/save failures, getUpdates/parse errors, setMyCommands failures, markets fetch failures). Keep new logging to error paths only.

This matters because `/sell` and `/buy` slash commands **don't transact** — they just post an inline keyboard with a `sell:<seller>:<fruits>:<price>` or `buy:<buyer>:<fruits>:<price>` payload. The actual fruit/coin exchange happens when the counterparty taps the button and the callback fires.

The `envelope:` callback prefix is still routed even though the `/envelope` command was removed: `/send <amount>` with no reply target (or replying to the bot) posts a red-envelope-style claim button, and that share path uses the same callback.

### Bet games

`/host <desc> <opt1> <opt2> ...` (fn `host` in `src/commands/host.rs`) creates a `BetGame`, stores it under `{chat_id}:{message_id}`, and posts an inline keyboard. All bet activity (place, close, settle, draw) flows through `gamble:` callbacks (the internal callback-data prefix kept its old name; only the user-facing command was renamed). Settlement writes balances to `Database` from the callback handler, not from the game struct itself.

### Internationalisation (`src/i18n.rs`)

`Lang` has 18 variants. Most user-facing strings are fully localized into 15
locales (English + Traditional & Simplified Chinese + Japanese, Korean, Russian,
French, Spanish, German, Vietnamese, Indonesian, Filipino, Thai, Dutch, Turkish).
The module is dependency-free: a `Lang` enum, a `tr!` macro that picks one of 15
literal arms in a fixed order (`en, hant, hans, ja, ko, ru, fr, es, de, vi, id,
fil, th, nl, tr`), and one `pub fn` per message so all locales for a message sit
together. Parameterised messages leave `{token}` placeholders in every arm and
substitute with `.replace(...)` (real `format!` can't take a runtime format
string).

**Português (`pt`), हिन्दी (`hi`) and العربية (`ar`)** were added as `Lang`
variants for the picker, auto-detect (`from_code`) and command menus, but their
message bodies are **not translated yet** — the `tr!` macro maps `Pt | Hi | Ar`
to the English (`$en`) arm as a fallback. To translate, extend the macro arm
order and add the literals per message (or special-case in `tr!`). The picker
display order is driven by `Lang::ALL` (chunked two-per-row in
`menu::lang_picker_rows`), *not* the `tr!`/`native_label` arm order.

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

Adding a message: add a `pub fn` with all 15 `tr!` arms; the
`no_unfilled_placeholders_in_any_locale` test catches any arm that drops a token.

### `/markets` — read-only external feed

`src/commands/markets.rs` is the one command that talks to an outside service
rather than the local DB. It `GET`s `https://api.waterx.app/predict/browse`
(via `reqwest` with `rustls-tls`, reusing the rustls already in the tree) and
renders a Jupiter-style "market brief": sport matches only, **kicking off
within the next 24h or already live** (team vs team, kickoff time, and
per-outcome **decimal odds** = `100/oddsCents`, e.g. 65¢ → 1.54) — the feed's
crypto up/down pools are filtered out. It never touches the
water-coin ledger. Two gotchas baked into the structs: the response models only
the fields the brief reads (serde ignores the rest), and `oddsCents` **must** be
`f64` — some rounds report fractional cents (e.g. `99.9`), which would make an
`i64` field fail to deserialize the entire feed. The locale query param is the
caller's `Lang::menu_code()`; the API localizes Chinese and falls back to
English for anything it doesn't support, so any locale is safe to send.

### Database

`src/database/` wraps a single `rusqlite::Connection` in a `parking_lot::Mutex` so `Database` is `Send + Sync` and can sit behind an `Arc`. Two tables: `balance(user, balance, fruit)` and `buffer(chat, msg)` — the latter tracks live envelope/sell/buy messages so a callback can detect "someone already took this." The module is split by concern: `mod.rs` (struct + schema + `ensure_row` helper), `user.rs` (balance/`UserRow`), `fruit.rs`, `buffer.rs`. All sub-files add methods to the same `impl Database` block.

## Non-obvious gotchas

- **`InlineKeyboardButton::new(text, pay)`** takes two args. The second is `pay: bool` (a Telegram payment-button flag) — telexide's `#[build_struct]` macro promoted it to mandatory because it isn't `Option<T>`. Pass `false` unless you're actually building a payment button.
- **`rand::thread_rng()`** returns a non-`Send` `ThreadRng` — scope it in a block that ends before any `.await`, or the `#[command]` future fails the `Send` bound.
- **The `_COMMAND` statics are pub-glob-reexported** (`pub use start::*`) so `create_framework!(name, start, balance, ...)` can resolve them at the bot.rs call site. Don't move command fns into private modules or the macro expansion will fail to resolve `<name>_COMMAND`.

## Configuration

Env vars consumed by `BotConfig::from_env` (see `src/types.rs`): `BOT_TOKEN` / `BOT_OWNER` (required) and optional `BOT_DEV` (default `true`). `BOT_DEV` also selects the SQLite data file via `database::db_filename` — `waterx-dev.db` (dev, default) vs `waterx.db` (production). Both match the `*.db` gitignore rule. `bot.rs::run` calls `BotConfig::from_env`. `main` picks the dotenv file via the `ENV_FILE` var: unset → default `.env` lookup (production); set → that file (`ENV_FILE=.env.dev cargo run` for the dev bot). The repo ships two gitignored configs — `.env` (production, `BOT_DEV=false`) and `.env.dev` (development, `BOT_DEV=true`) — with `.env.example` / `.env.dev.example` as their tracked templates. Dev and production **must** use different bot tokens: one Telegram token can't be polled by two running bots at once (getUpdates conflicts).
