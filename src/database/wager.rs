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

/// One of a user's open (unsettled) wagers, for the `/status` positions list.
#[derive(Debug, Clone)]
pub struct Position {
    pub team_a: String,
    pub team_b: String,
    pub outcome: String,
    pub stake: i64,
    pub odds_cents: f64,
}

impl Position {
    /// Micro-coins this position pays out if it wins (stake × decimal odds).
    pub fn potential_payout(&self) -> i64 {
        decimal_payout(self.stake, self.odds_cents)
    }
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

/// Decimal-odds payout for a winning wager: `stake * 100 / odds_cents`
/// (= `stake × decimal_odds`), rounded to whole micro-coins. Shared by the
/// at-placement quote (`betting`), `Position::potential_payout`, and
/// `settle_market`, so the displayed payout and the settled payout can never
/// drift. The `f64` is a bounded, rounded odds intermediate — the ledger stays
/// integer micro-coins; at the bot's stake magnitudes there is no precision
/// loss (verified by `settle_pays_winner_by_decimal_odds`).
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
pub fn decimal_payout(stake: i64, odds_cents: f64) -> i64 {
    if odds_cents <= 0.0 {
        return stake; // degenerate quote → just return the stake
    }
    (stake as f64 * 100.0 / odds_cents).round() as i64
}

impl Database {
    /// **Atomically** debit `stake` (micro-coins) from the user and record the
    /// wager in one transaction. Returns `Ok(false)` (writing nothing) when the
    /// user can't cover the stake. This replaces the old debit-then-insert split
    /// where an insert failure after the debit could silently lose the stake.
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
    ) -> SqlResult<bool> {
        self.ensure_row(user)?;
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let debited = tx.execute(
            "UPDATE balance SET balance = balance - ?1 WHERE user = ?2 AND balance - ?1 >= 0",
            params![stake, user],
        )?;
        if debited != 1 {
            return Ok(false); // insufficient funds → rollback, nothing recorded
        }
        tx.execute(
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
        tx.commit()?;
        Ok(true)
    }

    /// A user's open (unsettled) wagers, newest last, for `/status`.
    pub fn list_open_wagers(&self, user: i64) -> SqlResult<Vec<Position>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT team_a, team_b, outcome, stake, odds_cents
             FROM wagers WHERE user = ?1 AND status = 'open'
             ORDER BY placed_at",
        )?;
        let rows = stmt
            .query_map(params![user], |r| {
                Ok(Position {
                    team_a: r.get(0)?,
                    team_b: r.get(1)?,
                    outcome: r.get(2)?,
                    stake: r.get(3)?,
                    odds_cents: r.get(4)?,
                })
            })?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(rows)
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
        let mut conn = self.conn.lock();
        let now = current_unix_time();
        // One transaction over the whole market: either every winner is paid and
        // every wager flipped, or nothing changes. Prevents a mid-loop failure
        // from paying some winners while leaving others `open` (which a re-run
        // would then double-pay).
        let tx = conn.transaction()?;
        let rows: Vec<(i64, i64, String, i64, f64)> = {
            let mut stmt = tx.prepare(
                "SELECT id, user, outcome, stake, odds_cents
                 FROM wagers WHERE market_id = ?1 AND status = 'open'",
            )?;
            let collected = stmt
                .query_map(params![market_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })?
                .collect::<SqlResult<Vec<_>>>()?;
            collected
        };

        let mut out = Vec::with_capacity(rows.len());
        for (id, user, outcome, stake, odds_cents) in rows {
            let won = outcome == winner;
            let payout = if won { decimal_payout(stake, odds_cents) } else { 0 };
            if payout > 0 {
                tx.execute(
                    "INSERT OR IGNORE INTO balance (user, balance, fruit) VALUES (?1, 0, '')",
                    params![user],
                )?;
                tx.execute(
                    "UPDATE balance SET balance = balance + ?1 WHERE user = ?2",
                    params![payout, user],
                )?;
            }
            tx.execute(
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
        tx.commit()?;
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
        // place_wager now debits the stake, so fund both bettors first.
        db.force_change(10, 5 * COIN).unwrap();
        db.force_change(20, 5 * COIN).unwrap();
        assert!(db.place_wager(10, "m1", "slug", "A", "B", "teamA", 5 * COIN, 50.0, 0).unwrap());
        assert!(db.place_wager(20, "m1", "slug", "A", "B", "teamB", 5 * COIN, 50.0, 0).unwrap());

        let s = db.settle_market("m1", "teamA").unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(db.get_user_info(10).unwrap().balance, 10 * COIN); // won: stake*2
        assert_eq!(db.get_user_info(20).unwrap().balance, 0); // lost
        // Idempotent: nothing open left to settle.
        assert!(db.settle_market("m1", "teamA").unwrap().is_empty());
    }

    #[test]
    fn place_wager_debits_atomically_and_rejects_insufficient() {
        let db = Database::new(":memory:", 1).unwrap();
        // No funds → rejected, nothing recorded.
        assert!(!db.place_wager(7, "m", "s", "A", "B", "teamA", 5 * COIN, 50.0, 0).unwrap());
        assert!(db.list_open_wagers(7).unwrap().is_empty());
        assert_eq!(db.get_user_info(7).unwrap().balance, 0);
        // Funded → placed, stake debited in the same transaction.
        db.force_change(7, 5 * COIN).unwrap();
        assert!(db.place_wager(7, "m", "s", "A", "B", "teamA", 5 * COIN, 50.0, 0).unwrap());
        assert_eq!(db.get_user_info(7).unwrap().balance, 0);
        assert_eq!(db.list_open_wagers(7).unwrap().len(), 1);
    }

    #[test]
    fn decimal_payout_matches_decimal_odds_and_guards_zero() {
        // 5 coins at 50¢ (decimal 2.0) → 10 coins; same formula the quote shows.
        assert_eq!(decimal_payout(5 * COIN, 50.0), 10 * COIN);
        // 10 coins at 250¢ (decimal 0.4) → 4 coins.
        assert_eq!(decimal_payout(10 * COIN, 250.0), 4 * COIN);
        // Degenerate quote returns the stake rather than dividing by zero.
        assert_eq!(decimal_payout(7 * COIN, 0.0), 7 * COIN);
    }

    #[test]
    fn list_open_wagers_returns_only_users_open_bets() {
        let db = Database::new(":memory:", 1).unwrap();
        // user 10: one bet at 50¢ (decimal 2.0) — potential payout = 2× stake.
        db.force_change(10, 5 * COIN).unwrap();
        db.force_change(20, 3 * COIN).unwrap();
        db.place_wager(10, "m1", "s", "A", "B", "teamA", 5 * COIN, 50.0, 0).unwrap();
        // user 20: a different bet, must not show up for user 10.
        db.place_wager(20, "m2", "s", "C", "D", "teamB", 3 * COIN, 25.0, 0).unwrap();

        let open = db.list_open_wagers(10).unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].outcome, "teamA");
        assert_eq!(open[0].potential_payout(), 10 * COIN);

        // Settled bets drop out of the open list.
        db.settle_market("m1", "teamB").unwrap();
        assert!(db.list_open_wagers(10).unwrap().is_empty());
    }
}
