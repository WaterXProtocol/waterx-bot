use super::{Database, COIN};
use crate::core::lmsr;
use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult, Transaction};

/// Shares are integer **micro-shares** (6-decimal fixed-point), mirroring `COIN`
/// for coins. A share of the winning outcome settles to exactly one coin, so
/// `SHARE == COIN`.
pub(crate) const SHARE: i64 = 1_000_000;

/// Result of an AMM buy/sell attempt against the ledger.
#[derive(Debug, PartialEq, Eq)]
pub enum TradeOutcome {
    /// Executed. `shares` = signed micro-shares (+buy / −sell); `coins` = signed
    /// micro-coins on the trader's balance (−spent / +received); `fee` =
    /// micro-coins retained in the pool for the host.
    Filled { shares: i64, coins: i64, fee: i64 },
    /// Not enough balance (buy) or not enough shares held (sell), or the spend is
    /// too small to mint a whole micro-share. Nothing written.
    Rejected,
    /// Event missing, not an AMM event, not open, or a bad outcome index.
    Unavailable,
}

/// `amount · bps / 10000`, floored, in i128 to avoid mid-product overflow.
fn mul_bps(amount: i64, bps: i64) -> i64 {
    (amount as i128 * bps as i128 / 10_000) as i64
}

/// Host seed escrow `⌈C(0)·COIN⌉ = ⌈b·ln(k)·COIN⌉` micro-coins — rounded **up**
/// so the funded pool is never a hair short of the worst-case subsidy.
fn escrow_micro(b: i64, k: usize) -> i64 {
    (lmsr::seed_escrow(b as f64, k) * COIN as f64).ceil() as i64
}

/// Load `(b_param, fee_bps, pool)` for an event iff it's an **open AMM** event.
fn load_amm_open(tx: &Transaction, event_id: i64) -> SqlResult<Option<(i64, i64, i64)>> {
    tx.query_row(
        "SELECT b_param, fee_bps, pool FROM events
         WHERE id = ?1 AND kind = 'amm' AND state = 'open'",
        params![event_id],
        |r| Ok((r.get::<_, Option<i64>>(0)?.unwrap_or(0), r.get(1)?, r.get(2)?)),
    )
    .optional()
}

/// The event's net outstanding YES shares per outcome, ordered by `idx`.
fn load_q(tx: &Transaction, event_id: i64) -> SqlResult<Vec<i64>> {
    let mut stmt = tx.prepare("SELECT q_shares FROM markets WHERE event_id = ?1 ORDER BY idx")?;
    let v = stmt
        .query_map(params![event_id], |r| r.get(0))?
        .collect::<SqlResult<Vec<i64>>>()?;
    Ok(v)
}

