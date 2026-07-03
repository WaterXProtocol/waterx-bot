mod buffer;
mod chats;
mod dashboard;
mod event;
mod fruit;
mod history;
mod meta;
mod referral;
mod user;
mod wager;

pub use buffer::OfferOutcome;
pub use dashboard::Dashboard;
pub use event::{
    basis_for_sold, AmmBoard, ClaimKind, FundOutcome, LiquidityView, Payout, PositionView, SellContext,
    TradeOutcome, B_MEDIUM, FEE_BPS_DEFAULT, FEE_BPS_MAX, MIN_SEED,
};
pub use history::HistoryRow;
// Action tags for `/history` — re-exported so the command's label mapping shares
// the exact literals the record sites write (no drift).
pub(crate) use history::{
    HK_BUY, HK_CHECKIN, HK_CLAIM, HK_LP_FUND, HK_LP_RETURN, HK_MINT, HK_REFERRAL, HK_REFUND, HK_SELL,
    HK_SEND_IN, HK_SEND_OUT,
};
pub use user::UserRow;
pub use wager::decimal_payout;

use parking_lot::Mutex;
use rusqlite::{params, Connection, Result as SqlResult, Transaction};
use std::time::{SystemTime, UNIX_EPOCH};

/// On-disk SQLite filenames. Dev and production keep separate data files so a
/// development bot never reads or clobbers live balances. Both are covered by
/// the `*.db` gitignore rule. Selection is by the `BOT_DEV` flag — see
/// [`db_filename`].
pub const DB_FILENAME: &str = "waterx.db";
pub const DB_FILENAME_DEV: &str = "waterx-dev.db";

/// Balances are stored as integer micro-coins (6-decimal fixed-point): the DB
/// value `1_000_000` means 1 coin. `i64` (not `u64`) because balances can go
/// negative (debt / the overdraw guard). User-typed whole-coin amounts are
/// multiplied by `COIN` at the ledger boundary; balances are displayed with
/// `util::fmt_coins`.
pub const COIN: i64 = 1_000_000;

/// Pick the data file for the current run: `waterx-dev.db` when `dev` is set
/// (the default), `waterx.db` for production (`BOT_DEV=false`).
pub fn db_filename(dev: bool) -> &'static str {
    if dev {
        DB_FILENAME_DEV
    } else {
        DB_FILENAME
    }
}

/// Directory for the bot's **persistent** files — the SQLite DB and the
/// `/backup` snapshots. Set `DATA_DIR` to a mounted volume path on hosts with an
/// ephemeral filesystem (Railway, Fly, Docker) so the coin ledger survives a
/// redeploy; unset falls back to the current working directory (self-host + dev).
/// A trailing slash is trimmed.
pub fn data_dir() -> String {
    match std::env::var("DATA_DIR") {
        Ok(d) if !d.trim().is_empty() => d.trim_end_matches('/').to_string(),
        _ => ".".to_string(),
    }
}

/// Full path to the SQLite DB for this run — `<data_dir>/<db_filename>`.
pub fn db_path(dev: bool) -> String {
    format!("{}/{}", data_dir(), db_filename(dev))
}

/// Buffer rows are pruned this many seconds after creation, with their escrow
/// refunded to the original owner. Matches "drop stale offers/envelopes on
/// restart" semantics — restarts are rare enough that this is a cheap cleanup
/// without needing a background timer.
pub(super) const BUFFER_TTL_SECS: i64 = 24 * 3600;

pub(super) fn current_unix_time() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Atomically debit `amount` micro-coins from `user` within `tx`, guarded so the
/// balance can never go negative — a single conditional `UPDATE … WHERE balance −
/// ?1 >= 0` (no read-then-write TOCTOU, so concurrent debits can't overdraw).
/// Returns `true` iff the row was debited (the user could cover it). The
/// transaction-scoped twin of [`Database::balance_change`]'s debit; the single home
/// for the overdraw-safe guard every trade path (`transfer`, `create_amm_event`,
/// `add_liquidity`, `amm_buy`, `sourced_buy`) previously inlined.
pub(super) fn try_debit(tx: &Transaction, user: i64, amount: i64) -> SqlResult<bool> {
    let n = tx.execute(
        "UPDATE balance SET balance = balance - ?1 WHERE user = ?2 AND balance - ?1 >= 0",
        params![amount, user],
    )?;
    Ok(n == 1)
}

