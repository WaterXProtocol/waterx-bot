# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A Telegram bot for a small private group, written in Rust on top of [`telexide`](https://docs.rs/telexide). Each slash command is a Chinese-language utility (random pickers, balance/fruit/cloth ledger, betting games, fruit trading). State persists in a local SQLite file named after `cfg.name`. Configuration comes from environment variables (loaded from `.env` if present): `BOT_TOKEN`, `BOT_OWNER` (numeric Telegram user id), `BOT_NAME`, and optional `BOT_DEV` (`true`/`false`, default `false`).

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

`bot.rs` defines three keys (`DbKey`, `GamesKey`, `ConfigKey`). Handlers reach state through helpers in `src/commands/util.rs`:

- `db(&ctx) -> Arc<Database>`
- `games(&ctx) -> Arc<tokio::sync::Mutex<HashMap<String, BetGame>>>`
- `config(&ctx) -> Arc<BotConfig>`

These helpers also encapsulate the **Send-across-await gotcha**: `ctx.data.read()` returns a `parking_lot::RwLockReadGuard` that is **not `Send`**, so it cannot be held across `.await`. The helpers grab the value and let the guard drop on the same statement — always go through them rather than calling `ctx.data.read()` inline.

### Callback queries are routed separately

Telegram callback queries (inline-button presses) are *not* commands. They come through `src/commands/callbacks.rs::on_callback`, a `#[prepare_listener]` registered via `add_handler_func` in `bot.rs`. It matches `UpdateContent::CallbackQuery` and dispatches on the `cb.data` string prefix: `envelope:`, `gamble:`, `sell:`, `buy:`.

This matters because `/sell` and `/buy` slash commands **don't transact** — they just post an inline keyboard with a `sell:<seller>:<fruits>:<price>` or `buy:<buyer>:<fruits>:<price>` payload. The actual fruit/coin exchange happens when the counterparty taps the button and the callback fires.

### Bet games

`/gamble <desc> <opt1> <opt2> ...` creates a `BetGame`, stores it under `{chat_id}:{message_id}`, and posts an inline keyboard. All bet activity (place, close, settle, draw) flows through `gamble:` callbacks. Settlement writes balances to `Database` from the callback handler, not from the game struct itself.

### Database

`src/database/` wraps a single `rusqlite::Connection` in a `parking_lot::Mutex` so `Database` is `Send + Sync` and can sit behind an `Arc`. Two tables: `balance(user, balance, fruit, cloth)` and `buffer(chat, msg)` — the latter tracks live envelope/sell/buy messages so a callback can detect "someone already took this." The module is split by concern: `mod.rs` (struct + schema + `ensure_row` helper), `user.rs` (balance/cloth/`UserRow`), `fruit.rs`, `buffer.rs`. All sub-files add methods to the same `impl Database` block.

## Non-obvious gotchas

- **`InlineKeyboardButton::new(text, pay)`** takes two args. The second is `pay: bool` (a Telegram payment-button flag) — telexide's `#[build_struct]` macro promoted it to mandatory because it isn't `Option<T>`. Pass `false` unless you're actually building a payment button.
- **`rand::thread_rng()`** returns a non-`Send` `ThreadRng` — scope it in a block that ends before any `.await`, or the `#[command]` future fails the `Send` bound.
- **The `_COMMAND` statics are pub-glob-reexported** (`pub use start::*`) so `create_framework!(name, start, choose, ...)` can resolve them at the bot.rs call site. Don't move command fns into private modules or the macro expansion will fail to resolve `<name>_COMMAND`.
- **Owner gating**: commands like `/sleep`, `/status`, `/clear`, `/param`, `/reverse`, `/mint` early-return unless `is_owner(&ctx, uid)` (from `commands/util.rs`). `/mint` additionally requires `cfg.dev = true`.
- **Stubbed commands**: `dice` and `param` reply with a placeholder and contain a `TODO:` describing what's missing — `dice` needs telexide's dice-result polling, `param` needs runtime-mutable config fields.

## Configuration

Env vars consumed by `BotConfig::from_env` (see `src/types.rs`): `BOT_TOKEN` / `BOT_OWNER` / `BOT_NAME` (all required) and optional `BOT_DEV`. The SQLite database file is opened at the literal value of `BOT_NAME` (no extension). `bot.rs::run` calls `BotConfig::from_env` and `dotenvy::dotenv` is invoked from `main`. `.env.example` at the repo root is the template; copy it to `.env` (gitignored) and fill in real values.
