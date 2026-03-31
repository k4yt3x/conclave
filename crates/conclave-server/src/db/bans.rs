use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use crate::error::{Error, Result};

use super::{BannedUserRow, Database};

impl Database {
    pub fn ban_user(&self, group_id: Uuid, user_id: Uuid, banned_by: Uuid) -> Result<()> {
        let conn = self.lock_conn();
        let result = conn.execute(
            "INSERT INTO banned_users (group_id, user_id, banned_by) VALUES (?1, ?2, ?3)",
            params![
                group_id.to_string(),
                user_id.to_string(),
                banned_by.to_string()
            ],
        );
        match result {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(Error::Conflict(
                    "user is already banned from this group".into(),
                ))
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn unban_user(&self, group_id: Uuid, user_id: Uuid) -> Result<bool> {
        let conn = self.lock_conn();
        let rows = conn.execute(
            "DELETE FROM banned_users WHERE group_id = ?1 AND user_id = ?2",
            params![group_id.to_string(), user_id.to_string()],
        )?;
        Ok(rows > 0)
    }

    pub fn is_banned(&self, group_id: Uuid, user_id: Uuid) -> Result<bool> {
        let conn = self.lock_conn();
        let exists: Option<i64> = conn
            .prepare("SELECT 1 FROM banned_users WHERE group_id = ?1 AND user_id = ?2")?
            .query_row(params![group_id.to_string(), user_id.to_string()], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(exists.is_some())
    }

    pub fn list_banned_users(&self, group_id: Uuid) -> Result<Vec<BannedUserRow>> {
        let conn = self.lock_conn();
        let mut stmt = conn.prepare(
            "SELECT b.user_id, u.username, u.alias, b.banned_by, b.banned_at
             FROM banned_users b
             JOIN users u ON b.user_id = u.id
             WHERE b.group_id = ?1
             ORDER BY b.banned_at ASC",
        )?;
        let rows = stmt
            .query_map(params![group_id.to_string()], |row| {
                let user_id_str: String = row.get(0)?;
                let username: String = row.get(1)?;
                let alias: Option<String> = row.get(2)?;
                let banned_by_str: String = row.get(3)?;
                let banned_at: i64 = row.get(4)?;
                Ok((user_id_str, username, alias, banned_by_str, banned_at))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut result = Vec::with_capacity(rows.len());
        for (user_id_str, username, alias, banned_by_str, banned_at) in rows {
            let user_id = Uuid::parse_str(&user_id_str)
                .map_err(|e| Error::Internal(format!("invalid user UUID: {e}")))?;
            let banned_by = Uuid::parse_str(&banned_by_str)
                .map_err(|e| Error::Internal(format!("invalid banned_by UUID: {e}")))?;
            result.push(BannedUserRow {
                user_id,
                username,
                alias,
                banned_by,
                banned_at,
            });
        }
        Ok(result)
    }

    pub fn delete_pending_invites_for_user_in_group(
        &self,
        group_id: Uuid,
        user_id: Uuid,
    ) -> Result<()> {
        let conn = self.lock_conn();
        conn.execute(
            "DELETE FROM pending_invites WHERE group_id = ?1 AND invitee_id = ?2",
            params![group_id.to_string(), user_id.to_string()],
        )?;
        Ok(())
    }
}
