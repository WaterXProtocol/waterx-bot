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

/// Minimum pooled seed (whole coins) for a funding stage to open into trading.
/// Below this the event **voids** and refunds every LP — it keeps `b ≥ 1` and the
/// LMSR solvent even for a fully one-sided opening (worst-case subsidy
/// `b·ln(1/PRICE_FLOOR)`).
pub const MIN_SEED: i64 = 10;

/// Result of an `add_liquidity` attempt against a funding-stage event.
#[derive(Debug, PartialEq, Eq)]
pub enum FundOutcome {
    /// Contributed `total` micro-coins to the pool.
    Funded { total: i64 },
    /// Not enough balance, or a zero / malformed allocation. Nothing written.
    Rejected,
    /// Event missing, not an AMM event, or its funding window is closed.
    Unavailable,
}

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

/// A user's open liquidity-provider stake in one host AMM event, for `/assets`.
/// `liquidity` rows only survive while the event is unsettled (they're deleted
/// when the residual is distributed), so every row is live capital in a pool.
#[derive(Debug, Clone)]
pub struct LiquidityView {
    pub event_id: i64,
    pub event_title: String,
    /// Event state — `funding` (still seeding) or `open` (live trading).
    pub state: String,
    /// Micro-coins this user has committed to the pool.
    pub contributed: i64,
}

