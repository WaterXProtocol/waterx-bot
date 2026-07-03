use super::Database;
use rusqlite::{params, Result as SqlResult};

impl Database {
    /// Record `referrer` as the inviter of `referee`, but only when:
    ///   - they differ and `referrer` is a positive id,
    ///   - `referrer` is already a known user (they got their link from the bot),
    ///   - `referee` is brand new (no `balance` row yet).
    ///
    /// Returns `true` only when the referral was newly recorded (so the caller
    /// rewards the referrer exactly once). Creates the referee's row with the
    /// referrer set; other columns take their schema defaults.
    pub fn set_referrer_if_new(&self, referee: i64, referrer: i64) -> SqlResult<bool> {
        if referrer <= 0 || referrer == referee {
            return Ok(false);
        }
        let conn = self.conn.lock();
        let referrer_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM balance WHERE user = ?1)",
            params![referrer],
            |r| r.get(0),
        )?;
        if !referrer_exists {
            return Ok(false);
        }
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO balance (user, referrer) VALUES (?1, ?2)",
            params![referee, referrer],
        )?;
        Ok(inserted == 1)
    }

    /// The user's recorded `(referrer, co_referrer)` ids (0 = none). For the
    /// owner-only `/profile` inspector; assumes the row exists (the caller gates
    /// on `user_exists` first, so this never creates a phantom row).
    pub fn get_referrers(&self, user: i64) -> SqlResult<(i64, i64)> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT referrer, co_referrer FROM balance WHERE user = ?1",
            params![user],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    }

    /// How many users were referred by `user`.
    pub fn count_referrals(&self, user: i64) -> SqlResult<i64> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT COUNT(*) FROM balance WHERE referrer = ?1",
            params![user],
            |r| r.get(0),
        )
    }

    /// Group-add bind with an optional **co-referrer** (the group owner). Inserts
    /// the brand-new `referee` with `referrer` (the adder) and, when distinct from
    /// both the adder and referee, `co_referrer` (the owner). The caller
    /// force-creates the referrer/owner rows first, so there's no "referrer exists"
    /// check here. Returns `true` only when the row was newly inserted (no farming
    /// — `INSERT OR IGNORE` no-ops on an existing user).
    pub fn bind_group_referral(&self, referee: i64, referrer: i64, co_referrer: i64) -> SqlResult<bool> {
        if referrer <= 0 || referrer == referee {
            return Ok(false);
        }
        let co = if co_referrer > 0 && co_referrer != referrer && co_referrer != referee {
            co_referrer
        } else {
            0
        };
        let conn = self.conn.lock();
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO balance (user, referrer, co_referrer) VALUES (?1, ?2, ?3)",
            params![referee, referrer, co],
        )?;
        Ok(inserted == 1)
    }

    /// Pay a freshly-bound group referral's signup bonus: the referee gets
    /// `amount`, and the referrer-side `amount` is **split 50/50** with the
    /// co-referrer (the owner) when one is set — else the referrer gets it all.
    /// One transaction (all legs or none).
    pub fn reward_group_signup(
        &self,
        referrer: i64,
        co_referrer: i64,
        referee: i64,
        amount: i64,
    ) -> SqlResult<()> {
        self.ensure_row(referrer)?;
        self.ensure_row(referee)?;
        let split = co_referrer > 0 && co_referrer != referrer;
        if split {
            self.ensure_row(co_referrer)?;
        }
        self.with_tx(|tx| {
            if split {
                let half = amount / 2;
                super::credit_and_log(tx, referrer, half, super::HK_REFERRAL, None, Some(referee))?;
                super::credit_and_log(
                    tx,
                    co_referrer,
                    amount - half,
                    super::HK_REFERRAL,
                    None,
                    Some(referee),
                )?;
            } else {
                super::credit_and_log(tx, referrer, amount, super::HK_REFERRAL, None, Some(referee))?;
            }
            super::credit_and_log(tx, referee, amount, super::HK_REFERRAL, None, Some(referrer))?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::COIN;
    use super::*;

    #[test]
    fn checkin_cascade_pays_three_levels_up() {
        let db = Database::new(":memory:", 1).unwrap();
        // Chain: 1 <- 2 <- 3 <- 4  (4 referred by 3, 3 by 2, 2 by 1)
        db.force_change(1, 0).unwrap(); // user 1 exists (top referrer)
        assert!(db.set_referrer_if_new(2, 1).unwrap());
        assert!(db.set_referrer_if_new(3, 2).unwrap());
        assert!(db.set_referrer_if_new(4, 3).unwrap());

        assert!(db.try_checkin(4, 10 * COIN).unwrap());
        assert_eq!(db.get_user_info(4).unwrap().balance, 10 * COIN); // own reward
        assert_eq!(db.get_user_info(3).unwrap().balance, COIN); // direct referrer +1
        assert_eq!(db.get_user_info(2).unwrap().balance, COIN / 10); // +0.1
        assert_eq!(db.get_user_info(1).unwrap().balance, COIN / 100); // +0.01
    }

    #[test]
    fn existing_user_does_not_rebind() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 0).unwrap();
        db.force_change(2, 0).unwrap(); // user 2 already exists
        assert!(!db.set_referrer_if_new(2, 1).unwrap()); // not new → no bind
        assert!(!db.set_referrer_if_new(1, 1).unwrap()); // self → no bind
    }

    #[test]
    fn group_bind_sequence_only_binds_new_members() {
        // Mirrors what `referral::maybe_bind_group` does at the DB level: a group
        // whose adder is known binds a brand-new member but skips an existing one.
        let db = Database::new(":memory:", 1).unwrap();
        let (group, adder, newbie, existing) = (-100, 10, 20, 30);
        db.set_group_adder(group, adder).unwrap();
        db.force_change(existing, 0).unwrap(); // existing member already has a row

        // user_exists fast-path: newbie is new, existing is not.
        assert!(!db.user_exists(newbie).unwrap());
        assert!(db.user_exists(existing).unwrap());

        // Brand-new member binds to the adder.
        assert_eq!(db.group_adder(group).unwrap(), Some(adder));
        db.force_change(adder, 0).unwrap();
        assert!(db.set_referrer_if_new(newbie, adder).unwrap());
        // Existing member never binds (already has a row).
        assert!(!db.set_referrer_if_new(existing, adder).unwrap());
    }

    #[test]
    fn bind_group_referral_records_co_referrer_when_distinct() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(10, 0).unwrap(); // adder
        db.force_change(20, 0).unwrap(); // owner
        assert!(db.bind_group_referral(30, 10, 20).unwrap());
        let (r, c): (i64, i64) = db
            .conn
            .lock()
            .query_row(
                "SELECT referrer, co_referrer FROM balance WHERE user = ?1",
                rusqlite::params![30],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(r, 10);
        assert_eq!(c, 20);
        assert!(!db.bind_group_referral(30, 10, 20).unwrap()); // existing → no re-bind
                                                               // owner == adder → no co-referrer stored.
        assert!(db.bind_group_referral(40, 10, 10).unwrap());
        let c2: i64 = db
            .conn
            .lock()
            .query_row(
                "SELECT co_referrer FROM balance WHERE user = ?1",
                rusqlite::params![40],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(c2, 0);
    }

    #[test]
    fn member_adder_takes_priority_over_bot_adder() {
        // Mirrors `referral::maybe_bind_group`'s referrer resolution: whoever added
        // *this member* wins over whoever added the bot; a missing record falls back.
        let db = Database::new(":memory:", 1).unwrap();
        let (group, bot_adder, member_adder, newbie, other) = (-100, 10, 11, 20, 21);
        db.set_group_adder(group, bot_adder).unwrap();
        db.record_group_add(group, newbie, member_adder).unwrap();

        // The member was explicitly added → their adder is the referrer.
        assert_eq!(db.group_add_referrer(group, newbie).unwrap(), Some(member_adder));
        // A member with no recorded adder (self-join) → fall back to the bot-adder.
        assert_eq!(db.group_add_referrer(group, other).unwrap(), None);
        assert_eq!(db.group_adder(group).unwrap(), Some(bot_adder));
    }

    #[test]
    fn record_group_add_keeps_first_adder_and_rejects_self() {
        let db = Database::new(":memory:", 1).unwrap();
        db.record_group_add(-100, 20, 10).unwrap();
        db.record_group_add(-100, 20, 11).unwrap(); // first-adder-wins: ignored
        assert_eq!(db.group_add_referrer(-100, 20).unwrap(), Some(10));
        // A self-join (adder == member) records nothing.
        db.record_group_add(-100, 30, 30).unwrap();
        assert_eq!(db.group_add_referrer(-100, 30).unwrap(), None);
    }

    #[test]
    fn reward_group_signup_splits_when_owner_distinct() {
        let db = Database::new(":memory:", 1).unwrap();
        db.reward_group_signup(10, 20, 30, 10 * COIN).unwrap();
        assert_eq!(db.get_user_info(10).unwrap().balance, 5 * COIN); // adder half
        assert_eq!(db.get_user_info(20).unwrap().balance, 5 * COIN); // owner half
        assert_eq!(db.get_user_info(30).unwrap().balance, 10 * COIN); // referee full
                                                                      // No owner → adder gets the whole referrer share.
        db.reward_group_signup(11, 0, 31, 10 * COIN).unwrap();
        assert_eq!(db.get_user_info(11).unwrap().balance, 10 * COIN);
        assert_eq!(db.get_user_info(31).unwrap().balance, 10 * COIN);
    }

    #[test]
    fn checkin_splits_level1_between_referrer_and_co_referrer() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(10, 0).unwrap(); // adder
        db.force_change(20, 0).unwrap(); // owner
        assert!(db.bind_group_referral(30, 10, 20).unwrap()); // referee co-referred
        assert!(db.try_checkin(30, 10 * COIN).unwrap());
        assert_eq!(db.get_user_info(30).unwrap().balance, 10 * COIN); // own reward
        assert_eq!(db.get_user_info(10).unwrap().balance, COIN / 2); // adder +0.5
        assert_eq!(db.get_user_info(20).unwrap().balance, COIN / 2); // owner +0.5
    }
}
