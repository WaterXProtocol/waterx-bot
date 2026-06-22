# 0622 報告 — 三項任務

針對 `eason_goals/0622.md` 的三個項目，以下是分析、實作與使用方式。Bot 已用新程式碼重新啟動（`@HiveRelayBot`, production mode），`cargo build` / `cargo test`（69 passed）皆通過。

---

## 1. 時區：能不能自動從用戶發言推斷？為什麼還要主動問？

### 結論：技術上「推不準」，所以改成「不主動問、用語言猜一個預設值、之後可自行調整」

Telegram **不會**在任何訊息欄位提供用戶時區。我們能拿到的只有：

- `message.date`：一律是 **UTC** 時間戳，不含當地時差。
- `User.language_code`：是「語言」不是「地區」。例如 `en` 可能是美國、英國、澳洲、印度…；`es` 橫跨歐洲到中南美；`zh` 本身也不帶時差資訊。
- 用戶分享位置 (`request_location`) 才有座標 → 但那本身就要用戶主動操作，跟「問時區」一樣麻煩。

所以「從發言內容自動推斷時區」沒有可靠訊號。先前「強制跳一個時區選單」純粹是 onboarding 的摩擦，收益很低。

### 已做的調整

1. **拿掉 onboarding 的強制時區步驟。** 原本流程是 `語言 → 時區 → 主選單`，現在選完語言**直接進主選單**（少一步）。
   - 程式：`src/commands/callbacks.rs::handle_set_lang`。
2. **用語言做 best-effort 猜測**，只在用戶還沒設過時區時填入：
   - `src/core/i18n.rs::Lang::default_tz_offset()`
   - 只對「單一時區」的語言給值：繁中/簡中/菲律賓語 → UTC+8、日/韓 → +9、泰/越/印尼 → +7、印地語 → +5:30、土耳其語 → +3。
   - 對「橫跨多時區」的語言（英、西、葡、法、德、荷、俄、阿拉伯）回傳 `None` → 預設 UTC，由用戶自行調整。
3. **保留手動設定入口**：`/settings → 🕐 Timezone` 與隱藏指令 `/timezone` 仍可隨時改。

> 影響：時區仍用於私聊的 `/markets` 開賽時間、`/predict` 截止時間的「當地時間顯示」。群組訊息是共享的，維持 UTC 不變。

---

## 2. 串接 Mini App（MVP）

### 怎麼做：Mini App = 一個 HTTPS 網頁，用三種方式從 bot 開啟

1. **Chat menu button**（輸入框旁邊那顆「選單」鈕）— 這就是你說的「menu 上該有的」。Bot 啟動時用 `setChatMenuButton` 指向你的 Web App。
2. **Inline 按鈕** — 私聊主選單新增 `🚀 Open app`（`web_app` 按鈕，只能在私聊用）。
3. *(可選)* BotFather `/newapp` 註冊 `t.me/<bot>/<app>` 短連結。

### 已做的 MVP（程式 + 範例頁）

**Bot 端（全部以 `MINI_APP_URL` 環境變數開關，沒設就完全不啟用）：**

- `src/core/types.rs`：`BotConfig` 新增 `mini_app_url`（讀 `MINI_APP_URL`，必須是 `https://`）。
- `src/bot.rs`：啟動時若有設 URL → 呼叫 `setChatMenuButton`（scope = 所有私聊），best-effort。
- `src/commands/tg.rs`：`build_keyboard` 支援 `webapp:<url>` 前綴 → 產生 Telegram `web_app` 按鈕。
- `src/commands/menu.rs`：私聊主選單在有設 URL 時加上 `🚀 Open app`。
- `src/core/i18n.rs`：`btn_open_app`（18 語系）。

**範例 Mini App（可直接部署）：**

- `miniapp/index.html`：單檔、零相依。載入 `telegram-web-app.js`、套用 Telegram 佈景、用名字打招呼、抓 `api.waterx.app/predict/browse` 顯示今日盤口。**唯讀**。
- `miniapp/README.md`：部署與 BotFather 設定步驟。

### 你要做的事（才會真的出現）

1. 把 `miniapp/` 放到任一 HTTPS 靜態主機（GitHub Pages / Cloudflare Pages / 你的網域）。
2. 在 `.env` 加上 `MINI_APP_URL=https://你的網域/miniapp/`，重啟 bot。
3. （目前 production `.env` 尚未設 `MINI_APP_URL`，所以選單鈕還沒啟用。）

### 限制（重要）

