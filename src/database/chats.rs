use super::{current_unix_time, Database};
use rusqlite::{params, OptionalExtension, Result as SqlResult};

impl Database {
    /// Record (or refresh) a chat the bot has seen, so `/broadcast` can reach
    /// it. Private chats have positive ids (== the user id); groups, supergroups
    /// and channels have negative ids — both are stored.
    pub fn touch_chat(&self, chat_id: i64) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO chats (chat, seen_at) VALUES (?1, ?2)
             ON CONFLICT(chat) DO UPDATE SET seen_at = excluded.seen_at",
            params![chat_id, current_unix_time()],
        )?;
        Ok(())
    }

    /// Record who added the bot to a group, keeping the **first** adder (the
    /// referrer for that group's check-in binds). Creates the chat row if new.
    pub fn set_group_adder(&self, chat_id: i64, user_id: i64) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO chats (chat, seen_at, added_by) VALUES (?1, ?2, ?3)
             ON CONFLICT(chat) DO UPDATE SET
                 added_by = CASE WHEN chats.added_by = 0 THEN excluded.added_by ELSE chats.added_by END",
            params![chat_id, current_unix_time(), user_id],
        )?;
        Ok(())
    }

    /// The user who first added the bot to `chat_id`, if known.
    pub fn group_adder(&self, chat_id: i64) -> SqlResult<Option<i64>> {
        let conn = self.conn.lock();
        let v: Option<i64> = conn
            .query_row(
                "SELECT added_by FROM chats WHERE chat = ?1",
                params![chat_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.filter(|id| *id > 0))
    }

    /// Cache the group's **owner** (Telegram creator), resolved lazily from
    /// `getChatAdministrators` — distinct from `added_by`. Creates the chat row if
    /// new; overwrites a prior value (the creator is a fact, not first-wins).
    pub fn set_group_owner(&self, chat_id: i64, owner_id: i64) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO chats (chat, seen_at, owner) VALUES (?1, ?2, ?3)
             ON CONFLICT(chat) DO UPDATE SET owner = excluded.owner",
            params![chat_id, current_unix_time(), owner_id],
        )?;
        Ok(())
    }

    /// The cached group owner (Telegram creator) of `chat_id`, if known.
    pub fn group_owner(&self, chat_id: i64) -> SqlResult<Option<i64>> {
        let conn = self.conn.lock();
        let v: Option<i64> = conn
            .query_row(
                "SELECT owner FROM chats WHERE chat = ?1",
                params![chat_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.filter(|id| *id > 0))
    }

    /// Lock the bot to a single forum **topic** in `chat_id` (`/onlyreplyhere`):
    /// store the topic's `message_thread_id`. Creates the chat row if new.
    /// Outside this topic the bot ignores commands/buttons and stays silent.
    pub fn set_reply_thread(&self, chat_id: i64, thread: i64) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO chats (chat, seen_at, reply_thread) VALUES (?1, ?2, ?3)
             ON CONFLICT(chat) DO UPDATE SET reply_thread = excluded.reply_thread",
            params![chat_id, current_unix_time(), thread],
        )?;
        Ok(())
    }

    /// Clear any topic lock on `chat_id` (`/replyanywhere`) — the bot replies in
    /// any topic again. No-op if the chat was never locked.
    pub fn clear_reply_thread(&self, chat_id: i64) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE chats SET reply_thread = 0 WHERE chat = ?1",
            params![chat_id],
        )?;
        Ok(())
    }

    /// The forum topic `chat_id` is locked to, if any (`None` = reply anywhere).
    pub fn reply_thread(&self, chat_id: i64) -> SqlResult<Option<i64>> {
        let conn = self.conn.lock();
        let v: Option<i64> = conn
            .query_row(
                "SELECT reply_thread FROM chats WHERE chat = ?1",
                params![chat_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.filter(|t| *t > 0))
    }

    /// Every chat id the bot has seen — private DMs and groups alike.
    pub fn all_chat_ids(&self) -> SqlResult<Vec<i64>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT chat FROM chats")?;
        let ids = stmt
            .query_map([], |r| r.get(0))?
            .collect::<SqlResult<Vec<i64>>>()?;
        Ok(ids)
    }
}
