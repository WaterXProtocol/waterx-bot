# WaterX Mini App (MVP)

A minimal [Telegram Mini App](https://core.telegram.org/bots/webapps) (Web App)
for the bot. `index.html` is a single, dependency-free static page: it loads
Telegram's `telegram-web-app.js`, greets the user, applies the client theme, and
shows today's markets from the same public feed `/markets` uses
(`https://api.waterx.app/predict/browse`).

It is intentionally **read-only**. See "Going further" before letting a Mini App
touch the coin ledger.

## How a Mini App connects to the bot

There are several ways to open a Web App; this repo wires up the first two:

1. **Chat menu button** — the button next to the message input in every private
   chat. The bot sets this on startup (`setChatMenuButton`) when `MINI_APP_URL`
   is configured. This is the main "menu" entry point.
2. **Inline button** — the `🚀 Open app` button on the private home menu
   (a `web_app` inline button; these only work in private chats).
3. *(not used here)* a reply-keyboard `web_app` button, or a `t.me/<bot>/<app>`
   direct link configured via BotFather `/newapp`.

## Setup steps

1. **Host `index.html` over HTTPS.** Telegram requires `https://`. Any static
   host works — GitHub Pages, Cloudflare Pages, Netlify, or your own server
   (e.g. behind the same domain as `api.waterx.app`). The URL should point at the
   directory or the file, e.g. `https://your-domain/miniapp/`.

2. **Tell the bot the URL.** Add to `.env` (production) or `.env.dev`:

   ```env
   MINI_APP_URL=https://your-domain/miniapp/
   ```

   Restart the bot. On startup it points the chat menu button at this URL and
   adds the `🚀 Open app` home-menu button. Unset → both are skipped (no change).

3. *(Optional)* **Register a named app with BotFather** for a shareable
   `t.me/<bot>/<appname>` deep link: send `/newapp` to @BotFather, pick the bot,
   and give it the same URL. Not required for the menu-button integration above.

## Local preview

```bash
cd miniapp && python3 -m http.server 8080
```

Then browse `http://localhost:8080` — the markets list renders, but
`window.Telegram.WebApp` is only populated when opened **inside** Telegram, so
the greeting/theme only light up there. To test inside Telegram against a local
server, expose it over HTTPS with a tunnel (e.g. `cloudflared tunnel` or
`ngrok http 8080`) and set `MINI_APP_URL` to the tunnel URL.

## Going further (writes need a server)

`initDataUnsafe` is **not trusted**. To do anything that moves coins, the Mini
App must send `Telegram.WebApp.initData` to a backend that validates its HMAC
signature with the bot token (per the
[Web App data-validation docs](https://core.telegram.org/bots/webapps#validating-data-received-via-the-mini-app))
before acting. This bot has no HTTP server today; a write-capable Mini App would
need one added (or an endpoint on `api.waterx.app`) — keep the page read-only
until then.
