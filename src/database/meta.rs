use super::Database;
use rusqlite::{params, OptionalExtension, Result as SqlResult};

/// `meta` key for the admin pause kill-switch.
const PAUSED_KEY: &str = "paused";
/// `meta` key tracking the applied DB schema version (0 = never migrated). Lets
/// `/migrate` be idempotent — a re-run after a successful cutover is a no-op.
const SCHEMA_VERSION_KEY: &str = "schema_version";

impl Database {
    /// The applied schema version (`0` if `/migrate` has never run).
    pub fn schema_version(&self) -> SqlResult<i64> {
        let conn = self.conn.lock();
        let value: Option<String> = conn
            .query_row("SELECT value FROM meta WHERE key = ?1", params![SCHEMA_VERSION_KEY], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(value.and_then(|v| v.parse().ok()).unwrap_or(0))
    }

    /// Stamp the applied schema version (called once by `/migrate`).
    pub fn set_schema_version(&self, version: i64) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![SCHEMA_VERSION_KEY, version.to_string()],
        )?;
        Ok(())
    }

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
