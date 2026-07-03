use super::Database;
use rusqlite::{params, Connection, Result as SqlResult, Transaction};

/// One rendered row of a user's activity statement (`/history`). Language-neutral
/// like `positions` — the `kind` maps to a localized label and the optional
/// `event_title` is rendered at read time by the command.
#[derive(Debug, Clone)]
pub struct HistoryRow {
    /// Unix seconds the action occurred.
    pub at: i64,
    /// Action tag — one of the `HK_*` consts below.
    pub kind: String,
    /// Signed micro-coin balance delta (+credit / −debit).
    pub delta: i64,
    /// Title of the related market event, when the action targets one (buy/sell/
    /// claim/refund/lp_*). `None` for pure-coin actions or a since-wiped event.
    pub event_title: Option<String>,
}

/// A `/history` filter tab. Mining = coins earned from the house (check-in /
/// referral / mint); Trading = market activity (buy / sell / claim / LP + a
/// market void refund); Transfer = peer moves (`/send` + envelope, incl. an
/// envelope refund). The overloaded `refund` tag is split by whether the row
/// targets a market (`event_id` set → Trading, else → Transfer).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryTab {
    Mining,
    Trading,
    Transfer,
}

// Action tags stored verbatim in `history.kind`. Kept as consts so the record
// sites (in `event.rs`/`user.rs`) and the `/history` label mapping can't drift.
pub(crate) const HK_BUY: &str = "buy";
pub(crate) const HK_SELL: &str = "sell";
pub(crate) const HK_SEND_OUT: &str = "send_out";
pub(crate) const HK_SEND_IN: &str = "send_in";
pub(crate) const HK_CHECKIN: &str = "checkin";
pub(crate) const HK_REFERRAL: &str = "referral";
pub(crate) const HK_CLAIM: &str = "claim";
pub(crate) const HK_REFUND: &str = "refund";
pub(crate) const HK_MINT: &str = "mint";
pub(crate) const HK_LP_FUND: &str = "lp_fund";
pub(crate) const HK_LP_RETURN: &str = "lp_return";

/// Append one action to a user's history **within the caller's transaction**, so
/// a recorded row can never drift from the ledger move that produced it (a failed
/// insert rolls the whole money mutation back). `event_id` / `counter` are
/// optional context (the related market / the transfer counterparty).
pub(super) fn record(
    tx: &Transaction,
    user: i64,
    kind: &str,
    delta: i64,
    event_id: Option<i64>,
    counter: Option<i64>,
) -> SqlResult<()> {
    tx.execute(
        "INSERT INTO history (user, at, kind, delta, event_id, counter)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![user, super::current_unix_time(), kind, delta, event_id, counter],
    )?;
    Ok(())
}

