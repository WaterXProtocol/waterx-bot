# Error-surface audit

A whole-codebase audit of every place that can panic, swallow an error, fail
silently, or lose/mint money, plus the remediation status of each finding.

- **Audited:** 2026-06-20 (35 source files, 338 raw findings → ~120 distinct).
- **Method:** parallel per-file analyzers → dedup/synthesis, then each fix was
  adversarially re-verified (transaction semantics, behavioral-regression, and
  overflow/refund lenses) before commit.
- **Remediation commits:**
  - `d06edf6` — High-severity money-safety fixes.
  - `7ba5542` — Medium-severity fixes.

| Tier | Count | Status |
|---|---|---|
| 🔴 High | 9 + 2 noted | ✅ all fixed (`d06edf6`) |
| 🟠 Medium | ~22 | ✅ valuable ones fixed (`7ba5542`); a few consciously deferred (below) |
| ⚪ Low / info | ~90 | No action — deliberate best-effort, startup fail-fast, or provably unreachable |

The systemic root causes were **non-atomic multi-statement money mutations** (no
SQL transactions) and **silently swallowed money-write errors**. The fix was
systemic: every multi-statement money/state mutation now runs in one rusqlite
transaction, `balance_change` is a single atomic conditional `UPDATE`, and
callback-supplied amounts pass through `util::to_micro` (caps `MAX_COINS`,
`checked_mul`) before any `× COIN`. See the "Atomicity invariant" note in
`CLAUDE.md`.

---

## 🔴 High severity — all FIXED (`d06edf6`)

| # | Finding | Location | Fix |
|---|---|---|---|
| 1 | Non-atomic fill (debit/credit/credit + delete) → replay/double-spend on retap | `buffer.rs::consume_buy`/`consume_sell` | wrapped in one transaction |
| 2 | Non-atomic escrow (debit then insert) → coins/fruit removed with no escrow row | `buffer.rs::open_buy_offer`/`open_sell_offer` | wrapped in one transaction |
| 3 | Self-cancel refund + delete not atomic → re-refund (mint) on retap | `buffer.rs::consume_buy` self-cancel | folded into the `consume_buy` transaction |
| 4 | Per-winner credit + status-flip not atomic → mid-loop failure double-pays on re-run | `wager.rs::settle_market` | whole market settles in one transaction |
| 5 | TOCTOU: read balance (lock dropped) then write → concurrent debits overdraw | `user.rs::balance_change` | single conditional `UPDATE … WHERE balance + ?1 >= 0` |
| 6 | Debit then post then `insert_buffer` → coins debited but envelope untracked/unclaimable | `send.rs::send` (envelope) | refund-on-failure (`refund_or_log`) if post/insert fails |
| 7 | Debit then credit as two calls → coins destroyed if credit fails | `send.rs::send` (direct) | new atomic `Database::transfer` |
| 8 | `total * COIN` on unclamped callback `total` → i64 overflow can mint/destroy coins | `callbacks.rs::handle_game_place` | `util::to_micro` guard before any `× COIN` |
| 9 | Both referral credits `.ok()`-swallowed, non-atomic | `referral.rs::pay_referral` | atomic `Database::reward_referral` + logged failure |
| n1 | Swallowed rollback after a failed wager insert | `betting.rs::handle_size_place` | atomic `place_wager` (debit + insert), returns `bool` — no rollback to lose |
| n2 | Winner-credit `.ok()`-swallowed during self-host settle | `callbacks.rs::handle_gamble` | logged on failure, `saturating_mul` |

New tests: `balance_change` overdraw guard, atomic `transfer`, `reward_referral`,
`place_wager` atomic debit/insufficient.

---

## 🟠 Medium severity

### FIXED (`7ba5542`)

