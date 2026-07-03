use super::Database;
use rusqlite::Result as SqlResult;

/// Bot-wide aggregate counters for the owner-only `/dashboard` command. Coin
/// fields are micro-coins (render with `fmt_coins`); count fields are plain
/// integers.
#[derive(Debug, Default, Clone)]
pub struct Dashboard {
    /// Rows in `balance` — every user the bot has a ledger for.
    pub users: i64,
    /// Users who joined via a referral link (`referrer != 0`).
    pub referred_users: i64,
    /// Users who claimed the daily check-in today (current UTC day).
    pub checked_in_today: i64,
    /// Groups/channels the bot is in (negative chat ids).
    pub groups: i64,
    /// Private chats the bot has seen (positive chat ids).
    pub private_chats: i64,
    /// Circulating coin supply = sum of every balance (micro-coins).
    pub total_supply: i64,
    /// Open Polymarket-sourced events (`/events`, `state = 'open'`).
    pub open_sourced: i64,
    /// Open host-run AMM `/predict` events (`state = 'open'`).
    pub open_amm: i64,
    /// Open share positions across all events (`shares > 0`).
    pub open_positions: i64,
    /// Coins committed to positions = Σ cost basis (micro-coins).
    pub committed_coins: i64,
}

impl Database {
    /// One-shot snapshot of bot-wide metrics for `/dashboard`: a single lock and a
    /// handful of aggregate queries over `balance`/`chats` and the unified market
    /// engine (`events`/`positions`).
    pub fn dashboard(&self) -> SqlResult<Dashboard> {
        let conn = self.conn.lock();
        let count = |sql: &str| -> SqlResult<i64> { conn.query_row(sql, [], |r| r.get::<_, i64>(0)) };
        Ok(Dashboard {
            users: count("SELECT COUNT(*) FROM balance")?,
            referred_users: count("SELECT COUNT(*) FROM balance WHERE referrer != 0")?,
            // `last_checkin` stores the last claimed UTC day index (unix/86400);
            // compare against today's index computed the same way.
            checked_in_today: count(
                "SELECT COUNT(*) FROM balance \
                 WHERE last_checkin = CAST(strftime('%s','now') AS INTEGER) / 86400 \
                 AND last_checkin > 0",
            )?,
            groups: count("SELECT COUNT(*) FROM chats WHERE chat < 0")?,
            private_chats: count("SELECT COUNT(*) FROM chats WHERE chat > 0")?,
            total_supply: count("SELECT COALESCE(SUM(balance), 0) FROM balance")?,
            open_sourced: count("SELECT COUNT(*) FROM events WHERE kind = 'sourced' AND state = 'open'")?,
            open_amm: count("SELECT COUNT(*) FROM events WHERE kind = 'amm' AND state = 'open'")?,
            open_positions: count("SELECT COUNT(*) FROM positions WHERE shares > 0")?,
            committed_coins: count("SELECT COALESCE(SUM(cost), 0) FROM positions")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::COIN;
    use super::*;

    #[test]
    fn counts_users_chats_and_supply() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(10, 5 * COIN).unwrap();
        db.force_change(20, 3 * COIN).unwrap();
        db.touch_chat(10).unwrap(); // private chat (positive id)
        db.touch_chat(-100).unwrap(); // group (negative id)
        db.touch_chat(-200).unwrap(); // group (negative id)

        let s = db.dashboard().unwrap();
        assert_eq!(s.users, 2);
        assert_eq!(s.total_supply, 8 * COIN);
        assert_eq!(s.groups, 2);
        assert_eq!(s.private_chats, 1);
        assert_eq!(s.open_positions, 0);
        assert_eq!(s.committed_coins, 0);
    }

    #[test]
    fn counts_referred_users() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 0).unwrap(); // top referrer, no referrer of their own
        assert!(db.set_referrer_if_unset(2, 1).unwrap());
        assert!(db.set_referrer_if_unset(3, 1).unwrap());

        let s = db.dashboard().unwrap();
        assert_eq!(s.users, 3);
        assert_eq!(s.referred_users, 2);
    }

    #[test]
    fn aggregates_open_engine_events_and_positions() {
        // One open sourced event + one open amm event + one resolved (ignored);
        // two open positions on the open events. Aggregated straight from the engine.
        let db = Database::new(":memory:", 1).unwrap();
        {
            let conn = db.conn.lock();
            conn.execute(
                "INSERT INTO events (id, kind, state) VALUES
                 (1, 'sourced', 'open'), (2, 'amm', 'open'), (3, 'sourced', 'resolved')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO positions (event_id, market_idx, user, shares, cost) VALUES
                 (1, 0, 10, 5000000, 3000000), (2, 1, 11, 4000000, 2000000)",
                [],
            )
            .unwrap();
        }

        let s = db.dashboard().unwrap();
        assert_eq!(s.open_sourced, 1);
        assert_eq!(s.open_amm, 1);
        assert_eq!(s.open_positions, 2);
        assert_eq!(s.committed_coins, 5 * COIN); // (3 + 2) coins of cost basis
    }
}
