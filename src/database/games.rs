use super::{current_unix_time, Database};
use crate::game::{BetGame, OptionData};
use crate::i18n::Lang;
use crate::types::{BetState, OddsFormat};
use rusqlite::{params, Connection, Result as SqlResult};
use std::collections::HashMap;

/// Stable on-disk code for a game's state. Only `betting`/`closed` are ever
/// written (settling drops a game from storage), but the codec stays total.
fn state_code(s: &BetState) -> &'static str {
    match s {
        BetState::betting => "betting",
        BetState::closed => "closed",
        BetState::settled => "settled",
        BetState::draw => "draw",
    }
}

fn state_from_code(code: &str) -> BetState {
    match code {
        "closed" => BetState::closed,
        "settled" => BetState::settled,
        "draw" => BetState::draw,
        _ => BetState::betting,
    }
}

impl Database {
    /// Create the normalized self-host `/predict` game tables. Co-located with
    /// the game persistence so the schema and the read/write code can't drift.
    /// One row per game in `games`, one per option in `game_options` (ordered by
    /// `idx`), one per (option, bettor) in `game_stakes`.
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

    /// One-time migration: fold any legacy JSON-blob `bet_games` rows into the
    /// normalized tables (open games only — terminal ones are dropped), then drop
    /// the old table. No-op on fresh DBs (the table never existed). Run from
    /// [`Database::new`] before the connection is wrapped, after the tables exist.
    pub(super) fn migrate_blob_games(conn: &Connection) -> SqlResult<()> {
        let present: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'bet_games'",
            [],
            |r| r.get(0),
        )?;
        if present == 0 {
            return Ok(());
        }
        let rows: Vec<(String, String)> = {
            let mut stmt = conn.prepare("SELECT id, blob FROM bet_games")?;
            let v = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<SqlResult<Vec<_>>>()?;
            v
        };
        let tx = conn.unchecked_transaction()?;
        for (id, blob) in rows {
            match serde_json::from_str::<BetGame>(&blob) {
                // Only carry over live games; settled/draw are dropped on purpose.
                Ok(g) if matches!(g.state, BetState::betting | BetState::closed) => {
                    write_game_rows(&tx, &g)?;
                }
                Ok(_) => {}
                Err(e) => eprintln!("bet_games migrate: skipping {id}: {e}"),
            }
        }
        tx.execute("DROP TABLE bet_games", [])?;
        tx.commit()?;
        Ok(())
    }

    /// Persist an **open** (`betting`/`closed`) game into the normalized tables.
    /// A **terminal** (`settled`/`draw`) game is instead **deleted** — settling
    /// drops a game from storage, so the tables only ever hold live games and
    /// never grow unbounded. Runs in one transaction, so a crash can't half-write.
    pub fn save_bet_game(&self, game: &BetGame) -> SqlResult<()> {
        let conn = self.conn.lock();
        if matches!(game.state, BetState::settled | BetState::draw) {
            return delete_game_rows(&conn, &game.id);
        }
        let tx = conn.unchecked_transaction()?;
        write_game_rows(&tx, game)?;
        tx.commit()?;
        Ok(())
    }

    /// Delete a game and all its option/stake rows.
    pub fn delete_bet_game(&self, id: &str) -> SqlResult<()> {
        let conn = self.conn.lock();
        delete_game_rows(&conn, id)
    }

    /// Load every persisted (open) game, reconstructing each [`BetGame`] from its
    /// normalized rows. `inputs` is recomputed by summing each user's stakes;
    /// settle-derived fields (`outputs`/`changes`) stay empty since only open
    /// games are stored.
    pub fn load_all_bet_games(&self) -> SqlResult<Vec<(String, BetGame)>> {
        let conn = self.conn.lock();
        // (id, host, lang, description, state, total, ends_at, odds_fmt)
        type GameRow = (String, i64, String, String, String, i64, i64, String);
        let games: Vec<GameRow> = {
            let mut stmt = conn.prepare(
                "SELECT id, host, lang, description, state, total, ends_at, odds_fmt FROM games",
            )?;
            let v = stmt
                .query_map([], |r| {
                    Ok((
                        r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?,
                        r.get(7)?,
                    ))
                })?
                .collect::<SqlResult<Vec<_>>>()?;
            v
        };

        let mut opt_stmt =
            conn.prepare("SELECT name, bet FROM game_options WHERE game_id = ?1 ORDER BY idx")?;
        let mut stake_stmt = conn.prepare(
            "SELECT option_name, user, amount, bettor_name FROM game_stakes WHERE game_id = ?1",
        )?;

        let mut out = Vec::with_capacity(games.len());
        for (id, host, lang, description, state, total, ends_at, odds_fmt) in games {
            let mut game = BetGame {
                id: id.clone(),
                host,
                lang: Lang::from_store_code(&lang).unwrap_or_default(),
                description,
                state: state_from_code(&state),
                options: HashMap::new(),
                option_order: Vec::new(),
                total,
                inputs: HashMap::new(),
                outputs: HashMap::new(),
                changes: HashMap::new(),
                names: HashMap::new(),
                ends_at,
                odds_fmt: OddsFormat::from_store_code(&odds_fmt),
            };
            // Options, in `option_order`.
            let opts = opt_stmt.query_map(params![id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })?;
            for row in opts {
                let (name, bet) = row?;
                game.option_order.push(name.clone());
                game.options.insert(name, OptionData { bet, ..OptionData::default() });
            }
            // Stakes → per-option detail + bettor names + recomputed inputs.
            let stakes = stake_stmt.query_map(params![id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?;
            for row in stakes {
                let (option_name, user, amount, bettor_name) = row?;
                if let Some(o) = game.options.get_mut(&option_name) {
                    o.detail.insert(user, amount);
                }
                game.names.insert(user, bettor_name);
                *game.inputs.entry(user).or_insert(0) += amount;
            }
            recompute_odds(&mut game);
            out.push((id, game));
        }
        Ok(out)
    }
}

/// Write a game's rows (caller wraps in a transaction). Shared by
/// [`Database::save_bet_game`] and the legacy blob migration. Rewrites the
/// option/stake rows wholesale (small sets) so it doubles as the update path.
fn write_game_rows(conn: &Connection, game: &BetGame) -> SqlResult<()> {
    conn.execute(
        "INSERT OR REPLACE INTO games
            (id, host, lang, description, state, total, ends_at, odds_fmt, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            game.id,
            game.host,
            game.lang.store_code(),
            game.description,
            state_code(&game.state),
            game.total,
            game.ends_at,
            game.odds_fmt.store_code(),
            current_unix_time(),
        ],
    )?;
    conn.execute("DELETE FROM game_options WHERE game_id = ?1", params![game.id])?;
    conn.execute("DELETE FROM game_stakes WHERE game_id = ?1", params![game.id])?;
    for (idx, name) in game.option_order.iter().enumerate() {
        let bet = game.options.get(name).map_or(0, |o| o.bet);
        conn.execute(
            "INSERT INTO game_options (game_id, idx, name, bet) VALUES (?1, ?2, ?3, ?4)",
            params![game.id, idx as i64, name, bet],
        )?;
    }
    for (name, opt) in &game.options {
        for (user, amount) in &opt.detail {
            let bettor = game.names.get(user).map(String::as_str).unwrap_or("");
            conn.execute(
                "INSERT INTO game_stakes (game_id, option_name, user, amount, bettor_name)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![game.id, name, user, amount, bettor],
            )?;
        }
    }
    Ok(())
}

fn delete_game_rows(conn: &Connection, id: &str) -> SqlResult<()> {
    conn.execute("DELETE FROM games WHERE id = ?1", params![id])?;
    conn.execute("DELETE FROM game_options WHERE game_id = ?1", params![id])?;
    conn.execute("DELETE FROM game_stakes WHERE game_id = ?1", params![id])?;
    Ok(())
}

/// Recompute each option's displayed `odd` string from the pool, matching what
/// [`BetGame::stake`] writes — so a loaded game is byte-identical to one built
/// live. (The board renders via `option_odds`; `odd` lingers as stored/tested
/// data.)
#[allow(clippy::cast_precision_loss)]
fn recompute_odds(game: &mut BetGame) {
    let total = game.total;
    for o in game.options.values_mut() {
        if o.bet > 0 {
            let odd = (total as f64 / o.bet as f64 * 1000.0).round() / 1000.0;
            o.odd = format!("{odd:.2}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_db() -> Database {
        Database::new(":memory:", 999).expect("open in-memory db")
    }

    #[test]
    fn save_load_round_trip() {
        let db = mk_db();
        let mut g = BetGame::new(123, Lang::Hant, "晚餐吃啥", &["麵", "飯"]);
        g.set_id(100, 200);
        g.stake(1, "麵", 10, "Alice");
        g.stake(2, "飯", 5, "Bob");
        db.save_bet_game(&g).unwrap();

        let loaded = db.load_all_bet_games().unwrap();
        assert_eq!(loaded.len(), 1);
        let (id, g2) = &loaded[0];
        assert_eq!(id, &g.id);
        assert_eq!(g2.host, 123);
        assert_eq!(g2.lang, Lang::Hant);
        assert_eq!(g2.total, 15);
        // option_order preserved.
        assert_eq!(g2.option_order, vec!["麵".to_string(), "飯".to_string()]);
        assert_eq!(g2.options["麵"].bet, 10);
        assert_eq!(g2.options["飯"].bet, 5);
        assert_eq!(g2.options["麵"].detail[&1], 10);
        assert_eq!(g2.inputs[&1], 10);
        assert_eq!(g2.inputs[&2], 5);
        // Bettor names round-trip for the settlement readout.
        assert_eq!(g2.names[&1], "Alice");
        assert_eq!(g2.names[&2], "Bob");
        // Recomputed odds match a live game.
        assert_eq!(g2.options["麵"].odd, "1.50");
    }

    #[test]
    fn save_is_upsert() {
        let db = mk_db();
        let mut g = BetGame::new(123, Lang::En, "t", &["A", "B"]);
        g.set_id(100, 200);
        g.stake(1, "A", 5, "Alice");
        db.save_bet_game(&g).unwrap();
        g.stake(1, "A", 3, "Alice");
        db.save_bet_game(&g).unwrap();
        let loaded = db.load_all_bet_games().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].1.options["A"].bet, 8);
        assert_eq!(loaded[0].1.inputs[&1], 8);
    }

    #[test]
    fn closed_state_round_trips() {
        let db = mk_db();
        let mut g = BetGame::new(7, Lang::En, "t", &["A", "B"]);
        g.set_id(1, 2);
        g.stake(1, "A", 4, "Alice");
        g.close();
        db.save_bet_game(&g).unwrap();
        let loaded = db.load_all_bet_games().unwrap();
        assert!(matches!(loaded[0].1.state, BetState::closed));
    }

    #[test]
    fn delete_works() {
        let db = mk_db();
        let mut g = BetGame::new(123, Lang::En, "t", &["A"]);
        g.set_id(100, 200);
        db.save_bet_game(&g).unwrap();
        db.delete_bet_game(&g.id).unwrap();
        assert!(db.load_all_bet_games().unwrap().is_empty());
    }

    #[test]
    fn settling_drops_the_game_from_storage() {
        let db = mk_db();
        let mut g = BetGame::new(123, Lang::En, "t", &["A", "B"]);
        g.set_id(100, 200);
        g.stake(1, "A", 10, "Alice");
        db.save_bet_game(&g).unwrap();
        assert_eq!(db.load_all_bet_games().unwrap().len(), 1);
        // Settling moves the game to a terminal state; saving it then drops it.
        g.close();
        let _ = g.settle("A");
        db.save_bet_game(&g).unwrap();
        assert!(db.load_all_bet_games().unwrap().is_empty());
    }

    #[test]
    fn migrate_blob_games_folds_open_drops_terminal() {
        // Build a DB with a legacy `bet_games` blob table holding one open and
        // one settled game, then re-open via `Database::new` (which migrates).
        let path = std::env::temp_dir().join(format!("waterx_migrate_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let p = path.to_str().unwrap();
        {
            let conn = Connection::open(p).unwrap();
            conn.execute(
                "CREATE TABLE bet_games (id TEXT PRIMARY KEY, blob TEXT NOT NULL, created_at INTEGER NOT NULL)",
                [],
            )
            .unwrap();
            let mut open = BetGame::new(5, Lang::En, "open", &["A", "B"]);
            open.set_id(10, 20);
            open.stake(1, "A", 7, "Alice");
            let mut done = BetGame::new(6, Lang::En, "done", &["A", "B"]);
            done.set_id(11, 21);
            done.stake(2, "A", 3, "Bob");
            done.close();
            let _ = done.settle("A");
            for g in [&open, &done] {
                conn.execute(
                    "INSERT INTO bet_games (id, blob, created_at) VALUES (?1, ?2, 0)",
                    params![g.id, serde_json::to_string(g).unwrap()],
                )
                .unwrap();
            }
        }
        let db = Database::new(p, 999).unwrap();
        let loaded = db.load_all_bet_games().unwrap();
        assert_eq!(loaded.len(), 1, "only the open game survives migration");
        assert!(loaded[0].1.description.ends_with("open"));
        assert_eq!(loaded[0].1.options["A"].bet, 7);
        assert_eq!(loaded[0].1.names[&1], "Alice");
        // Old table is gone.
        let still: i64 = db
            .conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='bet_games'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(still, 0, "legacy bet_games table dropped after migration");
        let _ = std::fs::remove_file(&path);
    }
}