/// Render data for a host AMM (`/predict`) event's shared board: the question +
/// state + each outcome's net shares (the renderer derives prices via LMSR over
/// `b_param`). `winning_market` is set once the host resolves.
#[derive(Debug, Clone)]
pub struct AmmBoard {
    pub event_id: i64,
    pub host: i64,
    pub question: String,
    pub state: String,
    pub lang: String,
    pub odds_fmt: String,
    pub tz_offset: i64,
    pub ends_at: i64,
    pub b_param: i64,
    pub fee_bps: i64,
    pub winning_market: Option<i64>,
    pub pool: i64,
    /// `(outcome name, net q_shares)` ordered by idx.
    pub options: Vec<(String, i64)>,
    /// When the funding window closes → trading opens (`state == "funding"` only).
    pub open_at: i64,
    /// Per-outcome funding-stage allocation (micro-coins), ordered by idx — the
    /// price-discovery tally shown on the funding board.
    pub funded: Vec<i64>,
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

/// Credit `amount` micro-coins to `user`, creating the balance row if absent.
fn credit(tx: &Transaction, user: i64, amount: i64) -> SqlResult<()> {
    tx.execute(
        "INSERT OR IGNORE INTO balance (user, balance, fruit) VALUES (?1, 0, '')",
        params![user],
    )?;
    tx.execute("UPDATE balance SET balance = balance + ?1 WHERE user = ?2", params![amount, user])?;
    Ok(())
}

/// The event's per-outcome funding-stage allocation (micro-coins), ordered by `idx`.
fn load_funded(tx: &Transaction, event_id: i64) -> SqlResult<Vec<i64>> {
    let mut stmt = tx.prepare("SELECT funded FROM markets WHERE event_id = ?1 ORDER BY idx")?;
    let v = stmt.query_map(params![event_id], |r| r.get(0))?.collect::<SqlResult<Vec<i64>>>()?;
    Ok(v)
}

/// Refund each LP their full contribution and delete the `liquidity` rows — the
/// path for an under-seeded / never-traded funding stage. Returns coins refunded.
fn refund_liquidity(tx: &Transaction, event_id: i64) -> SqlResult<i64> {
    let lps: Vec<(i64, i64)> = {
        let mut stmt = tx.prepare("SELECT user, contributed FROM liquidity WHERE event_id = ?1")?;
        let v = stmt
            .query_map(params![event_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<SqlResult<Vec<_>>>()?;
        v
    };
    let mut total = 0i64;
    for (user, amount) in &lps {
        if *amount > 0 {
            credit(tx, *user, *amount)?;
            total += amount;
        }
    }
    tx.execute("DELETE FROM liquidity WHERE event_id = ?1", params![event_id])?;
    Ok(total)
}

/// Split `residual` micro-coins among the event's LPs pro-rata by contribution
/// (largest-remainder, exact) and delete the `liquidity` rows. Falls back to the
/// `creator` when there are no LP rows (defensive — every real AMM event seeds at
/// least the host's row).
fn distribute_residual(
    tx: &Transaction,
    event_id: i64,
    creator: i64,
    residual: i64,
) -> SqlResult<()> {
    let lps: Vec<(i64, i64)> = {
        let mut stmt = tx.prepare("SELECT user, contributed FROM liquidity WHERE event_id = ?1")?;
        let v = stmt
            .query_map(params![event_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<SqlResult<Vec<_>>>()?;
        v
    };
    if residual > 0 {
        if lps.is_empty() {
            credit(tx, creator, residual)?;
        } else {
            let contributions: Vec<i64> = lps.iter().map(|(_, c)| *c).collect();
            let split = crate::core::lmsr_fund::pro_rata(&contributions, residual);
            for ((user, _), amount) in lps.iter().zip(split.iter()) {
                if *amount > 0 {
                    credit(tx, *user, *amount)?;
                }
            }
        }
    }
    tx.execute("DELETE FROM liquidity WHERE event_id = ?1", params![event_id])?;
    Ok(())
}

/// Lazily finalize a funding-stage event whose window has closed (`now ≥ open_at`):
/// if the pooled seed clears [`MIN_SEED`], compute the opening `b`/`q⁰` from the
/// per-outcome funding tally and flip `funding → open`; otherwise **void** it and
/// refund every LP. No-op if the event isn't a funding-stage AMM event, or the
/// window is still open. Runs inside the caller's transaction.
fn maybe_open_funding(tx: &Transaction, event_id: i64, now: i64) -> SqlResult<()> {
    let Some((state, pool, open_at)) = tx
        .query_row(
            "SELECT state, pool, open_at FROM events WHERE id = ?1 AND kind = 'amm'",
            params![event_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)),
        )
        .optional()?
    else {
        return Ok(());
    };
    if state != "funding" || now < open_at {
        return Ok(());
    }
    // Under-seeded → void + refund LPs (no trading happened, so no positions).
    if pool < MIN_SEED.saturating_mul(COIN) {
        refund_liquidity(tx, event_id)?;
        tx.execute(
            "UPDATE events SET state = 'void', pool = 0, resolved_at = ?1 WHERE id = ?2",
            params![now, event_id],
        )?;
        return Ok(());
    }
    // Discover the opening: prices from the funding tally, b from the pooled seed.
    let funded = load_funded(tx, event_id)?;
    let prices = crate::core::lmsr_fund::opening_prices(&funded);
    let seed_whole = pool as f64 / COIN as f64;
    let b = (crate::core::lmsr_fund::seed_to_b(seed_whole, &prices).floor() as i64).max(1);
    let q0 = crate::core::lmsr_fund::opening_q(&prices, b as f64);
    for (idx, qi) in q0.iter().enumerate() {
        let q_shares = (qi * SHARE as f64).round() as i64;
        tx.execute(
            "UPDATE markets SET q_shares = ?1 WHERE event_id = ?2 AND idx = ?3",
            params![q_shares, event_id, idx as i64],
        )?;
    }
    tx.execute(
        "UPDATE events SET state = 'open', b_param = ?1 WHERE id = ?2",
        params![b, event_id],
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
    ///   and `reset_events` refunds).
    ///
    /// Co-located with the trade engine so the schema and read/write code can't drift.
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
                resolved_at    INTEGER NOT NULL DEFAULT 0,
                open_at        INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS markets (
                event_id INTEGER NOT NULL,
                idx      INTEGER NOT NULL,
                name     TEXT    NOT NULL DEFAULT '',
                q_shares INTEGER NOT NULL DEFAULT 0,
                funded   INTEGER NOT NULL DEFAULT 0,
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
        // Per-LP funding-stage contributions (micro-coins). LP share of the pool =
        // contributed / Σ contributed; the residual + fees are split back pro-rata
        // at resolution (see `core::lmsr_fund`).
        conn.execute(
            "CREATE TABLE IF NOT EXISTS liquidity (
                event_id    INTEGER NOT NULL,
                user        INTEGER NOT NULL,
                contributed INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (event_id, user)
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
        // The host is the sole LP (their seed == the pool), so resolution / void /
        // reset distribute the residual back to them through the same `liquidity`
        // path multi-LP funding uses.
        tx.execute(
            "INSERT INTO liquidity (event_id, user, contributed) VALUES (?1, ?2, ?3)",
            params![event_id, host, escrow],
        )?;
        tx.commit()?;
        Ok(Some(event_id))
    }

    /// Open a host AMM event in the **funding stage**: insert the event
    /// (`state='funding'`, `pool=0`, `b_param=0`) + its markets (`funded=0`) and set
    /// `open_at` (when the funding window closes → trading opens, lazily). Charges
    /// nothing — liquidity arrives via [`add_liquidity`]; the opening prices + `b`
    /// are discovered from the pooled allocation when funding closes
    /// ([`maybe_open_funding`]). Returns the new event id.
    #[allow(clippy::too_many_arguments)]
    pub fn create_funding_event(
        &self,
        host: i64,
        title: &str,
        lang: &str,
        odds_fmt: &str,
        tz_offset: Option<i64>,
        ends_at: i64,
        options: &[String],
        fee_bps: i64,
        open_at: i64,
        now: i64,
    ) -> SqlResult<i64> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO events
                (kind, title, creator, lang, odds_fmt, tz_offset, ends_at, state,
                 b_param, fee_bps, pool, open_at, created_at)
             VALUES ('amm', ?1, ?2, ?3, ?4, ?5, ?6, 'funding', 0, ?7, 0, ?8, ?9)",
            params![title, host, lang, odds_fmt, tz_offset, ends_at, fee_bps, open_at, now],
        )?;
        let event_id = tx.last_insert_rowid();
        for (idx, name) in options.iter().enumerate() {
            tx.execute(
                "INSERT INTO markets (event_id, idx, name, q_shares, funded) VALUES (?1, ?2, ?3, 0, 0)",
                params![event_id, idx as i64, name],
            )?;
        }
        tx.commit()?;
        Ok(event_id)
    }

    /// Provide liquidity to a funding-stage event: `alloc[idx]` micro-coins toward
    /// each outcome (uneven is the point — the allocation sets the opening price).
    /// Atomically debits the LP `Σ alloc`, tallies it into `markets.funded` (price
    /// discovery), their `liquidity` row (pool share), and `events.pool` (seed).
    /// Rejected if the window has closed, the alloc is empty/malformed, or the LP
    /// can't afford it. One transaction.
    pub fn add_liquidity(&self, event_id: i64, user: i64, alloc: &[i64], now: i64) -> SqlResult<FundOutcome> {
        let total: i64 = alloc.iter().map(|&a| a.max(0)).sum();
        if total <= 0 || alloc.iter().any(|&a| a < 0) {
            return Ok(FundOutcome::Rejected);
        }
        self.ensure_row(user)?;
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        // Window must still be open: a funding event, not yet past open_at.
        let Some((state, open_at, k)) = tx
            .query_row(
                "SELECT state, open_at, (SELECT COUNT(*) FROM markets WHERE event_id = e.id)
                 FROM events e WHERE id = ?1 AND kind = 'amm'",
                params![event_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)),
            )
            .optional()?
        else {
            return Ok(FundOutcome::Unavailable);
        };
        if state != "funding" || now >= open_at || alloc.len() as i64 != k {
            return Ok(FundOutcome::Unavailable);
        }
        let debited = tx.execute(
            "UPDATE balance SET balance = balance - ?1 WHERE user = ?2 AND balance - ?1 >= 0",
            params![total, user],
        )?;
        if debited != 1 {
            return Ok(FundOutcome::Rejected); // insufficient funds → rollback
        }
        for (idx, &a) in alloc.iter().enumerate() {
            if a > 0 {
                tx.execute(
                    "UPDATE markets SET funded = funded + ?1 WHERE event_id = ?2 AND idx = ?3",
                    params![a, event_id, idx as i64],
                )?;
            }
        }
        tx.execute(
            "INSERT INTO liquidity (event_id, user, contributed) VALUES (?1, ?2, ?3)
             ON CONFLICT(event_id, user) DO UPDATE SET contributed = contributed + excluded.contributed",
            params![event_id, user, total],
        )?;
        tx.execute("UPDATE events SET pool = pool + ?1 WHERE id = ?2", params![total, event_id])?;
        tx.commit()?;
        Ok(FundOutcome::Funded { total })
    }

    /// Lazily finalize a funding stage whose window has closed, in its **own
    /// committed transaction**, so a following trade (or anything) sees the
    /// opened-or-voided state. Kept separate from the trade tx because a trade that
    /// then finds the event voided early-returns and would otherwise roll the
    /// finalize back. No-op unless the event is a funding-stage AMM past `open_at`.
    /// Public so the board can flip a stale funding card to the open market on the
    /// first interaction after the window closes.
    pub fn finalize_funding_if_due(&self, event_id: i64) -> SqlResult<()> {
        let now = super::current_unix_time();
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        maybe_open_funding(&tx, event_id, now)?;
        tx.commit()
    }

    /// Buy YES shares of outcome `idx` in an AMM event by spending `spend`
    /// micro-coins. The **whole spend** enters the pool; the LMSR cost
    /// (`spend − fee`) sets how many shares are minted, **floored** so the pool is
    /// never short. `q_shares` and `pool` advance. One transaction.
    pub fn amm_buy(&self, event_id: i64, idx: i64, user: i64, spend: i64) -> SqlResult<TradeOutcome> {
        if spend <= 0 {
            return Ok(TradeOutcome::Rejected);
        }
        self.finalize_funding_if_due(event_id)?;
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
        self.finalize_funding_if_due(event_id)?;
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
        let flipped = {
            let conn = self.conn.lock();
            conn.execute(
                "UPDATE events SET state = 'resolved', winning_market = ?1, resolved_at = ?2
                 WHERE id = ?3 AND state = 'open'",
                params![winning_idx, now, event_id],
            )? == 1
        };
        if flipped {
            self.settle_if_no_positions(event_id)?;
        }
        Ok(flipped)
    }

    /// Void an open **or funding-stage** event (no clear winner, or funding that
    /// won't open). Settlement refunds every trader's cost basis and every LP's
    /// pooled liquidity. Returns `true` if it flipped the event to `void`.
    pub fn void_event(&self, event_id: i64, now: i64) -> SqlResult<bool> {
        let flipped = {
            let conn = self.conn.lock();
            conn.execute(
                "UPDATE events SET state = 'void', resolved_at = ?1
                 WHERE id = ?2 AND state IN ('open', 'funding')",
                params![now, event_id],
            )? == 1
        };
        if flipped {
            self.settle_if_no_positions(event_id)?;
        }
        Ok(flipped)
    }

    /// After a resolve/void, eagerly settle **iff the event has no positions** — so
    /// an AMM event that never traded (a funding-stage void, or an opened market
    /// nobody touched) still returns its pooled liquidity to the LPs without waiting
    /// for a `/claim` that will never come. Events with positions stay lazy.
    fn settle_if_no_positions(&self, event_id: i64) -> SqlResult<()> {
        let has_positions = {
            let conn = self.conn.lock();
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM positions WHERE event_id = ?1)",
                params![event_id],
                |r| r.get::<_, i64>(0),
            )? == 1
        };
        if !has_positions {
            self.settle_event(event_id, None)?;
        }
        Ok(())
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
        // When the last position is gone, finalise: the AMM residual (seed + fees −
        // payouts) returns to the LPs pro-rata by contribution, and the event closes.
        let remaining: i64 = tx.query_row(
            "SELECT COUNT(*) FROM positions WHERE event_id = ?1",
            params![event_id],
            |r| r.get(0),
        )?;
        if remaining == 0 {
            if amm {
                distribute_residual(&tx, event_id, creator, pool)?;
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

    /// Dev `/reset` (Markets) — refund every committed coin and wipe the whole
    /// trading engine (`events`/`markets`/`positions`/`liquidity`) in one
    /// transaction. Each trader gets their position `cost` basis back; each LP gets
    /// their `liquidity.contributed` back (this covers both multi-LP funding pools
    /// and the single-host seed, which is itself a `liquidity` row). Together those
    /// equal the whole `pool`, so it **conserves supply** — every committed coin
    /// returns to a balance, nothing stranded or minted. Returns
    /// `(events_cleared, micro_coins_refunded)`.
    pub fn reset_events(&self) -> SqlResult<(i64, i64)> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let events_cleared: i64 = tx.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))?;
        let mut refunded = 0i64;
        // (1) Every trader's position cost basis back to their balance.
        let holders: Vec<(i64, i64)> = {
            let mut stmt =
                tx.prepare("SELECT user, COALESCE(SUM(cost), 0) FROM positions GROUP BY user")?;
            let v = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<SqlResult<Vec<_>>>()?;
            v
        };
        // (2) Every LP's pooled contribution back to their balance.
        let lps: Vec<(i64, i64)> = {
            let mut stmt = tx
                .prepare("SELECT user, COALESCE(SUM(contributed), 0) FROM liquidity GROUP BY user")?;
            let v = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<SqlResult<Vec<_>>>()?;
            v
        };
        for (user, amount) in holders.iter().chain(lps.iter()) {
            if *amount > 0 {
                credit(&tx, *user, *amount)?;
                refunded += amount;
            }
        }
        tx.execute("DELETE FROM positions", [])?;
        tx.execute("DELETE FROM markets", [])?;
        tx.execute("DELETE FROM liquidity", [])?;
        tx.execute("DELETE FROM events", [])?;
        tx.commit()?;
        Ok((events_cleared, refunded))
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

    /// The user's open liquidity-provider stakes — host AMM events they've funded
    /// that haven't settled yet (`funding` or `open`), with the event title +
    /// state. For the `/assets` view; rows vanish once the residual is distributed.
    pub fn user_liquidity(&self, user: i64) -> SqlResult<Vec<LiquidityView>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT l.event_id, e.title, e.state, l.contributed
             FROM liquidity l JOIN events e ON e.id = l.event_id
             WHERE l.user = ?1 AND l.contributed > 0
             ORDER BY l.event_id",
        )?;
        let v = stmt
            .query_map(params![user], |r| {
                Ok(LiquidityView {
                    event_id: r.get(0)?,
                    event_title: r.get(1)?,
                    state: r.get(2)?,
                    contributed: r.get(3)?,
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

    /// Every open **sourced** event with its `(id, slug, outcome_count)` — the
    /// public `/settle` sweep fetches each one's Polymarket resolution (the count
    /// maps the Gamma winner to the event's stored outcome index).
    pub fn all_open_sourced(&self) -> SqlResult<Vec<(i64, String, i64)>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, COALESCE(source_ref, ''),
                    (SELECT COUNT(*) FROM markets m WHERE m.event_id = events.id)
             FROM events WHERE kind = 'sourced' AND state = 'open'",
        )?;
        let v = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(v)
    }

    /// The caller's distinct open **sourced** events with their `(id, slug,
    /// outcome_count)` — for detecting Polymarket resolution at `/claim` time. The
    /// outcome count comes from a correlated subquery so multiple positions in one
    /// event don't inflate it.
    pub fn user_open_sourced(&self, user: i64) -> SqlResult<Vec<(i64, String, i64)>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT e.id, COALESCE(e.source_ref, ''),
                    (SELECT COUNT(*) FROM markets m WHERE m.event_id = e.id)
             FROM positions p JOIN events e ON e.id = p.event_id
             WHERE p.user = ?1 AND e.kind = 'sourced' AND e.state = 'open'",
        )?;
        let v = stmt
            .query_map(params![user], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(v)
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

    /// Load an AMM event's board render data (host `/predict` card). `None` for a
    /// missing or non-AMM event.
    pub fn amm_board(&self, event_id: i64) -> SqlResult<Option<AmmBoard>> {
        let conn = self.conn.lock();
        let ev = conn
            .query_row(
                "SELECT creator, title, state, lang, odds_fmt, COALESCE(tz_offset, 0), ends_at,
                        COALESCE(b_param, 0), fee_bps, winning_market, pool, open_at
                 FROM events WHERE id = ?1 AND kind = 'amm'",
                params![event_id],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, i64>(5)?,
                        r.get::<_, i64>(6)?,
                        r.get::<_, i64>(7)?,
                        r.get::<_, i64>(8)?,
                        r.get::<_, Option<i64>>(9)?,
                        r.get::<_, i64>(10)?,
                        r.get::<_, i64>(11)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            host,
            question,
            state,
            lang,
            odds_fmt,
            tz_offset,
            ends_at,
            b_param,
            fee_bps,
            winning_market,
            pool,
            open_at,
        )) = ev
        else {
            return Ok(None);
        };
        let rows = {
            let mut stmt = conn
                .prepare("SELECT name, q_shares, funded FROM markets WHERE event_id = ?1 ORDER BY idx")?;
            let v = stmt
                .query_map(params![event_id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
                })?
                .collect::<SqlResult<Vec<_>>>()?;
            v
        };
        let options: Vec<(String, i64)> = rows.iter().map(|(n, q, _)| (n.clone(), *q)).collect();
        let funded: Vec<i64> = rows.iter().map(|(_, _, f)| *f).collect();
        Ok(Some(AmmBoard {
            event_id,
            host,
            question,
            state,
            lang,
            odds_fmt,
            tz_offset,
            ends_at,
            b_param,
            fee_bps,
            winning_market,
            pool,
            options,
            open_at,
            funded,
        }))
    }

    /// Read-only preview of how many shares `spend` micro-coins buys of an AMM
    /// outcome (mirrors [`Database::amm_buy`]'s share math) — for the buy builder.
    /// `None` if the event isn't an open AMM, the idx is bad, or the spend is too
    /// small to mint a share.
    pub fn amm_buy_quote(&self, event_id: i64, idx: i64, spend: i64) -> SqlResult<Option<i64>> {
        if spend <= 0 {
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
        let cost = spend - mul_bps(spend, fee_bps);
        let q_whole: Vec<f64> = q.iter().map(|x| *x as f64 / SHARE as f64).collect();
        let shares = (lmsr::shares_for_budget(&q_whole, b as f64, idx as usize, cost as f64 / COIN as f64)
            * SHARE as f64)
            .floor() as i64;
        Ok((shares > 0).then_some(shares))
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

    #[test]
    fn amm_board_and_buy_quote() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 1000 * COIN).unwrap();
        db.force_change(2, 1000 * COIN).unwrap();
        let ev = db
            .create_amm_event(
                1,
                "Who wins?",
                "hant",
                "",
                Some(480),
                0,
                &opts(&["A", "B", "C"]),
                B_MEDIUM,
                FEE_BPS_DEFAULT,
                100,
            )
            .unwrap()
            .unwrap();
        let board = db.amm_board(ev).unwrap().unwrap();
        assert_eq!(board.question, "Who wins?");
        assert_eq!(board.state, "open");
        assert_eq!(board.lang, "hant");
        assert_eq!(board.options.len(), 3);
        assert_eq!(board.options[0], ("A".to_string(), 0));
        // The read-only quote must match the share count an actual buy mints.
        let quoted = db.amm_buy_quote(ev, 0, 50 * COIN).unwrap().unwrap();
        assert!(quoted > 0);
        let TradeOutcome::Filled { shares, .. } = db.amm_buy(ev, 0, 2, 50 * COIN).unwrap() else {
            panic!()
        };
        assert_eq!(quoted, shares, "quote matches the executed buy");
    }

    #[test]
    fn reset_events_refunds_cost_and_amm_escrow_then_wipes() {
        // AMM event: pool = host escrow (70) + trader's cost (30) = 100.
        // Sourced event: house-banked (no pool), trader cost 20.
        let db = Database::new(":memory:", 1).unwrap();
        {
            let conn = db.conn.lock();
            conn.execute(
                "INSERT INTO events (id, kind, creator, state, pool) VALUES (1, 'amm', 99, 'open', ?1)",
                [100 * COIN],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO positions (event_id, market_idx, user, shares, cost) VALUES (1, 0, 10, 50, ?1)",
                [30 * COIN],
            )
            .unwrap();
            // The host is the AMM event's LP — their 70-coin seed sits in `liquidity`.
            conn.execute(
                "INSERT INTO liquidity (event_id, user, contributed) VALUES (1, 99, ?1)",
                [70 * COIN],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO events (id, kind, state, pool) VALUES (2, 'sourced', 'open', 0)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO positions (event_id, market_idx, user, shares, cost) VALUES (2, 1, 11, 40, ?1)",
                [20 * COIN],
            )
            .unwrap();
        }

        let (cleared, refunded) = db.reset_events().unwrap();
        assert_eq!(cleared, 2, "both events counted");
        // trader10 cost 30 + host99 LP contribution 70 + trader11 cost 20 = 120.
        assert_eq!(refunded, 120 * COIN, "every committed coin returned (conserved)");
        assert_eq!(db.get_user_info(10).unwrap().balance, 30 * COIN);
        assert_eq!(db.get_user_info(99).unwrap().balance, 70 * COIN);
        assert_eq!(db.get_user_info(11).unwrap().balance, 20 * COIN);
        // Engine fully wiped.
        let n: i64 =
            db.conn.lock().query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
        let p: i64 =
            db.conn.lock().query_row("SELECT COUNT(*) FROM positions", [], |r| r.get(0)).unwrap();
        assert_eq!(p, 0);
    }

    fn bal_sum(db: &Database) -> i64 {
        db.conn
            .lock()
            .query_row("SELECT COALESCE(SUM(balance), 0) FROM balance", [], |r| r.get(0))
            .unwrap()
    }
    fn pool_sum(db: &Database) -> i64 {
        db.conn.lock().query_row("SELECT COALESCE(SUM(pool), 0) FROM events", [], |r| r.get(0)).unwrap()
    }

    // `open_at = 1000` is far below the real wall clock, so the first trade's
    // `current_unix_time()` finalizes the funding stage; `add_liquidity` is called
    // with an explicit `now` < 1000 so its own window check still sees it open.
    const PAST_OPEN_AT: i64 = 1000;

    #[test]
    fn funding_lifecycle_conserves_balance_plus_pool() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 100 * COIN).unwrap(); // LP A (host)
        db.force_change(2, 100 * COIN).unwrap(); // LP B
        db.force_change(3, 100 * COIN).unwrap(); // trader
        let supply = bal_sum(&db);
        let opts = vec!["A".to_string(), "B".to_string()];
        let ev = db
            .create_funding_event(1, "Q?", "en", "", None, 0, &opts, 200, PAST_OPEN_AT, 0)
            .unwrap();

        // LP A funds 80/20, LP B funds 0/50 → opening tally A:80 B:70.
        assert_eq!(
            db.add_liquidity(ev, 1, &[80 * COIN, 20 * COIN], 0).unwrap(),
            FundOutcome::Funded { total: 100 * COIN }
        );
        assert_eq!(
            db.add_liquidity(ev, 2, &[0, 50 * COIN], 0).unwrap(),
            FundOutcome::Funded { total: 50 * COIN }
        );
        // Past the window → further funding rejected.
        assert_eq!(db.add_liquidity(ev, 2, &[5 * COIN, 0], 2000).unwrap(), FundOutcome::Unavailable);
        // Coins moved into the pool; balance + pool is conserved.
        assert_eq!(pool_sum(&db), 150 * COIN);
        assert_eq!(bal_sum(&db) + pool_sum(&db), supply);

        // First trade lazily finalizes the funding stage into trading.
        assert!(matches!(db.amm_buy(ev, 0, 3, 10 * COIN).unwrap(), TradeOutcome::Filled { .. }));
        assert_eq!(state_of(&db, ev), "open", "funding finalized into trading");
        assert_eq!(bal_sum(&db) + pool_sum(&db), supply, "the trade conserves balance + pool");

        // Host resolves A; trader claims → pool fully distributed, supply restored.
        assert!(db.resolve_event(ev, 0, 2_000_000_000).unwrap());
        let _ = db.claim(3).unwrap();
        assert_eq!(pool_sum(&db), 0, "residual fully distributed to LPs");
        assert_eq!(bal_sum(&db), supply, "AMM minted / burned nothing end to end");
    }

