# Changelog

All notable changes to **waterx-bot** are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). This bot is continuously
deployed (via `/redeploy`), so **[Unreleased]** tracks what is live on `main` and
is cut into a dated release when the version is bumped.

## [Unreleased]

### Added

- **Liquidity-provider funding stage for host `/predict` markets.** A `/predict`
  market now opens in a **funding stage** instead of charging the host upfront:
  anyone can provide liquidity during a funding window by **allocating coins across
  outcomes**, and the pooled allocation *discovers the opening prices*
  (`priceₖ = fundedₖ / Σ funded`). When the window closes the stage finalizes
  lazily — the pooled seed sets the LMSR depth and trading opens; an under-seeded
  pool (`< MIN_SEED`) is **voided and the LPs refunded**. At resolution the pool's
  residual **plus accrued trading fees** is split **pro-rata** among LPs by
  contribution. LPs bear real, bounded risk (earn the fee, subsidize traders;
  net anywhere from ~0 to seed+fees) — and the house still never mints, so the
  coin-supply invariant holds. New `/predict` builder steps: a **trading-fee picker**
  (2 / 5 / 10%) and a **funding-window picker** (1h / 6h / 24h / 3d); a funding
  board shows each outcome's implied opening price + pool with a `[💧 Fund]` flow.
- **Liquidity stakes shown in `/assets` (and `/bets`).** Open LP contributions now
  render under a **💧 Liquidity provided** section — event title + committed coins,
  tagged 🌱 (funding) or 🟢 (live) — next to your open positions.
- **Automatic full-DB backups.** The bot snapshots the **whole SQLite DB** every
  5 minutes to a single rolling `<db>.bak` on the data volume (`VACUUM INTO`, written to a
  temp file then atomically renamed; overwritten each time — no timestamp, so it
  never grows and captures **every** table, not just balances). `/backup`
  (owner-only) forces one on demand; the `[Everything]` `/reset` snapshots before
  wiping. Restore is a file swap (stop the bot, copy the `.bak` over the live DB,
  restart). Replaces the earlier JSON `balances-*.json` export + `/load` restore,
  which were removed.

### Changed

- **`/events` (the Polymarket-sourced feed) rebuilt onto a cleaner model.** The
  sport-specific internals were replaced by a generic outcome-list model, so the
  bet/sell flow is now **outcome-index based**. Display and bet odds now come from
  the **waterx feed's relayed Polymarket price** (one feed fetch) — the separate
  per-event Gamma odds overlay was removed. Scope is unchanged: **sport 1X2 only**;
  the feed's award (multi-outcome) and crypto categories remain excluded, and
  Yes/No props were investigated but can't auto-settle (no reliable waterx→Gamma
  market link), so they're not ingested. Match settlement is unchanged.
### Security

- **`#![forbid(unsafe_code)]` crate-wide** — a compile-time guarantee that no
  `unsafe` can enter the money path (ledger, market engine, LMSR/funding math).
  The crate already used zero `unsafe`.
- **`cargo audit` clean** — 0 vulnerabilities / 0 warnings across 238 dependencies;
  earlier transitive advisories were resolved by bumping `quinn-proto` and `anyhow`.
