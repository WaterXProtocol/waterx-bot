mod buffer;
mod fruit;
mod games;
mod user;

pub use buffer::OfferOutcome;
pub use user::UserRow;

use parking_lot::Mutex;
use rusqlite::{params, Connection, Result as SqlResult};
use std::time::{SystemTime, UNIX_EPOCH};

/// The on-disk SQLite filename. Fixed so the data file is predictable
/// across deploys and easy to gitignore.
pub const DB_FILENAME: &str = "waterx.db";

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
                user    INTEGER PRIMARY KEY,
                balance INTEGER NOT NULL DEFAULT 0,
                fruit   TEXT    NOT NULL DEFAULT '',
                cloth   TEXT    NOT NULL DEFAULT ''
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
        // Best-effort migrations for older buffer tables (predate escrow / TTL).
        // Each statement errors with "duplicate column name" if the column
        // already exists; we swallow those errors.
        let _ = conn.execute("ALTER TABLE buffer ADD COLUMN kind TEXT NOT NULL DEFAULT 'envelope'", []);
        let _ = conn.execute("ALTER TABLE buffer ADD COLUMN owner INTEGER", []);
        let _ = conn.execute("ALTER TABLE buffer ADD COLUMN fruits TEXT", []);
        let _ = conn.execute("ALTER TABLE buffer ADD COLUMN price INTEGER", []);
        let _ = conn.execute("ALTER TABLE buffer ADD COLUMN created_at INTEGER NOT NULL DEFAULT 0", []);

        // Prune buffer rows older than 24h. Pre-TTL rows have created_at=0 and
        // get cleared too, which is what we want — a restart drops all dangling
        // sell/buy offers and stale envelopes (their fruit/coin escrow stays
        // with the original owner because escrow rows hold the data inline).
        // Actually wait: escrow is held *inside* the buffer row itself, so
        // pruning would silently delete escrowed fruit/coin. Refund instead.
        let cutoff = current_unix_time() - BUFFER_TTL_SECS;
        Self::refund_and_prune_old_buffer(&conn, cutoff)?;

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
        for (_chat, _msg, kind, owner, fruits, price) in &rows {
            match (kind.as_str(), owner, fruits, price) {
                ("sell", Some(seller), Some(fruits), Some(_)) => {
                    // Return escrowed fruit to the seller.
                    conn.execute(
                        "INSERT OR IGNORE INTO balance (user, balance, fruit, cloth) VALUES (?1, 0, '', '')",
                        params![seller],
                    )?;
                    let current: String = conn.query_row(
                        "SELECT fruit FROM balance WHERE user = ?1",
                        params![seller],
                        |r| r.get(0),
                    )?;
                    let new = format!("{current}{fruits}");
                    conn.execute(
                        "UPDATE balance SET fruit = ?1 WHERE user = ?2",
                        params![new, seller],
                    )?;
                }
                ("buy", Some(buyer), _, Some(price)) => {
                    conn.execute(
                        "INSERT OR IGNORE INTO balance (user, balance, fruit, cloth) VALUES (?1, 0, '', '')",
                        params![buyer],
                    )?;
                    conn.execute(
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
        conn.execute("DELETE FROM buffer WHERE created_at < ?1", params![cutoff])?;
        Ok(())
    }

    pub(super) fn ensure_row(&self, user_id: i64) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR IGNORE INTO balance (user, balance, fruit, cloth) VALUES (?1, 0, '', '')",
            params![user_id],
        )?;
        Ok(())
    }
}