範例頁是**唯讀**的。任何會動到「金幣帳本」的功能，必須由後端**先驗證 `Telegram.WebApp.initData` 的 HMAC 簽章**（用 bot token）再執行 —— `initDataUnsafe` 不可信。這個 bot 目前沒有 HTTP server，要做「會寫入的 Mini App」需另外加一個後端（或用 `api.waterx.app` 的 endpoint）。詳見 `miniapp/README.md`。

---

## 3. `/onlyreplyhere`：群組管理員把 bot 鎖在某個 topic

### 行為

- 群組**管理員**在某個論壇 topic 內輸入 `/onlyreplyhere` → bot 之後**只在這個 topic 出現**：
  - 其他 topic 的指令 / 按鈕一律**靜默忽略**（不回話）。
  - bot 自己發出的群組訊息會**自動帶上該 topic 的 thread**（否則非回覆訊息會跑到「General」）。
- `/replyanywhere` → 解除鎖定，恢復在任何 topic 回覆。
- 兩個指令都是**群組限定 + 管理員限定**（用 `getChatMember` 驗證），且**不放進 `/` 公開選單**（屬群管工具，跟 `/timezone` 一樣可打但隱藏）。

### 實作

- **資料庫**：`chats` 表新增 `reply_thread`（0 = 不鎖；>0 = 鎖定的 topic thread id）+ 舊 DB 的 `ALTER` migration。
  - `src/database/chats.rs`：`set_reply_thread` / `clear_reply_thread` / `reply_thread`。
- **指令**：`src/commands/onlyreplyhere.rs`、`src/commands/replyanywhere.rs`（已註冊到 `mod.rs` 與 `bot.rs` 的 `create_framework!`）。
  - 只接受**真正的論壇 topic**（同時檢查 `is_topic_message`），避免把訊息丟進不存在的 thread 被 Telegram 退回。
  - 這兩個指令**刻意不走 `paused_block`**（topic 閘門就在那裡）—— 否則舊的鎖會擋住管理員改鎖/解鎖。
- **進站閘門**（忽略其他 topic）：`src/commands/util.rs::out_of_locked_topic`，接在：
  - `paused_block`（所有一般指令的頂端）
  - `callbacks::on_callback`（所有按鈕）
- **出站 thread**（訊息回到鎖定 topic）：`src/commands/tg.rs` 的送訊 helper（`send_with_buttons` / `..._reply` / `send_text_reply` / `send_html`）與 `util::send_text` 都會在群組鎖定時自動帶 `message_thread_id`。回覆型訊息（`reply`）靠 `reply_to` 本來就會落在原 topic。

### 限制 / 注意

- 只支援**論壇群組的具名 topic**（非 General）。在 General 或非論壇群組執行會回「請在某個 topic 裡使用」。
- 若鎖定的 topic 之後被刪除，bot 在該群會變安靜 —— 管理員用 `/replyanywhere` 即可恢復。
- 文案已做 18 語系（依操作者語言顯示）。

---

## 變更檔案總覽

| 檔案 | 任務 | 內容 |
|---|---|---|
| `src/core/i18n.rs` | 1,2,3 | `default_tz_offset`、`btn_open_app`、`onlyreply_*` 文案 |
| `src/commands/callbacks.rs` | 1,2,3 | onboarding 跳過時區、選單帶 mini app、按鈕 topic 閘門 |
| `src/core/types.rs` | 2 | `BotConfig.mini_app_url` |
| `src/bot.rs` | 2,3 | 啟動設 menu button、註冊兩個新指令 |
| `src/commands/tg.rs` | 2,3 | `webapp:` 按鈕、`is_chat_admin`、出站 thread |
| `src/commands/menu.rs` | 2 | 主選單加 `🚀 Open app` |
| `src/commands/util.rs` | 2,3 | `mini_app_url`、`out_of_locked_topic`、`send_text` 帶 thread |
| `src/database/{mod,chats}.rs` | 3 | `reply_thread` 欄位 + 存取方法 |
| `src/commands/{onlyreplyhere,replyanywhere}.rs` | 3 | 新指令 |
| `src/commands/{mod}.rs` | 3 | 註冊模組 |
| `miniapp/{index.html,README.md}` | 2 | 範例 Mini App + 部署說明 |
| `.env.example`, `.env.dev.example` | 2 | `MINI_APP_URL` 範例 |

## 環境備註

- 啟動前 toolchain 從 rustc 1.85.1 升到 **1.96.0**（相依套件 `icu_*` 需要 ≥1.86），用 `rustup update stable`。
- 目前 bot 以 `cargo run`（前景背景）執行，session 結束會停。長期運行建議用 `DEPLOY.md` 的 systemd 設定。