/// Credit `amount` micro-coins to `user` within `tx`, creating the balance row if
/// absent. The tx-scoped credit primitive shared across the engine, referral,
/// check-in, and envelope paths.
pub(super) fn credit(tx: &Transaction, user: i64, amount: i64) -> SqlResult<()> {
    tx.execute(
        "INSERT OR IGNORE INTO balance (user, balance, fruit) VALUES (?1, 0, '')",
        params![user],
    )?;
    tx.execute(
        "UPDATE balance SET balance = balance + ?1 WHERE user = ?2",
        params![amount, user],
    )?;
    Ok(())
}

/// [`credit`] `user`, then append the `/history` entry in the same tx — the tight
/// credit-then-record pair every money-in path (LP return, settle/claim, transfer,
/// referral, check-in, envelope) repeats. Callers that already `ensure_row` still
/// go through this; the extra `INSERT OR IGNORE` is a harmless no-op there.
pub(super) fn credit_and_log(
    tx: &Transaction,
    user: i64,
    amount: i64,
    kind: &str,
    event_id: Option<i64>,
    counter: Option<i64>,
) -> SqlResult<()> {
    credit(tx, user, amount)?;
    history::record(tx, user, kind, amount, event_id, counter)?;
    Ok(())
}

pub struct Database {
    conn: Mutex<Connection>,
    /// Bot's own Telegram user id. Fruit sent to this id is *consumed* by the
    /// bot rather than added to its inventory (see `fruit_transfer`).
    pub(super) bot_id: i64,
}

