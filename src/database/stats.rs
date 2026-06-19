use super::Database;
use rusqlite::Result as SqlResult;

/// Bot-wide aggregate counters for the owner-only `/stats` command. Coin fields
/// are micro-coins (render with `fmt_coins`); count fields are plain integers.
#[derive(Debug, Default, Clone)]
pub struct BotStats {
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
    /// Open (unsettled) real-money match bets.
    pub open_wagers: i64,
    /// Coins locked in open match bets (micro-coins).
    pub open_stake: i64,
    /// All-time match bets placed (any status).
    pub total_wagers: i64,
    /// All-time match-bet volume (micro-coins).
    pub total_volume: i64,
}

impl Database {
    /// One-shot snapshot of bot-wide metrics for `/stats`: a single lock and a
    /// handful of aggregate queries. The in-memory self-host game figures are
    /// added by the caller (they live in `GamesKey`, not the DB).
    pub fn stats(&self) -> SqlResult<BotStats> {
        let conn = self.conn.lock();
        let count = |sql: &str| -> SqlResult<i64> { conn.query_row(sql, [], |r| r.get::<_, i64>(0)) };
        Ok(BotStats {
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
            open_wagers: count("SELECT COUNT(*) FROM wagers WHERE status = 'open'")?,
            open_stake: count("SELECT COALESCE(SUM(stake), 0) FROM wagers WHERE status = 'open'")?,
            total_wagers: count("SELECT COUNT(*) FROM wagers")?,
            total_volume: count("SELECT COALESCE(SUM(stake), 0) FROM wagers")?,
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

        let s = db.stats().unwrap();
        assert_eq!(s.users, 2);
        assert_eq!(s.total_supply, 8 * COIN);
        assert_eq!(s.groups, 2);
        assert_eq!(s.private_chats, 1);
        assert_eq!(s.open_wagers, 0);
        assert_eq!(s.total_volume, 0);
    }

    #[test]
    fn counts_referred_users() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 0).unwrap(); // top referrer, no referrer of their own
        assert!(db.set_referrer_if_new(2, 1).unwrap());
        assert!(db.set_referrer_if_new(3, 1).unwrap());

        let s = db.stats().unwrap();
        assert_eq!(s.users, 3);
        assert_eq!(s.referred_users, 2);
    }
}