impl Database {
    /// Create the append-only per-user action log backing `/history`, plus the
    /// `(user, id DESC)` index the newest-first read walks.
    pub(super) fn create_history_table(conn: &Connection) -> SqlResult<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS history (
                id       INTEGER PRIMARY KEY AUTOINCREMENT,
                user     INTEGER NOT NULL,
                at       INTEGER NOT NULL DEFAULT 0,
                kind     TEXT    NOT NULL,
                delta    INTEGER NOT NULL DEFAULT 0,
                event_id INTEGER,
                counter  INTEGER
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_history_user ON history (user, id DESC)",
            [],
        )?;
        Ok(())
    }

    /// Standalone (own-lock) history append — for a money move that doesn't run
    /// inside an engine transaction (currently only the owner `/mint`, which uses
    /// the generic `force_change`). Best-effort: a lone INSERT.
    pub fn record_action(
        &self,
        user: i64,
        kind: &str,
        delta: i64,
        event_id: Option<i64>,
        counter: Option<i64>,
    ) -> SqlResult<()> {
        // Delegate to the shared [`record`] so the INSERT lives in one place; a
        // one-statement transaction gives it the `&Transaction` it wants.
        self.with_tx(|tx| record(tx, user, kind, delta, event_id, counter))
    }

    /// The user's most recent actions, newest first, capped at `limit`. Left-joins
    /// the event title for market actions (`None` when the event was since wiped).
    pub fn user_history(&self, user: i64, limit: i64) -> SqlResult<Vec<HistoryRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT h.at, h.kind, h.delta, e.title
             FROM history h LEFT JOIN events e ON e.id = h.event_id
             WHERE h.user = ?1 ORDER BY h.id DESC LIMIT ?2",
        )?;
        let v = stmt
            .query_map(params![user, limit], |r| {
                Ok(HistoryRow {
                    at: r.get(0)?,
                    kind: r.get(1)?,
                    delta: r.get(2)?,
                    event_title: r.get(3)?,
                })
            })?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(v)
    }

    /// Like [`Database::user_history`] but limited to one [`HistoryTab`]. The tab
    /// predicate is built from the `HK_*` tags (all trusted compile-time literals,
    /// so inlining them is injection-safe); the `refund` tag is bucketed by
    /// `event_id` (market void → Trading, envelope → Transfer).
    pub fn user_history_by(&self, user: i64, tab: HistoryTab, limit: i64) -> SqlResult<Vec<HistoryRow>> {
        let pred = match tab {
            HistoryTab::Mining => {
                format!("h.kind IN ('{HK_CHECKIN}','{HK_REFERRAL}','{HK_MINT}')")
            }
            HistoryTab::Trading => format!(
                "(h.kind IN ('{HK_BUY}','{HK_SELL}','{HK_CLAIM}','{HK_LP_FUND}','{HK_LP_RETURN}') \
                 OR (h.kind = '{HK_REFUND}' AND h.event_id IS NOT NULL))"
            ),
            HistoryTab::Transfer => format!(
                "(h.kind IN ('{HK_SEND_IN}','{HK_SEND_OUT}') \
                 OR (h.kind = '{HK_REFUND}' AND h.event_id IS NULL))"
            ),
        };
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT h.at, h.kind, h.delta, e.title
             FROM history h LEFT JOIN events e ON e.id = h.event_id
             WHERE h.user = ?1 AND {pred} ORDER BY h.id DESC LIMIT ?2"
        ))?;
        let v = stmt
            .query_map(params![user, limit], |r| {
                Ok(HistoryRow {
                    at: r.get(0)?,
                    kind: r.get(1)?,
                    delta: r.get(2)?,
                    event_title: r.get(3)?,
                })
            })?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::super::COIN;
    use super::*;

    #[test]
    fn transfer_records_both_legs() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(1, 10 * COIN).unwrap();
        assert!(db.transfer(1, 2, 4 * COIN).unwrap());

        let out = db.user_history(1, 10).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, HK_SEND_OUT);
        assert_eq!(out[0].delta, -4 * COIN);

        let inb = db.user_history(2, 10).unwrap();
        assert_eq!(inb.len(), 1);
        assert_eq!(inb[0].kind, HK_SEND_IN);
        assert_eq!(inb[0].delta, 4 * COIN);
    }

    #[test]
    fn checkin_records_reward() {
        let db = Database::new(":memory:", 1).unwrap();
        assert!(db.try_checkin(7, 10 * COIN).unwrap());
        let h = db.user_history(7, 10).unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].kind, HK_CHECKIN);
        assert_eq!(h[0].delta, 10 * COIN);
    }

    #[test]
    fn record_action_appends_standalone() {
        let db = Database::new(":memory:", 1).unwrap();
        db.record_action(9, HK_MINT, 50 * COIN, None, None).unwrap();
        let h = db.user_history(9, 10).unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].kind, HK_MINT);
        assert_eq!(h[0].delta, 50 * COIN);
        assert!(h[0].event_title.is_none());
    }

    #[test]
    fn newest_first_and_limit() {
        let db = Database::new(":memory:", 1).unwrap();
        for _ in 0..5 {
            db.record_action(3, HK_MINT, COIN, None, None).unwrap();
        }
        db.record_action(3, HK_REFERRAL, COIN, None, None).unwrap();
        let h = db.user_history(3, 3).unwrap();
        assert_eq!(h.len(), 3); // capped
        assert_eq!(h[0].kind, HK_REFERRAL); // most recent first
    }

    #[test]
    fn reset_all_clears_history() {
        let db = Database::new(":memory:", 1).unwrap();
        db.record_action(1, HK_MINT, COIN, None, None).unwrap();
        db.reset_all().unwrap();
        assert!(db.user_history(1, 10).unwrap().is_empty());
    }

    #[test]
    fn envelope_send_and_claim_are_logged() {
        let db = Database::new(":memory:", 1).unwrap();
        db.force_change(5, 10 * COIN).unwrap();
        // Mirror `send::send`: debit the sender, then post the envelope (which
        // logs the send_out inside insert_buffer's tx).
        assert!(db.balance_change(5, -3 * COIN).unwrap());
        db.insert_buffer(100, 200, 5, 3 * COIN).unwrap();
        let s = db.user_history(5, 10).unwrap();
        assert_eq!(s[0].kind, HK_SEND_OUT);
        assert_eq!(s[0].delta, -3 * COIN);
        // Claimer grabs it → send_in logged for them.
        assert_eq!(db.claim_envelope(100, 200, 6).unwrap(), Some(3 * COIN));
        let c = db.user_history(6, 10).unwrap();
        assert_eq!(c[0].kind, HK_SEND_IN);
        assert_eq!(c[0].delta, 3 * COIN);
    }

    #[test]
    fn history_tabs_partition_by_category() {
        let db = Database::new(":memory:", 1).unwrap();
        let u = 1;
        // Mining
        db.record_action(u, HK_CHECKIN, COIN, None, None).unwrap();
        db.record_action(u, HK_REFERRAL, COIN, None, None).unwrap();
        db.record_action(u, HK_MINT, COIN, None, None).unwrap();
        // Trading (buy/claim/lp + a *market* void refund, which carries an event_id)
        db.record_action(u, HK_BUY, -COIN, Some(7), None).unwrap();
        db.record_action(u, HK_CLAIM, COIN, Some(7), None).unwrap();
        db.record_action(u, HK_LP_FUND, -COIN, Some(7), None).unwrap();
        db.record_action(u, HK_REFUND, COIN, Some(7), None).unwrap();
        // Transfer (send + an *envelope* refund, which has no event_id)
        db.record_action(u, HK_SEND_OUT, -COIN, None, None).unwrap();
        db.record_action(u, HK_SEND_IN, COIN, None, None).unwrap();
        db.record_action(u, HK_REFUND, COIN, None, None).unwrap();

        let mining = db.user_history_by(u, HistoryTab::Mining, 50).unwrap();
        assert_eq!(mining.len(), 3);
        assert!(mining
            .iter()
            .all(|h| [HK_CHECKIN, HK_REFERRAL, HK_MINT].contains(&h.kind.as_str())));

        // buy, claim, lp_fund, and the refund whose event_id is set.
        let trading = db.user_history_by(u, HistoryTab::Trading, 50).unwrap();
        assert_eq!(trading.len(), 4);
        assert!(trading.iter().any(|h| h.kind == HK_REFUND));

        // send_out, send_in, and the refund with no event_id.
        let transfer = db.user_history_by(u, HistoryTab::Transfer, 50).unwrap();
        assert_eq!(transfer.len(), 3);
        assert!(transfer.iter().any(|h| h.kind == HK_REFUND));
    }
}
