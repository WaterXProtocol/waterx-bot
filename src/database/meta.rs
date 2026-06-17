use super::Database;
use rusqlite::{params, OptionalExtension, Result as SqlResult};

/// `meta` key for the admin pause kill-switch.
const PAUSED_KEY: &str = "paused";

impl Database {
    /// Whether the bot is currently paused by an admin (`/pause`).
    pub fn is_paused(&self) -> SqlResult<bool> {
        let conn = self.conn.lock();
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![PAUSED_KEY],
                |r| r.get(0),
            )
            .optional()?;
        Ok(value.as_deref() == Some("1"))
    }

    /// Set or clear the pause flag (`/pause` / `/unpause`).
    pub fn set_paused(&self, paused: bool) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![PAUSED_KEY, if paused { "1" } else { "0" }],
        )?;
        Ok(())
    }
}
