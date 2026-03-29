use rusqlite::params;
use uuid::Uuid;

use crate::db::StoredMessageRow;
use crate::error::{Error, Result};

use super::Database;

impl Database {
    pub fn store_message(
        &self,
        group_id: Uuid,
        sender_id: Uuid,
        mls_message: &[u8],
    ) -> Result<i64> {
        let conn = self.lock_conn();
        let gid_str = group_id.to_string();
        let next_seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(sequence_num), 0) + 1 FROM messages WHERE group_id = ?1",
            params![gid_str],
            |row| row.get(0),
        )?;
        conn.execute(
            "INSERT INTO messages (group_id, sender_id, mls_message, sequence_num)
             VALUES (?1, ?2, ?3, ?4)",
            params![gid_str, sender_id.to_string(), mls_message, next_seq],
        )?;
        Ok(next_seq)
    }

    pub fn get_messages(
        &self,
        group_id: Uuid,
        after_seq: i64,
        limit: i64,
    ) -> Result<Vec<StoredMessageRow>> {
        let conn = self.lock_conn();
        let mut stmt = conn.prepare(
            "SELECT sequence_num, sender_id, mls_message, created_at
             FROM messages
             WHERE group_id = ?1 AND sequence_num > ?2
             ORDER BY sequence_num ASC
             LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(params![group_id.to_string(), after_seq, limit], |row| {
                let sender_id_str: String = row.get(1)?;
                Ok((row.get(0)?, sender_id_str, row.get(2)?, row.get(3)?))
            })?
            .collect::<std::result::Result<Vec<(i64, String, Vec<u8>, i64)>, _>>()?;
        let mut result = Vec::with_capacity(rows.len());
        for (sequence_num, sender_id_str, mls_message, created_at) in rows {
            let sender_id = Uuid::parse_str(&sender_id_str)
                .map_err(|e| Error::Internal(format!("invalid sender UUID: {e}")))?;
            result.push(StoredMessageRow {
                sequence_num,
                sender_id,
                mls_message,
                created_at,
            });
        }
        Ok(result)
    }

    /// Delete messages that exceed the server-wide retention or per-group expiry.
    /// Returns the total number of rows deleted.
    pub fn cleanup_expired_messages(&self, server_retention_seconds: i64) -> Result<u64> {
        let conn = self.lock_conn();
        let mut total: u64 = 0;

        // Server-wide retention (when > 0).
        if server_retention_seconds > 0 {
            let count = conn.execute(
                "DELETE FROM messages WHERE created_at < (unixepoch() - ?1)",
                params![server_retention_seconds],
            )?;
            total += count as u64;
        }

        // Per-group expiry (when > 0).
        let count = conn.execute(
            "DELETE FROM messages WHERE group_id IN (
                SELECT id FROM groups WHERE message_expiry_seconds > 0
            ) AND created_at < (
                unixepoch() - (SELECT message_expiry_seconds FROM groups WHERE groups.id = messages.group_id)
            )",
            [],
        )?;
        total += count as u64;

        Ok(total)
    }

    /// Delete messages in groups with `message_expiry_seconds = 0` where all current
    /// members have fetched past that sequence number.
    pub fn cleanup_fully_fetched_messages(&self) -> Result<u64> {
        let conn = self.lock_conn();
        let count = conn.execute(
            "DELETE FROM messages WHERE group_id IN (
                SELECT id FROM groups WHERE message_expiry_seconds = 0
            ) AND sequence_num <= (
                SELECT MIN(COALESCE(mfw.last_fetched_seq, 0))
                FROM group_members gm
                LEFT JOIN message_fetch_watermarks mfw
                    ON mfw.group_id = gm.group_id AND mfw.user_id = gm.user_id
                WHERE gm.group_id = messages.group_id
            )",
            [],
        )?;
        Ok(count as u64)
    }

    /// Upsert the fetch watermark for a user in a group.
    pub fn update_fetch_watermark(
        &self,
        group_id: Uuid,
        user_id: Uuid,
        sequence_num: i64,
    ) -> Result<()> {
        let conn = self.lock_conn();
        conn.execute(
            "INSERT INTO message_fetch_watermarks (group_id, user_id, last_fetched_seq)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(group_id, user_id) DO UPDATE SET
                last_fetched_seq = MAX(last_fetched_seq, excluded.last_fetched_seq)",
            params![group_id.to_string(), user_id.to_string(), sequence_num],
        )?;
        Ok(())
    }
}