impl Database {
    pub fn new(db_name: &str, bot_id: i64) -> SqlResult<Self> {
        let conn = Connection::open(db_name)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS balance (
                user         INTEGER PRIMARY KEY,
                balance      INTEGER NOT NULL DEFAULT 0,
                fruit        TEXT    NOT NULL DEFAULT '',
                last_checkin INTEGER NOT NULL DEFAULT 0,
                lang         TEXT    NOT NULL DEFAULT '',
                referrer     INTEGER NOT NULL DEFAULT 0,
                co_referrer  INTEGER NOT NULL DEFAULT 0,
                tz_offset    INTEGER,
                odds_fmt     TEXT    NOT NULL DEFAULT ''
            )",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS buffer (
                chat       INTEGER NOT NULL,
                msg        INTEGER NOT NULL,
                kind       TEXT    NOT NULL DEFAULT 'envelope',
                owner      INTEGER,
                fruits     TEXT,
                price      INTEGER,
                created_at INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (chat, msg)
            )",
            [],
        )?;
        // Unified Polymarket-style market schema (events/markets/positions) — the
        // share-trading model. Schema owned by `event.rs`.
        Self::create_event_tables(&conn)?;
        // Append-only per-user action log backing the user-facing `/history`.
        Self::create_history_table(&conn)?;
        // Drop the dead legacy bet tables from any pre-rewrite data file (best-effort,
        // no-op on a fresh DB) — the share engine replaced both the fixed-odds
        // `wagers` and the pari-mutuel `games` system.
        for t in ["wagers", "games", "game_options", "game_stakes", "bet_games"] {
            let _ = conn.execute(&format!("DROP TABLE IF EXISTS {t}"), []);
        }
        // AMM `/predict` board location columns, for older event tables.
        let _ = conn.execute(
            "ALTER TABLE events ADD COLUMN card_chat INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE events ADD COLUMN card_msg INTEGER NOT NULL DEFAULT 0",
            [],
        );
        // Funding-stage columns (LP price-discovery), for older event tables:
        // `events.open_at` = when funding closes → trading opens; `markets.funded` =
        // per-outcome LP allocation that sets the opening price.
        let _ = conn.execute(
            "ALTER TABLE events ADD COLUMN open_at INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE markets ADD COLUMN funded INTEGER NOT NULL DEFAULT 0",
            [],
        );
        // Small key/value table for bot-wide flags (currently just the admin
        // `paused` kill-switch). Persisted so a pause survives restarts.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS meta (
                key   TEXT NOT NULL PRIMARY KEY,
                value TEXT NOT NULL
            )",
            [],
        )?;
        // Every chat the bot has been active in, so `/broadcast` can reach both
        // private DMs (positive ids) and groups/channels (negative ids).
        conn.execute(
            "CREATE TABLE IF NOT EXISTS chats (
                chat         INTEGER NOT NULL PRIMARY KEY,
                seen_at      INTEGER NOT NULL DEFAULT 0,
                added_by     INTEGER NOT NULL DEFAULT 0,
                owner        INTEGER NOT NULL DEFAULT 0,
                reply_thread INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;
        // `added_by` (the user who added the bot to this group) for older DBs.
        let _ = conn.execute(
            "ALTER TABLE chats ADD COLUMN added_by INTEGER NOT NULL DEFAULT 0",
            [],
        );
        // `owner` (the group's Telegram creator) — co-refers new members with the
        // adder, for older DBs.
        let _ = conn.execute(
            "ALTER TABLE chats ADD COLUMN owner INTEGER NOT NULL DEFAULT 0",
            [],
        );
        // `reply_thread` (the forum topic the bot is locked to via /onlyreplyhere;
        // 0 = reply anywhere), for older DBs.
        let _ = conn.execute(
            "ALTER TABLE chats ADD COLUMN reply_thread INTEGER NOT NULL DEFAULT 0",
            [],
        );
        // Who added a specific member to a group (a Telegram "new member" service
        // message). Consulted lazily at that member's first interaction to bind
        // them to the person who actually added them — taking priority over the
        // bot-adder (`chats.added_by`), with the group owner as the 0.5
        // co-referrer. See `referral::maybe_bind_group`.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS group_adds (
                chat   INTEGER NOT NULL,
                member INTEGER NOT NULL,
                adder  INTEGER NOT NULL,
                PRIMARY KEY (chat, member)
            )",
            [],
        )?;
        // `co_referrer` (the group owner, when distinct from the adder) — a
        // level-1 co-credit in the check-in cascade, for older DBs.
        let _ = conn.execute(
            "ALTER TABLE balance ADD COLUMN co_referrer INTEGER NOT NULL DEFAULT 0",
            [],
        );
        // Best-effort migrations for older buffer tables (predate escrow / TTL).
        // Each statement errors with "duplicate column name" if the column
        // already exists; we swallow those errors.
        let _ = conn.execute(
            "ALTER TABLE buffer ADD COLUMN kind TEXT NOT NULL DEFAULT 'envelope'",
            [],
        );
        let _ = conn.execute("ALTER TABLE buffer ADD COLUMN owner INTEGER", []);
        let _ = conn.execute("ALTER TABLE buffer ADD COLUMN fruits TEXT", []);
        let _ = conn.execute("ALTER TABLE buffer ADD COLUMN price INTEGER", []);
        let _ = conn.execute(
            "ALTER TABLE buffer ADD COLUMN created_at INTEGER NOT NULL DEFAULT 0",
            [],
        );
        // Drop the vestigial `cloth` column from older `balance` tables. Errors
        // (column absent on fresh DBs, or SQLite < 3.35 without DROP COLUMN) are
        // harmless and swallowed — the column simply stays unused if it can't go.
        let _ = conn.execute("ALTER TABLE balance DROP COLUMN cloth", []);
        // Add the daily-checkin tracker to older `balance` tables (stores the
        // last claimed UTC day index; errors with "duplicate column" on tables
        // that already have it, which we swallow).
        let _ = conn.execute(
            "ALTER TABLE balance ADD COLUMN last_checkin INTEGER NOT NULL DEFAULT 0",
            [],
        );
        // Add the persisted UI locale (empty = not yet chosen via /start).
        let _ = conn.execute("ALTER TABLE balance ADD COLUMN lang TEXT NOT NULL DEFAULT ''", []);
        // Add the referrer link (0 = joined without a referral).
        let _ = conn.execute(
            "ALTER TABLE balance ADD COLUMN referrer INTEGER NOT NULL DEFAULT 0",
            [],
        );
        // Add the user's UTC offset in minutes (NULL = not yet chosen).
        let _ = conn.execute("ALTER TABLE balance ADD COLUMN tz_offset INTEGER", []);
        // Add the user's odds display format (empty = Decimal default).
        let _ = conn.execute(
            "ALTER TABLE balance ADD COLUMN odds_fmt TEXT NOT NULL DEFAULT ''",
            [],
        );
        // Host timezone pinned per prediction, for rendering the deadline in the
        // builder's local time. Ignored once present / on fresh DBs.
        let _ = conn.execute(
            "ALTER TABLE games ADD COLUMN tz_offset INTEGER NOT NULL DEFAULT 0",
            [],
        );

        // Prune buffer rows older than 24h. Pre-TTL rows have created_at=0 and
        // get cleared too, which is what we want — a restart drops all dangling
        // sell/buy offers and stale envelopes (their fruit/coin escrow stays
        // with the original owner because escrow rows hold the data inline).
        // Actually wait: escrow is held *inside* the buffer row itself, so
        // pruning would silently delete escrowed fruit/coin. Refund instead.
        let cutoff = current_unix_time() - BUFFER_TTL_SECS;
        Self::refund_and_prune_old_buffer(&conn, cutoff)?;
        // NB: balances are stored directly in micro-coins (see `COIN`); there is
        // no startup rescale. An earlier `×COIN` "legacy migration" was removed —
        // it double-scaled balances whenever `/reset` wiped its `meta` guard flag.

        Ok(Self {
            conn: Mutex::new(conn),
            bot_id,
        })
    }

    fn refund_and_prune_old_buffer(conn: &Connection, cutoff: i64) -> SqlResult<()> {
        // (chat, msg, kind, owner, fruits, price)
        type BufferRefundRow = (i64, i64, String, Option<i64>, Option<String>, Option<i64>);
        let mut stmt =
            conn.prepare("SELECT chat, msg, kind, owner, fruits, price FROM buffer WHERE created_at < ?1")?;
        let rows: Vec<BufferRefundRow> = stmt
            .query_map(params![cutoff], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
            })?
            .collect::<SqlResult<Vec<_>>>()?;
        drop(stmt);
        // Refund every expired offer AND delete the rows in one transaction, so a
        // crash mid-refund can't leave rows behind to be refunded again next
        // startup (double-credit).
        let tx = conn.unchecked_transaction()?;
        for (_chat, _msg, kind, owner, fruits, price) in &rows {
            match (kind.as_str(), owner, fruits, price) {
                ("sell", Some(seller), Some(fruits), Some(_)) => {
                    // Return escrowed fruit to the seller.
                    tx.execute(
                        "INSERT OR IGNORE INTO balance (user, balance, fruit) VALUES (?1, 0, '')",
                        params![seller],
                    )?;
                    let current: String = tx.query_row(
                        "SELECT fruit FROM balance WHERE user = ?1",
                        params![seller],
                        |r| r.get(0),
                    )?;
                    let new = format!("{current}{fruits}");
                    tx.execute(
                        "UPDATE balance SET fruit = ?1 WHERE user = ?2",
                        params![new, seller],
                    )?;
                }
                ("buy", Some(buyer), _, Some(price)) => {
                    tx.execute(
                        "INSERT OR IGNORE INTO balance (user, balance, fruit) VALUES (?1, 0, '')",
                        params![buyer],
                    )?;
                    tx.execute(
                        "UPDATE balance SET balance = balance + ?1 WHERE user = ?2",
                        params![price, buyer],
                    )?;
                }
                ("envelope", Some(owner), _, Some(amount)) => {
                    // Unclaimed envelope past its TTL: the sender was debited at
                    // creation and the escrow lives in this row (`price`), so return
                    // it to them — the DELETE below would otherwise burn the coins.
                    // Mirrors the `buy` refund; the send_out logged at creation now
                    // has its matching refund.
                    tx.execute(
                        "INSERT OR IGNORE INTO balance (user, balance, fruit) VALUES (?1, 0, '')",
                        params![owner],
                    )?;
                    tx.execute(
                        "UPDATE balance SET balance = balance + ?1 WHERE user = ?2",
                        params![amount, owner],
                    )?;
                    history::record(&tx, *owner, HK_REFUND, *amount, None, None)?;
                }
                _ => {
                    // malformed row (missing owner/amount) — nothing to refund.
                }
            }
        }
        tx.execute("DELETE FROM buffer WHERE created_at < ?1", params![cutoff])?;
        tx.commit()?;
        Ok(())
    }

    /// Wipe every table (dev-only `/reset` → [Everything]) — balances, chats, meta
    /// flags, the buffer escrow, the group-add referral map, and the whole market
    /// engine (`events`/`markets`/`positions`).
    pub fn reset_all(&self) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute_batch(
            "DELETE FROM balance;
             DELETE FROM buffer;
             DELETE FROM meta;
             DELETE FROM chats;
             DELETE FROM group_adds;
             DELETE FROM positions;
             DELETE FROM markets;
             DELETE FROM events;
             DELETE FROM history;",
        )
    }

    /// Dev-only `/delete` — remove a single user's footprint so they count as
    /// **brand-new** for referral binding (which gates on a `balance` row's
    /// existence). Deletes their `balance` row and any open `positions` in one
    /// transaction. Returns `true` when a balance row existed (so the caller can
    /// report "not found" otherwise). It's a referral-test helper — it drops the
    /// positions outright (no settle/refund).
    pub fn delete_user(&self, user_id: i64) -> SqlResult<bool> {
        self.with_tx(|tx| {
            let deleted = tx.execute("DELETE FROM balance WHERE user = ?1", params![user_id])?;
            tx.execute("DELETE FROM positions WHERE user = ?1", params![user_id])?;
            tx.execute("DELETE FROM history WHERE user = ?1", params![user_id])?;
            // Forget who added them to any group, so they re-bind cleanly (via
            // either the member-adder or the bot-adder) on the next interaction.
            tx.execute("DELETE FROM group_adds WHERE member = ?1", params![user_id])?;
            Ok(deleted == 1)
        })
    }

    /// Snapshot every user with coins **or** fruit as `(user, micro_coins, fruit)`
    /// — what `/backup` (and the `[Everything]` reset) writes to disk and `/load`
    /// restores. A user holding only fruit (zero coins) is still captured.
    pub fn export_accounts(&self) -> SqlResult<Vec<(i64, i64, String)>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT user, balance, COALESCE(fruit, '') FROM balance \
             WHERE balance > 0 OR (fruit IS NOT NULL AND fruit != '')",
        )?;
        let v = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(v)
    }

    /// Restore coins + fruit from a `/load` backup: upsert each user's `balance`
    /// **and** `fruit` (creating the row if needed), leaving every other column
    /// untouched, in one transaction. Returns the number of accounts written.
    pub fn import_accounts(&self, rows: &[(i64, i64, String)]) -> SqlResult<usize> {
        self.with_tx(|tx| {
            for (user, balance, fruit) in rows {
                tx.execute(
                    "INSERT INTO balance (user, balance, fruit) VALUES (?1, ?2, ?3)
                     ON CONFLICT(user) DO UPDATE SET balance = excluded.balance, fruit = excluded.fruit",
                    params![user, balance, fruit],
                )?;
            }
            Ok(rows.len())
        })
    }

    pub(super) fn ensure_row(&self, user_id: i64) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR IGNORE INTO balance (user, balance, fruit) VALUES (?1, 0, '')",
            params![user_id],
        )?;
        Ok(())
    }

    /// Run `f` inside a single write transaction: **commit** on `Ok`, roll back on
    /// `Err`. Encapsulates the `lock → transaction → commit` ceremony every money
    /// mutation repeats. `f` receives `&Transaction` and must use only that (and
    /// free helpers like [`credit`] / [`try_debit`]) — it must **not** call a
    /// `&self` method that locks `self.conn` (the lock is already held: parking_lot
    /// mutexes aren't reentrant, so that would deadlock). An early `Ok(reject)` from
    /// `f` commits a transaction that made no writes — equivalent to a rollback, and
    /// the invariant every reject path in this engine already upholds (rejects fire
    /// before any write).
    pub(super) fn with_tx<T>(&self, f: impl FnOnce(&Transaction) -> SqlResult<T>) -> SqlResult<T> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }
}

