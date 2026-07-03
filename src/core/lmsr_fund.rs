//! Funding-stage math for host LMSR markets (`/predict`).
//!
//! During the **funding stage** LPs deposit coins allocated across the outcomes
//! (uneven is the point). Two things fall out of the pooled allocation, computed
//! once when funding closes:
//!   - **opening prices** `pₖ = fundedₖ / Σ funded` (each clamped to a floor so a
//!     fully one-sided split can't demand unbounded subsidy), and
//!   - **liquidity `b`** from the pooled seed `S = Σ funded`: `b = S / ln(1/p_min)`
//!     — the exact LMSR subsidy bound for opening at a non-uniform price (it
//!     reduces to the familiar `S = b·ln k` when the open is uniform).
//!
//! The opening baseline `q⁰ᵢ = b·ln(pᵢ)` then realizes those prices at depth `b`
//! and has `cost(q⁰) = 0`, so the pooled seed *is* exactly the subsidy escrow.
//! After funding closes the market is ordinary LMSR ([`crate::core::lmsr`]).
//!
//! At resolution the pool's residual (`pool − winning payout`, always ≥ 0 by the
//! LMSR solvency bound) plus accrued fees are split back to the LPs **pro-rata**
//! by contribution via [`pro_rata`].
//!
//! Like `core::lmsr`, the curve math here is pure `f64` in **whole units**
//! (coins/shares); the ledger boundary (micro-units + directed rounding) lives in
//! the engine. `pro_rata` is the one integer (micro-coin) helper — it's exact, so
//! it belongs with the conservation proofs.

/// Smallest opening price any outcome is clamped to (largest is `1 − FLOOR`), so a
/// fully one-sided funding split can't drive `b → 0` or demand unbounded seed.
pub const PRICE_FLOOR: f64 = 0.02;

/// Opening price vector from per-outcome funded amounts (any integer unit):
/// `pₖ = fundedₖ / Σ`, each clamped to `[FLOOR, 1−FLOOR]`, then renormalized so the
/// result is a valid distribution summing to 1 with no zero. Empty or all-zero
/// funding → uniform `1/k`.
pub fn opening_prices(funded: &[i64]) -> Vec<f64> {
    let k = funded.len();
    if k == 0 {
        return Vec::new();
    }
    let total: i64 = funded.iter().map(|&f| f.max(0)).sum();
    let raw: Vec<f64> = if total <= 0 {
        vec![1.0 / k as f64; k]
    } else {
        funded.iter().map(|&f| f.max(0) as f64 / total as f64).collect()
    };
    let clamped: Vec<f64> = raw
        .iter()
        .map(|&p| p.clamp(PRICE_FLOOR, 1.0 - PRICE_FLOOR))
        .collect();
    let s: f64 = clamped.iter().sum();
    clamped.iter().map(|&p| p / s).collect()
}

/// LMSR depth `b` (whole shares) from the pooled seed (whole coins) and opening
/// prices: `b = seed / ln(1/p_min)`. The floor on `p_min` keeps the denominator
/// finite and positive; for a uniform open this is `seed / ln k`, matching
/// [`lmsr::seed_escrow`].
pub fn seed_to_b(seed: f64, prices: &[f64]) -> f64 {
    let p_min = prices
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min)
        .max(PRICE_FLOOR);
    let denom = (1.0 / p_min).ln();
    if denom <= 0.0 {
        return seed; // degenerate (k < 2 / p_min == 1) — never happens for k ≥ 2
    }
    seed / denom
}

/// Opening baseline `q⁰ᵢ = b·ln(pᵢ)` (whole shares) that realizes `prices` at depth
/// `b` and has `cost(q⁰) = 0` (so the pooled seed is exactly the subsidy escrow).
pub fn opening_q(prices: &[f64], b: f64) -> Vec<f64> {
    prices.iter().map(|&p| b * p.ln()).collect()
}

/// Finalize a funding stage: the opening `(b, q⁰)` for a per-outcome funding
/// allocation and pooled `seed` (whole coins).
pub fn finalize(funded: &[i64], seed: f64) -> (f64, Vec<f64>) {
    let prices = opening_prices(funded);
    let b = seed_to_b(seed, &prices);
    let q0 = opening_q(&prices, b);
    (b, q0)
}

