use super::Database;
use rand::seq::IteratorRandom;
use rusqlite::{params, Result as SqlResult};

impl Database {
    pub fn fruit_transfer(&self, sender: i64, receiver: i64, fruit: &str) -> SqlResult<bool> {
        let send_info = self.get_user_info(sender)?;
        if !send_info.fruit.contains(fruit) {
            return Ok(false);
        }
        let to_bot = receiver == self.bot_id;
        if !to_bot {
            let recv_info = self.get_user_info(receiver)?;
            if recv_info.fruit.chars().count() >= 5 {
                return Ok(false);
            }
            let new_recv = format!("{}{}", recv_info.fruit, fruit);
            let new_send = send_info.fruit.replacen(fruit, "", 1);
            // Debit sender + credit receiver atomically: a partial failure must
            // not destroy the sender's fruit without the receiver gaining it.
            let mut conn = self.conn.lock();
            let tx = conn.transaction()?;
            tx.execute(
                "UPDATE balance SET fruit = ?1 WHERE user = ?2",
                params![new_send, sender],
            )?;
            tx.execute(
                "UPDATE balance SET fruit = ?1 WHERE user = ?2",
                params![new_recv, receiver],
            )?;
            tx.commit()?;
        } else {
            // Bot is the receiver — fruit is eaten, not stored.
            let new_send = send_info.fruit.replacen(fruit, "", 1);
            let conn = self.conn.lock();
            conn.execute(
                "UPDATE balance SET fruit = ?1 WHERE user = ?2",
                params![new_send, sender],
            )?;
        }
        Ok(true)
    }

    pub fn fruit_change(&self, user_id: i64, fruit: &str, inc: bool) -> SqlResult<bool> {
        let info = self.get_user_info(user_id)?;
        let new_fruit = if inc {
            if info.fruit.chars().count() >= 5 {
                return Ok(false);
            }
            format!("{}{}", info.fruit, fruit)
        } else {
            if !info.fruit.contains(fruit) {
                return Ok(false);
            }
            info.fruit.replacen(fruit, "", 1)
        };
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE balance SET fruit = ?1 WHERE user = ?2",
            params![new_fruit, user_id],
        )?;
        Ok(true)
    }

    pub fn fruit_pop(&self, user_id: i64) -> SqlResult<Option<String>> {
        let info = self.get_user_info(user_id)?;
        if info.fruit.is_empty() {
            return Ok(None);
        }
        let mut rng = rand::thread_rng();
        let chars: Vec<char> = info.fruit.chars().collect();
        let &chosen = chars.iter().choose(&mut rng).unwrap();
        let s = chosen.to_string();
        let new_fruit = info.fruit.replacen(&s, "", 1);
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE balance SET fruit = ?1 WHERE user = ?2",
            params![new_fruit, user_id],
        )?;
        Ok(Some(s))
    }
}
