use rusqlite::{params, OptionalExtension};

use crate::db::PendingWelcomeRow;
use crate::error::Result;

use super::Database;

impl Database {
    pub fn store_pending_welcome(
        &self,
        group_id: i64,
        group_alias: Option<&str>,
        user_id: i64,
        welcome_data: &[u8],
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO pending_welcomes (group_id, group_alias, user_id, welcome_data)
             VALUES (?1, ?2, ?3, ?4)",
            params![group_id, group_alias, user_id, welcome_data],
        )?;
        Ok(())
    }

    /// Returns (welcome_id, group_id, group_alias, welcome_data).
    pub fn get_pending_welcomes(&self, user_id: i64) -> Result<Vec<PendingWelcomeRow>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, group_id, group_alias, welcome_data
             FROM pending_welcomes
             WHERE user_id = ?1
             ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![user_id], |row| {
                Ok(PendingWelcomeRow {
                    welcome_id: row.get(0)?,
                    group_id: row.get(1)?,
                    group_alias: row.get(2)?,
                    welcome_data: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn delete_pending_welcome(&self, welcome_id: i64, user_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "DELETE FROM pending_welcomes WHERE id = ?1 AND user_id = ?2",
            params![welcome_id, user_id],
        )?;
        Ok(())
    }

    pub fn store_group_info(&self, group_id: i64, group_info_data: &[u8]) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO group_infos (group_id, group_info_data, updated_at)
             VALUES (?1, ?2, unixepoch())
             ON CONFLICT(group_id) DO UPDATE SET
                 group_info_data = excluded.group_info_data,
                 updated_at = excluded.updated_at",
            params![group_id, group_info_data],
        )?;
        Ok(())
    }

    pub fn get_group_info(&self, group_id: i64) -> Result<Option<Vec<u8>>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt =
            conn.prepare("SELECT group_info_data FROM group_infos WHERE group_id = ?1")?;
        let result = stmt
            .query_row(params![group_id], |row| row.get(0))
            .optional()?;
        Ok(result)
    }

    /// Process a commit upload atomically: store group info and store the
    /// commit message. Returns the stored message sequence number (if any),
    /// for SSE notification after commit.
    pub fn process_commit(
        &self,
        group_id: i64,
        sender_id: i64,
        group_info: &[u8],
        commit_message: &[u8],
    ) -> Result<super::CommitResult> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let transaction = conn.savepoint()?;

        if !group_info.is_empty() {
            transaction.execute(
                "INSERT INTO group_infos (group_id, group_info_data, updated_at)
                 VALUES (?1, ?2, unixepoch())
                 ON CONFLICT(group_id) DO UPDATE SET
                     group_info_data = excluded.group_info_data,
                     updated_at = excluded.updated_at",
                params![group_id, group_info],
            )?;
        }

        let sequence_number = if !commit_message.is_empty() {
            let max_seq: Option<i64> = transaction
                .prepare("SELECT MAX(sequence_num) FROM messages WHERE group_id = ?1")?
                .query_row(params![group_id], |row| row.get(0))
                .optional()?
                .flatten();
            let next_seq = max_seq.unwrap_or(0) + 1;

            transaction.execute(
                "INSERT INTO messages (group_id, sender_id, mls_message, sequence_num, created_at)
                 VALUES (?1, ?2, ?3, ?4, unixepoch())",
                params![group_id, sender_id, commit_message, next_seq],
            )?;
            Some(next_seq)
        } else {
            None
        };

        transaction.commit()?;
        Ok(super::CommitResult { sequence_number })
    }
}
