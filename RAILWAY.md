# Deploying to Railway

The bot is a **long-poll worker**: it talks to Telegram via `getUpdates`, opens
**no port**, and needs **no public domain**. Three things matter for a correct
deploy:

1. **A persistent volume for the SQLite ledger.** Railway's container filesystem
   is wiped on every redeploy. The bot writes its DB (and `.bak` snapshots) to the
   directory named by `DATA_DIR`; point that at a mounted volume or you lose every
   balance on the next deploy.
2. **Exactly one replica per service.** Polling the same `BOT_TOKEN` from two
   processes is a 409 conflict. `railway.toml` pins `numReplicas = 1` — don't raise
   it. (Two *different* services with *different* tokens is fine — see below.)
3. **A git push is the deploy.** Railway auto-builds and redeploys the service when
   its connected branch changes. There is **no in-bot `/redeploy` command** — that
   was a self-hosted-systemd relic and has been removed.

This repo ships everything Railway needs: a `Dockerfile`, `.dockerignore`, and
`railway.toml`. Railway uses the Dockerfile automatically.

## Recommended: two services — production + dev test

Run **two Railway services off the same repo**, each tracking a different branch
with its **own bot token and its own volume**. The `dev` service is your internal
test bot; `main` is live.

| | **production** | **dev (internal test)** |
|---|---|---|
| Deploy branch | `main` | `dev` |
| `BOT_TOKEN` | prod bot (e.g. `@WixyBot`) | **separate** test bot (e.g. `@WixyDevBot`) |
| `BOT_DEV` | `false` | `true` |
| `DATA_DIR` | `/data` | `/data` |
| Volume | its own | its **own, separate** volume |
| `BOT_OWNER` | your numeric id | your numeric id (same person) |

Why this works with zero code changes:

- `BotConfig::from_env` reads `BOT_TOKEN` / `BOT_OWNER` / `BOT_DEV` straight from the
  process environment — set them in each service's **Variables** tab. (No `.env`
  file on Railway; those are local-only and gitignored.)
- **Different tokens is mandatory** — one Telegram token can't be polled by two
  running bots at once (getUpdates 409). So you need two BotFather bots.
- **`BOT_DEV=true` on the dev service** gives you a real sandbox: `/reset` and
  `/delete` are `util::is_dev`-gated, so they work on the dev bot and are inert on
  prod — wipe/re-seed the test bot freely without touching the prod ledger.
- **Separate volumes** are the real isolation boundary — the two ledgers and their
  `.bak` snapshots never share a filesystem. (The `BOT_DEV` filename split,
  `waterx.db` vs `waterx-dev.db`, is a bonus belt, not the boundary. **Never share a
  volume between the two services.**)

Resulting flow: **push to `dev`** → the test bot redeploys (poke at it internally);
**merge `dev`→`main`** (PR + CI) → the prod bot redeploys. The PR-to-main gate stays
in front of production.

> Railway builds independently and only blocks a deploy if `cargo build` **fails** —
> it does **not** run the test suite. A commit that compiles but fails tests/clippy
> *will* go live on whichever service tracks that branch. Run `cargo fmt --check`,
> `cargo clippy --all-targets -- -D warnings`, and `cargo test` locally before every
> push. (CI on GitHub Actions still runs, but it won't hold Railway back.)

## Prerequisites

- **Two** bot tokens from [@BotFather](https://t.me/BotFather) — one prod, one dev.
- Your numeric Telegram user id (message [@userinfobot](https://t.me/userinfobot))
  — this becomes `BOT_OWNER`.

## Steps (do once per service)

1. **Create the service.** In Railway: **New Project → Deploy from GitHub repo →**
   `WaterXProtocol/waterx-bot`. Then in **Settings → Source**, set the **deploy
   branch** (`main` for prod, `dev` for the test service). Railway detects the
   `Dockerfile` + `railway.toml`; the first build takes a few minutes (large Rust
   dep tree).

2. **Add a volume.** Service → **Settings → Volumes → New Volume**, mount path
   **`/data`**. Each service gets its **own** volume — never share.

3. **Set variables.** Service → **Variables** (see the table above):

   | Variable | prod | dev |
   |----------|------|-----|
   | `BOT_TOKEN` | prod token | dev token |
   | `BOT_OWNER` | your id | your id |
   | `BOT_DEV` | `false` | `true` |
   | `DATA_DIR` | `/data` | `/data` |

   Do **not** set `ENV_FILE` — with no `.env` file the bot reads these process
   variables directly.

4. **Deploy** and open the **Deploy Logs**. Success is one line:
   `waterx-bot ready: @YourBot (id …), production mode` (or `dev mode` on the test
   service). Then message `/start` to that bot in Telegram.

Repeat for the second service on the other branch.

## Deploys

- Push to `dev` → the dev/test service rebuilds and restarts.
- Merge `dev`→`main` (PR + CI) → the prod service rebuilds and restarts.
- Or hit **Redeploy** on a service in the Railway dashboard to redeploy its current
  branch.

Because each DB lives on its service's `/data` volume, balances survive redeploys.

## Backups

Each service auto-snapshots its **whole SQLite DB** every 5 minutes to a single
rolling `<db>.bak` **in its `DATA_DIR`** (its own volume) — overwritten each time,
no timestamp, so it never grows. `/backup` (or the dashboard **💾 Backup** button)
forces one on demand; the `[Everything]` `/reset` also snapshots before wiping. It's
a full `VACUUM INTO` copy (every table), so it's transactionally consistent.

You can additionally enable **Railway's volume backups** (Service → **Backups** tab:
daily/weekly/monthly, with retention) for off-box history — they're complementary:
the in-bot `.bak` gives a ~5-min, guaranteed-consistent restore point, Railway's
snapshots give longer history and survive the volume itself dying. Since the clean
`.bak` sits on the volume, Railway's snapshot captures a consistent copy too.

**Restore** (a live DB file can't be hot-swapped under the running bot):

1. Stop the service (scale to 0, or Remove the deployment).
2. In the Railway **shell** / `railway ssh`: `cp /data/waterx.db.bak /data/waterx.db`
   (and delete any stale `/data/waterx.db-wal` / `-shm`). On the dev service the
   files are `waterx-dev.db{,.bak}`.
3. Start the service again.

## Notes & gotchas

- **No health check** — it's a worker, not a web service. Leave `healthcheckPath`
  unset (a health check would fail; nothing listens on a port).
- **Don't scale a service past 1 replica.** `numReplicas` must stay `1` (getUpdates
  409). Two *services* with *different* tokens is the supported way to run two bots.
- **Separate volumes per service** — sharing one would let dev and prod stomp each
  other's ledger.
- **Secrets:** `.env` / `.env.dev` are gitignored and `.dockerignore`d — real config
  only ever lives in Railway's Variables.
- **CI:** the GitHub Actions workflow (`.github/workflows/ci.yml`) gates every push
  with fmt + clippy + test + release build. Railway doesn't wait on it, so keep the
  branch green and run the gate locally before pushing.

## Alternative: Nixpacks (no Dockerfile)

Railway can also build Rust with Nixpacks (zero config). The Dockerfile is preferred
here because it's deterministic and pins the C toolchain rusqlite's bundled SQLite
needs. To try Nixpacks, delete/rename the `Dockerfile` (or set
`build.builder = "NIXPACKS"` in `railway.toml`) — but the Dockerfile path is the
tested one.