    #[test]
    fn underseeded_funding_voids_and_refunds_lps() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 100 * COIN).unwrap();
        db.force_change(3, 100 * COIN).unwrap();
        let supply = bal_sum(&db);
        let opts = vec!["A".to_string(), "B".to_string()];
        let ev = db
            .create_funding_event(1, "Q?", "en", "", None, 0, &opts, 200, PAST_OPEN_AT, 0)
            .unwrap();
        // Only 4 coins funded (< MIN_SEED = 10) → must void + refund on finalize.
        db.add_liquidity(ev, 1, &[3 * COIN, COIN], 0).unwrap();
        assert_eq!(db.amm_buy(ev, 0, 3, 10 * COIN).unwrap(), TradeOutcome::Unavailable);
        assert_eq!(state_of(&db, ev), "void");
        assert_eq!(db.get_user_info(1).unwrap().balance, 100 * COIN, "LP fully refunded");
        assert_eq!(db.get_user_info(3).unwrap().balance, 100 * COIN, "trader untouched");
        assert_eq!(bal_sum(&db), supply, "no coins lost");
    }

    #[test]
    fn voiding_funding_event_refunds_lps_pro_rata() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 100 * COIN).unwrap();
        db.force_change(2, 100 * COIN).unwrap();
        let supply = bal_sum(&db);
        let opts = vec!["A".to_string(), "B".to_string()];
        // Window far in the future — the event is voided before it ever opens.
        let ev = db
            .create_funding_event(1, "Q?", "en", "", None, 0, &opts, 200, 9_999_999_999, 0)
            .unwrap();
        db.add_liquidity(ev, 1, &[30 * COIN, 10 * COIN], 0).unwrap();
        db.add_liquidity(ev, 2, &[0, 20 * COIN], 0).unwrap();
        assert!(db.void_event(ev, 1).unwrap());
        assert_eq!(db.get_user_info(1).unwrap().balance, 100 * COIN);
        assert_eq!(db.get_user_info(2).unwrap().balance, 100 * COIN);
        assert_eq!(bal_sum(&db), supply, "every LP refunded, conserved");
    }

    #[test]
    fn user_liquidity_lists_funded_events() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 100 * COIN).unwrap();
        let opts = vec!["A".to_string(), "B".to_string()];
        let ev1 = db
            .create_funding_event(1, "Q1", "en", "", None, 0, &opts, 200, 9_999_999_999, 0)
            .unwrap();
        let ev2 = db
            .create_funding_event(1, "Q2", "en", "", None, 0, &opts, 200, 9_999_999_999, 0)
            .unwrap();
        db.add_liquidity(ev1, 1, &[20 * COIN, 10 * COIN], 0).unwrap(); // 30
        db.add_liquidity(ev2, 1, &[5 * COIN, 0], 0).unwrap(); // 5

        let lps = db.user_liquidity(1).unwrap();
        assert_eq!(lps.len(), 2);
        assert_eq!(lps[0].event_id, ev1);
        assert_eq!(lps[0].contributed, 30 * COIN);
        assert_eq!(lps[0].state, "funding");
        assert_eq!(lps[1].contributed, 5 * COIN);
        // A user who's funded nothing sees nothing.
        assert!(db.user_liquidity(999).unwrap().is_empty());
    }
}
