mod buffer;
mod chats;
mod dashboard;
mod fruit;
mod games;
mod meta;
mod referral;
mod user;
mod wager;

pub use buffer::OfferOutcome;
pub use dashboard::Dashboard;
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
        // Self-host `/predict` games, normalized across games/game_options/
        // game_stakes (schema owned by `games.rs`). A one-time migration folds any
        // legacy JSON-blob `bet_games` rows in (and drops that table) below.
        Self::create_game_tables(&conn)?;
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

        // One-time: migrate legacy JSON-blob `/predict` games into the normalized
        // tables created above, then drop the old `bet_games` table. No-op once
        // migrated (the old table is gone) and on fresh DBs (it never existed).
        Self::migrate_blob_games(&conn)?;

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
             DELETE FROM games;
             DELETE FROM game_options;
             DELETE FROM game_stakes;
             DELETE FROM meta;
             DELETE FROM chats;
             DELETE FROM wagers;",
        )
    }

    /// Selective `/reset` — refund + clear all real-money **match bets**: credit
    /// every *open* wager's stake back to its bettor, then wipe the `wagers` table
    /// (open + settled history) in one transaction. Returns
    /// `(open_bets_refunded, micro_coins_refunded)`.
    pub fn reset_wagers(&self) -> SqlResult<(i64, i64)> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let open: Vec<(i64, i64)> = {
            let mut stmt = tx.prepare("SELECT user, stake FROM wagers WHERE status = 'open'")?;
            let v = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<SqlResult<Vec<_>>>()?;
            v
        };
        let mut refunded = 0i64;
        for (user, stake) in &open {
            tx.execute("INSERT OR IGNORE INTO balance (user, balance, fruit) VALUES (?1, 0, '')", params![user])?;
            tx.execute("UPDATE balance SET balance = balance + ?1 WHERE user = ?2", params![stake, user])?;
            refunded += stake;
        }
        tx.execute("DELETE FROM wagers", [])?;
        tx.commit()?;
        Ok((open.len() as i64, refunded))
    }

    /// Selective `/reset` — refund + clear all self-host **predictions**: credit
    /// every game stake (whole coins → micro) back to its bettor, then wipe the
    /// `games`/`game_options`/`game_stakes` tables in one transaction. The caller
    /// must also clear the in-memory `GamesKey` map. Returns
    /// `(games_cleared, micro_coins_refunded)`.
    pub fn reset_predictions(&self) -> SqlResult<(i64, i64)> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let games_cleared: i64 = tx.query_row("SELECT COUNT(*) FROM games", [], |r| r.get(0))?;
        let stakes: Vec<(i64, i64)> = {
            let mut stmt = tx.prepare("SELECT user, amount FROM game_stakes")?;
            let v = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<SqlResult<Vec<_>>>()?;
            v
        };
        let mut refunded = 0i64;
        for (user, amount) in &stakes {
            let micro = amount.saturating_mul(COIN);
            tx.execute("INSERT OR IGNORE INTO balance (user, balance, fruit) VALUES (?1, 0, '')", params![user])?;
            tx.execute("UPDATE balance SET balance = balance + ?1 WHERE user = ?2", params![micro, user])?;
            refunded += micro;
        }
        tx.execute("DELETE FROM game_stakes", [])?;
        tx.execute("DELETE FROM game_options", [])?;
        tx.execute("DELETE FROM games", [])?;
        tx.commit()?;
        Ok((games_cleared, refunded))
    }

    /// Selective `/reset` — zero every coin **balance** (the `balance` column),
    /// leaving each user's row and settings (lang/tz/referrer/odds_fmt) intact.
    /// Returns the number of accounts zeroed.
    pub fn reset_balances(&self) -> SqlResult<i64> {
        let conn = self.conn.lock();
        let n = conn.execute("UPDATE balance SET balance = 0", [])?;
        Ok(n as i64)
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
                          + (SELECT COUNT(*) FROM games)
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

    #[test]
    fn reset_wagers_refunds_open_and_clears() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(10, 100 * COIN).unwrap();
        // Stake 30 → balance 70, one open wager.
        assert!(db.place_wager(10, "m1", "s", "A", "B", "teamA", 30 * COIN, 200.0, 0).unwrap());
        assert_eq!(db.get_user_info(10).unwrap().balance, 70 * COIN);

        let (n, refunded) = db.reset_wagers().unwrap();
        assert_eq!(n, 1);
        assert_eq!(refunded, 30 * COIN);
        assert_eq!(db.get_user_info(10).unwrap().balance, 100 * COIN); // stake returned
        assert!(db.list_open_wagers(10).unwrap().is_empty()); // table cleared
    }

    #[test]
    fn reset_predictions_refunds_stakes_and_clears() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(10, 0).unwrap();
        db.force_change(20, 0).unwrap();
        let mut g = crate::game::BetGame::new(1, crate::i18n::Lang::En, "q", &["A", "B"]);
        g.set_id(5, 5);
        g.stake(10, "A", 4, "Ann");
        g.stake(20, "B", 6, "Bob");
        db.save_bet_game(&g).unwrap();

        let (games_cleared, refunded) = db.reset_predictions().unwrap();
        assert_eq!(games_cleared, 1);
        assert_eq!(refunded, 10 * COIN); // (4 + 6) whole coins → micro
        assert_eq!(db.get_user_info(10).unwrap().balance, 4 * COIN);
        assert_eq!(db.get_user_info(20).unwrap().balance, 6 * COIN);
        assert!(db.load_all_bet_games().unwrap().is_empty());
    }

    #[test]
    fn reset_balances_zeroes_coins_keeps_rows_and_settings() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(10, 50 * COIN).unwrap();
        db.set_lang(10, crate::i18n::Lang::Hans).unwrap();

        let n = db.reset_balances().unwrap();
        assert_eq!(n, 1);
        assert_eq!(db.get_user_info(10).unwrap().balance, 0);
        // Row + settings survive (only the coin balance is zeroed).
        assert_eq!(db.get_lang(10).unwrap(), Some(crate::i18n::Lang::Hans));
        // Zeroing keeps the row, so a referral still can't re-bind.
        assert!(db.user_exists(10).unwrap());
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
}
