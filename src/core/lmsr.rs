//! Logarithmic Market Scoring Rule (LMSR) — the automated market maker behind
//! host-run (`amm`) prediction events.
//!
//! This module is **pure `f64` curve math in whole units** (shares and coins).
//! It deliberately knows nothing about the ledger: the micro-coin/micro-share
//! integer boundary and the directed, pool-favorable rounding that keeps the
//! escrow solvent live in the trade engine, never here.
//!
//! A share of the winning outcome settles to exactly one coin, so a price is
//! coins-per-share in `(0, 1)` and prices over all outcomes always sum to 1.
//!
//! For `k` outcomes, liquidity `b` (in shares), and net outstanding shares `q_i`:
//! - cost   `C(q) = b · ln Σ_j exp(q_j / b)`
//! - price  `p_i  = softmax(q / b)_i = exp(q_i/b) / Σ_j exp(q_j/b)`
//! - buying `Δ` shares of `i` costs `C(q + Δ·e_i) − C(q)`; selling refunds the
//!   negative of that.
//!
//! Solvency: `C(q) ≥ max_i q_i`, so an escrow of `C(0) = b · ln k` always covers
//! the winning side — the guarantee the engine relies on to never over-pay.

/// `max_i q_i`, or `-inf` for an empty slice. The shared shift used by the
/// log-sum-exp trick so `exp` can't overflow for large share counts.
fn max_q(q: &[f64]) -> f64 {
    q.iter().copied().fold(f64::NEG_INFINITY, f64::max)
}

/// LMSR cost function `C(q) = b · ln Σ exp(q_i / b)` (whole coins), computed via
/// the numerically stable log-sum-exp form `m + b·ln Σ exp((q_i − m)/b)`.
pub fn cost(q: &[f64], b: f64) -> f64 {
    let m = max_q(q);
    let sum: f64 = q.iter().map(|qi| ((qi - m) / b).exp()).sum();
    m + b * sum.ln()
}

/// Price of outcome `i` — `softmax(q / b)_i`, in `(0, 1)`. Prices over all
/// outcomes sum to 1.
pub fn price(q: &[f64], b: f64, i: usize) -> f64 {
    let m = max_q(q);
    let denom: f64 = q.iter().map(|qj| ((qj - m) / b).exp()).sum();
    ((q[i] - m) / b).exp() / denom
}

/// Cost (whole coins, ≥ 0) to buy `shares` of outcome `i` from state `q`.
pub fn cost_to_buy(q: &[f64], b: f64, i: usize, shares: f64) -> f64 {
    let mut q2 = q.to_vec();
    q2[i] += shares;
    cost(&q2, b) - cost(q, b)
}

/// Refund (whole coins, ≥ 0) for selling `shares` of outcome `i` from state `q`.
pub fn refund_to_sell(q: &[f64], b: f64, i: usize, shares: f64) -> f64 {
    let mut q2 = q.to_vec();
    q2[i] -= shares;
    cost(q, b) - cost(&q2, b)
}

/// Shares of outcome `i` obtainable for a `budget` of whole coins — the
/// closed-form inverse of [`cost_to_buy`]:
///
/// `Δ = m + b·ln(e^{budget/b}·Sr − Sr_other) − q_i`, where `Sr = Σ e^{(q_j−m)/b}`
/// and `Sr_other = Sr − e^{(q_i−m)/b}`. `budget` must be ≥ 0. (For an enormous
/// budget `e^{budget/b}` can overflow to `+inf`; the engine bounds spend by the
/// buyer's balance, so this stays finite in practice.)
pub fn shares_for_budget(q: &[f64], b: f64, i: usize, budget: f64) -> f64 {
    let m = max_q(q);
    let sr: f64 = q.iter().map(|qj| ((qj - m) / b).exp()).sum();
    let sr_other = sr - ((q[i] - m) / b).exp();
    let inner = (budget / b).exp() * sr - sr_other;
    m + b * inner.ln() - q[i]
}

/// The host's seed escrow `C(0) = b · ln k` (whole coins) for a `k`-outcome
/// market — the worst-case LMSR subsidy, and exactly enough to keep the pool
/// solvent through any sequence of trades.
pub fn seed_escrow(b: f64, k: usize) -> f64 {
    cost(&vec![0.0; k], b)
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: f64 = 50.0;

    fn sum_prices(q: &[f64], b: f64) -> f64 {
        (0..q.len()).map(|i| price(q, b, i)).sum()
    }

    #[test]
    fn flat_market_is_uniform() {
        for k in 2..=5 {
            let q = vec![0.0; k];
            for i in 0..k {
                assert!((price(&q, B, i) - 1.0 / k as f64).abs() < 1e-12);
            }
            assert!((sum_prices(&q, B) - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn prices_sum_to_one_when_skewed() {
        let q = [300.0, -120.0, 40.0];
        assert!((sum_prices(&q, B) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn buying_raises_own_price_and_lowers_others() {
        let q = [0.0, 0.0, 0.0];
        let before = [price(&q, B, 0), price(&q, B, 1), price(&q, B, 2)];
        let mut q2 = q;
        q2[0] += 100.0;
        let after = [price(&q2, B, 0), price(&q2, B, 1), price(&q2, B, 2)];
        assert!(after[0] > before[0]);
        assert!(after[1] < before[1] && after[2] < before[2]);
    }

    #[test]
    fn budget_inverts_cost() {
        let q = [10.0, -5.0, 3.0];
        for &budget in &[1.0, 7.5, 40.0, 100.0] {
            let shares = shares_for_budget(&q, B, 1, budget);
            let cost = cost_to_buy(&q, B, 1, shares);
            assert!((cost - budget).abs() < 1e-6, "budget {budget}: cost {cost}");
        }
    }

    #[test]
    fn buy_then_sell_is_zero_sum_on_the_curve() {
        // The curve itself has no spread: selling back exactly what you bought
        // refunds exactly the buy cost. Any real loss comes from the engine's
        // directed rounding + the host fee, not from here.
        let q = [0.0, 0.0];
        let delta = 80.0;
        let buy = cost_to_buy(&q, B, 0, delta);
        let mut q2 = q;
        q2[0] += delta;
        let refund = refund_to_sell(&q2, B, 0, delta);
        assert!((buy - refund).abs() < 1e-9);
    }

    #[test]
    fn cost_dominates_max_share_count() {
        // Solvency: C(q) ≥ max_i q_i for any state, so escrow C(0) covers the
        // winning side no matter how lopsided the book gets.
        for q in [[0.0, 0.0], [500.0, -200.0], [-30.0, 900.0]] {
            assert!(cost(&q, B) >= max_q(&q) - 1e-9);
        }
    }

    #[test]
    fn seed_escrow_is_cost_at_zero() {
        for k in 2..=4 {
            let expect = B * (k as f64).ln();
            assert!((seed_escrow(B, k) - expect).abs() < 1e-9);
            assert!((seed_escrow(B, k) - cost(&vec![0.0; k], B)).abs() < 1e-12);
        }
    }

    #[test]
    fn stable_for_large_share_counts() {
        // log-sum-exp keeps everything finite even when naive exp(q/b) overflows.
        let q = [50_000.0, 0.0];
        assert!(cost(&q, B).is_finite());
        assert!(price(&q, B, 0).is_finite() && price(&q, B, 0) > 0.999);
        assert!((sum_prices(&q, B) - 1.0).abs() < 1e-9);
    }
}
