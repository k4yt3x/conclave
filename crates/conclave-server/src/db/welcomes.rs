use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use crate::db::PendingWelcomeRow;
use crate::error::{Error, Result};

use super::Database;

impl Database {
    pub fn store_pending_welcome(
        &self,
        group_id: Uuid,
        group_alias: Option<&str>,
        user_id: Uuid,
        welcome_data: &[u8],
    ) -> Result<()> {
        let welcome_id = Uuid::new_v4();
        let conn = self.lock_conn();
        conn.execute(
            "INSERT INTO pending_welcomes (id, group_id, group_alias, user_id, welcome_data)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                welcome_id.to_string(),
                group_id.to_string(),
                group_alias,
                user_id.to_string(),
                welcome_data
            ],
        )?;
        Ok(())
    }

    /// Returns (welcome_id, group_id, group_alias, welcome_data).
    pub fn get_pending_welcomes(&self, user_id: Uuid) -> Result<Vec<PendingWelcomeRow>> {
        let conn = self.lock_conn();
        let mut stmt = conn.prepare(
            "SELECT id, group_id, group_alias, welcome_data
             FROM pending_welcomes
             WHERE user_id = ?1
             ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![user_id.to_string()], |row| {
                let id_str: String = row.get(0)?;
                let gid_str: String = row.get(1)?;
                Ok((id_str, gid_str, row.get(2)?, row.get(3)?))
            })?
            .collect::<std::result::Result<Vec<(String, String, Option<String>, Vec<u8>)>, _>>()?;
        let mut result = Vec::with_capacity(rows.len());
        for (id_str, gid_str, group_alias, welcome_data) in rows {
            let welcome_id = Uuid::parse_str(&id_str)
                .map_err(|e| Error::Internal(format!("invalid welcome UUID: {e}")))?;
            let group_id = Uuid::parse_str(&gid_str)
                .map_err(|e| Error::Internal(format!("invalid group UUID: {e}")))?;
            result.push(PendingWelcomeRow {
                welcome_id,
                group_id,
                group_alias,
                welcome_data,
            });
        }
        Ok(result)
    }

    pub fn delete_pending_welcome(&self, welcome_id: Uuid, user_id: Uuid) -> Result<()> {
        let conn = self.lock_conn();
        conn.execute(
            "DELETE FROM pending_welcomes WHERE id = ?1 AND user_id = ?2",
            params![welcome_id.to_string(), user_id.to_string()],
        )?;
        Ok(())
    }

    pub fn store_group_info(&self, group_id: Uuid, group_info_data: &[u8]) -> Result<()> {
        let conn = self.lock_conn();
        conn.execute(
            "INSERT INTO group_infos (group_id, group_info_data, updated_at)
             VALUES (?1, ?2, unixepoch())
             ON CONFLICT(group_id) DO UPDATE SET
                 group_info_data = excluded.group_info_data,
                 updated_at = excluded.updated_at",
            params![group_id.to_string(), group_info_data],
        )?;
        Ok(())
    }

    pub fn get_group_info(&self, group_id: Uuid) -> Result<Option<Vec<u8>>> {
        let conn = self.lock_conn();
        let mut stmt =
            conn.prepare("SELECT group_info_data FROM group_infos WHERE group_id = ?1")?;
        let result = stmt
            .query_row(params![group_id.to_string()], |row| row.get(0))
            .optional()?;
        Ok(result)
    }

    /// Process a commit upload atomically: store group info and store the
    /// commit message. Returns the stored message sequence number (if any),
    /// for SSE notification after commit.
    pub fn process_commit(
        &self,
        group_id: Uuid,
        sender_id: Uuid,
        group_info: &[u8],
        commit_message: &[u8],
    ) -> Result<super::CommitResult> {
        let mut conn = self.lock_conn();
        let transaction = conn.savepoint()?;
        let gid_str = group_id.to_string();

        if !group_info.is_empty() {
            transaction.execute(
                "INSERT INTO group_infos (group_id, group_info_data, updated_at)
                 VALUES (?1, ?2, unixepoch())
                 ON CONFLICT(group_id) DO UPDATE SET
                     group_info_data = excluded.group_info_data,
                     updated_at = excluded.updated_at",
                params![gid_str, group_info],
            )?;
        }

        let sequence_number = if !commit_message.is_empty() {
            let max_seq: Option<i64> = transaction
                .prepare("SELECT MAX(sequence_num) FROM messages WHERE group_id = ?1")?
                .query_row(params![gid_str], |row| row.get(0))
                .optional()?
                .flatten();
            let next_seq = max_seq.unwrap_or(0) + 1;

            transaction.execute(
                "INSERT INTO messages (group_id, sender_id, mls_message, sequence_num, created_at)
                 VALUES (?1, ?2, ?3, ?4, unixepoch())",
                params![gid_str, sender_id.to_string(), commit_message, next_seq],
            )?;
            Some(next_seq)
        } else {
            None
        };

        transaction.commit()?;
        Ok(super::CommitResult { sequence_number })
    }
}
