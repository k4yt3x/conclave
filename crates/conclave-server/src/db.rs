use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// Hash a token with SHA-256 for safe storage.
fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    hex::encode(digest)
}

/// Server-side SQLite database for users, sessions, groups, and messages.
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// Open (or create) the database at the given path.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.initialize()?;
        Ok(db)
    }

    /// Create an in-memory database (for testing).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.initialize()?;
        Ok(db)
    }

    fn initialize(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute_batch("PRAGMA journal_mode = WAL;")?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT UNIQUE NOT NULL,
                password_hash TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE TABLE IF NOT EXISTS sessions (
                token TEXT PRIMARY KEY,
                user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                expires_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS key_packages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                key_package_data BLOB NOT NULL,
                is_last_resort INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE TABLE IF NOT EXISTS groups (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                creator_id INTEGER NOT NULL REFERENCES users(id),
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE TABLE IF NOT EXISTS group_members (
                group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                PRIMARY KEY (group_id, user_id)
            );

            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                sender_id INTEGER NOT NULL REFERENCES users(id),
                mls_message BLOB NOT NULL,
                sequence_num INTEGER NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE INDEX IF NOT EXISTS idx_messages_group_seq
                ON messages(group_id, sequence_num);

            CREATE TABLE IF NOT EXISTS pending_welcomes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                group_name TEXT NOT NULL,
                user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                welcome_data BLOB NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE INDEX IF NOT EXISTS idx_pending_welcomes_user
                ON pending_welcomes(user_id);

            CREATE TABLE IF NOT EXISTS group_infos (
                group_id TEXT PRIMARY KEY REFERENCES groups(id) ON DELETE CASCADE,
                group_info_data BLOB NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            ",
        )?;

        // Migration: add is_last_resort column if upgrading from an older schema.
        let _ = conn.execute_batch(
            "ALTER TABLE key_packages ADD COLUMN is_last_resort INTEGER NOT NULL DEFAULT 0;",
        );

        Ok(())
    }

    // ── Users ──────────────────────────────────────────────────────

    pub fn create_user(&self, username: &str, password_hash: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO users (username, password_hash) VALUES (?1, ?2)",
            params![username, password_hash],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(err, _)
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Error::Conflict(format!("username '{username}' already exists"))
            }
            other => Error::Database(other),
        })?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_user_by_username(&self, username: &str) -> Result<Option<(i64, String, String)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt =
            conn.prepare("SELECT id, username, password_hash FROM users WHERE username = ?1")?;
        let result = stmt
            .query_row(params![username], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .optional()?;
        Ok(result)
    }

    pub fn get_user_by_id(&self, user_id: i64) -> Result<Option<(i64, String)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare("SELECT id, username FROM users WHERE id = ?1")?;
        let result = stmt
            .query_row(params![user_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?;
        Ok(result)
    }

    pub fn get_user_id_by_username(&self, username: &str) -> Result<Option<i64>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare("SELECT id FROM users WHERE username = ?1")?;
        let result = stmt
            .query_row(params![username], |row| row.get(0))
            .optional()?;
        Ok(result)
    }

    // ── Sessions ───────────────────────────────────────────────────

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

    // ── Key Packages ───────────────────────────────────────────────

    /// Maximum number of regular (non-last-resort) key packages per user.
    const MAX_KEY_PACKAGES_PER_USER: i64 = 10;

    /// Store a key package for a user.
    ///
    /// If `is_last_resort` is true, any existing last-resort package for this
    /// user is replaced (at most one last-resort package per user).  Regular
    /// packages accumulate up to [`MAX_KEY_PACKAGES_PER_USER`](Self::MAX_KEY_PACKAGES_PER_USER).
    pub fn store_key_package(&self, user_id: i64, data: &[u8], is_last_resort: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());

        if is_last_resort {
            // At most one last-resort key package per user — replace the old one.
            conn.execute(
                "DELETE FROM key_packages WHERE user_id = ?1 AND is_last_resort = 1",
                params![user_id],
            )?;
        } else {
            // Enforce cap on regular key packages — silently skip if at capacity.
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM key_packages WHERE user_id = ?1 AND is_last_resort = 0",
                params![user_id],
                |row| row.get(0),
            )?;
            if count >= Self::MAX_KEY_PACKAGES_PER_USER {
                return Ok(());
            }
        }

        conn.execute(
            "INSERT INTO key_packages (user_id, key_package_data, is_last_resort) \
             VALUES (?1, ?2, ?3)",
            params![user_id, data, is_last_resort as i32],
        )?;
        Ok(())
    }

    /// Consume a key package for the given user.
    ///
    /// Prefers regular (non-last-resort) packages and deletes them on consumption.
    /// Falls back to the last-resort package if no regular ones remain — the
    /// last-resort package is returned but **not** deleted.
    pub fn consume_key_package(&self, user_id: i64) -> Result<Option<Vec<u8>>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());

        // Try a regular key package first (FIFO by created_at).
        let mut stmt = conn.prepare(
            "SELECT id, key_package_data FROM key_packages \
             WHERE user_id = ?1 AND is_last_resort = 0 \
             ORDER BY created_at ASC LIMIT 1",
        )?;
        let regular: Option<(i64, Vec<u8>)> = stmt
            .query_row(params![user_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?;

        if let Some((id, data)) = regular {
            conn.execute("DELETE FROM key_packages WHERE id = ?1", params![id])?;
            return Ok(Some(data));
        }

        // Fall back to the last-resort package (do NOT delete it).
        let mut stmt = conn.prepare(
            "SELECT key_package_data FROM key_packages \
             WHERE user_id = ?1 AND is_last_resort = 1 LIMIT 1",
        )?;
        let last_resort: Option<Vec<u8>> = stmt
            .query_row(params![user_id], |row| row.get(0))
            .optional()?;

        Ok(last_resort)
    }

    /// Count key packages for a user: (regular, last_resort).
    pub fn count_key_packages(&self, user_id: i64) -> Result<(i64, i64)> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let regular: i64 = conn.query_row(
            "SELECT COUNT(*) FROM key_packages WHERE user_id = ?1 AND is_last_resort = 0",
            params![user_id],
            |row| row.get(0),
        )?;
        let last_resort: i64 = conn.query_row(
            "SELECT COUNT(*) FROM key_packages WHERE user_id = ?1 AND is_last_resort = 1",
            params![user_id],
            |row| row.get(0),
        )?;
        Ok((regular, last_resort))
    }

    // ── Groups ─────────────────────────────────────────────────────

    pub fn create_group(&self, group_id: &str, name: &str, creator_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO groups (id, name, creator_id) VALUES (?1, ?2, ?3)",
            params![group_id, name, creator_id],
        )?;
        // Creator is automatically a member.
        conn.execute(
            "INSERT INTO group_members (group_id, user_id) VALUES (?1, ?2)",
            params![group_id, creator_id],
        )?;
        Ok(())
    }

    pub fn add_group_member(&self, group_id: &str, user_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT OR IGNORE INTO group_members (group_id, user_id) VALUES (?1, ?2)",
            params![group_id, user_id],
        )?;
        Ok(())
    }

    pub fn is_group_member(&self, group_id: &str, user_id: i64) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt =
            conn.prepare("SELECT 1 FROM group_members WHERE group_id = ?1 AND user_id = ?2")?;
        let exists: Option<i64> = stmt
            .query_row(params![group_id, user_id], |row| row.get(0))
            .optional()?;
        Ok(exists.is_some())
    }

    pub fn list_user_groups(&self, user_id: i64) -> Result<Vec<(String, String, i64, i64)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT g.id, g.name, g.creator_id, g.created_at
             FROM groups g
             JOIN group_members gm ON g.id = gm.group_id
             WHERE gm.user_id = ?1
             ORDER BY g.created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![user_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn remove_group_member(&self, group_id: &str, user_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "DELETE FROM group_members WHERE group_id = ?1 AND user_id = ?2",
            params![group_id, user_id],
        )?;
        Ok(())
    }

    pub fn get_group_members(&self, group_id: &str) -> Result<Vec<(i64, String)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT u.id, u.username
             FROM users u
             JOIN group_members gm ON u.id = gm.user_id
             WHERE gm.group_id = ?1",
        )?;
        let rows = stmt
            .query_map(params![group_id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ── Messages ───────────────────────────────────────────────────

    pub fn store_message(&self, group_id: &str, sender_id: i64, mls_message: &[u8]) -> Result<i64> {
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
        group_id: &str,
        after_seq: i64,
        limit: i64,
    ) -> Result<Vec<(i64, i64, String, Vec<u8>, i64)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT m.sequence_num, m.sender_id, u.username, m.mls_message, m.created_at
             FROM messages m
             JOIN users u ON m.sender_id = u.id
             WHERE m.group_id = ?1 AND m.sequence_num > ?2
             ORDER BY m.sequence_num ASC
             LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(params![group_id, after_seq, limit], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ── Pending Welcomes ───────────────────────────────────────────

    pub fn store_pending_welcome(
        &self,
        group_id: &str,
        group_name: &str,
        user_id: i64,
        welcome_data: &[u8],
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO pending_welcomes (group_id, group_name, user_id, welcome_data)
             VALUES (?1, ?2, ?3, ?4)",
            params![group_id, group_name, user_id, welcome_data],
        )?;
        Ok(())
    }

    pub fn get_pending_welcomes(
        &self,
        user_id: i64,
    ) -> Result<Vec<(i64, String, String, Vec<u8>)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, group_id, group_name, welcome_data
             FROM pending_welcomes
             WHERE user_id = ?1
             ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![user_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
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

    // ── Group Info (for External Commits) ──────────────────────────

    pub fn store_group_info(&self, group_id: &str, group_info_data: &[u8]) -> Result<()> {
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

    pub fn get_group_info(&self, group_id: &str) -> Result<Option<Vec<u8>>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt =
            conn.prepare("SELECT group_info_data FROM group_infos WHERE group_id = ?1")?;
        let result = stmt
            .query_row(params![group_id], |row| row.get(0))
            .optional()?;
        Ok(result)
    }

    // ── Account Reset ──────────────────────────────────────────────

    pub fn delete_key_packages(&self, user_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "DELETE FROM key_packages WHERE user_id = ?1",
            params![user_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Existing tests (1-3) ───────────────────────────────────────

    #[test]
    fn test_user_crud() {
        let db = Database::open_in_memory().unwrap();

        let id = db.create_user("alice", "hash123").unwrap();
        assert!(id > 0);

        let user = db.get_user_by_username("alice").unwrap().unwrap();
        assert_eq!(user.0, id);
        assert_eq!(user.1, "alice");
        assert_eq!(user.2, "hash123");

        // Duplicate username should fail.
        let result = db.create_user("alice", "hash456");
        assert!(result.is_err());
    }

    #[test]
    fn test_key_packages() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("bob", "hash").unwrap();

        db.store_key_package(uid, b"kp_data", false).unwrap();
        let kp = db.consume_key_package(uid).unwrap().unwrap();
        assert_eq!(kp, b"kp_data");

        // Consumed, should be gone.
        assert!(db.consume_key_package(uid).unwrap().is_none());
    }

    #[test]
    fn test_groups_and_messages() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();

        db.create_group("g1", "test-group", alice).unwrap();
        db.add_group_member("g1", bob).unwrap();

        assert!(db.is_group_member("g1", alice).unwrap());
        assert!(db.is_group_member("g1", bob).unwrap());

        let groups = db.list_user_groups(alice).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].1, "test-group");

        let members = db.get_group_members("g1").unwrap();
        assert_eq!(members.len(), 2);

        let seq1 = db.store_message("g1", alice, b"msg1").unwrap();
        let seq2 = db.store_message("g1", bob, b"msg2").unwrap();
        assert_eq!(seq1, 1);
        assert_eq!(seq2, 2);

        let msgs = db.get_messages("g1", 0, 100).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].3, b"msg1");
        assert_eq!(msgs[1].3, b"msg2");

        // Fetch after seq 1.
        let msgs = db.get_messages("g1", 1, 100).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].3, b"msg2");
    }

    // ── New tests (4-31) ───────────────────────────────────────────

    #[test]
    fn test_duplicate_username_returns_conflict() {
        let db = Database::open_in_memory().unwrap();
        db.create_user("alice", "hash123").unwrap();
        let result = db.create_user("alice", "hash456");
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("already exists"),
            "Expected error to contain 'already exists', got: {err_msg}"
        );
    }

    #[test]
    fn test_get_nonexistent_user() {
        let db = Database::open_in_memory().unwrap();
        let result = db.get_user_by_username("nobody").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_get_user_by_id() {
        let db = Database::open_in_memory().unwrap();
        let id = db.create_user("alice", "hash").unwrap();
        let user = db.get_user_by_id(id).unwrap().unwrap();
        assert_eq!(user.0, id);
        assert_eq!(user.1, "alice");
    }

    #[test]
    fn test_get_user_by_id_nonexistent() {
        let db = Database::open_in_memory().unwrap();
        let result = db.get_user_by_id(9999).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_session_create_validate() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();
        let token = "test-session-token";
        db.create_session(token, uid, i64::MAX).unwrap();
        let result = db.validate_session(token).unwrap();
        assert_eq!(result, Some(uid));
    }

    #[test]
    fn test_session_invalid_token() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();
        db.create_session("real-token", uid, i64::MAX).unwrap();
        let result = db.validate_session("wrong-token").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_session_expired() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();
        let token = "expired-token";
        db.create_session(token, uid, 0).unwrap();
        let result = db.validate_session(token).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_session_delete() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();
        let token = "delete-me";
        db.create_session(token, uid, i64::MAX).unwrap();

        // Session is valid before deletion.
        assert!(db.validate_session(token).unwrap().is_some());

        db.delete_session(token).unwrap();

        // Session is gone after deletion.
        assert!(db.validate_session(token).unwrap().is_none());
    }

    #[test]
    fn test_cleanup_expired_sessions() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();

        // One expired session (expires_at = 0, in the past).
        db.create_session("expired-token", uid, 0).unwrap();

        // One valid session (expires_at far in the future).
        db.create_session("valid-token", uid, i64::MAX).unwrap();

        let cleaned = db.cleanup_expired_sessions().unwrap();
        assert_eq!(cleaned, 1);

        // The valid session should still work.
        assert_eq!(db.validate_session("valid-token").unwrap(), Some(uid));

        // The expired session should already have been gone, but confirm.
        assert!(db.validate_session("expired-token").unwrap().is_none());
    }

    #[test]
    fn test_key_package_accumulate() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();

        db.store_key_package(uid, b"kp1", false).unwrap();
        db.store_key_package(uid, b"kp2", false).unwrap();

        // Should consume oldest first (FIFO).
        let kp = db.consume_key_package(uid).unwrap().unwrap();
        assert_eq!(kp, b"kp1");

        let kp = db.consume_key_package(uid).unwrap().unwrap();
        assert_eq!(kp, b"kp2");

        // Both consumed.
        assert!(db.consume_key_package(uid).unwrap().is_none());
    }

    #[test]
    fn test_last_resort_key_package() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();

        db.store_key_package(uid, b"last_resort", true).unwrap();
        db.store_key_package(uid, b"regular1", false).unwrap();

        // Should consume the regular one first.
        let kp = db.consume_key_package(uid).unwrap().unwrap();
        assert_eq!(kp, b"regular1");

        // Now only last-resort remains; returned but NOT deleted.
        let kp = db.consume_key_package(uid).unwrap().unwrap();
        assert_eq!(kp, b"last_resort");

        // Still there on second consume.
        let kp = db.consume_key_package(uid).unwrap().unwrap();
        assert_eq!(kp, b"last_resort");
    }

    #[test]
    fn test_key_package_cap() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();

        for i in 0..Database::MAX_KEY_PACKAGES_PER_USER {
            db.store_key_package(uid, format!("kp{i}").as_bytes(), false)
                .unwrap();
        }

        // Next regular package should be silently skipped (not stored).
        db.store_key_package(uid, b"kp_overflow", false).unwrap();
        let (regular, _) = db.count_key_packages(uid).unwrap();
        assert_eq!(regular, Database::MAX_KEY_PACKAGES_PER_USER);

        // But a last-resort should still work (separate limit).
        db.store_key_package(uid, b"last_resort", true).unwrap();
    }

    #[test]
    fn test_last_resort_replacement() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();

        db.store_key_package(uid, b"lr1", true).unwrap();
        db.store_key_package(uid, b"lr2", true).unwrap();

        // Only the most recent last-resort should exist.
        let (regular_count, lr_count) = db.count_key_packages(uid).unwrap();
        assert_eq!(regular_count, 0);
        assert_eq!(lr_count, 1);

        let kp = db.consume_key_package(uid).unwrap().unwrap();
        assert_eq!(kp, b"lr2");
    }

    #[test]
    fn test_consume_key_package_nonexistent_user() {
        let db = Database::open_in_memory().unwrap();
        let result = db.consume_key_package(9999).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_group_membership_check_nonmember() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();

        db.create_group("g1", "test-group", alice).unwrap();

        assert!(db.is_group_member("g1", alice).unwrap());
        assert!(!db.is_group_member("g1", bob).unwrap());
    }

    #[test]
    fn test_add_group_member_idempotent() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();

        db.create_group("g1", "test-group", alice).unwrap();
        db.add_group_member("g1", bob).unwrap();
        // Adding the same member again should not error.
        db.add_group_member("g1", bob).unwrap();

        let members = db.get_group_members("g1").unwrap();
        assert_eq!(members.len(), 2);
    }

    #[test]
    fn test_remove_group_member() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();

        db.create_group("g1", "test-group", alice).unwrap();
        db.add_group_member("g1", bob).unwrap();
        assert!(db.is_group_member("g1", bob).unwrap());

        db.remove_group_member("g1", bob).unwrap();
        assert!(!db.is_group_member("g1", bob).unwrap());
    }

    #[test]
    fn test_remove_group_member_nonexistent() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();

        db.create_group("g1", "test-group", alice).unwrap();

        // Removing a non-member should not panic or error.
        db.remove_group_member("g1", bob).unwrap();
    }

    #[test]
    fn test_list_user_groups_empty() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();

        let groups = db.list_user_groups(alice).unwrap();
        assert!(groups.is_empty());
    }

    #[test]
    fn test_get_group_members_after_removal() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();

        db.create_group("g1", "test-group", alice).unwrap();
        db.add_group_member("g1", bob).unwrap();

        let members = db.get_group_members("g1").unwrap();
        assert_eq!(members.len(), 2);

        db.remove_group_member("g1", bob).unwrap();

        let members = db.get_group_members("g1").unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].0, alice);
        assert_eq!(members[0].1, "alice");
    }

    #[test]
    fn test_messages_sequence_numbers() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();

        db.create_group("g1", "test-group", alice).unwrap();

        let seq1 = db.store_message("g1", alice, b"msg1").unwrap();
        let seq2 = db.store_message("g1", alice, b"msg2").unwrap();
        let seq3 = db.store_message("g1", alice, b"msg3").unwrap();

        assert_eq!(seq1, 1);
        assert_eq!(seq2, 2);
        assert_eq!(seq3, 3);

        let msgs = db.get_messages("g1", 0, 100).unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].0, 1);
        assert_eq!(msgs[1].0, 2);
        assert_eq!(msgs[2].0, 3);
    }

    #[test]
    fn test_get_messages_empty() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();

        db.create_group("g1", "test-group", alice).unwrap();

        let msgs = db.get_messages("g1", 0, 100).unwrap();
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_get_messages_limit() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();

        db.create_group("g1", "test-group", alice).unwrap();

        for i in 1..=5 {
            db.store_message("g1", alice, format!("msg{i}").as_bytes())
                .unwrap();
        }

        let msgs = db.get_messages("g1", 0, 2).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].0, 1);
        assert_eq!(msgs[1].0, 2);
    }

    #[test]
    fn test_pending_welcomes_crud() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();

        db.create_group("g1", "test-group", alice).unwrap();
        db.store_pending_welcome("g1", "test-group", alice, b"welcome_data")
            .unwrap();

        let welcomes = db.get_pending_welcomes(alice).unwrap();
        assert_eq!(welcomes.len(), 1);
        assert_eq!(welcomes[0].1, "g1");
        assert_eq!(welcomes[0].2, "test-group");
        assert_eq!(welcomes[0].3, b"welcome_data");

        let welcome_id = welcomes[0].0;
        db.delete_pending_welcome(welcome_id, alice).unwrap();

        let welcomes = db.get_pending_welcomes(alice).unwrap();
        assert!(welcomes.is_empty());
    }

    #[test]
    fn test_pending_welcomes_empty() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();

        let welcomes = db.get_pending_welcomes(alice).unwrap();
        assert!(welcomes.is_empty());
    }

    #[test]
    fn test_store_group_info_insert_and_upsert() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();

        db.create_group("g1", "test-group", alice).unwrap();

        // Initial insert.
        db.store_group_info("g1", b"info_v1").unwrap();
        let info = db.get_group_info("g1").unwrap().unwrap();
        assert_eq!(info, b"info_v1");

        // Upsert (replace).
        db.store_group_info("g1", b"info_v2").unwrap();
        let info = db.get_group_info("g1").unwrap().unwrap();
        assert_eq!(info, b"info_v2");
    }

    #[test]
    fn test_get_group_info_nonexistent() {
        let db = Database::open_in_memory().unwrap();
        let result = db.get_group_info("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_delete_key_packages() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();

        db.store_key_package(uid, b"kp_data", false).unwrap();
        db.store_key_package(uid, b"last_resort", true).unwrap();
        db.delete_key_packages(uid).unwrap();

        let result = db.consume_key_package(uid).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_delete_key_packages_no_packages() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();

        // Deleting when none exist should not error.
        db.delete_key_packages(uid).unwrap();
    }

    #[test]
    fn test_get_user_id_by_username() {
        let db = Database::open_in_memory().unwrap();
        let id = db.create_user("alice", "hash").unwrap();

        let result = db.get_user_id_by_username("alice").unwrap().unwrap();
        assert_eq!(result, id);
    }

    #[test]
    fn test_get_user_id_by_username_nonexistent() {
        let db = Database::open_in_memory().unwrap();
        let result = db.get_user_id_by_username("nobody").unwrap();
        assert!(result.is_none());
    }
}
