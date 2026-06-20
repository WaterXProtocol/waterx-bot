mod buffer;
mod chats;
mod fruit;
mod games;
mod meta;
mod referral;
mod stats;
mod user;
mod wager;

pub use buffer::OfferOutcome;
pub use stats::BotStats;
pub use user::UserRow;
pub use wager::{decimal_payout, OpenMarket, Position, Settlement};

use parking_lot::Mutex;
use rusqlite::{params, Connection, Result as SqlResult};
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
        conn.execute(
            "CREATE TABLE IF NOT EXISTS bet_games (
                id         TEXT    NOT NULL PRIMARY KEY,
                blob       TEXT    NOT NULL,
                created_at INTEGER NOT NULL
            )",
            [],
        )?;
        // Small key/value table for bot-wide flags (currently just the admin
        // `paused` kill-switch). Persisted so a pause survives restarts.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS meta (
                key   TEXT NOT NULL PRIMARY KEY,
                value TEXT NOT NULL
            )",
            [],
        )?;
        // Real-money match bets. `stake` is micro-coins; `odds_cents` is the
        // YES odds (cents) locked at placement; payout on a win is
        // `stake * 100 / odds_cents`. Settled manually by an admin.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS wagers (
                id         INTEGER PRIMARY KEY,
                user       INTEGER NOT NULL,
                market_id  TEXT    NOT NULL,
                slug       TEXT    NOT NULL DEFAULT '',
                team_a     TEXT    NOT NULL DEFAULT '',
                team_b     TEXT    NOT NULL DEFAULT '',
                outcome    TEXT    NOT NULL,
                stake      INTEGER NOT NULL,
                odds_cents REAL    NOT NULL,
                placed_at  INTEGER NOT NULL,
                ends_at    INTEGER NOT NULL DEFAULT 0,
                status     TEXT    NOT NULL DEFAULT 'open',
                settled_at INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;
        // Every chat the bot has been active in, so `/broadcast` can reach both
        // private DMs (positive ids) and groups/channels (negative ids).
        conn.execute(
            "CREATE TABLE IF NOT EXISTS chats (
                chat     INTEGER NOT NULL PRIMARY KEY,
                seen_at  INTEGER NOT NULL DEFAULT 0,
                added_by INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;
        // `added_by` (the user who added the bot to this group) for older DBs.
        let _ = conn.execute(
            "ALTER TABLE chats ADD COLUMN added_by INTEGER NOT NULL DEFAULT 0",
            [],
        );
        // Best-effort migrations for older buffer tables (predate escrow / TTL).
        // Each statement errors with "duplicate column name" if the column
        // already exists; we swallow those errors.
        let _ = conn.execute("ALTER TABLE buffer ADD COLUMN kind TEXT NOT NULL DEFAULT 'envelope'", []);
        let _ = conn.execute("ALTER TABLE buffer ADD COLUMN owner INTEGER", []);
        let _ = conn.execute("ALTER TABLE buffer ADD COLUMN fruits TEXT", []);
        let _ = conn.execute("ALTER TABLE buffer ADD COLUMN price INTEGER", []);
        let _ = conn.execute("ALTER TABLE buffer ADD COLUMN created_at INTEGER NOT NULL DEFAULT 0", []);
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
        let _ = conn.execute(
            "ALTER TABLE balance ADD COLUMN lang TEXT NOT NULL DEFAULT ''",
            [],
        );
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
        let mut stmt = conn.prepare(
            "SELECT chat, msg, kind, owner, fruits, price FROM buffer WHERE created_at < ?1",
        )?;
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
                _ => {
                    // envelope or malformed row — nothing to refund (envelope
                    // amount lives in the callback_data, not the row).
                }
            }
        }
        tx.execute("DELETE FROM buffer WHERE created_at < ?1", params![cutoff])?;
        tx.commit()?;
        Ok(())
    }

    /// Wipe every table (dev-only `/reset`). In-memory bet games must be
    /// cleared separately by the caller (they live in `GamesKey`, not the DB).
    pub fn reset_all(&self) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute_batch(
            "DELETE FROM balance;
             DELETE FROM buffer;
             DELETE FROM bet_games;
             DELETE FROM meta;
             DELETE FROM chats;
             DELETE FROM wagers;",
        )
    }

    pub(super) fn ensure_row(&self, user_id: i64) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR IGNORE INTO balance (user, balance, fruit) VALUES (?1, 0, '')",
            params![user_id],
        )?;
        Ok(())
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
        db.set_lang(100, crate::i18n::Lang::Hans).unwrap();
        db.insert_buffer(-200, 5, 100, 10).unwrap();
        db.touch_chat(-200).unwrap();
        db.set_group_adder(-200, 100).unwrap();
        db.set_paused(true).unwrap();

        let total = |db: &Database| -> i64 {
            db.conn
                .lock()
                .query_row(
                    "SELECT (SELECT COUNT(*) FROM balance)
                          + (SELECT COUNT(*) FROM buffer)
                          + (SELECT COUNT(*) FROM bet_games)
                          + (SELECT COUNT(*) FROM meta)
                          + (SELECT COUNT(*) FROM chats)",
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
}
