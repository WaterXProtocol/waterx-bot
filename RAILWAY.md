# Deploying to Railway

The bot is a **long-poll worker**: it talks to Telegram via `getUpdates`, opens
**no port**, and needs **no public domain**. Two things matter for a correct
deploy:

1. **A persistent volume for the SQLite ledger.** Railway's container filesystem
   is wiped on every redeploy. The bot writes its DB (and `/backup` snapshots) to
   the directory named by `DATA_DIR`; point that at a mounted volume or you lose
   every balance on the next deploy.
2. **Exactly one replica.** Polling the same `BOT_TOKEN` from two processes is a
   409 conflict. `railway.toml` pins `numReplicas = 1` — don't raise it.

This repo ships everything Railway needs: a `Dockerfile`, `.dockerignore`, and
`railway.toml`. Railway will use the Dockerfile automatically.

## Prerequisites

- A **production** bot token from [@BotFather](https://t.me/BotFather) — a
  *different* token from any dev bot (one token can't be polled by two running
  bots).
- Your numeric Telegram user id (message [@userinfobot](https://t.me/userinfobot))
  — this becomes `BOT_OWNER` (admin commands).

## Steps

1. **Create the service.** In Railway: **New Project → Deploy from GitHub repo →**
   `WaterXProtocol/waterx-bot`, branch `main`. Railway detects the `Dockerfile`
   and `railway.toml`; the first build takes a few minutes (large Rust dep tree).

2. **Add a volume.** Service → **Variables/Settings → Volumes → New Volume**, mount
   path **`/data`**. (Any path works as long as it matches `DATA_DIR` below.)

3. **Set variables.** Service → **Variables**:

   | Variable | Value | Notes |
   |----------|-------|-------|
   | `BOT_TOKEN` | `123456:ABC…` | from BotFather |
   | `BOT_OWNER` | `<your numeric id>` | admin gate |
   | `BOT_DEV` | `false` | **required** for production (uses `waterx.db`, prod envelope rules) |
   | `DATA_DIR` | `/data` | **must equal the volume mount path** |

   Do **not** set `ENV_FILE` — with no `.env` file the bot reads these process
   variables directly.

4. **Deploy** and open the **Deploy Logs**. Success looks like one line:
   `waterx-bot ready: @YourBot (id …), production mode`. Then message `/start`
   to the bot in Telegram.

## Redeploys

Push to `main` → Railway auto-builds and redeploys (or hit **Redeploy** in the
dashboard). Because the DB lives on the `/data` volume, balances survive.

> The bot's own `/redeploy` command drives a **systemd** unit (self-host only) —
> it's inert on Railway. Redeploy via `git push` or the Railway dashboard instead.

## Backups

The bot auto-snapshots the **whole SQLite DB** hourly to a single rolling
`waterx.db.bak` **in `DATA_DIR`** (the volume) — overwritten each time, no
timestamp, so it never grows. `/backup` forces one on demand; the `[Everything]`
`/reset` also snapshots before wiping. It's a full `VACUUM INTO` copy (every
table, not just balances).

**Restore** (a full DB file can't be hot-swapped under the running bot):

1. Stop the service (Railway → the service → **Remove/Stop**, or scale to 0).
2. In the Railway **shell** (or `railway ssh`): `cp /data/waterx.db.bak /data/waterx.db`
   (and delete any stale `/data/waterx.db-wal` / `-shm`).
3. Start the service again.

To pull a copy off the box for safekeeping, read `/data/waterx.db.bak` via the
shell / `railway ssh`.

## Notes & gotchas

- **No health check** — it's a worker, not a web service. Leave `healthcheckPath`
  unset (a health check would fail, since nothing listens on a port).
- **Don't scale horizontally.** `numReplicas` must stay `1` (getUpdates conflict).
- **Secrets:** `.env` / `.env.dev` are gitignored and `.dockerignore`d — real
  config only ever lives in Railway's Variables.
- **CI:** the GitHub Actions workflow (`.github/workflows/ci.yml`) gates every push
  with fmt + clippy + test + release build; a red build won't magically break a
  running deploy, but keep `main` green so redeploys are safe.

## Alternative: Nixpacks (no Dockerfile)

Railway can also build Rust with Nixpacks (zero config). The Dockerfile is
preferred here because it's deterministic and pins the C toolchain that
rusqlite's bundled SQLite needs. To try Nixpacks instead, delete/rename the
`Dockerfile` (or set `build.builder = "NIXPACKS"` in `railway.toml`) — but the
Dockerfile path is the tested one.
