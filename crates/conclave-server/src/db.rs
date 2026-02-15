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
    #[cfg(test)]
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

    pub fn store_key_package(&self, user_id: i64, data: &[u8]) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        // Replace any existing key package for this user (one active at a time).
        conn.execute(
            "DELETE FROM key_packages WHERE user_id = ?1",
            params![user_id],
        )?;
        conn.execute(
            "INSERT INTO key_packages (user_id, key_package_data) VALUES (?1, ?2)",
            params![user_id, data],
        )?;
        Ok(())
    }

    /// Consume (fetch and delete) a key package for the given user.
    pub fn consume_key_package(&self, user_id: i64) -> Result<Option<Vec<u8>>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn
            .prepare("SELECT id, key_package_data FROM key_packages WHERE user_id = ?1 LIMIT 1")?;
        let result: Option<(i64, Vec<u8>)> = stmt
            .query_row(params![user_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?;
        if let Some((id, data)) = result {
            conn.execute("DELETE FROM key_packages WHERE id = ?1", params![id])?;
            Ok(Some(data))
        } else {
            Ok(None)
        }
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

        db.store_key_package(uid, b"kp_data").unwrap();
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
}
