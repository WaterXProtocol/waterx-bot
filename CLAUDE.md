# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A Telegram bot for a small private group, written in Rust on top of [`telexide`](https://docs.rs/telexide). The slash commands are Chinese-language utilities: random picker, balance/fruit ledger, coin/fruit transfers, dice betting, bet games, and fruit trading. State persists in `waterx.db` (a SQLite file in the working directory; constant defined at `database::DB_FILENAME`). Configuration comes from environment variables (loaded from `.env` if present): `BOT_TOKEN`, `BOT_OWNER` (numeric Telegram user id), and optional `BOT_DEV` (default `true`; set `false` for production).

The current command set is `start, random, balance, fruit, send, dice, gamble, sell, buy`. The DB schema (`balance(user, balance, fruit, cloth)` + `buffer` + `bet_games`) still carries a `cloth` column and a `fruit_pop` helper from the prior larger command set — a future redesign will rework the schema.

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

Telegram callback queries (inline-button presses) are *not* commands. They come through `src/commands/callbacks.rs::on_callback`, a `#[prepare_listener]` registered via `add_handler_func` in `bot.rs`. It matches `UpdateContent::CallbackQuery` and dispatches on the `cb.data` string prefix: `envelope:`, `gamble:`, `sell:`, `buy:`.

This matters because `/sell` and `/buy` slash commands **don't transact** — they just post an inline keyboard with a `sell:<seller>:<fruits>:<price>` or `buy:<buyer>:<fruits>:<price>` payload. The actual fruit/coin exchange happens when the counterparty taps the button and the callback fires.

The `envelope:` callback prefix is still routed even though the `/envelope` command was removed: `/send <amount>` with no reply target (or replying to the bot) posts a red-envelope-style claim button, and that share path uses the same callback.

### Bet games

`/gamble <desc> <opt1> <opt2> ...` creates a `BetGame`, stores it under `{chat_id}:{message_id}`, and posts an inline keyboard. All bet activity (place, close, settle, draw) flows through `gamble:` callbacks. Settlement writes balances to `Database` from the callback handler, not from the game struct itself.

### Database

`src/database/` wraps a single `rusqlite::Connection` in a `parking_lot::Mutex` so `Database` is `Send + Sync` and can sit behind an `Arc`. Two tables: `balance(user, balance, fruit, cloth)` and `buffer(chat, msg)` — the latter tracks live envelope/sell/buy messages so a callback can detect "someone already took this." The module is split by concern: `mod.rs` (struct + schema + `ensure_row` helper), `user.rs` (balance/cloth/`UserRow`), `fruit.rs`, `buffer.rs`. All sub-files add methods to the same `impl Database` block.

## Non-obvious gotchas

- **`InlineKeyboardButton::new(text, pay)`** takes two args. The second is `pay: bool` (a Telegram payment-button flag) — telexide's `#[build_struct]` macro promoted it to mandatory because it isn't `Option<T>`. Pass `false` unless you're actually building a payment button.
- **`rand::thread_rng()`** returns a non-`Send` `ThreadRng` — scope it in a block that ends before any `.await`, or the `#[command]` future fails the `Send` bound.
- **The `_COMMAND` statics are pub-glob-reexported** (`pub use start::*`) so `create_framework!(name, start, random, ...)` can resolve them at the bot.rs call site. Don't move command fns into private modules or the macro expansion will fail to resolve `<name>_COMMAND`.

## Configuration

Env vars consumed by `BotConfig::from_env` (see `src/types.rs`): `BOT_TOKEN` / `BOT_OWNER` (required) and optional `BOT_DEV` (default `true`). The SQLite database file path is hardcoded — `database::DB_FILENAME = "waterx.db"`. `bot.rs::run` calls `BotConfig::from_env` and `dotenvy::dotenv` is invoked from `main`. `.env.example` at the repo root is the template; copy it to `.env` (gitignored) and fill in real values.
