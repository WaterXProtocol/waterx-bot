use super::{current_unix_time, Database};
use rusqlite::{params, Result as SqlResult};

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
