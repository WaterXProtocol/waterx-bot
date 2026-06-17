use super::Database;
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
