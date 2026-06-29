use super::Database;
use rusqlite::{Connection, Result as SqlResult};

impl Database {
    /// Create the unified Polymarket-style market schema, shared by both betting
    /// surfaces (it supersedes the old `wagers` table *and* the
    /// `games`/`game_options`/`game_stakes` prediction tables):
    ///
    /// - **`events`** — one row per event: a Polymarket-`sourced` match (each
    ///   outcome priced live from Gamma) or an `amm` host prediction (priced from a
    ///   stored LMSR curve). `pool` holds the AMM escrow in micro-coins;
    ///   `b_param`/`fee_bps` are the host's LMSR liquidity and trading fee (bps);
    ///   `winning_market` is the resolved outcome idx; `state` is
    ///   `open`/`resolved`/`closed`/`void`.
    /// - **`markets`** — one row per tradeable outcome within an event (the unit a
    ///   user holds YES shares of). `q_shares` is net YES shares outstanding in
    ///   micro-shares (the AMM price input; unused for sourced events, which read
    ///   their price from Gamma).
    /// - **`positions`** — one row per (event, outcome, user) YES-share holding:
    ///   `shares` (micro-shares) and `cost` (micro-coin basis, for avg-price display
    ///   and `/migrate` refunds).
    ///
    /// Co-located with the trade engine so the schema and read/write code can't
    /// drift, mirroring [`Database::create_game_tables`].
    pub(super) fn create_event_tables(conn: &Connection) -> SqlResult<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS events (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                kind           TEXT    NOT NULL,
                source_ref     TEXT,
                title          TEXT    NOT NULL DEFAULT '',
                creator        INTEGER NOT NULL DEFAULT 0,
                lang           TEXT    NOT NULL DEFAULT '',
                odds_fmt       TEXT    NOT NULL DEFAULT '',
                tz_offset      INTEGER,
                ends_at        INTEGER NOT NULL DEFAULT 0,
                state          TEXT    NOT NULL DEFAULT 'open',
                winning_market INTEGER,
                b_param        INTEGER,
                fee_bps        INTEGER NOT NULL DEFAULT 200,
                pool           INTEGER NOT NULL DEFAULT 0,
                created_at     INTEGER NOT NULL DEFAULT 0,
                resolved_at    INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS markets (
                event_id INTEGER NOT NULL,
                idx      INTEGER NOT NULL,
                name     TEXT    NOT NULL DEFAULT '',
                q_shares INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (event_id, idx)
            )",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS positions (
                event_id   INTEGER NOT NULL,
                market_idx INTEGER NOT NULL,
                user       INTEGER NOT NULL,
                shares     INTEGER NOT NULL DEFAULT 0,
                cost       INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (event_id, market_idx, user)
            )",
            [],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_schema_is_created_with_defaults_and_composite_keys() {
        let db = Database::new(":memory:", 1).unwrap();
        let conn = db.conn.lock();
        conn.execute(
            "INSERT INTO events (id, kind, title, creator, created_at)
             VALUES (1, 'amm', 'Who wins?', 42, 100)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO markets (event_id, idx, name) VALUES (1, 0, 'A'), (1, 1, 'B')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO positions (event_id, market_idx, user, shares, cost)
             VALUES (1, 0, 7, 200000000, 10000000)",
            [],
        )
        .unwrap();

        // Column defaults applied.
        let (state, fee_bps, pool): (String, i64, i64) = conn
            .query_row(
                "SELECT state, fee_bps, pool FROM events WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(state, "open");
        assert_eq!(fee_bps, 200);
        assert_eq!(pool, 0);

        let markets: i64 = conn
            .query_row("SELECT COUNT(*) FROM markets WHERE event_id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(markets, 2);

        // Composite PK rejects a duplicate (event_id, idx).
        assert!(conn
            .execute(
                "INSERT INTO markets (event_id, idx, name) VALUES (1, 0, 'dup')",
                [],
            )
            .is_err());
    }
}