#[cfg(test)]
mod reset_tests {
    use super::*;

    #[test]
    fn reset_all_clears_every_table() {
        let db = Database::new(":memory:", 1).unwrap();
        // Populate balance, buffer, chats (+ group adder) and meta.
        db.balance_change(100, 50).unwrap();
        db.set_lang(100, crate::core::i18n::Lang::Hans).unwrap();
        db.insert_buffer(-200, 5, 100, 10).unwrap();
        db.touch_chat(-200).unwrap();
        db.set_group_adder(-200, 100).unwrap();
        db.record_group_add(-200, 300, 100).unwrap();
        db.set_paused(true).unwrap();

        let total = |db: &Database| -> i64 {
            db.conn
                .lock()
                .query_row(
                    "SELECT (SELECT COUNT(*) FROM balance)
                          + (SELECT COUNT(*) FROM buffer)
                          + (SELECT COUNT(*) FROM events)
                          + (SELECT COUNT(*) FROM positions)
                          + (SELECT COUNT(*) FROM meta)
                          + (SELECT COUNT(*) FROM chats)
                          + (SELECT COUNT(*) FROM group_adds)",
                    [],
                    |r| r.get(0),
                )
                .unwrap()
        };

        assert!(total(&db) > 0, "expected rows before reset");
        db.reset_all().unwrap();
        assert_eq!(total(&db), 0, "every table should be empty after reset");
        assert!(!db.is_paused().unwrap(), "pause flag cleared");
    }

