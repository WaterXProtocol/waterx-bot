use super::Database;
use rusqlite::{params, Result as SqlResult};

impl Database {
    /// Record `referrer` as the inviter of `referee`, but only when:
    ///   - they differ and `referrer` is a positive id,
    ///   - `referrer` is already a known user (they got their link from the bot),
    ///   - `referee` is brand new (no `balance` row yet).
    ///
    /// Returns `true` only when the referral was newly recorded (so the caller
    /// rewards the referrer exactly once). Creates the referee's row with the
    /// referrer set; other columns take their schema defaults.
    pub fn set_referrer_if_new(&self, referee: i64, referrer: i64) -> SqlResult<bool> {
        if referrer <= 0 || referrer == referee {
            return Ok(false);
        }
        let conn = self.conn.lock();
        let referrer_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM balance WHERE user = ?1)",
            params![referrer],
            |r| r.get(0),
        )?;
        if !referrer_exists {
            return Ok(false);
        }
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO balance (user, referrer) VALUES (?1, ?2)",
            params![referee, referrer],
        )?;
        Ok(inserted == 1)
    }

    /// How many users were referred by `user`.
    pub fn count_referrals(&self, user: i64) -> SqlResult<i64> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT COUNT(*) FROM balance WHERE referrer = ?1",
            params![user],
            |r| r.get(0),
        )
    }
}
