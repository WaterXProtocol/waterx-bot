use super::Database;
use crate::i18n::Lang;
use rusqlite::{params, OptionalExtension, Result as SqlResult};

/// Micro-coins paid up the referral chain on each successful check-in: direct
/// referrer 1 coin, then 0.1 and 0.01 for the two levels above.
const CHECKIN_UPLINE: [i64; 3] = [super::COIN, super::COIN / 10, super::COIN / 100];

#[derive(Debug, Default, Clone)]
pub struct UserRow {
    /// Balance in micro-coins (6-decimal fixed-point; see `database::COIN`).
    pub balance: i64,
    pub fruit: String,
}

impl Database {
    pub fn get_user_info(&self, user_id: i64) -> SqlResult<UserRow> {
        self.ensure_row(user_id)?;
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT balance, fruit FROM balance WHERE user = ?1")?;
        let row = stmt.query_row(params![user_id], |r| {
            Ok(UserRow {
                balance: r.get(0)?,
                fruit: r.get(1)?,
            })
        })?;
        Ok(row)
    }

    /// Adds `change` to user balance. Returns false when the operation would
    /// push balance below zero (and nothing is written).
    ///
    /// The guard and the write are a **single atomic statement** (the
    /// `WHERE balance + ?1 >= 0` clause), so two concurrent debits can't both
    /// pass a stale read and overdraw the account (the previous read-then-write
    /// split had that TOCTOU race).
    pub fn balance_change(&self, user_id: i64, change: i64) -> SqlResult<bool> {
        self.ensure_row(user_id)?;
        let conn = self.conn.lock();
        let affected = conn.execute(
            "UPDATE balance SET balance = balance + ?1 WHERE user = ?2 AND balance + ?1 >= 0",
            params![change, user_id],
        )?;
        Ok(affected == 1)
    }

    /// Atomically move `amount` (micro-coins, must be ≥ 0) from `from` to `to`
    /// in one transaction: a single failure rolls back both legs, so coins can
    /// never be debited without being credited (or vice-versa). Returns false
    /// (writing nothing) when `from` can't cover `amount`.
    pub fn transfer(&self, from: i64, to: i64, amount: i64) -> SqlResult<bool> {
        self.ensure_row(from)?;
        self.ensure_row(to)?;
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let debited = tx.execute(
            "UPDATE balance SET balance = balance - ?1 WHERE user = ?2 AND balance - ?1 >= 0",
            params![amount, from],
        )?;
        if debited != 1 {
            return Ok(false); // tx drops here → rollback, nothing written
        }
        tx.execute(
            "UPDATE balance SET balance = balance + ?1 WHERE user = ?2",
            params![amount, to],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Credit `amount` (micro-coins) to **both** `a` and `b` in one transaction
    /// — used for the referral signup reward so the referrer and referee are
    /// paid atomically (both or neither). No non-negative guard: it's a credit.
    pub fn reward_referral(&self, a: i64, b: i64, amount: i64) -> SqlResult<()> {
        self.ensure_row(a)?;
        self.ensure_row(b)?;
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE balance SET balance = balance + ?1 WHERE user = ?2",
            params![amount, a],
        )?;
        tx.execute(
            "UPDATE balance SET balance = balance + ?1 WHERE user = ?2",
            params![amount, b],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The user's persisted UI locale, or `None` if they haven't picked one
    /// via `/start` yet.
    pub fn get_lang(&self, user_id: i64) -> SqlResult<Option<Lang>> {
        self.ensure_row(user_id)?;
        let conn = self.conn.lock();
        let code: String = conn.query_row(
            "SELECT lang FROM balance WHERE user = ?1",
            params![user_id],
            |r| r.get(0),
        )?;
        Ok(Lang::from_store_code(&code))
    }

    /// Persist the user's chosen UI locale.
    pub fn set_lang(&self, user_id: i64, lang: Lang) -> SqlResult<()> {
        self.ensure_row(user_id)?;
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE balance SET lang = ?1 WHERE user = ?2",
            params![lang.store_code(), user_id],
        )?;
        Ok(())
    }

    /// The user's UTC offset in minutes east, or `None` if they haven't picked
    /// one yet (so `/start` can prompt once).
    pub fn get_tz(&self, user_id: i64) -> SqlResult<Option<i64>> {
        self.ensure_row(user_id)?;
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT tz_offset FROM balance WHERE user = ?1",
            params![user_id],
            |r| r.get::<_, Option<i64>>(0),
        )
    }

    pub fn set_tz(&self, user_id: i64, minutes: i64) -> SqlResult<()> {
        self.ensure_row(user_id)?;
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE balance SET tz_offset = ?1 WHERE user = ?2",
            params![minutes, user_id],
        )?;
        Ok(())
    }

