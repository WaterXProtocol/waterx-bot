use super::{current_unix_time, Database};
use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};

/// Outcome of attempting to consume a sell or buy offer. All variants other
/// than `Filled` and `SelfCancelled` mean the DB was left untouched.
#[derive(Debug, Clone)]
pub enum OfferOutcome {
    /// Buffer row missing — someone else already took or cancelled it.
    AlreadyTaken,
    /// The poster tapped their own button. Escrow has been refunded and the
    /// buffer row deleted; the caller should also delete the chat message.
    SelfCancelled,
    /// Trade completed. `fruits`/`price` describe what changed hands.
    Filled { fruits: String, price: i64 },
    /// (sell-offer only) Taker (the buyer) lacked the asking price.
    TakerNotEnoughBalance,
    /// Taker would exceed the 5-fruit inventory cap.
    TakerFruitFull,
    /// (buy-offer only) Taker (the seller) doesn't own all listed fruits.
    TakerMissingFruit(char),
}

fn ensure_row_locked(conn: &Connection, user_id: i64) -> SqlResult<()> {
    conn.execute(
        "INSERT OR IGNORE INTO balance (user, balance, fruit) VALUES (?1, 0, '')",
        params![user_id],
    )?;
    Ok(())
}

impl Database {
    /// Record a live envelope, escrowing `amount` micro-coins for `owner`. The
    /// escrowed amount is stored **in the row** (the `price` column) so the claim
    /// credits exactly what was escrowed — never a value taken from untrusted
    /// callback data. `owner` lets an unclaimed/cancelled envelope be refunded.
    pub fn insert_buffer(&self, chat: i64, msg: i64, owner: i64, amount: i64) -> SqlResult<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO buffer (chat, msg, kind, owner, price, created_at) VALUES (?1, ?2, 'envelope', ?3, ?4, ?5)",
            params![chat, msg, owner, amount, current_unix_time()],
        )?;
        // The sender was already debited (see `send::send`); log the outgoing
        // envelope alongside the escrow row so the `send_out` lands iff the
        // envelope actually goes live — this tx rolls back on failure, and the
        // caller then refunds the debited coins. Counter is unknown (anyone can
        // grab it) until a claim.
        super::history::record(&tx, owner, super::HK_SEND_OUT, -amount, None, None)?;
        tx.commit()?;
        Ok(())
    }

    /// True iff a buffer row exists, regardless of kind.
    pub fn has_buffer(&self, chat: i64, msg: i64) -> SqlResult<bool> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT 1 FROM buffer WHERE chat = ?1 AND msg = ?2")?;
        let mut rows = stmt.query(params![chat, msg])?;
        Ok(rows.next()?.is_some())
    }

    pub fn delete_buffer(&self, chat: i64, msg: i64) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM buffer WHERE chat = ?1 AND msg = ?2",
            params![chat, msg],
        )?;
        Ok(())
    }

    /// Atomically claim a coin envelope: in one transaction, delete its buffer
    /// row and — only if this call won that delete (so a concurrent double-tap
    /// produces exactly one winner) — credit the **escrowed** amount stored in
    /// the row (`price`) to `user`. The credit is **never** taken from caller
    /// input: the callback data that triggers a claim is attacker-controlled, so
    /// trusting it would let a crafted `envelope:<huge>` mint coins. Returns
    /// `Ok(Some(credited))` (micro-coins) on success, `Ok(None)` when the row was
    /// already gone / not an escrowed envelope (legacy rows with a NULL price are
    /// left in place for the TTL refund-sweep rather than credited as zero).
    pub fn claim_envelope(&self, chat: i64, msg: i64, user: i64) -> SqlResult<Option<i64>> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        // Read the escrow (+ owner, for the history counter) first; only
        // delete+credit when there's a real amount.
        let row: Option<(Option<i64>, Option<i64>)> = tx
            .query_row(
                "SELECT price, owner FROM buffer WHERE chat = ?1 AND msg = ?2 AND kind = 'envelope'",
                params![chat, msg],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((Some(amount), owner)) = row else {
            return Ok(None); // already taken, missing, or a NULL-price legacy row
        };
        let deleted = tx.execute(
            "DELETE FROM buffer WHERE chat = ?1 AND msg = ?2",
            params![chat, msg],
        )?;
        if deleted != 1 {
            return Ok(None); // lost the race → rollback, no credit
        }
        ensure_row_locked(&tx, user)?;
        tx.execute(
            "UPDATE balance SET balance = balance + ?1 WHERE user = ?2",
            params![amount, user],
        )?;
        super::history::record(&tx, user, super::HK_SEND_IN, amount, None, owner)?;
        tx.commit()?;
        Ok(Some(amount))
    }

    /// Cancel an envelope (a forged non-positive claim hit it): delete the row and
    /// **refund** the escrow to its owner, so the coins are neither minted nor
    /// stranded. No-op for rows missing an owner/amount (legacy / non-envelope).
    pub fn refund_envelope(&self, chat: i64, msg: i64) -> SqlResult<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let row: Option<(Option<i64>, Option<i64>)> = tx
            .query_row(
                "SELECT owner, price FROM buffer WHERE chat = ?1 AND msg = ?2 AND kind = 'envelope'",
                params![chat, msg],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let deleted = tx.execute(
            "DELETE FROM buffer WHERE chat = ?1 AND msg = ?2",
            params![chat, msg],
        )?;
        if deleted == 1 {
            if let Some((Some(owner), Some(amount))) = row {
                ensure_row_locked(&tx, owner)?;
                tx.execute(
                    "UPDATE balance SET balance = balance + ?1 WHERE user = ?2",
                    params![amount, owner],
                )?;
                super::history::record(&tx, owner, super::HK_REFUND, amount, None, None)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Open a sell offer. Atomically deducts each char of `fruits` that the
    /// seller actually owns from their inventory and inserts a sell-kind
    /// buffer row holding the escrow. Returns the *actually-escrowed* fruits
    /// (subset of input — any fruit the seller didn't have is dropped). If
    /// nothing was escrowed, returns an empty string and no buffer row is
    /// inserted.
    pub fn open_sell_offer(
        &self,
        chat: i64,
        msg: i64,
        seller: i64,
        fruits: &str,
        price: i64,
    ) -> SqlResult<String> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        ensure_row_locked(&tx, seller)?;
        let mut sf: String = tx.query_row(
            "SELECT fruit FROM balance WHERE user = ?1",
            params![seller],
            |r| r.get(0),
        )?;
        let mut escrowed = String::new();
        for f in fruits.chars() {
            let s = f.to_string();
            if let Some(idx) = sf.find(&s) {
                sf.remove(idx);
                escrowed.push(f);
            }
        }
        if escrowed.is_empty() {
            return Ok(escrowed); // tx drops → rollback (only the ensure_row insert)
        }
        // Debit the escrowed fruit and record the offer atomically: a failure on
        // either statement leaves the seller's inventory untouched.
        tx.execute(
            "UPDATE balance SET fruit = ?1 WHERE user = ?2",
            params![sf, seller],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO buffer (chat, msg, kind, owner, fruits, price, created_at)
             VALUES (?1, ?2, 'sell', ?3, ?4, ?5, ?6)",
            params![chat, msg, seller, escrowed, price, current_unix_time()],
        )?;
        tx.commit()?;
        Ok(escrowed)
    }

    /// Open a buy offer. Atomically deducts `price` from the buyer's balance
    /// (returns false if they can't afford it) and inserts a buy-kind buffer
    /// row holding the escrow.
    pub fn open_buy_offer(
        &self,
        chat: i64,
        msg: i64,
        buyer: i64,
        fruits: &str,
        price: i64,
    ) -> SqlResult<bool> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        ensure_row_locked(&tx, buyer)?;
        let bal: i64 = tx.query_row(
            "SELECT balance FROM balance WHERE user = ?1",
            params![buyer],
            |r| r.get(0),
        )?;
        if bal < price {
            return Ok(false); // tx drops → rollback
        }
        // Escrow the coins and record the offer atomically: a failure on either
        // statement leaves the buyer's balance untouched (no coins-into-the-void).
        tx.execute(
            "UPDATE balance SET balance = balance - ?1 WHERE user = ?2",
            params![price, buyer],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO buffer (chat, msg, kind, owner, fruits, price, created_at)
             VALUES (?1, ?2, 'buy', ?3, ?4, ?5, ?6)",
            params![chat, msg, buyer, fruits, price, current_unix_time()],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Settle a sell offer: `taker` is the buyer pressing the button. On
    /// success, the buffer row is deleted, the buyer pays the asking price,
    /// the seller gets the coins, and the escrowed fruit moves to the buyer.
    pub fn consume_sell(&self, chat: i64, msg: i64, taker: i64) -> SqlResult<OfferOutcome> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let row = tx
            .query_row(
                "SELECT kind, owner, fruits, price FROM buffer WHERE chat = ?1 AND msg = ?2",
                params![chat, msg],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<i64>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((kind, Some(seller), Some(fruits), Some(price))) = row else {
            return Ok(OfferOutcome::AlreadyTaken);
        };
        if kind != "sell" {
            return Ok(OfferOutcome::AlreadyTaken);
        }

        // Self-cancel: seller tapped own sell button → return fruit, delete buffer
        if taker == seller {
            ensure_row_locked(&tx, seller)?;
            let sf: String = tx.query_row(
                "SELECT fruit FROM balance WHERE user = ?1",
                params![seller],
                |r| r.get(0),
            )?;
            let new_sf = format!("{sf}{fruits}");
            tx.execute(
                "UPDATE balance SET fruit = ?1 WHERE user = ?2",
                params![new_sf, seller],
            )?;
            tx.execute(
                "DELETE FROM buffer WHERE chat = ?1 AND msg = ?2",
                params![chat, msg],
            )?;
            tx.commit()?;
            return Ok(OfferOutcome::SelfCancelled);
        }

        // Validate taker side
        ensure_row_locked(&tx, taker)?;
        let (bal, bf): (i64, String) = tx.query_row(
            "SELECT balance, fruit FROM balance WHERE user = ?1",
            params![taker],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if bal < price {
            return Ok(OfferOutcome::TakerNotEnoughBalance); // tx drops → rollback
        }
        if bf.chars().count() + fruits.chars().count() > 5 {
            return Ok(OfferOutcome::TakerFruitFull); // tx drops → rollback
        }

        // Fill: pay seller, move fruit to taker, drop the offer — all atomic so a
        // partial failure can't leave the buyer charged without the fruit (or the
        // buffer row surviving for a replay).
        ensure_row_locked(&tx, seller)?;
        let new_bf = format!("{bf}{fruits}");
        tx.execute(
            "UPDATE balance SET balance = ?1, fruit = ?2 WHERE user = ?3",
            params![bal - price, new_bf, taker],
        )?;
        tx.execute(
            "UPDATE balance SET balance = balance + ?1 WHERE user = ?2",
            params![price, seller],
        )?;
        tx.execute(
            "DELETE FROM buffer WHERE chat = ?1 AND msg = ?2",
            params![chat, msg],
        )?;
        tx.commit()?;
        Ok(OfferOutcome::Filled { fruits, price })
    }

    /// Settle a buy offer: `taker` is the seller pressing the button. On
    /// success the buyer (escrow owner) gets the listed fruit, the seller
    /// loses that fruit and gains the escrowed coins.
    pub fn consume_buy(&self, chat: i64, msg: i64, taker: i64) -> SqlResult<OfferOutcome> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let row = tx
            .query_row(
                "SELECT kind, owner, fruits, price FROM buffer WHERE chat = ?1 AND msg = ?2",
                params![chat, msg],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<i64>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((kind, Some(buyer), Some(fruits), Some(price))) = row else {
            return Ok(OfferOutcome::AlreadyTaken);
        };
        if kind != "buy" {
            return Ok(OfferOutcome::AlreadyTaken);
        }

        // Self-cancel: buyer tapped own buy button → refund coin, delete buffer
        if taker == buyer {
            ensure_row_locked(&tx, buyer)?;
            // Refund and delete atomically: if the DELETE failed after the
            // refund committed, the buyer could retap and be refunded again.
            tx.execute(
                "UPDATE balance SET balance = balance + ?1 WHERE user = ?2",
                params![price, buyer],
            )?;
            tx.execute(
                "DELETE FROM buffer WHERE chat = ?1 AND msg = ?2",
                params![chat, msg],
            )?;
            tx.commit()?;
            return Ok(OfferOutcome::SelfCancelled);
        }

        // Validate taker (seller) side
        ensure_row_locked(&tx, taker)?;
        let sf: String = tx.query_row(
            "SELECT fruit FROM balance WHERE user = ?1",
            params![taker],
            |r| r.get(0),
        )?;
        let mut sf_check = sf.clone();
        for f in fruits.chars() {
            let s = f.to_string();
            if let Some(idx) = sf_check.find(&s) {
                sf_check.remove(idx);
            } else {
                return Ok(OfferOutcome::TakerMissingFruit(f)); // tx drops → rollback
            }
        }

        // Validate buyer fruit space (escrowed at post time but buyer's
        // inventory may have grown since)
        ensure_row_locked(&tx, buyer)?;
        let bf: String = tx.query_row(
            "SELECT fruit FROM balance WHERE user = ?1",
            params![buyer],
            |r| r.get(0),
        )?;
        if bf.chars().count() + fruits.chars().count() > 5 {
            return Ok(OfferOutcome::TakerFruitFull); // tx drops → rollback
        }

        let new_bf = format!("{bf}{fruits}");
        // Coin was already escrowed away from buyer at post time, so we only
        // credit the seller (taker) here — fruit move, coin credit, and buffer
        // delete are one atomic unit.
        tx.execute(
            "UPDATE balance SET fruit = ?1, balance = balance + ?2 WHERE user = ?3",
            params![sf_check, price, taker],
        )?;
        tx.execute(
            "UPDATE balance SET fruit = ?1 WHERE user = ?2",
            params![new_bf, buyer],
        )?;
        tx.execute(
            "DELETE FROM buffer WHERE chat = ?1 AND msg = ?2",
            params![chat, msg],
        )?;
        tx.commit()?;
        Ok(OfferOutcome::Filled { fruits, price })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOT_ID: i64 = 999;
    const SELLER: i64 = 1;
    const BUYER: i64 = 2;
    const CHAT: i64 = 100;
    const MSG: i64 = 200;

    fn mk_db() -> Database {
        Database::new(":memory:", BOT_ID).expect("open in-memory db")
    }

    fn give_fruit(db: &Database, user: i64, fruits: &str) {
        for f in fruits.chars() {
            db.fruit_change(user, &f.to_string(), true).unwrap();
        }
    }

    fn give_coin(db: &Database, user: i64, amount: i64) {
        db.force_change(user, amount).unwrap();
    }

    #[test]
    fn open_sell_escrows_owned_fruit_only() {
        let db = mk_db();
        give_fruit(&db, SELLER, "🍑");
        let escrowed = db.open_sell_offer(CHAT, MSG, SELLER, "🍑🍓", 50).unwrap();
        assert_eq!(escrowed, "🍑");
        assert_eq!(db.get_user_info(SELLER).unwrap().fruit, "");
    }

    #[test]
    fn open_sell_no_fruit_no_buffer() {
        let db = mk_db();
        let escrowed = db.open_sell_offer(CHAT, MSG, SELLER, "🍑", 50).unwrap();
        assert!(escrowed.is_empty());
        assert!(!db.has_buffer(CHAT, MSG).unwrap());
    }

    #[test]
    fn consume_sell_completes_trade() {
        let db = mk_db();
        give_fruit(&db, SELLER, "🍑");
        give_coin(&db, BUYER, 100);
        db.open_sell_offer(CHAT, MSG, SELLER, "🍑", 50).unwrap();
        let outcome = db.consume_sell(CHAT, MSG, BUYER).unwrap();
        match outcome {
            OfferOutcome::Filled { fruits, price } => {
                assert_eq!(fruits, "🍑");
                assert_eq!(price, 50);
            }
            other => panic!("expected Filled, got {other:?}"),
        }
        assert_eq!(db.get_user_info(SELLER).unwrap().balance, 50);
        assert_eq!(db.get_user_info(BUYER).unwrap().balance, 50);
        assert_eq!(db.get_user_info(BUYER).unwrap().fruit, "🍑");
        assert!(!db.has_buffer(CHAT, MSG).unwrap());
    }

    #[test]
    fn consume_sell_self_cancel_refunds_fruit() {
        let db = mk_db();
        give_fruit(&db, SELLER, "🍑");
        db.open_sell_offer(CHAT, MSG, SELLER, "🍑", 50).unwrap();
        let outcome = db.consume_sell(CHAT, MSG, SELLER).unwrap();
        assert!(matches!(outcome, OfferOutcome::SelfCancelled));
        assert_eq!(db.get_user_info(SELLER).unwrap().fruit, "🍑");
        assert!(!db.has_buffer(CHAT, MSG).unwrap());
    }

    #[test]
    fn consume_sell_taker_low_balance() {
        let db = mk_db();
        give_fruit(&db, SELLER, "🍑");
        db.open_sell_offer(CHAT, MSG, SELLER, "🍑", 50).unwrap();
        let outcome = db.consume_sell(CHAT, MSG, BUYER).unwrap();
        assert!(matches!(outcome, OfferOutcome::TakerNotEnoughBalance));
        assert!(db.has_buffer(CHAT, MSG).unwrap()); // offer still live
        assert_eq!(db.get_user_info(BUYER).unwrap().balance, 0); // no spend
    }

    #[test]
    fn consume_sell_taker_fruit_full() {
        let db = mk_db();
        give_fruit(&db, SELLER, "🍑");
        give_fruit(&db, BUYER, "🍓🍎🍊🥭🍍"); // 5 fruits already
        give_coin(&db, BUYER, 100);
        db.open_sell_offer(CHAT, MSG, SELLER, "🍑", 50).unwrap();
        let outcome = db.consume_sell(CHAT, MSG, BUYER).unwrap();
        assert!(matches!(outcome, OfferOutcome::TakerFruitFull));
        assert_eq!(db.get_user_info(BUYER).unwrap().balance, 100); // unchanged
    }

    #[test]
    fn consume_sell_already_taken() {
        let db = mk_db();
        let outcome = db.consume_sell(CHAT, MSG, BUYER).unwrap();
        assert!(matches!(outcome, OfferOutcome::AlreadyTaken));
    }

    #[test]
    fn open_buy_escrows_coin() {
        let db = mk_db();
        give_coin(&db, BUYER, 100);
        assert!(db.open_buy_offer(CHAT, MSG, BUYER, "🍑", 50).unwrap());
        assert_eq!(db.get_user_info(BUYER).unwrap().balance, 50);
    }

    #[test]
    fn open_buy_rejects_when_broke() {
        let db = mk_db();
        assert!(!db.open_buy_offer(CHAT, MSG, BUYER, "🍑", 50).unwrap());
        assert!(!db.has_buffer(CHAT, MSG).unwrap());
    }

    #[test]
    fn claim_envelope_credits_escrowed_amount_exactly_once() {
        let db = mk_db();
        // SELLER escrows 50 for the envelope.
        db.insert_buffer(CHAT, MSG, SELLER, 50).unwrap();
        // First claim wins: credits the ESCROWED 50 and consumes the row. The
        // amount is read from the row — there is no caller-supplied value to
        // forge (a crafted callback can't change what's credited).
        assert_eq!(db.claim_envelope(CHAT, MSG, BUYER).unwrap(), Some(50));
        assert_eq!(db.get_user_info(BUYER).unwrap().balance, 50);
        assert!(!db.has_buffer(CHAT, MSG).unwrap());
        // A second (racing) claim finds the row gone → None, no double-credit.
        assert_eq!(db.claim_envelope(CHAT, MSG, BUYER).unwrap(), None);
        assert_eq!(db.get_user_info(BUYER).unwrap().balance, 50);
    }

    #[test]
    fn refund_envelope_returns_escrow_to_owner() {
        let db = mk_db();
        db.insert_buffer(CHAT, MSG, SELLER, 40).unwrap();
        db.refund_envelope(CHAT, MSG).unwrap();
        // Owner got the escrow back; the row is gone; a later claim finds nothing.
        assert_eq!(db.get_user_info(SELLER).unwrap().balance, 40);
        assert!(!db.has_buffer(CHAT, MSG).unwrap());
        assert_eq!(db.claim_envelope(CHAT, MSG, BUYER).unwrap(), None);
    }

    #[test]
    fn consume_buy_completes_trade() {
        let db = mk_db();
        give_coin(&db, BUYER, 100);
        give_fruit(&db, SELLER, "🍑");
        db.open_buy_offer(CHAT, MSG, BUYER, "🍑", 50).unwrap();
        let outcome = db.consume_buy(CHAT, MSG, SELLER).unwrap();
        match outcome {
            OfferOutcome::Filled { fruits, price } => {
                assert_eq!(fruits, "🍑");
                assert_eq!(price, 50);
            }
            other => panic!("expected Filled, got {other:?}"),
        }
        assert_eq!(db.get_user_info(SELLER).unwrap().balance, 50);
        assert_eq!(db.get_user_info(SELLER).unwrap().fruit, "");
        assert_eq!(db.get_user_info(BUYER).unwrap().balance, 50); // already deducted
        assert_eq!(db.get_user_info(BUYER).unwrap().fruit, "🍑");
        assert!(!db.has_buffer(CHAT, MSG).unwrap());
    }

    #[test]
    fn consume_buy_self_cancel_refunds_coin() {
        let db = mk_db();
        give_coin(&db, BUYER, 100);
        db.open_buy_offer(CHAT, MSG, BUYER, "🍑", 50).unwrap();
        let outcome = db.consume_buy(CHAT, MSG, BUYER).unwrap();
        assert!(matches!(outcome, OfferOutcome::SelfCancelled));
        assert_eq!(db.get_user_info(BUYER).unwrap().balance, 100); // full refund
    }

    #[test]
    fn consume_buy_taker_missing_fruit() {
        let db = mk_db();
        give_coin(&db, BUYER, 100);
        db.open_buy_offer(CHAT, MSG, BUYER, "🍑", 50).unwrap();
        // SELLER (taker here) has no fruit
        let outcome = db.consume_buy(CHAT, MSG, SELLER).unwrap();
        assert!(matches!(outcome, OfferOutcome::TakerMissingFruit('🍑')));
    }
}

