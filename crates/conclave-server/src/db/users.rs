use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use crate::db::{UserInfo, UserRow};
use crate::error::{Error, Result};
use crate::validation::validate_alias;

use super::Database;

impl Database {
    pub fn create_user(&self, username: &str, password_hash: &str) -> Result<Uuid> {
        let user_id = Uuid::new_v4();
        let conn = self.lock_conn();
        conn.execute(
            "INSERT INTO users (id, username, password_hash) VALUES (?1, ?2, ?3)",
            params![user_id.to_string(), username, password_hash],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(err, _)
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Error::Conflict(format!("username '{username}' already exists"))
            }
            other => Error::Database(other),
        })?;
        Ok(user_id)
    }

    pub fn get_user_by_username(&self, username: &str) -> Result<Option<UserRow>> {
        let conn = self.lock_conn();
        let mut stmt = conn.prepare(
            "SELECT id, username, password_hash, alias, signing_key_fingerprint
             FROM users WHERE username = ?1",
        )?;
        let result = stmt
            .query_row(params![username], |row| {
                let id_str: String = row.get(0)?;
                Ok((id_str, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
            })
            .optional()?;
        match result {
            Some((id_str, username, password_hash, alias, signing_key_fingerprint)) => {
                let user_id = Uuid::parse_str(&id_str)
                    .map_err(|e| Error::Internal(format!("invalid user UUID: {e}")))?;
                Ok(Some(UserRow {
                    user_id,
                    username,
                    password_hash,
                    alias,
                    signing_key_fingerprint,
                }))
            }
            None => Ok(None),
        }
    }

    pub fn get_user_by_id(&self, user_id: Uuid) -> Result<Option<UserInfo>> {
        let conn = self.lock_conn();
        let mut stmt = conn.prepare(
            "SELECT id, username, alias, signing_key_fingerprint FROM users WHERE id = ?1",
        )?;
        let result = stmt
            .query_row(params![user_id.to_string()], |row| {
                let id_str: String = row.get(0)?;
                Ok((id_str, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .optional()?;
        match result {
            Some((id_str, username, alias, signing_key_fingerprint)) => {
                let user_id = Uuid::parse_str(&id_str)
                    .map_err(|e| Error::Internal(format!("invalid user UUID: {e}")))?;
                Ok(Some(UserInfo {
                    user_id,
                    username,
                    alias,
                    signing_key_fingerprint,
                }))
            }
            None => Ok(None),
        }
    }

    pub fn get_password_hash(&self, user_id: Uuid) -> Result<Option<String>> {
        let conn = self.lock_conn();
        let mut stmt = conn.prepare("SELECT password_hash FROM users WHERE id = ?1")?;
        let result = stmt
            .query_row(params![user_id.to_string()], |row| row.get(0))
            .optional()?;
        Ok(result)
    }

    pub fn get_user_id_by_username(&self, username: &str) -> Result<Option<Uuid>> {
        let conn = self.lock_conn();
        let mut stmt = conn.prepare("SELECT id FROM users WHERE username = ?1")?;
        let result: Option<String> = stmt
            .query_row(params![username], |row| row.get(0))
            .optional()?;
        match result {
            Some(id_str) => {
                let uid = Uuid::parse_str(&id_str)
                    .map_err(|e| Error::Internal(format!("invalid user UUID: {e}")))?;
                Ok(Some(uid))
            }
            None => Ok(None),
        }
    }

    pub fn update_user_password(&self, user_id: Uuid, password_hash: &str) -> Result<()> {
        let conn = self.lock_conn();
        conn.execute(
            "UPDATE users SET password_hash = ?1 WHERE id = ?2",
            params![password_hash, user_id.to_string()],
        )?;
        Ok(())
    }

    pub fn update_signing_key_fingerprint(&self, user_id: Uuid, fingerprint: &str) -> Result<()> {
        let conn = self.lock_conn();
        conn.execute(
            "UPDATE users SET signing_key_fingerprint = ?1 WHERE id = ?2",
            params![fingerprint, user_id.to_string()],
        )?;
        Ok(())
    }

    /// Delete a user and all their data. Manually deletes from tables without
    /// ON DELETE CASCADE (pending_invites, messages) before deleting the user
    /// row, which cascades to everything else.
    pub fn delete_user(&self, user_id: Uuid) -> Result<()> {
        let mut conn = self.lock_conn();
        let savepoint = conn.savepoint()?;
        let uid_str = user_id.to_string();
        savepoint.execute(
            "DELETE FROM pending_invites WHERE inviter_id = ?1 OR invitee_id = ?1",
            params![uid_str],
        )?;
        savepoint.execute(
            "DELETE FROM messages WHERE sender_id = ?1",
            params![uid_str],
        )?;
        savepoint.execute("DELETE FROM users WHERE id = ?1", params![uid_str])?;
        savepoint.commit()?;
        Ok(())
    }

    pub fn update_user_alias(&self, user_id: Uuid, alias: Option<&str>) -> Result<()> {
        if let Some(alias) = alias {
            validate_alias(alias)?;
        }
        let conn = self.lock_conn();
        conn.execute(
            "UPDATE users SET alias = ?1 WHERE id = ?2",
            params![alias, user_id.to_string()],
        )?;
        Ok(())
    }
}
