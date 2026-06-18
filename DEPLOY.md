# Deploy & self-update (`/redeploy`)

waterx-bot is a long-poll Telegram bot: **outbound HTTPS only** (no inbound
ports, no domain, no TLS cert). Its entire state is the SQLite file `waterx.db`
in the working directory — **back that up / keep it on persistent disk**.

## 1. Base service (systemd)

Build the release binary on the server (needs the Rust toolchain + a C compiler
for bundled SQLite: `sudo apt-get install -y build-essential`):

```bash
git clone git@github.com:WaterXProtocol/waterx-bot.git ~/waterx-bot
cd ~/waterx-bot && cargo build --release
```

`/etc/waterx-bot.env` (chmod 600 — holds the token):

```ini
BOT_TOKEN=123456789:your-real-token
BOT_OWNER=<your numeric Telegram id>
BOT_DEV=false
```

`/etc/systemd/system/waterx-bot.service` (replace `USER`):

```ini
[Unit]
Description=waterx-bot
After=network-online.target
Wants=network-online.target

[Service]
WorkingDirectory=/home/USER/waterx-bot
EnvironmentFile=/etc/waterx-bot.env
ExecStart=/home/USER/waterx-bot/target/release/waterx_bot
Restart=always
RestartSec=5
User=USER

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload && sudo systemctl enable --now waterx-bot
journalctl -u waterx-bot -f      # → "waterx-bot ready: @… production mode"
```

## 2. `/redeploy` (owner-only, chat-triggered self-update)

`/redeploy` fire-and-forgets a trigger and returns immediately; the real work
runs in a **separate** systemd oneshot (`waterx-deploy.service`) so the
`systemctl restart waterx-bot` step can't kill the deploy mid-build. The build
runs **before** the restart, so a broken build leaves the old bot running.

> ⚠️ This is remote code execution on your box, gated on `BOT_OWNER`. Only enable
> it if you accept that. It's **inert until you install the unit + sudoers
> below** — without them, `/redeploy` just replies that it couldn't start.

**a. Install the deploy unit** (`deploy/waterx-deploy.service` — edit `USER`):
```bash
sudo cp ~/waterx-bot/deploy/waterx-deploy.service /etc/systemd/system/
sudoedit /etc/systemd/system/waterx-deploy.service   # set USER + paths
sudo systemctl daemon-reload
```

**b. Allow the bot user to trigger it and restart the bot** — `sudo visudo -f
/etc/sudoers.d/waterx-bot` (replace `USER`):
```
USER ALL=(root) NOPASSWD: /usr/bin/systemctl start --no-block waterx-deploy.service, /usr/bin/systemctl restart waterx-bot
```
(`deploy/deploy.sh` calls `sudo systemctl restart waterx-bot`; the bot calls
`sudo systemctl start --no-block waterx-deploy.service`. Those are the only two
privileged commands granted.)

**c. (optional) override the trigger** — the bot runs `$REDEPLOY_CMD` (default
`sudo systemctl start --no-block waterx-deploy.service`). Set it in
`/etc/waterx-bot.env` to adapt to a non-systemd setup.

**Flow**: owner sends `/redeploy` → bot spawns the trigger, replies "🚀
Deploying…" → `waterx-deploy.service` runs `deploy/deploy.sh` (pull → build →
`systemctl restart waterx-bot`) → systemd relaunches the freshly built binary.

## Gotchas
- **One instance per token** — never run two processes polling the same
  `BOT_TOKEN` (getUpdates 409). The systemd restart swaps atomically.
- **Build resources** — `cargo build --release` needs real RAM/CPU; on a tiny
  VPS it can thrash. Consider building in CI and shipping the binary instead.
- **Persist `waterx.db`** — it's the whole ledger; the `WorkingDirectory` holds it.
- **Manual fallback** (no `/redeploy` needed):
  `cd ~/waterx-bot && git pull && cargo build --release && sudo systemctl restart waterx-bot`.
