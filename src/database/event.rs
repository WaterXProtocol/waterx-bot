use super::{Database, COIN};
use crate::core::lmsr;
use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult, Transaction};

/// Shares are integer **micro-shares** (6-decimal fixed-point), mirroring `COIN`
/// for coins. A share of the winning outcome settles to exactly one coin, so
/// `SHARE == COIN`.
pub(crate) const SHARE: i64 = 1_000_000;

/// LMSR liquidity (whole shares) for a host `/predict` AMM — the "medium" preset
/// (seed escrow ≈ b·ln k ≈ 35–55 coins for 2–3 outcomes). `create_amm_event`
/// takes `b` as a parameter so other presets can be wired later; the `/predict`
/// builder uses this one.
pub const B_MEDIUM: i64 = 50;
/// Host trading fee (basis points): 2% default, 10% ceiling (the builder clamps).
pub const FEE_BPS_DEFAULT: i64 = 200;
pub const FEE_BPS_MAX: i64 = 1_000;

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

/// How a user's stake in one settled event resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimKind {
    /// Held the winning outcome — paid `coins`.
    Won,
    /// Held only losing outcomes — paid nothing.
    Lost,
    /// Event voided — cost basis refunded.
    Refunded,
}

/// One user's settlement result for one event (the unit `/claim` and `/settle`
/// report and DM).
#[derive(Debug, Clone)]
pub struct Payout {
    pub event_id: i64,
    pub title: String,
    pub user: i64,
    /// Micro-coins credited to the user for this event (0 for a pure loss).
    pub coins: i64,
    pub kind: ClaimKind,
}

/// A user's open share holding, for the `/assets` / `/bets` positions view (the
/// `event_id`/`market_idx` let the sell flow address the exact holding).
#[derive(Debug, Clone)]
pub struct PositionView {
    pub event_id: i64,
    pub market_idx: i64,
    pub event_title: String,
    pub outcome: String,
    /// Micro-shares held (== potential payout in micro-coins on a win).
    pub shares: i64,
    /// Micro-coin cost basis.
    pub cost: i64,
}

/// Everything the sell flow needs to price + execute a sell of one holding:
/// `kind` selects the price source (sourced → Gamma by `source_ref` slug; amm →
/// LMSR over `q_shares` with `b_param`), `held` caps the sell amount.
#[derive(Debug, Clone)]
pub struct SellContext {
    pub kind: String,
    pub source_ref: String,
    pub b_param: i64,
    pub fee_bps: i64,
    pub q_shares: Vec<i64>,
    pub held: i64,
    pub outcome: String,
    pub title: String,
}