    #[test]
    fn export_then_import_round_trips_balances_and_fruit() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(10, 50 * COIN).unwrap();
        db.force_change(20, 7 * COIN).unwrap();
        db.force_change(30, 0).unwrap(); // zero coins…
        db.conn
            .lock()
            .execute("UPDATE balance SET fruit = '🍎🍌' WHERE user = 30", [])
            .unwrap(); // …but holds fruit
        db.force_change(40, 0).unwrap(); // zero coins + no fruit → not exported

        let snapshot = db.export_accounts().unwrap();
        assert_eq!(snapshot.len(), 3); // 10, 20 (coins) + 30 (fruit-only); 40 excluded
        assert!(snapshot.contains(&(10, 50 * COIN, String::new())));
        assert!(snapshot.contains(&(30, 0, "🍎🍌".to_string())));

        // Wipe, then restore from the snapshot.
        db.reset_all().unwrap();
        assert!(!db.user_exists(10).unwrap());
        let n = db.import_accounts(&snapshot).unwrap();
        assert_eq!(n, 3);
        assert_eq!(db.get_user_info(10).unwrap().balance, 50 * COIN);
        assert_eq!(db.get_user_info(20).unwrap().balance, 7 * COIN);
        let u30 = db.get_user_info(30).unwrap();
        assert_eq!(u30.balance, 0);
        assert_eq!(u30.fruit, "🍎🍌"); // fruit restored
    }

    #[test]
    fn delete_user_makes_one_user_brand_new_for_re_refer() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 0).unwrap(); // referrer exists
        assert!(db.set_referrer_if_new(2, 1).unwrap()); // referee binds once
        assert!(!db.set_referrer_if_new(2, 1).unwrap()); // existing row → no re-bind

        // Delete only the referee — the referrer is untouched.
        assert!(db.delete_user(2).unwrap());
        assert!(!db.user_exists(2).unwrap()); // brand-new again
        assert!(db.user_exists(1).unwrap()); // referrer still there
        assert!(!db.delete_user(2).unwrap()); // already gone → false

        // Re-refer now works immediately (referrer still exists).
        assert!(db.set_referrer_if_new(2, 1).unwrap());
    }

    #[test]
    fn delete_user_removes_their_positions() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(10, 100 * COIN).unwrap();
        db.conn
            .lock()
            .execute(
                "INSERT INTO positions (event_id, market_idx, user, shares, cost)
                 VALUES (1, 0, 10, 5000000, 3000000)",
                [],
            )
            .unwrap();

        assert!(db.delete_user(10).unwrap());
        assert!(!db.user_exists(10).unwrap());
        let left: i64 = db
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM positions WHERE user = 10", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0, "positions gone too");
    }

    #[test]
    fn reset_all_makes_users_brand_new_for_re_refer() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 0).unwrap(); // referrer exists
        assert!(db.set_referrer_if_new(2, 1).unwrap()); // referee binds once
        assert!(!db.set_referrer_if_new(2, 1).unwrap()); // existing row → no re-bind

        db.reset_all().unwrap(); // [Everything]: wipes balance (+ chats etc.)
        assert!(!db.user_exists(2).unwrap()); // brand-new again

        // Re-refer now works once the referrer is re-created (e.g. re-adding the
        // bot re-records the group adder; here we just recreate the row).
        db.force_change(1, 0).unwrap();
        assert!(db.set_referrer_if_new(2, 1).unwrap());
    }

    #[test]
    fn prune_refunds_unclaimed_envelope_escrow() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(7, 10 * COIN).unwrap();
        // Simulate a live envelope: the sender is debited and an escrow row is
        // written with an old timestamp (so the TTL sweep will collect it).
        assert!(db.balance_change(7, -4 * COIN).unwrap());
        {
            let conn = db.conn.lock();
            conn.execute(
                "INSERT INTO buffer (chat, msg, kind, owner, price, created_at)
                 VALUES (-1, 1, 'envelope', 7, ?1, 0)",
                rusqlite::params![4 * COIN],
            )
            .unwrap();
            // Sweep everything older than cutoff 100 (the row's created_at is 0).
            Database::refund_and_prune_old_buffer(&conn, 100).unwrap();
        }
        // Escrow returned to the sender, the buffer row is gone, and the refund
        // is logged to their /history.
        assert_eq!(db.get_user_info(7).unwrap().balance, 10 * COIN);
        let h = db.user_history(7, 10).unwrap();
        assert_eq!(h[0].kind, HK_REFUND);
        assert_eq!(h[0].delta, 4 * COIN);
    }
}