/// Add `add_shares` / `add_cost` to a (event, outcome, user) position, creating
/// the row if absent.
fn add_position(
    tx: &Transaction,
    event_id: i64,
    idx: i64,
    user: i64,
    add_shares: i64,
    add_cost: i64,
) -> SqlResult<()> {
    tx.execute(
        "INSERT INTO positions (event_id, market_idx, user, shares, cost)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(event_id, market_idx, user)
         DO UPDATE SET shares = shares + excluded.shares, cost = cost + excluded.cost",
        params![event_id, idx, user, add_shares, add_cost],
    )?;
    Ok(())
}

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

    /// Open a host-run **AMM** event: debit the host the LMSR seed escrow
    /// (`⌈b·ln k · COIN⌉`, the worst-case subsidy that keeps the pool solvent),
    /// then insert the event (`kind='amm'`, `pool` = escrow) and its `k` outcome
    /// markets (`q_shares = 0`). All in one transaction. Returns the new event id,
    /// or `None` if the host can't afford the escrow (nothing written). `b` is the
    /// LMSR liquidity in whole shares; `fee_bps` the host trading fee (the caller
    /// clamps it to `[0, FEE_BPS_MAX]`).
    #[allow(clippy::too_many_arguments)]
    pub fn create_amm_event(
        &self,
        host: i64,
        title: &str,
        lang: &str,
        odds_fmt: &str,
        tz_offset: Option<i64>,
        ends_at: i64,
        options: &[String],
        b: i64,
        fee_bps: i64,
        now: i64,
    ) -> SqlResult<Option<i64>> {
        let escrow = escrow_micro(b, options.len());
        self.ensure_row(host)?;
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let debited = tx.execute(
            "UPDATE balance SET balance = balance - ?1 WHERE user = ?2 AND balance - ?1 >= 0",
            params![escrow, host],
        )?;
        if debited != 1 {
            return Ok(None); // host can't fund the escrow → rollback on drop
        }
        tx.execute(
            "INSERT INTO events
                (kind, title, creator, lang, odds_fmt, tz_offset, ends_at, state,
                 b_param, fee_bps, pool, created_at)
             VALUES ('amm', ?1, ?2, ?3, ?4, ?5, ?6, 'open', ?7, ?8, ?9, ?10)",
            params![title, host, lang, odds_fmt, tz_offset, ends_at, b, fee_bps, escrow, now],
        )?;
        let event_id = tx.last_insert_rowid();
        for (idx, name) in options.iter().enumerate() {
            tx.execute(
                "INSERT INTO markets (event_id, idx, name, q_shares) VALUES (?1, ?2, ?3, 0)",
                params![event_id, idx as i64, name],
            )?;
        }
        tx.commit()?;
        Ok(Some(event_id))
    }

    /// Buy YES shares of outcome `idx` in an AMM event by spending `spend`
    /// micro-coins. The **whole spend** enters the pool; the LMSR cost
    /// (`spend − fee`) sets how many shares are minted, **floored** so the pool is
    /// never short. `q_shares` and `pool` advance. One transaction.
    pub fn amm_buy(&self, event_id: i64, idx: i64, user: i64, spend: i64) -> SqlResult<TradeOutcome> {
        if spend <= 0 {
            return Ok(TradeOutcome::Rejected);
        }
        self.ensure_row(user)?;
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let Some((b, fee_bps, _pool)) = load_amm_open(&tx, event_id)? else {
            return Ok(TradeOutcome::Unavailable);
        };
        let q = load_q(&tx, event_id)?;
        if idx < 0 || idx as usize >= q.len() {
            return Ok(TradeOutcome::Unavailable);
        }
        let fee = mul_bps(spend, fee_bps);
        let cost = spend - fee;
        let q_whole: Vec<f64> = q.iter().map(|qi| *qi as f64 / SHARE as f64).collect();
        let shares_whole =
            lmsr::shares_for_budget(&q_whole, b as f64, idx as usize, cost as f64 / COIN as f64);
        let shares = (shares_whole * SHARE as f64).floor() as i64;
        if shares <= 0 {
            return Ok(TradeOutcome::Rejected); // spend too small for one micro-share
        }
        let debited = tx.execute(
            "UPDATE balance SET balance = balance - ?1 WHERE user = ?2 AND balance - ?1 >= 0",
            params![spend, user],
        )?;
        if debited != 1 {
            return Ok(TradeOutcome::Rejected); // insufficient funds → rollback
        }
        add_position(&tx, event_id, idx, user, shares, spend)?;
        tx.execute(
            "UPDATE markets SET q_shares = q_shares + ?1 WHERE event_id = ?2 AND idx = ?3",
            params![shares, event_id, idx],
        )?;
        tx.execute("UPDATE events SET pool = pool + ?1 WHERE id = ?2", params![spend, event_id])?;
        tx.commit()?;
        Ok(TradeOutcome::Filled { shares, coins: -spend, fee })
    }

    /// Sell `shares` micro-shares of outcome `idx` back to the AMM. The refund is
    /// the LMSR refund (**floored** — pool keeps the remainder) minus the host
    /// fee, which stays in the pool. `q_shares` and `pool` retreat; the position's
    /// cost basis is reduced pro-rata (row cleared when fully sold). One
    /// transaction.
    pub fn amm_sell(&self, event_id: i64, idx: i64, user: i64, shares: i64) -> SqlResult<TradeOutcome> {
        if shares <= 0 {
            return Ok(TradeOutcome::Rejected);
        }
        self.ensure_row(user)?;
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let Some((b, fee_bps, _pool)) = load_amm_open(&tx, event_id)? else {
            return Ok(TradeOutcome::Unavailable);
        };
        let q = load_q(&tx, event_id)?;
        if idx < 0 || idx as usize >= q.len() {
            return Ok(TradeOutcome::Unavailable);
        }
        let (held, basis): (i64, i64) = tx
            .query_row(
                "SELECT shares, cost FROM positions WHERE event_id = ?1 AND market_idx = ?2 AND user = ?3",
                params![event_id, idx, user],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .unwrap_or((0, 0));
        if held < shares {
            return Ok(TradeOutcome::Rejected);
        }
        let q_whole: Vec<f64> = q.iter().map(|qi| *qi as f64 / SHARE as f64).collect();
        let refund_whole =
            lmsr::refund_to_sell(&q_whole, b as f64, idx as usize, shares as f64 / SHARE as f64);
        let refund = (refund_whole * COIN as f64).floor().max(0.0) as i64;
        let fee = mul_bps(refund, fee_bps);
        let proceeds = refund - fee;

        tx.execute(
            "UPDATE balance SET balance = balance + ?1 WHERE user = ?2",
            params![proceeds, user],
        )?;
        tx.execute(
            "UPDATE markets SET q_shares = q_shares - ?1 WHERE event_id = ?2 AND idx = ?3",
            params![shares, event_id, idx],
        )?;
        tx.execute("UPDATE events SET pool = pool - ?1 WHERE id = ?2", params![proceeds, event_id])?;
        if held == shares {
            tx.execute(
                "DELETE FROM positions WHERE event_id = ?1 AND market_idx = ?2 AND user = ?3",
                params![event_id, idx, user],
            )?;
        } else {
            // Reduce the cost basis in proportion to the shares sold.
            let basis_removed = (basis as i128 * shares as i128 / held as i128) as i64;
            tx.execute(
                "UPDATE positions SET shares = shares - ?1, cost = cost - ?2
                 WHERE event_id = ?3 AND market_idx = ?4 AND user = ?5",
                params![shares, basis_removed, event_id, idx, user],
            )?;
        }
        tx.commit()?;
        Ok(TradeOutcome::Filled { shares: -shares, coins: proceeds, fee })
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

    // --- AMM engine ---------------------------------------------------------

    fn opts(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    /// Every coin in the system: user balances + every event's escrow pool. AMM
    /// trading must leave this invariant (no minting / burning).
    fn coin_total(db: &Database) -> i64 {
        let conn = db.conn.lock();
        let bal: i64 = conn
            .query_row("SELECT COALESCE(SUM(balance), 0) FROM balance", [], |r| r.get(0))
            .unwrap();
        let pool: i64 = conn
            .query_row("SELECT COALESCE(SUM(pool), 0) FROM events", [], |r| r.get(0))
            .unwrap();
        bal + pool
    }

    fn pool_of(db: &Database, event_id: i64) -> i64 {
        db.conn
            .lock()
            .query_row("SELECT pool FROM events WHERE id = ?1", [event_id], |r| r.get(0))
            .unwrap()
    }

    fn price_of(db: &Database, event_id: i64, idx: usize) -> f64 {
        let conn = db.conn.lock();
        let q: Vec<i64> = conn
            .prepare("SELECT q_shares FROM markets WHERE event_id = ?1 ORDER BY idx")
            .unwrap()
            .query_map([event_id], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let b: i64 = conn
            .query_row("SELECT b_param FROM events WHERE id = ?1", [event_id], |r| r.get(0))
            .unwrap();
        let qf: Vec<f64> = q.iter().map(|x| *x as f64 / SHARE as f64).collect();
        crate::core::lmsr::price(&qf, b as f64, idx)
    }

    #[test]
    fn amm_create_moves_escrow_into_pool_conserving_total() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 1000 * COIN).unwrap();
        let before = coin_total(&db);
        let ev = db
            .create_amm_event(1, "Q?", "", "", None, 0, &opts(&["A", "B"]), 50, 200, 100)
            .unwrap()
            .unwrap();
        // Escrow = ⌈50·ln2·COIN⌉ ≈ 34.66 coins; debited from the host into the pool.
        let pool = pool_of(&db, ev);
        assert!(pool > 34 * COIN && pool < 35 * COIN);
        assert_eq!(db.get_user_info(1).unwrap().balance, 1000 * COIN - pool);
        assert_eq!(coin_total(&db), before, "escrow is a transfer, not a mint");
    }

    #[test]
    fn amm_create_rejected_when_host_cant_fund() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 5 * COIN).unwrap(); // < ~35-coin escrow
        let r = db
            .create_amm_event(1, "Q?", "", "", None, 0, &opts(&["A", "B"]), 50, 200, 100)
            .unwrap();
        assert!(r.is_none());
        assert_eq!(db.get_user_info(1).unwrap().balance, 5 * COIN, "nothing debited");
    }

    #[test]
    fn amm_buy_moves_spend_to_pool_mints_shares_raises_price() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 1000 * COIN).unwrap();
        db.force_change(2, 1000 * COIN).unwrap();
        let ev = db
            .create_amm_event(1, "Q?", "", "", None, 0, &opts(&["A", "B"]), 50, 200, 100)
            .unwrap()
            .unwrap();
        let total = coin_total(&db);
        let p0 = price_of(&db, ev, 0);

        let out = db.amm_buy(ev, 0, 2, 100 * COIN).unwrap();
        let TradeOutcome::Filled { shares, coins, fee } = out else {
            panic!("{out:?}")
        };
        assert!(shares > 0);
        assert_eq!(coins, -100 * COIN);
        assert_eq!(fee, 2 * COIN); // 2% of 100
        assert_eq!(db.get_user_info(2).unwrap().balance, 900 * COIN);
        assert_eq!(coin_total(&db), total, "buy conserves coins (no mint)");
        assert!(price_of(&db, ev, 0) > p0, "buying A raises its price");
    }

    #[test]
    fn amm_buy_then_full_sell_costs_only_fee_and_rounding() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 1000 * COIN).unwrap();
        db.force_change(2, 1000 * COIN).unwrap();
        // fee_bps = 0 so the only loss is sub-coin rounding.
        let ev = db
            .create_amm_event(1, "Q?", "", "", None, 0, &opts(&["A", "B"]), 50, 0, 100)
            .unwrap()
            .unwrap();
        let pool0 = pool_of(&db, ev);

        let TradeOutcome::Filled { shares, .. } = db.amm_buy(ev, 0, 2, 50 * COIN).unwrap() else {
            panic!()
        };
        let out = db.amm_sell(ev, 0, 2, shares).unwrap();
        let TradeOutcome::Filled { coins, .. } = out else {
            panic!("{out:?}")
        };
        // Can't profit, and loses < 0.01 coin to rounding (pool-favorable).
        assert!(coins <= 50 * COIN);
        assert!(coins >= 50 * COIN - COIN / 100);
        assert!(pool_of(&db, ev) >= pool0, "pool never dips below its seed");
        let held: i64 = db
            .conn
            .lock()
            .query_row(
                "SELECT COALESCE(SUM(shares), 0) FROM positions WHERE event_id = ?1 AND user = 2",
                [ev],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(held, 0, "position cleared after full sell");
    }

    #[test]
    fn amm_pool_stays_solvent_under_lopsided_trades() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 100_000 * COIN).unwrap();
        db.force_change(2, 100_000 * COIN).unwrap();
        let ev = db
            .create_amm_event(1, "Q?", "", "", None, 0, &opts(&["A", "B", "C"]), 50, 200, 100)
            .unwrap()
            .unwrap();
        for _ in 0..5 {
            db.amm_buy(ev, 0, 2, 200 * COIN).unwrap();
        }
        db.amm_buy(ev, 1, 2, 50 * COIN).unwrap();
        // Winner pays 1 coin/share = q_shares micro-coins (SHARE == COIN), so the
        // pool must cover the largest single-outcome obligation.
        let conn = db.conn.lock();
        let pool: i64 = conn
            .query_row("SELECT pool FROM events WHERE id = ?1", [ev], |r| r.get(0))
            .unwrap();
        let max_q: i64 = conn
            .query_row("SELECT MAX(q_shares) FROM markets WHERE event_id = ?1", [ev], |r| r.get(0))
            .unwrap();
        assert!(pool >= max_q, "pool {pool} must cover max winner payout {max_q}");
    }

    #[test]
    fn amm_buy_rejected_without_funds() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 1000 * COIN).unwrap();
        db.force_change(2, 3 * COIN).unwrap();
        let ev = db
            .create_amm_event(1, "Q?", "", "", None, 0, &opts(&["A", "B"]), 50, 200, 100)
            .unwrap()
            .unwrap();
        assert_eq!(db.amm_buy(ev, 0, 2, 100 * COIN).unwrap(), TradeOutcome::Rejected);
        assert_eq!(db.get_user_info(2).unwrap().balance, 3 * COIN, "nothing moved");
    }
}