/// Display precedence when a user held several outcomes in one event: a win
/// dominates a refund, which dominates a loss.
fn merge_kind(a: ClaimKind, b: ClaimKind) -> ClaimKind {
    use ClaimKind::*;
    match (a, b) {
        (Won, _) | (_, Won) => Won,
        (Refunded, _) | (_, Refunded) => Refunded,
        _ => Lost,
    }
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
                card_chat      INTEGER NOT NULL DEFAULT 0,
                card_msg       INTEGER NOT NULL DEFAULT 0,
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

    // --- Sourced (Polymarket-backed, house-banked) -------------------------

    /// Find the open `sourced` event for a Gamma `source_ref`, creating it (plus
    /// its outcome markets) on first trade. Lazily materialised like the old
    /// `wagers` rows were. Returns the event id.
    #[allow(clippy::too_many_arguments)]
    pub fn get_or_create_sourced_event(
        &self,
        source_ref: &str,
        title: &str,
        lang: &str,
        odds_fmt: &str,
        tz_offset: Option<i64>,
        ends_at: i64,
        outcomes: &[String],
        now: i64,
    ) -> SqlResult<i64> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        if let Some(id) = tx
            .query_row(
                "SELECT id FROM events WHERE kind = 'sourced' AND source_ref = ?1 AND state = 'open'",
                params![source_ref],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
        {
            return Ok(id); // read-only tx drops → rollback
        }
        tx.execute(
            "INSERT INTO events
                (kind, source_ref, title, creator, lang, odds_fmt, tz_offset, ends_at, state, pool, created_at)
             VALUES ('sourced', ?1, ?2, 0, ?3, ?4, ?5, ?6, 'open', 0, ?7)",
            params![source_ref, title, lang, odds_fmt, tz_offset, ends_at, now],
        )?;
        let id = tx.last_insert_rowid();
        for (idx, name) in outcomes.iter().enumerate() {
            tx.execute(
                "INSERT INTO markets (event_id, idx, name, q_shares) VALUES (?1, ?2, ?3, 0)",
                params![id, idx as i64, name],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// Buy YES shares of outcome `idx` in a sourced event at the live Gamma price
    /// (`price_cents` ∈ (0, 100]). House-banked: `shares = ⌊spend / price⌋`
    /// (floored), the spend leaves circulation, no pool. One transaction.
    pub fn sourced_buy(
        &self,
        event_id: i64,
        idx: i64,
        user: i64,
        spend: i64,
        price_cents: f64,
    ) -> SqlResult<TradeOutcome> {
        if spend <= 0 || !(price_cents.is_finite() && price_cents > 0.0) {
            return Ok(TradeOutcome::Rejected);
        }
        self.ensure_row(user)?;
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let open: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM events WHERE id = ?1 AND kind = 'sourced' AND state = 'open'",
                params![event_id],
                |r| r.get(0),
            )
            .optional()?;
        let has_idx: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM markets WHERE event_id = ?1 AND idx = ?2",
                params![event_id, idx],
                |r| r.get(0),
            )
            .optional()?;
        if open.is_none() || has_idx.is_none() {
            return Ok(TradeOutcome::Unavailable);
        }
        // shares = spend / (price_cents/100), in micro-shares (SHARE == COIN).
        let shares = (spend as f64 * 100.0 / price_cents).floor() as i64;
        if shares <= 0 {
            return Ok(TradeOutcome::Rejected);
        }
        let debited = tx.execute(
            "UPDATE balance SET balance = balance - ?1 WHERE user = ?2 AND balance - ?1 >= 0",
            params![spend, user],
        )?;
        if debited != 1 {
            return Ok(TradeOutcome::Rejected);
        }
        add_position(&tx, event_id, idx, user, shares, spend)?;
        tx.execute(
            "UPDATE markets SET q_shares = q_shares + ?1 WHERE event_id = ?2 AND idx = ?3",
            params![shares, event_id, idx],
        )?;
        tx.commit()?;
        Ok(TradeOutcome::Filled { shares, coins: -spend, fee: 0 })
    }

    /// Sell `shares` micro-shares of outcome `idx` in a sourced event at the live
    /// price. House-banked: credit `⌊shares · price⌋` (floored), no pool. One
    /// transaction.
    pub fn sourced_sell(
        &self,
        event_id: i64,
        idx: i64,
        user: i64,
        shares: i64,
        price_cents: f64,
    ) -> SqlResult<TradeOutcome> {
        if shares <= 0 || !(price_cents.is_finite() && price_cents > 0.0) {
            return Ok(TradeOutcome::Rejected);
        }
        self.ensure_row(user)?;
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let open: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM events WHERE id = ?1 AND kind = 'sourced' AND state = 'open'",
                params![event_id],
                |r| r.get(0),
            )
            .optional()?;
        if open.is_none() {
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
        let proceeds = (shares as f64 * price_cents / 100.0).floor() as i64;
        tx.execute(
            "INSERT OR IGNORE INTO balance (user, balance, fruit) VALUES (?1, 0, '')",
            params![user],
        )?;
        tx.execute(
            "UPDATE balance SET balance = balance + ?1 WHERE user = ?2",
            params![proceeds, user],
        )?;
        tx.execute(
            "UPDATE markets SET q_shares = q_shares - ?1 WHERE event_id = ?2 AND idx = ?3",
            params![shares, event_id, idx],
        )?;
        if held == shares {
            tx.execute(
                "DELETE FROM positions WHERE event_id = ?1 AND market_idx = ?2 AND user = ?3",
                params![event_id, idx, user],
            )?;
        } else {
            let basis_removed = (basis as i128 * shares as i128 / held as i128) as i64;
            tx.execute(
                "UPDATE positions SET shares = shares - ?1, cost = cost - ?2
                 WHERE event_id = ?3 AND market_idx = ?4 AND user = ?5",
                params![shares, basis_removed, event_id, idx, user],
            )?;
        }
        tx.commit()?;
        Ok(TradeOutcome::Filled { shares: -shares, coins: proceeds, fee: 0 })
    }

    // --- Resolution & settlement -------------------------------------------

    /// Record the winning outcome on an open event (sourced: from the Gamma
    /// oracle; amm: declared by the host). Returns `true` if it flipped an open
    /// event to `resolved`. Payouts happen later via [`Database::claim`] /
    /// [`Database::settle_all_sourced`].
    pub fn resolve_event(&self, event_id: i64, winning_idx: i64, now: i64) -> SqlResult<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE events SET state = 'resolved', winning_market = ?1, resolved_at = ?2
             WHERE id = ?3 AND state = 'open'",
            params![winning_idx, now, event_id],
        )?;
        Ok(n == 1)
    }

    /// Void an open event (no clear winner / 50-50). Settlement then refunds every
    /// holder's cost basis. Returns `true` if it flipped an open event to `void`.
    pub fn void_event(&self, event_id: i64, now: i64) -> SqlResult<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE events SET state = 'void', resolved_at = ?1 WHERE id = ?2 AND state = 'open'",
            params![now, event_id],
        )?;
        Ok(n == 1)
    }

    /// Settle the positions of one resolved/void event — for one user
    /// (`only_user`) or everyone (`None`). Winning shares pay 1 coin each; losers
    /// 0; a void refunds cost basis. AMM payouts/refunds come **from the pool**
    /// (and the residual returns to the host once the last position is settled,
    /// flipping the event to `closed`); sourced payouts are house-banked. One
    /// transaction. Returns one [`Payout`] per affected user.
    fn settle_event(&self, event_id: i64, only_user: Option<i64>) -> SqlResult<Vec<Payout>> {
        use std::collections::BTreeMap;
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let Some((kind, state, winning, mut pool, creator, title)) = tx
            .query_row(
                "SELECT kind, state, winning_market, pool, creator, title FROM events WHERE id = ?1",
                params![event_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()?
        else {
            return Ok(vec![]);
        };
        let voided = state == "void";
        if !voided && state != "resolved" {
            return Ok(vec![]); // not settleable yet
        }
        let amm = kind == "amm";

        // (user, market_idx, shares, cost)
        let positions: Vec<(i64, i64, i64, i64)> = if let Some(u) = only_user {
            let mut stmt = tx.prepare(
                "SELECT user, market_idx, shares, cost FROM positions WHERE event_id = ?1 AND user = ?2",
            )?;
            let v = stmt
                .query_map(params![event_id, u], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<SqlResult<Vec<_>>>()?;
            v
        } else {
            let mut stmt =
                tx.prepare("SELECT user, market_idx, shares, cost FROM positions WHERE event_id = ?1")?;
            let v = stmt
                .query_map(params![event_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<SqlResult<Vec<_>>>()?;
            v
        };

        let mut per_user: BTreeMap<i64, (i64, ClaimKind)> = BTreeMap::new();
        for (user, idx, shares, cost) in &positions {
            let (coins, kind_one) = if voided {
                (*cost, ClaimKind::Refunded)
            } else if Some(*idx) == winning {
                (*shares, ClaimKind::Won) // SHARE == COIN ⇒ 1 share = 1 coin
            } else {
                (0, ClaimKind::Lost)
            };
            if coins > 0 {
                tx.execute(
                    "INSERT OR IGNORE INTO balance (user, balance, fruit) VALUES (?1, 0, '')",
                    params![user],
                )?;
                tx.execute(
                    "UPDATE balance SET balance = balance + ?1 WHERE user = ?2",
                    params![coins, user],
                )?;
                if amm {
                    pool -= coins; // AMM payouts/refunds are funded by the pool
                }
            }
            tx.execute(
                "DELETE FROM positions WHERE event_id = ?1 AND market_idx = ?2 AND user = ?3",
                params![event_id, idx, user],
            )?;
            let e = per_user.entry(*user).or_insert((0, ClaimKind::Lost));
            e.0 += coins;
            e.1 = merge_kind(e.1, kind_one);
        }

        if amm {
            tx.execute("UPDATE events SET pool = ?1 WHERE id = ?2", params![pool, event_id])?;
        }
        // When the last position is gone, finalise: AMM residual (seed + fees −
        // payouts) returns to the host, and the event closes.
        let remaining: i64 = tx.query_row(
            "SELECT COUNT(*) FROM positions WHERE event_id = ?1",
            params![event_id],
            |r| r.get(0),
        )?;
        if remaining == 0 {
            if amm && pool > 0 {
                tx.execute(
                    "INSERT OR IGNORE INTO balance (user, balance, fruit) VALUES (?1, 0, '')",
                    params![creator],
                )?;
                tx.execute(
                    "UPDATE balance SET balance = balance + ?1 WHERE user = ?2",
                    params![pool, creator],
                )?;
                tx.execute("UPDATE events SET pool = 0 WHERE id = ?1", params![event_id])?;
            }
            tx.execute("UPDATE events SET state = 'closed' WHERE id = ?1", params![event_id])?;
        }
        tx.commit()?;

        Ok(per_user
            .into_iter()
            .map(|(user, (coins, kind))| Payout { event_id, title: title.clone(), user, coins, kind })
            .collect())
    }

    /// Collect a user's winnings: settle their positions across every
    /// resolved/void event they hold. Returns the per-event payouts (for the DM
    /// summary).
    pub fn claim(&self, user: i64) -> SqlResult<Vec<Payout>> {
        let ids: Vec<i64> = {
            let conn = self.conn.lock();
            let mut stmt = conn.prepare(
                "SELECT DISTINCT p.event_id FROM positions p JOIN events e ON e.id = p.event_id
                 WHERE p.user = ?1 AND e.state IN ('resolved', 'void')",
            )?;
            let v = stmt.query_map(params![user], |r| r.get(0))?.collect::<SqlResult<Vec<_>>>()?;
            v
        };
        let mut out = Vec::new();
        for ev in ids {
            out.extend(self.settle_event(ev, Some(user))?);
        }
        Ok(out)
    }

    /// Public `/settle` sweep — settle **all** positions of every resolved/void
    /// **sourced** event (Polymarket is the oracle). AMM events are skipped (they
    /// settle via the host + `/claim`). Returns every payout made.
    pub fn settle_all_sourced(&self) -> SqlResult<Vec<Payout>> {
        let ids: Vec<i64> = {
            let conn = self.conn.lock();
            let mut stmt = conn.prepare(
                "SELECT id FROM events WHERE kind = 'sourced' AND state IN ('resolved', 'void')
                 AND EXISTS (SELECT 1 FROM positions WHERE event_id = events.id)",
            )?;
            let v = stmt.query_map([], |r| r.get(0))?.collect::<SqlResult<Vec<_>>>()?;
            v
        };
        let mut out = Vec::new();
        for ev in ids {
            out.extend(self.settle_event(ev, None)?);
        }
        Ok(out)
    }

    /// The user's open share holdings (events still `open`), joined with their
    /// event title + outcome name — for the `/assets` / `/bets` view. Resolved
    /// events are excluded (they're collected via `/claim`).
    pub fn user_positions(&self, user: i64) -> SqlResult<Vec<PositionView>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT p.event_id, p.market_idx, e.title, m.name, p.shares, p.cost
             FROM positions p
             JOIN events e ON e.id = p.event_id
             JOIN markets m ON m.event_id = p.event_id AND m.idx = p.market_idx
             WHERE p.user = ?1 AND e.state = 'open'
             ORDER BY p.event_id, p.market_idx",
        )?;
        let v = stmt
            .query_map(params![user], |r| {
                Ok(PositionView {
                    event_id: r.get(0)?,
                    market_idx: r.get(1)?,
                    event_title: r.get(2)?,
                    outcome: r.get(3)?,
                    shares: r.get(4)?,
                    cost: r.get(5)?,
                })
            })?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(v)
    }

    /// Context for selling one open holding (`None` if the event isn't open or the
    /// user holds nothing there). The handler prices the proceeds from this:
    /// sourced via Gamma (`source_ref`), amm via [`Database::amm_sell_quote`].
    pub fn sell_context(&self, event_id: i64, idx: i64, user: i64) -> SqlResult<Option<SellContext>> {
        let conn = self.conn.lock();
        let ev = conn
            .query_row(
                "SELECT kind, COALESCE(source_ref, ''), COALESCE(b_param, 0), fee_bps, title
                 FROM events WHERE id = ?1 AND state = 'open'",
                params![event_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((kind, source_ref, b_param, fee_bps, title)) = ev else {
            return Ok(None);
        };
        let held: i64 = conn
            .query_row(
                "SELECT shares FROM positions WHERE event_id = ?1 AND market_idx = ?2 AND user = ?3",
                params![event_id, idx, user],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        if held <= 0 {
            return Ok(None);
        }
        let outcome: String = conn
            .query_row(
                "SELECT name FROM markets WHERE event_id = ?1 AND idx = ?2",
                params![event_id, idx],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_default();
        let q_shares = {
            let mut stmt =
                conn.prepare("SELECT q_shares FROM markets WHERE event_id = ?1 ORDER BY idx")?;
            let v = stmt
                .query_map(params![event_id], |r| r.get(0))?
                .collect::<SqlResult<Vec<i64>>>()?;
            v
        };
        Ok(Some(SellContext { kind, source_ref, b_param, fee_bps, q_shares, held, outcome, title }))
    }

    /// Read-only proceeds quote for selling `shares` micro-shares of an AMM
    /// outcome (LMSR refund, floored, minus the host fee) — for the sell builder's
    /// live preview. Mirrors [`Database::amm_sell`]'s math without mutating.
    pub fn amm_sell_quote(&self, event_id: i64, idx: i64, shares: i64) -> SqlResult<Option<i64>> {
        if shares <= 0 {
            return Ok(None);
        }
        let conn = self.conn.lock();
        let row = conn
            .query_row(
                "SELECT b_param, fee_bps FROM events WHERE id = ?1 AND kind = 'amm' AND state = 'open'",
                params![event_id],
                |r| Ok((r.get::<_, Option<i64>>(0)?.unwrap_or(0), r.get::<_, i64>(1)?)),
            )
            .optional()?;
        let Some((b, fee_bps)) = row else {
            return Ok(None);
        };
        let q = {
            let mut stmt =
                conn.prepare("SELECT q_shares FROM markets WHERE event_id = ?1 ORDER BY idx")?;
            let v = stmt
                .query_map(params![event_id], |r| r.get(0))?
                .collect::<SqlResult<Vec<i64>>>()?;
            v
        };
        if idx < 0 || idx as usize >= q.len() {
            return Ok(None);
        }
        let q_whole: Vec<f64> = q.iter().map(|x| *x as f64 / SHARE as f64).collect();
        let refund = (lmsr::refund_to_sell(&q_whole, b as f64, idx as usize, shares as f64 / SHARE as f64)
            * COIN as f64)
            .floor()
            .max(0.0) as i64;
        Ok(Some(refund - mul_bps(refund, fee_bps)))
    }

    /// Record where an AMM event's shared board message lives, so a later bet can
    /// re-render the board in place.
    pub fn set_event_card(&self, event_id: i64, chat: i64, msg: i64) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE events SET card_chat = ?1, card_msg = ?2 WHERE id = ?3",
            params![chat, msg, event_id],
        )?;
        Ok(())
    }

    /// The `(chat, msg)` of an event's board, or `None` if not posted yet.
    pub fn event_card(&self, event_id: i64) -> SqlResult<Option<(i64, i64)>> {
        let conn = self.conn.lock();
        let r = conn
            .query_row(
                "SELECT card_chat, card_msg FROM events WHERE id = ?1",
                params![event_id],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?;
        Ok(r.filter(|(c, m)| *c != 0 && *m != 0))
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

    // --- Sourced (house-banked) + settlement --------------------------------

    fn sourced(db: &Database) -> i64 {
        db.get_or_create_sourced_event("tkr", "A vs B", "", "", None, 0, &opts(&["A", "B"]), 100)
            .unwrap()
    }

    fn state_of(db: &Database, ev: i64) -> String {
        db.conn
            .lock()
            .query_row("SELECT state FROM events WHERE id = ?1", [ev], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn sourced_buy_burns_spend_and_mints_shares_at_price() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(2, 100 * COIN).unwrap();
        let ev = sourced(&db);
        // Buy 10 coins of A at 50¢ → 20 shares; spend leaves circulation (no pool).
        let out = db.sourced_buy(ev, 0, 2, 10 * COIN, 50.0).unwrap();
        let TradeOutcome::Filled { shares, coins, fee } = out else {
            panic!("{out:?}")
        };
        assert_eq!(shares, 20 * SHARE);
        assert_eq!(coins, -10 * COIN);
        assert_eq!(fee, 0);
        assert_eq!(db.get_user_info(2).unwrap().balance, 90 * COIN);
        assert_eq!(pool_of(&db, ev), 0, "sourced is house-banked, no pool");
    }

    #[test]
    fn sourced_win_pays_one_coin_per_share_house_minted() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(2, 100 * COIN).unwrap();
        let ev = sourced(&db);
        db.sourced_buy(ev, 0, 2, 10 * COIN, 50.0).unwrap(); // 20 shares, bal 90
        assert!(db.resolve_event(ev, 0, 200).unwrap());
        let payouts = db.claim(2).unwrap();
        assert_eq!(payouts.len(), 1);
        assert_eq!(payouts[0].kind, ClaimKind::Won);
        assert_eq!(payouts[0].coins, 20 * COIN); // 20 winning shares → 20 coins
        assert_eq!(db.get_user_info(2).unwrap().balance, 110 * COIN); // 90 + 20 (house mint)
        assert_eq!(state_of(&db, ev), "closed");
    }

    #[test]
    fn sourced_loss_pays_zero_and_clears() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(2, 100 * COIN).unwrap();
        let ev = sourced(&db);
        db.sourced_buy(ev, 0, 2, 10 * COIN, 50.0).unwrap();
        db.resolve_event(ev, 1, 200).unwrap(); // B wins; user held A
        let payouts = db.claim(2).unwrap();
        assert_eq!(payouts[0].kind, ClaimKind::Lost);
        assert_eq!(payouts[0].coins, 0);
        assert_eq!(db.get_user_info(2).unwrap().balance, 90 * COIN); // lost the 10
    }

    #[test]
    fn amm_settlement_pays_winners_from_pool_residual_to_host_conserved() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 1000 * COIN).unwrap(); // host
        db.force_change(2, 1000 * COIN).unwrap(); // backs A (winner)
        db.force_change(3, 1000 * COIN).unwrap(); // backs B (loser)
        let before = coin_total(&db);
        let ev = db
            .create_amm_event(1, "Q", "", "", None, 0, &opts(&["A", "B"]), 50, 200, 100)
            .unwrap()
            .unwrap();
        db.amm_buy(ev, 0, 2, 80 * COIN).unwrap();
        db.amm_buy(ev, 1, 3, 40 * COIN).unwrap();
        db.resolve_event(ev, 0, 200).unwrap(); // A wins
        db.claim(2).unwrap();
        db.claim(3).unwrap();
        assert_eq!(coin_total(&db), before, "AMM settlement conserves total coins");
        assert_eq!(pool_of(&db, ev), 0, "pool drained to winner + host residual");
        assert_eq!(state_of(&db, ev), "closed");
        let n: i64 = db
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM positions WHERE event_id = ?1", [ev], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn void_refunds_cost_basis_and_amm_returns_escrow_to_host() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 1000 * COIN).unwrap();
        db.force_change(2, 1000 * COIN).unwrap();
        let before = coin_total(&db);
        let ev = db
            .create_amm_event(1, "Q", "", "", None, 0, &opts(&["A", "B"]), 50, 200, 100)
            .unwrap()
            .unwrap();
        db.amm_buy(ev, 0, 2, 60 * COIN).unwrap();
        db.void_event(ev, 200).unwrap();
        let payouts = db.claim(2).unwrap();
        assert_eq!(payouts[0].kind, ClaimKind::Refunded);
        assert_eq!(db.get_user_info(2).unwrap().balance, 1000 * COIN, "trader refunded cost basis");
        assert_eq!(db.get_user_info(1).unwrap().balance, 1000 * COIN, "host got the escrow back");
        assert_eq!(coin_total(&db), before);
        assert_eq!(pool_of(&db, ev), 0);
    }

    #[test]
    fn settle_all_sourced_skips_amm_events() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 1000 * COIN).unwrap();
        db.force_change(2, 1000 * COIN).unwrap();
        let src = sourced(&db);
        db.sourced_buy(src, 0, 2, 10 * COIN, 50.0).unwrap();
        db.resolve_event(src, 0, 200).unwrap();
        let amm = db
            .create_amm_event(1, "Q", "", "", None, 0, &opts(&["A", "B"]), 50, 200, 100)
            .unwrap()
            .unwrap();
        db.amm_buy(amm, 0, 2, 20 * COIN).unwrap();
        db.resolve_event(amm, 0, 200).unwrap();

        let payouts = db.settle_all_sourced().unwrap();
        assert!(payouts.iter().all(|p| p.event_id == src), "only the sourced event settled");
        assert_eq!(state_of(&db, src), "closed");
        assert_eq!(state_of(&db, amm), "resolved", "amm untouched by /settle");
    }

    #[test]
    fn user_positions_lists_open_holdings_only() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(2, 100 * COIN).unwrap();
        let ev = sourced(&db); // outcomes ["A", "B"]
        db.sourced_buy(ev, 0, 2, 10 * COIN, 50.0).unwrap(); // 20 shares of A
        let v = db.user_positions(2).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].event_id, ev);
        assert_eq!(v[0].market_idx, 0);
        assert_eq!(v[0].outcome, "A");
        assert_eq!(v[0].shares, 20 * SHARE);
        assert_eq!(v[0].cost, 10 * COIN);
        // Once resolved, the event leaves 'open' → dropped from the live view
        // (collected via /claim instead).
        db.resolve_event(ev, 0, 200).unwrap();
        assert!(db.user_positions(2).unwrap().is_empty());
    }

    #[test]
    fn sell_context_and_amm_quote() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 1000 * COIN).unwrap();
        db.force_change(2, 1000 * COIN).unwrap();
        // Sourced context.
        let src = sourced(&db);
        db.sourced_buy(src, 0, 2, 10 * COIN, 50.0).unwrap(); // 20 shares of A
        let c = db.sell_context(src, 0, 2).unwrap().unwrap();
        assert_eq!(c.kind, "sourced");
        assert_eq!(c.held, 20 * SHARE);
        assert_eq!(c.outcome, "A");
        assert!(db.sell_context(src, 1, 2).unwrap().is_none(), "nothing held on idx 1");
        // AMM read-only sell quote can't exceed the spend (no rounding profit).
        let amm = db
            .create_amm_event(1, "Q", "", "", None, 0, &opts(&["A", "B"]), 50, 200, 100)
            .unwrap()
            .unwrap();
        let TradeOutcome::Filled { shares, .. } = db.amm_buy(amm, 0, 2, 50 * COIN).unwrap() else {
            panic!()
        };
        let q = db.amm_sell_quote(amm, 0, shares).unwrap().unwrap();
        assert!(q > 0 && q <= 50 * COIN);
    }

    #[test]
    fn event_card_round_trips() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 1000 * COIN).unwrap();
        let ev = db
            .create_amm_event(1, "Q", "", "", None, 0, &opts(&["A", "B"]), B_MEDIUM, FEE_BPS_DEFAULT, 100)
            .unwrap()
            .unwrap();
        assert!(db.event_card(ev).unwrap().is_none(), "no card until posted");
        db.set_event_card(ev, -100, 55).unwrap();
        assert_eq!(db.event_card(ev).unwrap(), Some((-100, 55)));
    }
}
