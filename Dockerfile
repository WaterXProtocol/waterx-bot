# syntax=docker/dockerfile:1
#
# Multi-stage build for the waterx Telegram bot.
#   - builder: compiles the release binary (rusqlite `bundled` builds SQLite from
#     C, so the builder needs a C toolchain; reqwest uses rustls, so no OpenSSL).
#   - runtime: a slim Debian image with just CA certs (reqwest verifies HTTPS to
#     the Telegram + waterx APIs against the system trust store).
#
# The bot is a long-poll worker: it opens no port and needs no public domain.
# All persistent state (the SQLite ledger + `/backup` snapshots) goes to the
# directory named by DATA_DIR — point that at a mounted volume (see RAILWAY.md).

FROM rust:1-slim-bookworm AS builder
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app

# 1) Build dependencies against a stub crate so the (slow) dependency layer is
#    cached and only rebuilt when Cargo.toml/Cargo.lock change.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src \
    && echo 'fn main() {}' > src/main.rs \
    && touch src/lib.rs \
    && cargo build --release --locked \
    && rm -rf src

# 2) Build the real binary. `touch` bumps the source mtimes so cargo recompiles
#    our crate while reusing the cached dependency artifacts above.
COPY . .
RUN touch src/main.rs src/lib.rs \
    && cargo build --release --locked

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/waterx_bot /usr/local/bin/waterx_bot
# Config comes from environment variables (BOT_TOKEN, BOT_OWNER, BOT_DEV,
# DATA_DIR) — set them on the host, not here. No ENV_FILE: the bot reads the
# process environment directly when no .env is present.
ENTRYPOINT ["waterx_bot"]
