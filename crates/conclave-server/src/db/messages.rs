use rusqlite::params;

use crate::db::StoredMessageRow;
use crate::error::Result;

use super::Database;

impl Database {
    pub fn store_message(&self, group_id: i64, sender_id: i64, mls_message: &[u8]) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let next_seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(sequence_num), 0) + 1 FROM messages WHERE group_id = ?1",
            params![group_id],
            |row| row.get(0),
        )?;
        conn.execute(
            "INSERT INTO messages (group_id, sender_id, mls_message, sequence_num)
             VALUES (?1, ?2, ?3, ?4)",
            params![group_id, sender_id, mls_message, next_seq],
        )?;
        Ok(next_seq)
    }

    pub fn get_messages(
        &self,
        group_id: i64,
        after_seq: i64,
        limit: i64,
    ) -> Result<Vec<StoredMessageRow>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT m.sequence_num, m.sender_id, u.username, u.alias, m.mls_message, m.created_at
             FROM messages m
             JOIN users u ON m.sender_id = u.id
             WHERE m.group_id = ?1 AND m.sequence_num > ?2
             ORDER BY m.sequence_num ASC
             LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(params![group_id, after_seq, limit], |row| {
                Ok(StoredMessageRow {
                    sequence_num: row.get(0)?,
                    sender_id: row.get(1)?,
                    sender_username: row.get(2)?,
                    sender_alias: row.get(3)?,
                    mls_message: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
