use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use crate::db::hash_token;
use crate::error::{Error, Result};

use super::Database;

impl Database {
    pub fn create_session(&self, token: &str, user_id: Uuid, expires_at: i64) -> Result<()> {
        let token_hash = hash_token(token);
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO sessions (token, user_id, expires_at) VALUES (?1, ?2, ?3)",
            params![token_hash, user_id.to_string(), expires_at],
        )?;
        Ok(())
    }

    /// Returns the user_id if the session is valid (not expired).
    pub fn validate_session(&self, token: &str) -> Result<Option<Uuid>> {
        let token_hash = hash_token(token);
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT user_id FROM sessions WHERE token = ?1 AND expires_at > unixepoch()",
        )?;
        let result: Option<String> = stmt
            .query_row(params![token_hash], |row| row.get(0))
            .optional()?;
        match result {
            Some(id_str) => {
                let uid = Uuid::parse_str(&id_str)
                    .map_err(|e| Error::Internal(format!("invalid user UUID in session: {e}")))?;
                Ok(Some(uid))
            }
            None => Ok(None),
        }
    }

    /// Delete a session by its raw token.
    pub fn delete_session(&self, token: &str) -> Result<()> {
        let token_hash = hash_token(token);
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute("DELETE FROM sessions WHERE token = ?1", params![token_hash])?;
        Ok(())
    }

    /// Extend a session's expiration time.
    pub fn extend_session(&self, token: &str, new_expires_at: i64) -> Result<()> {
        let token_hash = hash_token(token);
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE sessions SET expires_at = ?1 WHERE token = ?2",
            params![new_expires_at, token_hash],
        )?;
        Ok(())
    }

    /// Delete all sessions for a given user.
    pub fn delete_user_sessions(&self, user_id: Uuid) -> Result<u64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let count = conn.execute(
            "DELETE FROM sessions WHERE user_id = ?1",
            params![user_id.to_string()],
        )?;
        Ok(count as u64)
    }

    /// Delete all expired sessions.
    pub fn cleanup_expired_sessions(&self) -> Result<u64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let count = conn.execute("DELETE FROM sessions WHERE expires_at <= unixepoch()", [])?;
        Ok(count as u64)
    }
}
