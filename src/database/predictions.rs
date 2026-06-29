use super::Database;
use rusqlite::{Connection, Result as SqlResult};

impl Database {
    /// Create the normalized self-host `/predict` game tables. **Legacy** — the
    /// live `/predict` runs on the unified `events`/`markets`/`positions` engine
    /// now; these tables are only still created so `/migrate` can drain any
    /// leftover open stakes on the prod cutover (`Database::reset_predictions`
    /// reads `game_stakes` directly). Dropped entirely once `/migrate` has run.
    pub(super) fn create_game_tables(conn: &Connection) -> SqlResult<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS games (
                id          TEXT    NOT NULL PRIMARY KEY,
                host        INTEGER NOT NULL,
                lang        TEXT    NOT NULL DEFAULT '',
                description TEXT    NOT NULL DEFAULT '',
                state       TEXT    NOT NULL DEFAULT 'betting',
                total       INTEGER NOT NULL DEFAULT 0,
                ends_at     INTEGER NOT NULL DEFAULT 0,
                odds_fmt    TEXT    NOT NULL DEFAULT '',
                tz_offset   INTEGER NOT NULL DEFAULT 0,
                created_at  INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS game_options (
                game_id TEXT    NOT NULL,
                idx     INTEGER NOT NULL,
                name    TEXT    NOT NULL,
                bet     INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (game_id, idx)
            )",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS game_stakes (
                game_id     TEXT    NOT NULL,
                option_name TEXT    NOT NULL,
                user        INTEGER NOT NULL,
                amount      INTEGER NOT NULL DEFAULT 0,
                bettor_name TEXT    NOT NULL DEFAULT '',
                PRIMARY KEY (game_id, option_name, user)
            )",
            [],
        )?;
        Ok(())
    }
}
