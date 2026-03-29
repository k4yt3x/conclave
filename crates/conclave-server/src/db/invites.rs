use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use crate::db::{AcceptedInvite, PendingInviteRow};
use crate::error::{Error, Result};

use super::Database;

impl Database {
    pub fn create_pending_invite(
        &self,
        group_id: Uuid,
        inviter_id: Uuid,
        invitee_id: Uuid,
        commit_message: &[u8],
        welcome_data: &[u8],
        group_info: &[u8],
    ) -> Result<Uuid> {
        let invite_id = Uuid::new_v4();
        let conn = self.lock_conn();
        conn.execute(
            "INSERT INTO pending_invites (id, group_id, inviter_id, invitee_id, commit_message, welcome_data, group_info)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![invite_id.to_string(), group_id.to_string(), inviter_id.to_string(), invitee_id.to_string(), commit_message, welcome_data, group_info],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(err, _)
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Error::Conflict(
                    "a pending invite already exists for this user and group".into(),
                )
            }
            other => Error::Database(other),
        })?;
        Ok(invite_id)
    }

    pub fn get_pending_invite(&self, invite_id: Uuid) -> Result<Option<PendingInviteRow>> {
        let conn = self.lock_conn();
        let mut stmt = conn.prepare(
            "SELECT id, group_id, inviter_id, invitee_id, commit_message, welcome_data, group_info, created_at
             FROM pending_invites WHERE id = ?1",
        )?;
        let row = stmt
            .query_row(params![invite_id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                    row.get::<_, Vec<u8>>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            })
            .optional()?;
        match row {
            Some((
                id_str,
                gid_str,
                inviter_str,
                invitee_str,
                commit_message,
                welcome_data,
                group_info,
                created_at,
            )) => Ok(Some(PendingInviteRow {
                invite_id: Uuid::parse_str(&id_str)
                    .map_err(|e| Error::Internal(format!("invalid invite UUID: {e}")))?,
                group_id: Uuid::parse_str(&gid_str)
                    .map_err(|e| Error::Internal(format!("invalid group UUID: {e}")))?,
                inviter_id: Uuid::parse_str(&inviter_str)
                    .map_err(|e| Error::Internal(format!("invalid inviter UUID: {e}")))?,
                invitee_id: Uuid::parse_str(&invitee_str)
                    .map_err(|e| Error::Internal(format!("invalid invitee UUID: {e}")))?,
                commit_message,
                welcome_data,
                group_info,
                created_at,
            })),
            None => Ok(None),
        }
    }

    pub fn list_pending_invites_for_user(&self, user_id: Uuid) -> Result<Vec<PendingInviteRow>> {
        let conn = self.lock_conn();
        let mut stmt = conn.prepare(
            "SELECT id, group_id, inviter_id, invitee_id, commit_message, welcome_data, group_info, created_at
             FROM pending_invites WHERE invitee_id = ?1
             ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![user_id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                    row.get::<_, Vec<u8>>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut result = Vec::with_capacity(rows.len());
        for (
            id_str,
            gid_str,
            inviter_str,
            invitee_str,
            commit_message,
            welcome_data,
            group_info,
            created_at,
        ) in rows
        {
            result.push(PendingInviteRow {
                invite_id: Uuid::parse_str(&id_str)
                    .map_err(|e| Error::Internal(format!("invalid invite UUID: {e}")))?,
                group_id: Uuid::parse_str(&gid_str)
                    .map_err(|e| Error::Internal(format!("invalid group UUID: {e}")))?,
                inviter_id: Uuid::parse_str(&inviter_str)
                    .map_err(|e| Error::Internal(format!("invalid inviter UUID: {e}")))?,
                invitee_id: Uuid::parse_str(&invitee_str)
                    .map_err(|e| Error::Internal(format!("invalid invitee UUID: {e}")))?,
                commit_message,
                welcome_data,
                group_info,
                created_at,
            });
        }
        Ok(result)
    }

    /// Atomically accept a pending invite: remove from pending_invites, add to
    /// group_members, store the welcome in pending_welcomes, and store the commit
    /// as a group message.
    pub fn accept_pending_invite(&self, invite_id: Uuid) -> Result<AcceptedInvite> {
        let mut conn = self.lock_conn();
        let transaction = conn.savepoint()?;

        let invite = transaction
            .prepare(
                "SELECT id, group_id, inviter_id, invitee_id, commit_message, welcome_data, group_info
                 FROM pending_invites WHERE id = ?1",
            )?
            .query_row(params![invite_id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(1)?,  // group_id
                    row.get::<_, String>(2)?,  // inviter_id
                    row.get::<_, String>(3)?,  // invitee_id
                    row.get::<_, Vec<u8>>(4)?, // commit_message
                    row.get::<_, Vec<u8>>(5)?, // welcome_data
                    row.get::<_, Vec<u8>>(6)?, // group_info
                ))
            })
            .optional()?
            .ok_or_else(|| Error::NotFound("pending invite not found".into()))?;

        let (gid_str, inviter_str, invitee_str, commit_message, welcome_data, group_info) = invite;
        let group_id = Uuid::parse_str(&gid_str)
            .map_err(|e| Error::Internal(format!("invalid group UUID: {e}")))?;
        let inviter_id = Uuid::parse_str(&inviter_str)
            .map_err(|e| Error::Internal(format!("invalid inviter UUID: {e}")))?;
        let invitee_id = Uuid::parse_str(&invitee_str)
            .map_err(|e| Error::Internal(format!("invalid invitee UUID: {e}")))?;

        let invitee_username: String = transaction
            .prepare("SELECT username FROM users WHERE id = ?1")?
            .query_row(params![invitee_id.to_string()], |row| row.get(0))?;

        let group_alias: Option<String> = transaction
            .prepare("SELECT alias FROM groups WHERE id = ?1")?
            .query_row(params![group_id.to_string()], |row| row.get(0))?;

        // Verify the invitee is not already a member (could happen if they
        // joined via another path between invite creation and acceptance).
        let already_member: bool = transaction
            .prepare(
                "SELECT EXISTS(SELECT 1 FROM group_members WHERE group_id = ?1 AND user_id = ?2)",
            )?
            .query_row(
                params![group_id.to_string(), invitee_id.to_string()],
                |row| row.get(0),
            )?;
        if already_member {
            return Err(Error::Conflict(
                "user is already a member of this group".into(),
            ));
        }

        // Add the invitee to group members.
        transaction.execute(
            "INSERT INTO group_members (group_id, user_id) VALUES (?1, ?2)",
            params![group_id.to_string(), invitee_id.to_string()],
        )?;

        // Store the welcome for the invitee.
        let welcome_id = Uuid::new_v4();
        transaction.execute(
            "INSERT INTO pending_welcomes (id, group_id, group_alias, user_id, welcome_data, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, unixepoch())",
            params![welcome_id.to_string(), group_id.to_string(), group_alias, invitee_id.to_string(), welcome_data],
        )?;

        // Store the commit as a group message.
        let max_seq: Option<i64> = transaction
            .prepare("SELECT MAX(sequence_num) FROM messages WHERE group_id = ?1")?
            .query_row(params![group_id.to_string()], |row| row.get(0))
            .optional()?
            .flatten();
        let next_seq = max_seq.unwrap_or(0) + 1;

        transaction.execute(
            "INSERT INTO messages (group_id, sender_id, mls_message, sequence_num, created_at)
             VALUES (?1, ?2, ?3, ?4, unixepoch())",
            params![
                group_id.to_string(),
                inviter_id.to_string(),
                commit_message,
                next_seq
            ],
        )?;

        // Update group info.
        if !group_info.is_empty() {
            transaction.execute(
                "INSERT INTO group_infos (group_id, group_info_data, updated_at)
                 VALUES (?1, ?2, unixepoch())
                 ON CONFLICT(group_id) DO UPDATE SET
                     group_info_data = excluded.group_info_data,
                     updated_at = excluded.updated_at",
                params![group_id.to_string(), group_info],
            )?;
        }

        // Delete the pending invite.
        transaction.execute(
            "DELETE FROM pending_invites WHERE id = ?1",
            params![invite_id.to_string()],
        )?;

        transaction.commit()?;

        Ok(AcceptedInvite {
            group_id,
            inviter_id,
            invitee_id,
            invitee_username,
            group_alias,
            sequence_number: next_seq,
        })
    }

    pub fn list_pending_invites_for_group(&self, group_id: Uuid) -> Result<Vec<PendingInviteRow>> {
        let conn = self.lock_conn();
        let mut stmt = conn.prepare(
            "SELECT id, group_id, inviter_id, invitee_id, commit_message, welcome_data, group_info, created_at
             FROM pending_invites WHERE group_id = ?1
             ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![group_id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                    row.get::<_, Vec<u8>>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut result = Vec::with_capacity(rows.len());
        for (
            id_str,
            gid_str,
            inviter_str,
            invitee_str,
            commit_message,
            welcome_data,
            group_info,
            created_at,
        ) in rows
        {
            result.push(PendingInviteRow {
                invite_id: Uuid::parse_str(&id_str)
                    .map_err(|e| Error::Internal(format!("invalid invite UUID: {e}")))?,
                group_id: Uuid::parse_str(&gid_str)
                    .map_err(|e| Error::Internal(format!("invalid group UUID: {e}")))?,
                inviter_id: Uuid::parse_str(&inviter_str)
                    .map_err(|e| Error::Internal(format!("invalid inviter UUID: {e}")))?,
                invitee_id: Uuid::parse_str(&invitee_str)
                    .map_err(|e| Error::Internal(format!("invalid invitee UUID: {e}")))?,
                commit_message,
                welcome_data,
                group_info,
                created_at,
            });
        }
        Ok(result)
    }

    pub fn get_pending_invite_by_group_and_invitee(
        &self,
        group_id: Uuid,
        invitee_id: Uuid,
    ) -> Result<Option<PendingInviteRow>> {
        let conn = self.lock_conn();
        let mut stmt = conn.prepare(
            "SELECT id, group_id, inviter_id, invitee_id, commit_message, welcome_data, group_info, created_at
             FROM pending_invites WHERE group_id = ?1 AND invitee_id = ?2",
        )?;
        let row = stmt
            .query_row(
                params![group_id.to_string(), invitee_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                        row.get::<_, Vec<u8>>(5)?,
                        row.get::<_, Vec<u8>>(6)?,
                        row.get::<_, i64>(7)?,
                    ))
                },
            )
            .optional()?;
        match row {
            Some((
                id_str,
                gid_str,
                inviter_str,
                invitee_str,
                commit_message,
                welcome_data,
                group_info,
                created_at,
            )) => Ok(Some(PendingInviteRow {
                invite_id: Uuid::parse_str(&id_str)
                    .map_err(|e| Error::Internal(format!("invalid invite UUID: {e}")))?,
                group_id: Uuid::parse_str(&gid_str)
                    .map_err(|e| Error::Internal(format!("invalid group UUID: {e}")))?,
                inviter_id: Uuid::parse_str(&inviter_str)
                    .map_err(|e| Error::Internal(format!("invalid inviter UUID: {e}")))?,
                invitee_id: Uuid::parse_str(&invitee_str)
                    .map_err(|e| Error::Internal(format!("invalid invitee UUID: {e}")))?,
                commit_message,
                welcome_data,
                group_info,
                created_at,
            })),
            None => Ok(None),
        }
    }

    pub fn delete_pending_invite(&self, invite_id: Uuid) -> Result<()> {
        let conn = self.lock_conn();
        conn.execute(
            "DELETE FROM pending_invites WHERE id = ?1",
            params![invite_id.to_string()],
        )?;
        Ok(())
    }

    pub fn cleanup_expired_invites(&self, max_age_secs: i64) -> Result<u64> {
        let conn = self.lock_conn();
        let deleted = conn.execute(
            "DELETE FROM pending_invites WHERE created_at < (unixepoch() - ?1)",
            params![max_age_secs],
        )?;
        Ok(deleted as u64)
    }
}
