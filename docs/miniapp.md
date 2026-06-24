# Deploying the Telegram Mini App

How to put a [Telegram Mini App](https://core.telegram.org/bots/webapps) (Web App)
in front of this bot. The bot wires up a Mini App **only** when the `MINI_APP_URL`
environment variable is set — leave it unset and nothing about the bot changes.

The repo ships a ready-to-host sample at [`miniapp/`](../miniapp/): a single,
dependency-free `index.html` that loads Telegram's SDK, applies the client theme,
greets the user, and lists today's markets from the public
`api.waterx.app/predict/browse` feed. It is **read-only** — see
[Security](#security-reads-are-free-writes-are-not) before adding anything that
moves coins.

---

## How it connects

Once `MINI_APP_URL` is set, the bot exposes the app two ways:

1. **Chat menu button** — the button next to the message input in every private
   chat. Set on startup via `setChatMenuButton` (scope = all private chats).
2. **Home-menu button** — a `🚀 Open app` `web_app` button on the private
   `/start` menu. (Inline `web_app` buttons only work in private chats.)

A third option — a shareable `t.me/<bot>/<appname>` link — is optional and set up
via BotFather, see [step 4](#4-optional-named-app-deep-link).

---

## Steps

### 1. Host the page over HTTPS

Telegram **requires `https://`** (a plain `http://` URL will not open). Any static
host works; pick one:

#### Option A — GitHub Pages (free)
1. Put the contents of `miniapp/` at the root of a repo (or in a `/docs` folder of
   one).
2. Repo → **Settings → Pages** → Source = your branch (and `/root` or `/docs`).
3. Wait for the deploy; your URL is `https://<user>.github.io/<repo>/`.

#### Option B — Cloudflare Pages / Netlify / Vercel (free)
- Create a new project and point it at the `miniapp/` directory (or drag-and-drop
  the folder). You get an `https://<project>.pages.dev` (or similar) URL.

#### Option C — your own server / same domain as the API
- Serve `miniapp/index.html` from any path under a TLS domain, e.g.
  `https://app.waterx.app/` or `https://waterx.app/miniapp/`. Co-locating it with
  `api.waterx.app` keeps everything on one origin and simplifies a future backend.

Note the final URL — e.g. `https://app.waterx.app/miniapp/`.

### 2. Point the bot at it

Add the URL to the bot's env file (`.env` for production, `.env.dev` for the dev
bot):

```env
MINI_APP_URL=https://app.waterx.app/miniapp/
```

Restart the bot. On startup it sets the chat menu button and adds the
`🚀 Open app` home-menu button. (Unset the var and restart to remove both.)

### 3. Verify

- Open a **private** chat with the bot → the menu button (bottom-left, next to the
  input) should now read "Open app" and launch the page.
- Send `/start` → the menu shows a `🚀 Open app` button.

### 4. (Optional) Named-app deep link

For a shareable `t.me/<bot>/<appname>` link: message **@BotFather** → `/newapp` →
pick the bot → give it the same URL. Not needed for the menu/home-button
integration above.

---

## Local testing

```bash
cd miniapp && python3 -m http.server 8080
```

Browsing `http://localhost:8080` renders the markets list, but
`window.Telegram.WebApp` (name, theme) is only populated when the page is opened
**inside Telegram**. To test inside Telegram against your local server, expose it
over HTTPS with a tunnel and point `MINI_APP_URL` at the tunnel URL:

```bash
cloudflared tunnel --url http://localhost:8080   # or: ngrok http 8080
```

---

## Security: reads are free, writes are not

The sample only **reads** a public feed, so it ships no trust assumptions. The
moment a Mini App needs to do anything privileged (show a balance, place a bet,
move coins) the rules change:

- `Telegram.WebApp.initDataUnsafe` is **client-supplied and forgeable** — never
  trust it for anything that matters.
- The app must send the signed `Telegram.WebApp.initData` string to a backend that
  **validates its HMAC signature with the bot token** before acting. See the
  [official data-validation guide](https://core.telegram.org/bots/webapps#validating-data-received-via-the-mini-app).

This bot currently has **no HTTP server**, so a write-capable Mini App needs one
added (or an authenticated endpoint on `api.waterx.app`). Keep the page read-only
until that exists.
