#!/usr/bin/env bash
# Check out the `dev` branch, pull latest, build a release binary, and restart
# the bot.
#
# This self-host box tracks the **`dev`** branch (main is the Railway production
# deploy). We fetch + checkout dev explicitly so a `/redeploy` always lands on
# dev regardless of the branch the box happened to be on.
#
# Run this ONLY as the separate `waterx-deploy.service` oneshot unit (not as a
# child of waterx-bot) — that way `systemctl restart waterx-bot` below can't
# kill this script mid-build. Triggered by the owner's /redeploy command.
#
# Build happens BEFORE the restart, so a broken build leaves the old bot running.
set -euo pipefail

cd "$(dirname "$0")/.."          # repo root (this script lives in deploy/)

echo "[deploy] fetch + checkout dev, pull --ff-only"
git fetch origin
git checkout dev
git pull --ff-only origin dev

echo "[deploy] cargo build --release"
cargo build --release

echo "[deploy] systemctl restart waterx-bot"
sudo systemctl restart waterx-bot

echo "[deploy] done"