/// Split `residual` micro-coins among LPs by `contributions` (largest-remainder
/// apportionment): each LP gets `⌊residual · cᵢ / Σc⌋`, then the leftover micro-coins
/// (at most `n−1`) go one each to the largest fractional remainders. **`Σ result ==
/// residual` exactly** (conserving — no coin minted or lost), and a larger
/// contribution never receives a smaller share. Empty / zero contributions or a
/// non-positive residual → all zeros.
pub fn pro_rata(contributions: &[i64], residual: i64) -> Vec<i64> {
    let n = contributions.len();
    let total: i128 = contributions.iter().map(|&c| c.max(0) as i128).sum();
    if n == 0 || total <= 0 || residual <= 0 {
        return vec![0; n];
    }
    let mut shares = vec![0i64; n];
    let mut remainders: Vec<(i128, usize)> = Vec::with_capacity(n);
    let mut assigned: i64 = 0;
    for (i, &c) in contributions.iter().enumerate() {
        let num = residual as i128 * c.max(0) as i128;
        shares[i] = (num / total) as i64;
        assigned += shares[i];
        remainders.push((num % total, i));
    }
    // Hand out the leftover micro-coins, largest fractional remainder first
    // (ties broken by index for determinism).
    remainders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut leftover = residual - assigned;
    for &(_, idx) in remainders.iter() {
        if leftover == 0 {
            break;
        }
        shares[idx] += 1;
        leftover -= 1;
    }
    shares
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::lmsr;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn opening_prices_proportional_clamped_and_normalized() {
        // Even split → uniform.
        let p = opening_prices(&[50, 50]);
        assert!(approx(p[0], 0.5) && approx(p[1], 0.5));
        // Uneven split → proportional, still sums to 1.
        let p = opening_prices(&[80, 20]);
        assert!(approx(p[0], 0.8) && approx(p[1], 0.2));
        assert!(approx(p.iter().sum(), 1.0));
        // Fully one-sided → clamped to the floor, never zero, still sums to 1.
        let p = opening_prices(&[100, 0]);
        assert!(p[1] >= PRICE_FLOOR - 1e-12);
        assert!(approx(p.iter().sum(), 1.0));
        // All-zero / empty funding → uniform.
        let p = opening_prices(&[0, 0, 0]);
        for pi in &p {
            assert!(approx(*pi, 1.0 / 3.0));
        }
        assert!(opening_prices(&[]).is_empty());
    }

    #[test]
    fn seed_to_b_matches_uniform_escrow() {
        // For a uniform open, b = seed/ln(k) ⇒ seed_escrow(b,k) == seed.
        for k in 2..=5 {
            let prices = vec![1.0 / k as f64; k];
            let seed = 100.0;
            let b = seed_to_b(seed, &prices);
            assert!(approx(lmsr::seed_escrow(b, k), seed), "k={k}");
        }
    }

    #[test]
    fn opening_q_realizes_target_prices() {
        for funded in [[80, 20], [50, 50], [10, 90], [100, 0]] {
            let prices = opening_prices(&funded);
            let (b, q0) = finalize(&funded, 100.0);
            for (i, &target) in prices.iter().enumerate() {
                assert!(approx(lmsr::price(&q0, b, i), target), "funded={funded:?}");
            }
            // cost(q⁰) == 0, so the pooled seed is exactly the subsidy.
            assert!(lmsr::cost(&q0, b).abs() < 1e-6);
        }
    }

    #[test]
    fn funded_market_is_solvent_and_conserving() {
        // LPs fund A:80, B:20 (whole coins) → opens ~80/20; pooled seed = 100.
        let funded = [80, 20];
        let seed = 100.0;
        let (b, q0) = finalize(&funded, seed);

        // A sequence of trades on the curve; trader positions are q − q0.
        let mut q = q0.clone();
        let mut collected = 0.0;
        for &(i, delta) in &[(1usize, 30.0), (0usize, 50.0), (1usize, 10.0)] {
            collected += lmsr::cost_to_buy(&q, b, i, delta);
            q[i] += delta;
        }
        let pool = seed + collected;

        // Whichever outcome wins: the pool covers the winners (solvency) and
        // seed + trader spend == winner payout + LP residual (conservation).
        for w in 0..funded.len() {
            let payout = q[w] - q0[w]; // trader-held net winning shares
            assert!(pool >= payout - 1e-9, "insolvent if {w} wins: {pool} < {payout}");
            let residual = pool - payout;
            assert!(residual >= -1e-9, "negative residual if {w} wins");
            assert!(approx(pool, payout + residual));
        }
    }

    #[test]
    fn pro_rata_conserves_and_is_monotonic() {
        // 2:1 contributions, residual that doesn't divide evenly.
        let split = pro_rata(&[200, 100], 1001);
        assert_eq!(split.iter().sum::<i64>(), 1001, "exact conservation");
        assert!(split[0] >= split[1], "bigger contribution ≥ smaller");
        assert_eq!(split[0], 667); // ⌊1001·2/3⌋ = 667 (remainder 100/300)
        assert_eq!(split[1], 334); // ⌊1001·1/3⌋ = 333, +1 leftover (larger remainder 200/300)

        // Three LPs, a prime residual — still exact.
        let split = pro_rata(&[1, 1, 1], 100);
        assert_eq!(split.iter().sum::<i64>(), 100);

        // Degenerate inputs → all zeros.
        assert_eq!(pro_rata(&[0, 0], 500), vec![0, 0]);
        assert_eq!(pro_rata(&[5, 5], 0), vec![0, 0]);
        assert_eq!(pro_rata(&[], 100), Vec::<i64>::new());

        // No overflow at ledger magnitudes (micro-coins).
        let big = pro_rata(&[700_000 * 1_000_000, 300_000 * 1_000_000], 1_000_000 * 1_000_000);
        assert_eq!(big.iter().sum::<i64>(), 1_000_000 * 1_000_000);
        assert!(big[0] > big[1]);
    }
}
