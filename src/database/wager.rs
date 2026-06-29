/// Decimal-odds payout for a stake at `odds_cents`: `stake * 100 / odds_cents`
/// (= `stake × decimal_odds`), rounded to whole micro-coins. The live `/events`
/// buy builder uses it to show "spend → potential payout" — a YES share at price
/// `p` (cents) is bought for `p` and settles to 1 coin, so `decimal_payout` is the
/// share count. The `f64` is a bounded, rounded odds intermediate; the ledger
/// stays integer micro-coins and there's no precision loss at the bot's
/// magnitudes.
///
/// (This is all that survives from the removed fixed-odds `wagers` system.)
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
pub fn decimal_payout(stake: i64, odds_cents: f64) -> i64 {
    // Guard non-finite too: a NaN slips past `<= 0.0` (every NaN comparison is
    // false) and would otherwise become `NaN as i64 = 0`, silently paying a
    // winner nothing. Treat any degenerate quote as "return the stake".
    if !odds_cents.is_finite() || odds_cents <= 0.0 {
        return stake;
    }
    (stake as f64 * 100.0 / odds_cents).round() as i64
}

#[cfg(test)]
mod tests {
    use super::super::COIN;
    use super::*;

    #[test]
    fn decimal_payout_matches_decimal_odds_and_guards_zero() {
        // 5 coins at 50¢ (decimal 2.0) → 10 coins; the formula the buy builder shows.
        assert_eq!(decimal_payout(5 * COIN, 50.0), 10 * COIN);
        // 10 coins at 250¢ (decimal 0.4) → 4 coins.
        assert_eq!(decimal_payout(10 * COIN, 250.0), 4 * COIN);
        // Degenerate quote returns the stake rather than dividing by zero.
        assert_eq!(decimal_payout(7 * COIN, 0.0), 7 * COIN);
    }
}
