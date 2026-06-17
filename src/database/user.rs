use super::Database;
use crate::i18n::Lang;
use rusqlite::{params, Result as SqlResult};

#[derive(Debug, Default, Clone)]
pub struct UserRow {
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
    pub fn balance_change(&self, user_id: i64, change: i64) -> SqlResult<bool> {
        let info = self.get_user_info(user_id)?;
        if info.balance + change < 0 {
            return Ok(false);
        }
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE balance SET balance = balance + ?1 WHERE user = ?2",
            params![change, user_id],
        )?;
        Ok(true)
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

    /// Grants the daily check-in `reward` unless the user already claimed it
    /// today. "Today" is the UTC day index (`unix_secs / 86400`), so the
    /// window resets exactly at 00:00 UTC. Returns true if granted, false if
    /// already claimed today.
    pub fn try_checkin(&self, user_id: i64, reward: i64) -> SqlResult<bool> {
        self.ensure_row(user_id)?;
        let today = super::current_unix_time() / 86_400;
        let conn = self.conn.lock();
        let last: i64 = conn.query_row(
            "SELECT last_checkin FROM balance WHERE user = ?1",
            params![user_id],
            |r| r.get(0),
        )?;
        if last >= today {
            return Ok(false);
        }
        conn.execute(
            "UPDATE balance SET balance = balance + ?1, last_checkin = ?2 WHERE user = ?3",
            params![reward, today, user_id],
        )?;
        Ok(true)
    }

    /// Every known user id (one row per user who has interacted). Used by the
    /// admin `/broadcast` to DM each user in their private chat.
    pub fn all_user_ids(&self) -> SqlResult<Vec<i64>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT user FROM balance")?;
        let ids = stmt
            .query_map([], |r| r.get(0))?
            .collect::<SqlResult<Vec<i64>>>()?;
        Ok(ids)
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
