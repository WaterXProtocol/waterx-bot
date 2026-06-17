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

#[cfg(test)]
mod tests {
    use super::super::COIN;
    use super::*;

    #[test]
    fn checkin_cascade_pays_three_levels_up() {
        let db = Database::new(":memory:", 1).unwrap();
        // Chain: 1 <- 2 <- 3 <- 4  (4 referred by 3, 3 by 2, 2 by 1)
        db.force_change(1, 0).unwrap(); // user 1 exists (top referrer)
        assert!(db.set_referrer_if_new(2, 1).unwrap());
        assert!(db.set_referrer_if_new(3, 2).unwrap());
        assert!(db.set_referrer_if_new(4, 3).unwrap());

        assert!(db.try_checkin(4, 10 * COIN).unwrap());
        assert_eq!(db.get_user_info(4).unwrap().balance, 10 * COIN); // own reward
        assert_eq!(db.get_user_info(3).unwrap().balance, COIN); // direct referrer +1
        assert_eq!(db.get_user_info(2).unwrap().balance, COIN / 10); // +0.1
        assert_eq!(db.get_user_info(1).unwrap().balance, COIN / 100); // +0.01
    }

    #[test]
    fn existing_user_does_not_rebind() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 0).unwrap();
        db.force_change(2, 0).unwrap(); // user 2 already exists
        assert!(!db.set_referrer_if_new(2, 1).unwrap()); // not new → no bind
        assert!(!db.set_referrer_if_new(1, 1).unwrap()); // self → no bind
    }
}
