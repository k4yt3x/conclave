use rusqlite::{params, OptionalExtension};

use crate::db::hash_token;
use crate::error::Result;

use super::Database;

impl Database {
    pub fn create_session(&self, token: &str, user_id: i64, expires_at: i64) -> Result<()> {
        let token_hash = hash_token(token);
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO sessions (token, user_id, expires_at) VALUES (?1, ?2, ?3)",
            params![token_hash, user_id, expires_at],
        )?;
        Ok(())
    }

    /// Returns the user_id if the session is valid (not expired).
    pub fn validate_session(&self, token: &str) -> Result<Option<i64>> {
        let token_hash = hash_token(token);
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT user_id FROM sessions WHERE token = ?1 AND expires_at > unixepoch()",
        )?;
        let result = stmt
            .query_row(params![token_hash], |row| row.get(0))
            .optional()?;
        Ok(result)
    }

    /// Delete a session by its raw token.
    pub fn delete_session(&self, token: &str) -> Result<()> {
        let token_hash = hash_token(token);
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute("DELETE FROM sessions WHERE token = ?1", params![token_hash])?;
        Ok(())
    }

    /// Delete all expired sessions.
    pub fn cleanup_expired_sessions(&self) -> Result<u64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let count = conn.execute("DELETE FROM sessions WHERE expires_at <= unixepoch()", [])?;
        Ok(count as u64)
    }
}