**Atomicity / state-loss**
- `mod.rs::refund_and_prune_old_buffer` (startup) — refund + delete in one transaction (no double-credit if a crash hits between them).
- `user.rs::try_checkin` — reward + referral cascade in one transaction (no user-credited-but-upline-unpaid).
- `fruit.rs::fruit_transfer` — debit + credit in one transaction.
- `callbacks.rs::handle_envelope` (coin claim) — new atomic `Database::claim_envelope` (delete-then-credit, exactly-once under concurrent taps), closing the `has_buffer → credit → delete` double-claim race. +test.
- `wager.rs::decimal_payout` — guards non-finite odds (a NaN no longer pays a winner 0).

**Surfaced swallowed errors (no more misleading UX)**
- `assets.rs::assets_text` — a DB read error shows an error notice (+logs), not a fake zero balance / vanished positions.
- `admin.rs::settle`/`handle_settle_cb`/`broadcast` — DB errors are reported, not masked as "no markets" / "sent to 0 chats".
- `admin.rs::run_settle` — returns `(ok, summary)` so the button flow shows an honest toast (no false "Settled ✅").
- `tg.rs::edit_text_only` — logs logical rejects (parity with `edit_with_buttons`).
- `feedback.rs` — logs an owner-DM bounce (feedback no longer lost without a trace).

**Fail-safe defaults**
- `util.rs::paused_block` + `callbacks.rs::on_callback` — the pause kill-switch **fails closed**: an unreadable `is_paused` flag blocks non-owners (the owner is checked first, so they can still `/unpause`).

**Best-effort (no longer aborts on a cosmetic failure)**
- `predict.rs` — id-tail re-edit is best-effort, so a failed cosmetic edit can't orphan the posted board before the game is registered.
- `sell.rs` / `buy.rs` — listing refresh is best-effort (escrow + offer row are already committed; the placeholder already carries a working DB-backed button).
- `betting.rs::handle_size_confirm` / `builder_text_rows` — display-only stake previews routed through `to_micro` / `saturating_mul` so a crafted `total` can't overflow even the preview.

### No action needed (already correct)
- `games.rs::load_all_bet_games` / `bot.rs::run` — already skips-and-logs per row on a deserialize error and starts with an empty map (loudly logged) on a load failure.

### Consciously deferred (with rationale)
- **`markets.rs::brief`/`fetch_matches` — no owner alert on feed failure.** Adding an owner DM would spam the owner during an upstream outage (many users × many `/markets` taps). Errors are logged via `eprintln`. Left as-is.
- **`admin.rs::run_settle` — winner result-DM bounce.** Now logged, but a bounced Telegram DM is unavoidable best-effort; no further action.
- **`bot.rs::robust_poll` — 409 Conflict loop / un-deserializable update dropped.** Infra resilience with no clean mid-poll owner-alert path; documented footgun (two bots on one token). Left as-is.
- **`send.rs::send` — fruit-loop partial transfer / lost confirmation message.** Partial transfer is by-design (move what you can); the ledger stays correct. Left as-is.

---

## ⚪ Low / info (~90) — no action

All fall into deliberate or provably-safe classes:

- **Startup fail-fast** — config/DB/token errors `?`-propagate before serving anyone (systemd restarts).
- **`.expect("…Key missing")` on the TypeMap** — inserted at startup before polling; init failure returns `Err` first.
- **Best-effort UI edits / notify DMs** (`let _`/`.ok()`) — the documented edit-in-place convention; state stays correct, only the display may be stale, and "a bounced DM must never affect flow".
- **Locale-fallback swallows a DB error** — cosmetic wrong-language only, no money impact.
- **Bounded i64 arithmetic** — capped by `MAX_COINS`; SQLite surfaces overflow as `Err`, not a panic.
- **Saturating `f64 as i64` casts** — Rust `as` saturates (no UB); exact at the bot's stake magnitudes.
- **Char-boundary byte-slices** — proven safe on ASCII / self-synchronising UTF-8.
- **Silent blank-ack on forged/stale callbacks** — defensive against crafted data; only confuses an attacker.
- **Confirmed Send-across-await safe** — every `parking_lot` guard is dropped before `.await`; verified, not a bug.
