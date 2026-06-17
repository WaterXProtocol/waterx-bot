use super::{current_unix_time, Database};
use crate::game::BetGame;
use rusqlite::{params, Result as SqlResult};

impl Database {
    /// Upsert a `BetGame` keyed on `game.id`. Caller is expected to have
    /// already populated `game.id` via [`BetGame::set_id`].
    pub fn save_bet_game(&self, game: &BetGame) -> rusqlite::Result<()> {
        let blob = match serde_json::to_string(game) {
            Ok(s) => s,
            Err(e) => {
                return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(e)));
            }
        };
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR REPLACE INTO bet_games (id, blob, created_at) VALUES (?1, ?2, ?3)",
            params![game.id, blob, current_unix_time()],
        )?;
        Ok(())
    }

    pub fn delete_bet_game(&self, id: &str) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM bet_games WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Load every persisted bet game from disk. Rows whose `blob` fails to
    /// deserialise are logged and skipped — a schema-version skew shouldn't
    /// take down the whole startup. Returns `(id, BetGame)` pairs so callers
    /// can drop them straight into a `HashMap<String, BetGame>`.
    pub fn load_all_bet_games(&self) -> SqlResult<Vec<(String, BetGame)>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT id, blob FROM bet_games")?;
        let mut out = Vec::new();
        let rows = stmt.query_map([], |r| {
            let id: String = r.get(0)?;
            let blob: String = r.get(1)?;
            Ok((id, blob))
        })?;
        for row in rows {
            let (id, blob) = row?;
            match serde_json::from_str::<BetGame>(&blob) {
                Ok(g) => out.push((id, g)),
                Err(e) => {
                    eprintln!("bet_games load: skipping {id}: {e}");
                }
            }
        }
        Ok(out)
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
        let mut g = BetGame::new(123, crate::i18n::Lang::Hant, "晚餐吃啥", &["麵", "飯"]);
        g.set_id(100, 200);
        g.stake(1, "麵", 10);
        g.stake(2, "飯", 5);
        db.save_bet_game(&g).unwrap();

        let loaded = db.load_all_bet_games().unwrap();
        assert_eq!(loaded.len(), 1);
        let (id, g2) = &loaded[0];
        assert_eq!(id, &g.id);
        assert_eq!(g2.host, 123);
        assert_eq!(g2.total, 15);
        assert_eq!(g2.options["麵"].bet, 10);
        assert_eq!(g2.options["飯"].bet, 5);
        assert_eq!(g2.inputs[&1], 10);
        assert_eq!(g2.inputs[&2], 5);
    }

    #[test]
    fn save_is_upsert() {
        let db = mk_db();
        let mut g = BetGame::new(123, crate::i18n::Lang::En, "t", &["A", "B"]);
        g.set_id(100, 200);
        g.stake(1, "A", 5);
        db.save_bet_game(&g).unwrap();
        g.stake(1, "A", 3);
        db.save_bet_game(&g).unwrap();
        let loaded = db.load_all_bet_games().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].1.options["A"].bet, 8);
    }

    #[test]
    fn delete_works() {
        let db = mk_db();
        let mut g = BetGame::new(123, crate::i18n::Lang::En, "t", &["A"]);
        g.set_id(100, 200);
        db.save_bet_game(&g).unwrap();
        db.delete_bet_game(&g.id).unwrap();
        assert!(db.load_all_bet_games().unwrap().is_empty());
    }

    #[test]
    fn load_all_survives_state_transitions() {
        let db = mk_db();
        let mut g = BetGame::new(123, crate::i18n::Lang::En, "t", &["A", "B"]);
        g.set_id(100, 200);
        g.stake(1, "A", 10);
        g.close();
        let (_outputs, _display) = g.settle("A");
        db.save_bet_game(&g).unwrap();
        let loaded = db.load_all_bet_games().unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(matches!(loaded[0].1.state, crate::types::BetState::settled));
        assert_eq!(loaded[0].1.changes[&1], 0); // staked 10, won 10 back (sole bettor)
    }
}
