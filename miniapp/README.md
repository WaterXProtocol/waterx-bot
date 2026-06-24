# WaterX Mini App (sample)

A minimal [Telegram Mini App](https://core.telegram.org/bots/webapps): `index.html`
is a single, dependency-free static page that loads Telegram's
`telegram-web-app.js`, greets the user, applies the client theme, and lists today's
markets from the public `api.waterx.app/predict/browse` feed. It is intentionally
**read-only**.

**To deploy it (host + wire it to the bot), see [`docs/miniapp.md`](../docs/miniapp.md).**

Quick local preview (no Telegram context — markets render, but the greeting/theme
only light up inside Telegram):

```bash
cd miniapp && python3 -m http.server 8080
```
