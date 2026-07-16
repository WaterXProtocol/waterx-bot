use super::Database;
use crate::core::types::LeagueFilter;
use rusqlite::{params, OptionalExtension, Result as SqlResult};

/// `meta` key for the admin pause kill-switch.
const PAUSED_KEY: &str = "paused";
/// `meta` key for the owner-configured `/events` league allowlist (JSON array of
/// [`LeagueFilter`]). Absent = never customised → the built-in defaults are used;
/// present (even `[]`) = the owner's explicit list (`[]` shows nothing).
const LEAGUES_KEY: &str = "allowed_leagues";

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

    /// The owner-configured `/events` league allowlist, or `None` if never set
    /// (the caller then uses [`LeagueFilter::defaults`]). A corrupt/unparseable
    /// value is treated as unset (logged), so a bad row can't blank out `/events`.
    pub fn get_allowed_leagues(&self) -> SqlResult<Option<Vec<LeagueFilter>>> {
        let conn = self.conn.lock();
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![LEAGUES_KEY],
                |r| r.get(0),
            )
            .optional()?;
        Ok(value.and_then(|json| match serde_json::from_str(&json) {
            Ok(v) => Some(v),
            Err(e) => {
                eprintln!("waterx-bot: corrupt allowed_leagues meta, using defaults: {e}");
                None
            }
        }))
    }

    /// Persist the `/events` league allowlist (`/leagues` edits). Storing an empty
    /// list is meaningful — it means "surface nothing" (distinct from unset).
    pub fn set_allowed_leagues(&self, leagues: &[LeagueFilter]) -> SqlResult<()> {
        let json = serde_json::to_string(leagues).unwrap_or_else(|_| "[]".to_string());
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![LEAGUES_KEY, json],
        )?;
        Ok(())
    }

    /// Clear the stored allowlist entirely (`/leagues reset`) so `/events` falls
    /// back to [`LeagueFilter::defaults`] again — distinct from an empty list.
    pub fn clear_allowed_leagues(&self) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM meta WHERE key = ?1", params![LEAGUES_KEY])?;
        Ok(())
    }
}
