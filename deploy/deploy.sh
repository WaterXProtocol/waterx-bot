#!/usr/bin/env bash
# Pull latest, build a release binary, and restart the bot.
#
# Run this ONLY as the separate `waterx-deploy.service` oneshot unit (not as a
# child of waterx-bot) — that way `systemctl restart waterx-bot` below can't
# kill this script mid-build. Triggered by the owner's /redeploy command.
#
# Build happens BEFORE the restart, so a broken build leaves the old bot running.
set -euo pipefail

cd "$(dirname "$0")/.."          # repo root (this script lives in deploy/)

echo "[deploy] git pull --ff-only"
git pull --ff-only

echo "[deploy] cargo build --release"
cargo build --release

echo "[deploy] systemctl restart waterx-bot"
sudo systemctl restart waterx-bot

echo "[deploy] done"
