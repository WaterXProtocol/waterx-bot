use super::{current_unix_time, Database};
use rusqlite::{params, Result as SqlResult};

/// A market with open (unsettled) wagers, for the admin `/settle` list.
#[derive(Debug, Clone)]
pub struct OpenMarket {
    pub market_id: String,
    pub slug: String,
    pub team_a: String,
    pub team_b: String,
    pub count: i64,
    pub stake: i64,
}

/// One settled wager, returned so the caller can notify the bettor.
#[derive(Debug, Clone)]
pub struct Settlement {
    pub user: i64,
    pub outcome: String,
    pub stake: i64,
    pub payout: i64,
    pub won: bool,
}

/// Decimal-odds payout for a winning wager: `stake * 100 / odds_cents`,
/// rounded to whole micro-coins.
fn payout_units(stake: i64, odds_cents: f64) -> i64 {
    if odds_cents <= 0.0 {
        return stake; // degenerate quote → just return the stake
    }
    (stake as f64 * 100.0 / odds_cents).round() as i64
}

impl Database {
    /// Record a placed wager. The caller must have already debited `stake`
    /// (micro-coins) from the user's balance. Returns the new wager id.
    #[allow(clippy::too_many_arguments)]
    pub fn place_wager(
        &self,
        user: i64,
        market_id: &str,
        slug: &str,
        team_a: &str,
        team_b: &str,
        outcome: &str,
        stake: i64,
        odds_cents: f64,
        ends_at: i64,
    ) -> SqlResult<i64> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO wagers
                (user, market_id, slug, team_a, team_b, outcome, stake, odds_cents, placed_at, ends_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                user,
                market_id,
                slug,
                team_a,
                team_b,
                outcome,
                stake,
                odds_cents,
                current_unix_time(),
                ends_at
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Markets that still have open wagers, grouped for the admin `/settle`
    /// picker.
    pub fn list_open_markets(&self) -> SqlResult<Vec<OpenMarket>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT market_id, MAX(slug), MAX(team_a), MAX(team_b), COUNT(*), COALESCE(SUM(stake), 0)
             FROM wagers WHERE status = 'open'
             GROUP BY market_id ORDER BY MIN(placed_at)",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(OpenMarket {
                    market_id: r.get(0)?,
                    slug: r.get(1)?,
                    team_a: r.get(2)?,
                    team_b: r.get(3)?,
                    count: r.get(4)?,
                    stake: r.get(5)?,
                })
            })?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(rows)
    }

    /// Settle every open wager on `market_id` against `winner` (one of
    /// `teamA`/`teamB`/`draw`). Credits winners their payout, marks each wager
    /// won/lost, and returns the per-wager outcomes for notification.
    pub fn settle_market(&self, market_id: &str, winner: &str) -> SqlResult<Vec<Settlement>> {
        let conn = self.conn.lock();
        let now = current_unix_time();
        let mut stmt = conn.prepare(
            "SELECT id, user, outcome, stake, odds_cents
             FROM wagers WHERE market_id = ?1 AND status = 'open'",
        )?;
        let rows: Vec<(i64, i64, String, i64, f64)> = stmt
            .query_map(params![market_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<SqlResult<Vec<_>>>()?;
        drop(stmt);

        let mut out = Vec::with_capacity(rows.len());
        for (id, user, outcome, stake, odds_cents) in rows {
            let won = outcome == winner;
            let payout = if won { payout_units(stake, odds_cents) } else { 0 };
            if payout > 0 {
                conn.execute(
                    "INSERT OR IGNORE INTO balance (user, balance, fruit) VALUES (?1, 0, '')",
                    params![user],
                )?;
                conn.execute(
                    "UPDATE balance SET balance = balance + ?1 WHERE user = ?2",
                    params![payout, user],
                )?;
            }
            conn.execute(
                "UPDATE wagers SET status = ?1, settled_at = ?2 WHERE id = ?3",
                params![if won { "won" } else { "lost" }, now, id],
            )?;
            out.push(Settlement {
                user,
                outcome,
                stake,
                payout,
                won,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::super::COIN;
    use super::*;

    #[test]
    fn settle_pays_winner_by_decimal_odds() {
        let db = Database::new(":memory:", 1).unwrap();
        // 5-coin bets at 50¢ (decimal 2.0): winner should receive 10 coins.
        db.place_wager(10, "m1", "slug", "A", "B", "teamA", 5 * COIN, 50.0, 0).unwrap();
        db.place_wager(20, "m1", "slug", "A", "B", "teamB", 5 * COIN, 50.0, 0).unwrap();

        let s = db.settle_market("m1", "teamA").unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(db.get_user_info(10).unwrap().balance, 10 * COIN); // won: stake*2
        assert_eq!(db.get_user_info(20).unwrap().balance, 0); // lost
        // Idempotent: nothing open left to settle.
        assert!(db.settle_market("m1", "teamA").unwrap().is_empty());
    }
}