    /// Whether the user can claim today's check-in (read-only; grants nothing).
    /// Mirrors [`Database::try_checkin`]'s UTC-day window.
    pub fn checkin_available(&self, user_id: i64) -> SqlResult<bool> {
        self.ensure_row(user_id)?;
        let today = super::current_unix_time() / 86_400;
        let conn = self.conn.lock();
        let last: i64 = conn.query_row(
            "SELECT last_checkin FROM balance WHERE user = ?1",
            params![user_id],
            |r| r.get(0),
        )?;
        Ok(last < today)
    }

    /// Grants the daily check-in `reward` unless the user already claimed it
    /// today. "Today" is the UTC day index (`unix_secs / 86400`), so the
    /// window resets exactly at 00:00 UTC. Returns true if granted, false if
    /// already claimed today.
    pub fn try_checkin(&self, user_id: i64, reward: i64) -> SqlResult<bool> {
        self.ensure_row(user_id)?;
        let today = super::current_unix_time() / 86_400;
        let mut conn = self.conn.lock();
        // One transaction over the reward + the referral cascade: a mid-cascade
        // failure must not leave the user credited (and the day consumed) while
        // an upline referrer goes unpaid.
        let tx = conn.transaction()?;
        let last: i64 = tx.query_row(
            "SELECT last_checkin FROM balance WHERE user = ?1",
            params![user_id],
            |r| r.get(0),
        )?;
        if last >= today {
            return Ok(false); // already claimed today → rollback (no writes)
        }
        tx.execute(
            "UPDATE balance SET balance = balance + ?1, last_checkin = ?2 WHERE user = ?3",
            params![reward, today, user_id],
        )?;
        // Referral cascade: pay the direct referrer and up to two levels above.
        let mut up: i64 = tx.query_row(
            "SELECT referrer FROM balance WHERE user = ?1",
            params![user_id],
            |r| r.get(0),
        )?;
        for bonus in CHECKIN_UPLINE {
            if up == 0 {
                break;
            }
            tx.execute(
                "UPDATE balance SET balance = balance + ?1 WHERE user = ?2",
                params![bonus, up],
            )?;
            up = tx
                .query_row(
                    "SELECT referrer FROM balance WHERE user = ?1",
                    params![up],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or(0);
        }
        tx.commit()?;
        Ok(true)
    }

    /// Whether a `balance` row already exists for `user_id` — **without** creating
    /// one (unlike `get_user_info`/`ensure_row`). Used by the group-referral bind
    /// to skip all work for users who already exist (only brand-new users bind).
    pub fn user_exists(&self, user_id: i64) -> SqlResult<bool> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM balance WHERE user = ?1)",
            params![user_id],
            |r| r.get(0),
        )
    }

    /// Owner-only: applies a change without the non-negative guard.
    pub fn force_change(&self, user_id: i64, change: i64) -> SqlResult<()> {
        self.ensure_row(user_id)?;
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE balance SET balance = balance + ?1 WHERE user = ?2",
            params![change, user_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::COIN;
    use super::*;

    #[test]
    fn balance_change_rejects_overdraw_and_writes_nothing() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 5 * COIN).unwrap();
        assert!(!db.balance_change(1, -6 * COIN).unwrap()); // would go negative
        assert_eq!(db.get_user_info(1).unwrap().balance, 5 * COIN); // untouched
        assert!(db.balance_change(1, -5 * COIN).unwrap()); // exact spend ok
        assert_eq!(db.get_user_info(1).unwrap().balance, 0);
    }

    #[test]
    fn transfer_is_atomic_and_guards_funds() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 10 * COIN).unwrap();
        assert!(db.transfer(1, 2, 4 * COIN).unwrap());
        assert_eq!(db.get_user_info(1).unwrap().balance, 6 * COIN);
        assert_eq!(db.get_user_info(2).unwrap().balance, 4 * COIN);
        // Insufficient → both balances unchanged (neither leg applied).
        assert!(!db.transfer(1, 2, 100 * COIN).unwrap());
        assert_eq!(db.get_user_info(1).unwrap().balance, 6 * COIN);
        assert_eq!(db.get_user_info(2).unwrap().balance, 4 * COIN);
    }

    #[test]
    fn reward_referral_credits_both_sides() {
        let db = Database::new(":memory:", 1).unwrap();
        db.reward_referral(1, 2, 10 * COIN).unwrap();
        assert_eq!(db.get_user_info(1).unwrap().balance, 10 * COIN);
        assert_eq!(db.get_user_info(2).unwrap().balance, 10 * COIN);
    }
}
