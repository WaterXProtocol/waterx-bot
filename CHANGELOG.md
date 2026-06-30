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
- **`/backup` command** (owner-only) — snapshot every account with coins **or**
  fruit to a timestamped `balances-*.json` file *without* wiping anything; restore
  later with `/load`.

### Changed

- **`/events` (the Polymarket-sourced feed) rebuilt onto a cleaner model.** The
  sport-specific internals were replaced by a generic outcome-list model, so the
  bet/sell flow is now **outcome-index based**. Display and bet odds now come from
  the **waterx feed's relayed Polymarket price** (one feed fetch) — the separate
  per-event Gamma odds overlay was removed. Scope is unchanged: **sport 1X2 only**;
  the feed's award (multi-outcome) and crypto categories remain excluded, and
  Yes/No props were investigated but can't auto-settle (no reliable waterx→Gamma
  market link), so they're not ingested. Match settlement is unchanged.
- **Account snapshots now include fruit.** `/backup`, `/load`, and the
  `[Everything]` `/reset` safety snapshot persist `(user, balance, fruit)` (was
  balance-only); old balance-only snapshots still load (fruit → empty).

### Security

- **`#![forbid(unsafe_code)]` crate-wide** — a compile-time guarantee that no
  `unsafe` can enter the money path (ledger, market engine, LMSR/funding math).
  The crate already used zero `unsafe`.
- **`cargo audit` clean** — 0 vulnerabilities / 0 warnings across 238 dependencies;
  earlier transitive advisories were resolved by bumping `quinn-proto` and `anyhow`.
