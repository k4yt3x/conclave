use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// A user record from the `users` table.
#[derive(Debug)]
pub struct UserRow {
    pub user_id: i64,
    pub username: String,
    pub password_hash: String,
    pub alias: Option<String>,
}

/// A pending welcome from the `pending_welcomes` table.
#[derive(Debug)]
pub struct PendingWelcomeRow {
    pub welcome_id: i64,
    pub group_id: i64,
    pub group_alias: Option<String>,
    pub welcome_data: Vec<u8>,
}

/// Result of `process_commit`: newly added members and optional message sequence number.
#[derive(Debug)]
pub struct CommitResult {
    pub new_members: Vec<NewMember>,
    pub sequence_number: Option<i64>,
}

/// A newly added group member from a commit.
#[derive(Debug)]
pub struct NewMember {
    pub user_id: i64,
    pub username: String,
}

/// A row from the `groups` table joined with membership info.
pub struct UserGroupRow {
    pub group_id: i64,
    pub group_name: String,
    pub alias: Option<String>,
    pub created_at: i64,
    pub mls_group_id: Option<String>,
}

/// A row from the `messages` table joined with sender info.
pub struct StoredMessageRow {
    pub sequence_num: i64,
    pub sender_id: i64,
    pub sender_username: String,
    pub sender_alias: Option<String>,
    pub mls_message: Vec<u8>,
    pub created_at: i64,
}

/// Maximum alias length for users and groups.
const MAX_ALIAS_LENGTH: usize = 64;

/// Maximum username length.
const MAX_USERNAME_LENGTH: usize = 64;

/// Maximum group name length.
const MAX_GROUP_NAME_LENGTH: usize = 64;

/// Hash a token with SHA-256 for safe storage.
fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    hex::encode(digest)
}

/// Check whether a username is valid.
pub fn validate_username(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::Validation("username is required".to_string()));
    }
    if name.len() > MAX_USERNAME_LENGTH {
        return Err(Error::Validation(format!(
            "username exceeds maximum length of {MAX_USERNAME_LENGTH} characters"
        )));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || !name.starts_with(|c: char| c.is_ascii_alphanumeric())
    {
        return Err(Error::Validation(
            "username must start with a letter or digit and contain only ASCII letters, digits, and underscores".to_string(),
        ));
    }
    Ok(())
}

/// Check whether a group name is valid (same rules as username).
pub fn validate_group_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::Validation("group name is required".to_string()));
    }
    if name.len() > MAX_GROUP_NAME_LENGTH {
        return Err(Error::Validation(format!(
            "group name exceeds maximum length of {MAX_GROUP_NAME_LENGTH} characters"
        )));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || !name.starts_with(|c: char| c.is_ascii_alphanumeric())
    {
        return Err(Error::Validation(
            "group name must start with a letter or digit and contain only ASCII letters, digits, and underscores".to_string(),
        ));
    }
    Ok(())
}

/// Check whether an alias string is valid (no ASCII control characters, max 64 chars).
pub fn validate_alias(alias: &str) -> Result<()> {
    if alias.len() > MAX_ALIAS_LENGTH {
        return Err(Error::Validation(format!(
            "alias exceeds maximum length of {MAX_ALIAS_LENGTH} characters"
        )));
    }
    if alias.bytes().any(|b| b < 0x20 || b == 0x7F) {
        return Err(Error::Validation(
            "alias must not contain ASCII control characters".to_string(),
        ));
    }
    Ok(())
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
                alias TEXT,
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
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_name TEXT UNIQUE NOT NULL,
                alias TEXT,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                mls_group_id TEXT
            );

            CREATE TABLE IF NOT EXISTS group_members (
                group_id INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                role TEXT NOT NULL DEFAULT 'member',
                PRIMARY KEY (group_id, user_id)
            );

            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                sender_id INTEGER NOT NULL REFERENCES users(id),
                mls_message BLOB NOT NULL,
                sequence_num INTEGER NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                UNIQUE(group_id, sequence_num)
            );

            CREATE TABLE IF NOT EXISTS pending_welcomes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                group_alias TEXT,
                user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                welcome_data BLOB NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE INDEX IF NOT EXISTS idx_pending_welcomes_user
                ON pending_welcomes(user_id);

            CREATE TABLE IF NOT EXISTS group_infos (
                group_id INTEGER PRIMARY KEY REFERENCES groups(id) ON DELETE CASCADE,
                group_info_data BLOB NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            ",
        )?;

        // Migration: add mls_group_id column to existing databases.
        if let Err(error) = conn.execute_batch("ALTER TABLE groups ADD COLUMN mls_group_id TEXT;") {
            let message = error.to_string();
            if !message.contains("duplicate column") {
                tracing::warn!(%error, "migration failed: add mls_group_id column");
            }
        }

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

    pub fn get_user_by_username(&self, username: &str) -> Result<Option<UserRow>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn
            .prepare("SELECT id, username, password_hash, alias FROM users WHERE username = ?1")?;
        let result = stmt
            .query_row(params![username], |row| {
                Ok(UserRow {
                    user_id: row.get(0)?,
                    username: row.get(1)?,
                    password_hash: row.get(2)?,
                    alias: row.get(3)?,
                })
            })
            .optional()?;
        Ok(result)
    }

    pub fn get_user_by_id(&self, user_id: i64) -> Result<Option<(i64, String, Option<String>)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare("SELECT id, username, alias FROM users WHERE id = ?1")?;
        let result = stmt
            .query_row(params![user_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
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

    pub fn update_user_alias(&self, user_id: i64, alias: Option<&str>) -> Result<()> {
        if let Some(alias) = alias {
            validate_alias(alias)?;
        }
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE users SET alias = ?1 WHERE id = ?2",
            params![alias, user_id],
        )?;
        Ok(())
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

    pub fn group_exists(&self, group_id: i64) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare("SELECT 1 FROM groups WHERE id = ?1")?;
        let exists: Option<i64> = stmt
            .query_row(params![group_id], |row| row.get(0))
            .optional()?;
        Ok(exists.is_some())
    }

    pub fn create_group(
        &self,
        group_name: &str,
        alias: Option<&str>,
        creator_id: i64,
    ) -> Result<i64> {
        validate_group_name(group_name)?;
        if let Some(alias) = alias {
            validate_alias(alias)?;
        }
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO groups (group_name, alias) VALUES (?1, ?2)",
            params![group_name, alias],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(err, _)
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Error::Conflict(format!("group_name '{group_name}' already exists"))
            }
            other => Error::Database(other),
        })?;
        let group_id = conn.last_insert_rowid();
        // Creator is automatically a member with admin role.
        conn.execute(
            "INSERT INTO group_members (group_id, user_id, role) VALUES (?1, ?2, 'admin')",
            params![group_id, creator_id],
        )?;
        Ok(group_id)
    }

    pub fn add_group_member(&self, group_id: i64, user_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT OR IGNORE INTO group_members (group_id, user_id) VALUES (?1, ?2)",
            params![group_id, user_id],
        )?;
        Ok(())
    }

    pub fn is_group_member(&self, group_id: i64, user_id: i64) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt =
            conn.prepare("SELECT 1 FROM group_members WHERE group_id = ?1 AND user_id = ?2")?;
        let exists: Option<i64> = stmt
            .query_row(params![group_id, user_id], |row| row.get(0))
            .optional()?;
        Ok(exists.is_some())
    }

    pub fn list_user_groups(&self, user_id: i64) -> Result<Vec<UserGroupRow>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT g.id, g.group_name, g.alias, g.created_at, g.mls_group_id
             FROM groups g
             JOIN group_members gm ON g.id = gm.group_id
             WHERE gm.user_id = ?1
             ORDER BY g.created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![user_id], |row| {
                Ok(UserGroupRow {
                    group_id: row.get(0)?,
                    group_name: row.get(1)?,
                    alias: row.get(2)?,
                    created_at: row.get(3)?,
                    mls_group_id: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn set_mls_group_id(&self, group_id: i64, mls_group_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE groups SET mls_group_id = ?2 WHERE id = ?1 AND mls_group_id IS NULL",
            params![group_id, mls_group_id],
        )?;
        Ok(())
    }

    pub fn remove_group_member(&self, group_id: i64, user_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "DELETE FROM group_members WHERE group_id = ?1 AND user_id = ?2",
            params![group_id, user_id],
        )?;
        Ok(())
    }

    /// Returns (user_id, username, alias, role) for each group member.
    #[allow(clippy::type_complexity)]
    pub fn get_group_members(
        &self,
        group_id: i64,
    ) -> Result<Vec<(i64, String, Option<String>, String)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT u.id, u.username, u.alias, gm.role
             FROM users u
             JOIN group_members gm ON u.id = gm.user_id
             WHERE gm.group_id = ?1",
        )?;
        let rows = stmt
            .query_map(params![group_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Get the alias for a group.
    pub fn get_group_alias(&self, group_id: i64) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare("SELECT alias FROM groups WHERE id = ?1")?;
        let result: Option<Option<String>> = stmt
            .query_row(params![group_id], |row| row.get(0))
            .optional()?;
        Ok(result.flatten())
    }

    pub fn update_group_alias(&self, group_id: i64, alias: Option<&str>) -> Result<()> {
        if let Some(alias) = alias {
            validate_alias(alias)?;
        }
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE groups SET alias = ?1 WHERE id = ?2",
            params![alias, group_id],
        )?;
        Ok(())
    }

    pub fn update_group_name(&self, group_id: i64, group_name: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE groups SET group_name = ?1 WHERE id = ?2",
            params![group_name, group_id],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(err, _)
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Error::Conflict(format!(
                    "group_name '{}' already exists",
                    group_name.unwrap_or_default()
                ))
            }
            other => Error::Database(other),
        })?;
        Ok(())
    }

    /// Check whether a user is an admin of a group.
    pub fn is_group_admin(&self, group_id: i64, user_id: i64) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT 1 FROM group_members WHERE group_id = ?1 AND user_id = ?2 AND role = 'admin'",
        )?;
        let exists: Option<i64> = stmt
            .query_row(params![group_id, user_id], |row| row.get(0))
            .optional()?;
        Ok(exists.is_some())
    }

    /// Promote a member to admin.
    pub fn promote_member(&self, group_id: i64, user_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE group_members SET role = 'admin' WHERE group_id = ?1 AND user_id = ?2",
            params![group_id, user_id],
        )?;
        Ok(())
    }

    /// Demote an admin to regular member.
    pub fn demote_member(&self, group_id: i64, user_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE group_members SET role = 'member' WHERE group_id = ?1 AND user_id = ?2",
            params![group_id, user_id],
        )?;
        Ok(())
    }

    /// Count the number of admins in a group.
    pub fn count_group_admins(&self, group_id: i64) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM group_members WHERE group_id = ?1 AND role = 'admin'",
            params![group_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// List admin members of a group: (user_id, username, alias).
    pub fn get_group_admins(&self, group_id: i64) -> Result<Vec<(i64, String, Option<String>)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT u.id, u.username, u.alias
             FROM users u
             JOIN group_members gm ON u.id = gm.user_id
             WHERE gm.group_id = ?1 AND gm.role = 'admin'",
        )?;
        let rows = stmt
            .query_map(params![group_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ── Messages ───────────────────────────────────────────────────

    pub fn store_message(&self, group_id: i64, sender_id: i64, mls_message: &[u8]) -> Result<i64> {
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
        group_id: i64,
        after_seq: i64,
        limit: i64,
    ) -> Result<Vec<StoredMessageRow>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT m.sequence_num, m.sender_id, u.username, u.alias, m.mls_message, m.created_at
             FROM messages m
             JOIN users u ON m.sender_id = u.id
             WHERE m.group_id = ?1 AND m.sequence_num > ?2
             ORDER BY m.sequence_num ASC
             LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(params![group_id, after_seq, limit], |row| {
                Ok(StoredMessageRow {
                    sequence_num: row.get(0)?,
                    sender_id: row.get(1)?,
                    sender_username: row.get(2)?,
                    sender_alias: row.get(3)?,
                    mls_message: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ── Pending Welcomes ───────────────────────────────────────────

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

    // ── Group Info (for External Commits) ──────────────────────────

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

    /// Process a commit upload atomically: add new members, store welcomes,
    /// store group info, and store the commit message. Returns a list of
    /// (user_id, username) pairs for newly added members and the stored
    /// message sequence number (if any), for SSE notification after commit.
    pub fn process_commit(
        &self,
        group_id: i64,
        group_alias: Option<&str>,
        sender_id: i64,
        welcome_messages: &std::collections::HashMap<String, Vec<u8>>,
        group_info: &[u8],
        commit_message: &[u8],
    ) -> Result<CommitResult> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let transaction = conn.savepoint()?;

        let mut new_members = Vec::new();

        for (username, welcome_data) in welcome_messages {
            let user_id: Option<i64> = transaction
                .prepare("SELECT id FROM users WHERE username = ?1")?
                .query_row(params![username], |row| row.get(0))
                .optional()?;
            let user_id =
                user_id.ok_or_else(|| Error::NotFound(format!("user '{username}' not found")))?;

            transaction.execute(
                "INSERT INTO group_members (group_id, user_id) VALUES (?1, ?2)",
                params![group_id, user_id],
            )?;

            transaction.execute(
                "INSERT INTO pending_welcomes (group_id, group_alias, user_id, welcome_data, created_at)
                 VALUES (?1, ?2, ?3, ?4, unixepoch())",
                params![group_id, group_alias, user_id, welcome_data],
            )?;

            new_members.push(NewMember {
                user_id,
                username: username.clone(),
            });
        }

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
        Ok(CommitResult {
            new_members,
            sequence_number,
        })
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
        assert_eq!(user.user_id, id);
        assert_eq!(user.username, "alice");
        assert_eq!(user.password_hash, "hash123");
        assert!(user.alias.is_none());

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

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();
        db.add_group_member(group_id, bob).unwrap();

        assert!(db.is_group_member(group_id, alice).unwrap());
        assert!(db.is_group_member(group_id, bob).unwrap());

        let groups = db.list_user_groups(alice).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].alias, Some("test_group".to_string()));

        let members = db.get_group_members(group_id).unwrap();
        assert_eq!(members.len(), 2);

        let seq1 = db.store_message(group_id, alice, b"msg1").unwrap();
        let seq2 = db.store_message(group_id, bob, b"msg2").unwrap();
        assert_eq!(seq1, 1);
        assert_eq!(seq2, 2);

        let msgs = db.get_messages(group_id, 0, 100).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].mls_message, b"msg1");
        assert_eq!(msgs[1].mls_message, b"msg2");

        // Fetch after seq 1.
        let msgs = db.get_messages(group_id, 1, 100).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].mls_message, b"msg2");
    }

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
        assert!(user.2.is_none());
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
        db.create_session("real_token", uid, i64::MAX).unwrap();
        let result = db.validate_session("wrong_token").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_session_expired() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();
        let token = "expired_token";
        db.create_session(token, uid, 0).unwrap();
        let result = db.validate_session(token).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_session_delete() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();
        let token = "delete_me";
        db.create_session(token, uid, i64::MAX).unwrap();

        assert!(db.validate_session(token).unwrap().is_some());

        db.delete_session(token).unwrap();

        assert!(db.validate_session(token).unwrap().is_none());
    }

    #[test]
    fn test_cleanup_expired_sessions() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();

        db.create_session("expired_token", uid, 0).unwrap();
        db.create_session("valid_token", uid, i64::MAX).unwrap();

        let cleaned = db.cleanup_expired_sessions().unwrap();
        assert_eq!(cleaned, 1);

        assert_eq!(db.validate_session("valid_token").unwrap(), Some(uid));
        assert!(db.validate_session("expired_token").unwrap().is_none());
    }

    #[test]
    fn test_key_package_accumulate() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();

        db.store_key_package(uid, b"kp1", false).unwrap();
        db.store_key_package(uid, b"kp2", false).unwrap();

        let kp = db.consume_key_package(uid).unwrap().unwrap();
        assert_eq!(kp, b"kp1");

        let kp = db.consume_key_package(uid).unwrap().unwrap();
        assert_eq!(kp, b"kp2");

        assert!(db.consume_key_package(uid).unwrap().is_none());
    }

    #[test]
    fn test_last_resort_key_package() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();

        db.store_key_package(uid, b"last_resort", true).unwrap();
        db.store_key_package(uid, b"regular1", false).unwrap();

        let kp = db.consume_key_package(uid).unwrap().unwrap();
        assert_eq!(kp, b"regular1");

        let kp = db.consume_key_package(uid).unwrap().unwrap();
        assert_eq!(kp, b"last_resort");

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

        db.store_key_package(uid, b"kp_overflow", false).unwrap();
        let (regular, _) = db.count_key_packages(uid).unwrap();
        assert_eq!(regular, Database::MAX_KEY_PACKAGES_PER_USER);

        db.store_key_package(uid, b"last_resort", true).unwrap();
    }

    #[test]
    fn test_last_resort_replacement() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();

        db.store_key_package(uid, b"lr1", true).unwrap();
        db.store_key_package(uid, b"lr2", true).unwrap();

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

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();

        assert!(db.is_group_member(group_id, alice).unwrap());
        assert!(!db.is_group_member(group_id, bob).unwrap());
    }

    #[test]
    fn test_add_group_member_idempotent() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();
        db.add_group_member(group_id, bob).unwrap();
        db.add_group_member(group_id, bob).unwrap();

        let members = db.get_group_members(group_id).unwrap();
        assert_eq!(members.len(), 2);
    }

    #[test]
    fn test_remove_group_member() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();
        db.add_group_member(group_id, bob).unwrap();
        assert!(db.is_group_member(group_id, bob).unwrap());

        db.remove_group_member(group_id, bob).unwrap();
        assert!(!db.is_group_member(group_id, bob).unwrap());
    }

    #[test]
    fn test_remove_group_member_nonexistent() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();

        db.remove_group_member(group_id, bob).unwrap();
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

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();
        db.add_group_member(group_id, bob).unwrap();

        let members = db.get_group_members(group_id).unwrap();
        assert_eq!(members.len(), 2);

        db.remove_group_member(group_id, bob).unwrap();

        let members = db.get_group_members(group_id).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].0, alice);
        assert_eq!(members[0].1, "alice");
    }

    #[test]
    fn test_messages_sequence_numbers() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();

        let seq1 = db.store_message(group_id, alice, b"msg1").unwrap();
        let seq2 = db.store_message(group_id, alice, b"msg2").unwrap();
        let seq3 = db.store_message(group_id, alice, b"msg3").unwrap();

        assert_eq!(seq1, 1);
        assert_eq!(seq2, 2);
        assert_eq!(seq3, 3);

        let msgs = db.get_messages(group_id, 0, 100).unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].sequence_num, 1);
        assert_eq!(msgs[1].sequence_num, 2);
        assert_eq!(msgs[2].sequence_num, 3);
    }

    #[test]
    fn test_get_messages_empty() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();

        let msgs = db.get_messages(group_id, 0, 100).unwrap();
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_get_messages_limit() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();

        for i in 1..=5 {
            db.store_message(group_id, alice, format!("msg{i}").as_bytes())
                .unwrap();
        }

        let msgs = db.get_messages(group_id, 0, 2).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].sequence_num, 1);
        assert_eq!(msgs[1].sequence_num, 2);
    }

    #[test]
    fn test_pending_welcomes_crud() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();
        db.store_pending_welcome(group_id, Some("test_group"), alice, b"welcome_data")
            .unwrap();

        let welcomes = db.get_pending_welcomes(alice).unwrap();
        assert_eq!(welcomes.len(), 1);
        assert_eq!(welcomes[0].group_id, group_id);
        assert_eq!(welcomes[0].group_alias, Some("test_group".to_string()));
        assert_eq!(welcomes[0].welcome_data, b"welcome_data");

        let welcome_id = welcomes[0].welcome_id;
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

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();

        db.store_group_info(group_id, b"info_v1").unwrap();
        let info = db.get_group_info(group_id).unwrap().unwrap();
        assert_eq!(info, b"info_v1");

        db.store_group_info(group_id, b"info_v2").unwrap();
        let info = db.get_group_info(group_id).unwrap().unwrap();
        assert_eq!(info, b"info_v2");
    }

    #[test]
    fn test_get_group_info_nonexistent() {
        let db = Database::open_in_memory().unwrap();
        let result = db.get_group_info(9999).unwrap();
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

    #[test]
    fn test_process_commit_with_multiple_welcomes() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();
        let charlie = db.create_user("charlie", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();

        let mut welcomes = std::collections::HashMap::new();
        welcomes.insert("bob".to_string(), b"welcome_bob".to_vec());
        welcomes.insert("charlie".to_string(), b"welcome_charlie".to_vec());

        let result = db
            .process_commit(
                group_id,
                Some("test_group"),
                alice,
                &welcomes,
                b"group_info",
                b"commit_msg",
            )
            .unwrap();

        assert_eq!(result.new_members.len(), 2);
        assert!(db.is_group_member(group_id, bob).unwrap());
        assert!(db.is_group_member(group_id, charlie).unwrap());

        let bob_welcomes = db.get_pending_welcomes(bob).unwrap();
        assert_eq!(bob_welcomes.len(), 1);
        let charlie_welcomes = db.get_pending_welcomes(charlie).unwrap();
        assert_eq!(charlie_welcomes.len(), 1);

        assert_eq!(result.sequence_number, Some(1));
        let msgs = db.get_messages(group_id, 0, 100).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].mls_message, b"commit_msg");
    }

    #[test]
    fn test_process_commit_empty_commit_message() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        db.create_user("bob", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();

        let mut welcomes = std::collections::HashMap::new();
        welcomes.insert("bob".to_string(), b"welcome_bob".to_vec());

        let result = db
            .process_commit(
                group_id,
                Some("test_group"),
                alice,
                &welcomes,
                b"group_info",
                b"",
            )
            .unwrap();

        assert_eq!(result.new_members.len(), 1);
        assert_eq!(result.sequence_number, None);
        let msgs = db.get_messages(group_id, 0, 100).unwrap();
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_process_commit_empty_group_info() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();

        let welcomes = std::collections::HashMap::new();

        db.process_commit(
            group_id,
            Some("test_group"),
            alice,
            &welcomes,
            b"",
            b"commit_msg",
        )
        .unwrap();

        let info = db.get_group_info(group_id).unwrap();
        assert!(info.is_none());
    }

    #[test]
    fn test_process_commit_nonexistent_user() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();

        let mut welcomes = std::collections::HashMap::new();
        welcomes.insert("nonexistent".to_string(), b"welcome_data".to_vec());

        let result = db.process_commit(
            group_id,
            Some("test_group"),
            alice,
            &welcomes,
            b"group_info",
            b"commit_msg",
        );

        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("not found"),
            "Expected error to contain 'not found', got: {err_msg}"
        );
    }

    #[test]
    fn test_messages_isolated_between_groups() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();

        let g1 = db
            .create_group("group_one", Some("group_one"), alice)
            .unwrap();
        let g2 = db
            .create_group("group_two", Some("group_two"), alice)
            .unwrap();

        db.store_message(g1, alice, b"msg_g1").unwrap();
        db.store_message(g2, alice, b"msg_g2").unwrap();

        let msgs_g1 = db.get_messages(g1, 0, 100).unwrap();
        assert_eq!(msgs_g1.len(), 1);
        assert_eq!(msgs_g1[0].mls_message, b"msg_g1");

        let msgs_g2 = db.get_messages(g2, 0, 100).unwrap();
        assert_eq!(msgs_g2.len(), 1);
        assert_eq!(msgs_g2[0].mls_message, b"msg_g2");
    }

    #[test]
    fn test_group_exists() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();

        assert!(db.group_exists(group_id).unwrap());
        assert!(!db.group_exists(9999).unwrap());
    }

    #[test]
    fn test_multiple_pending_welcomes_for_same_user() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();

        db.store_pending_welcome(group_id, Some("test_group"), alice, b"welcome_1")
            .unwrap();
        db.store_pending_welcome(group_id, Some("test_group"), alice, b"welcome_2")
            .unwrap();

        let welcomes = db.get_pending_welcomes(alice).unwrap();
        assert_eq!(welcomes.len(), 2);
    }

    #[test]
    fn test_delete_pending_welcome_wrong_user() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();
        db.add_group_member(group_id, bob).unwrap();

        db.store_pending_welcome(group_id, Some("test_group"), alice, b"welcome_data")
            .unwrap();

        let welcomes = db.get_pending_welcomes(alice).unwrap();
        assert_eq!(welcomes.len(), 1);
        let welcome_id = welcomes[0].welcome_id;

        db.delete_pending_welcome(welcome_id, bob).unwrap();

        let welcomes = db.get_pending_welcomes(alice).unwrap();
        assert_eq!(welcomes.len(), 1);
    }

    #[test]
    fn test_count_key_packages() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();

        let (regular, last_resort) = db.count_key_packages(uid).unwrap();
        assert_eq!(regular, 0);
        assert_eq!(last_resort, 0);

        db.store_key_package(uid, b"kp1", false).unwrap();
        db.store_key_package(uid, b"kp2", false).unwrap();
        db.store_key_package(uid, b"kp3", false).unwrap();
        db.store_key_package(uid, b"lr1", true).unwrap();

        let (regular, last_resort) = db.count_key_packages(uid).unwrap();
        assert_eq!(regular, 3);
        assert_eq!(last_resort, 1);
    }

    #[test]
    fn test_session_token_hashed() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();
        let raw_token = "my-secret-token";

        db.create_session(raw_token, uid, i64::MAX).unwrap();

        let conn = db.conn.lock().unwrap_or_else(|e| e.into_inner());
        let found: Option<i64> = conn
            .prepare("SELECT user_id FROM sessions WHERE token = ?1")
            .unwrap()
            .query_row(params![raw_token], |row| row.get(0))
            .optional()
            .unwrap();
        assert!(
            found.is_none(),
            "raw token should not match the stored hashed token"
        );

        let token_hash = hash_token(raw_token);
        let found: Option<i64> = conn
            .prepare("SELECT user_id FROM sessions WHERE token = ?1")
            .unwrap()
            .query_row(params![token_hash], |row| row.get(0))
            .optional()
            .unwrap();
        assert_eq!(found, Some(uid));
    }

    #[test]
    fn test_user_alias() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();

        // Initially no alias.
        let user = db.get_user_by_id(uid).unwrap().unwrap();
        assert!(user.2.is_none());

        // Set alias.
        db.update_user_alias(uid, Some("Alice Wonderland")).unwrap();
        let user = db.get_user_by_id(uid).unwrap().unwrap();
        assert_eq!(user.2, Some("Alice Wonderland".to_string()));

        // Clear alias.
        db.update_user_alias(uid, None).unwrap();
        let user = db.get_user_by_id(uid).unwrap().unwrap();
        assert!(user.2.is_none());
    }

    #[test]
    fn test_alias_validation() {
        assert!(validate_alias("hello").is_ok());
        assert!(validate_alias("").is_ok());
        assert!(validate_alias(&"a".repeat(64)).is_ok());
        assert!(validate_alias(&"a".repeat(65)).is_err());
        assert!(validate_alias("hello\x00world").is_err());
        assert!(validate_alias("hello\x1Fworld").is_err());
        assert!(validate_alias("hello\x7Fworld").is_err());
    }

    #[test]
    fn test_group_name_uniqueness() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();

        db.create_group("my_group", None, alice).unwrap();
        let result = db.create_group("my_group", None, alice);
        assert!(result.is_err());
    }

    #[test]
    fn test_group_alias_and_name() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();

        let group_id = db
            .create_group("dev_chat", Some("Dev Chat Room"), alice)
            .unwrap();

        let groups = db.list_user_groups(alice).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].group_id, group_id);
        assert_eq!(groups[0].group_name, "dev_chat");
        assert_eq!(groups[0].alias, Some("Dev Chat Room".to_string()));

        // Update alias.
        db.update_group_alias(group_id, Some("New Dev Chat"))
            .unwrap();
        let alias = db.get_group_alias(group_id).unwrap();
        assert_eq!(alias, Some("New Dev Chat".to_string()));
    }

    #[test]
    fn test_alias_validation_control_char_at_start() {
        assert!(validate_alias("\x01hello").is_err());
    }

    #[test]
    fn test_alias_validation_control_char_at_end() {
        assert!(validate_alias("hello\x1F").is_err());
    }

    #[test]
    fn test_alias_validation_tab_char() {
        assert!(validate_alias("hello\tworld").is_err());
    }

    #[test]
    fn test_alias_validation_newline() {
        assert!(validate_alias("hello\nworld").is_err());
    }

    #[test]
    fn test_alias_validation_unicode_allowed() {
        assert!(validate_alias("日本語テスト").is_ok());
    }

    #[test]
    fn test_update_user_alias_to_none() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();

        db.update_user_alias(uid, Some("Alice")).unwrap();
        let user = db.get_user_by_id(uid).unwrap().unwrap();
        assert_eq!(user.2, Some("Alice".to_string()));

        db.update_user_alias(uid, None).unwrap();
        let user = db.get_user_by_id(uid).unwrap().unwrap();
        assert!(user.2.is_none());
    }

    #[test]
    fn test_update_group_alias_to_none() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();

        let group_id = db
            .create_group("dev_chat", Some("Dev Chat"), alice)
            .unwrap();

        let alias = db.get_group_alias(group_id).unwrap();
        assert_eq!(alias, Some("Dev Chat".to_string()));

        db.update_group_alias(group_id, None).unwrap();
        let alias = db.get_group_alias(group_id).unwrap();
        assert!(alias.is_none());
    }

    #[test]
    fn test_alias_validation_on_update_rejects_invalid() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();

        let result = db.update_user_alias(uid, Some("bad\x01alias"));
        assert!(result.is_err());
    }

    #[test]
    fn test_create_group_creator_is_admin() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();

        assert!(db.is_group_admin(group_id, alice).unwrap());

        let members = db.get_group_members(group_id).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].3, "admin");
    }

    #[test]
    fn test_add_member_default_role() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();
        db.add_group_member(group_id, bob).unwrap();

        assert!(!db.is_group_admin(group_id, bob).unwrap());

        let members = db.get_group_members(group_id).unwrap();
        let bob_member = members.iter().find(|m| m.0 == bob).unwrap();
        assert_eq!(bob_member.3, "member");
    }

    #[test]
    fn test_promote_member() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();
        db.add_group_member(group_id, bob).unwrap();

        assert!(!db.is_group_admin(group_id, bob).unwrap());
        db.promote_member(group_id, bob).unwrap();
        assert!(db.is_group_admin(group_id, bob).unwrap());

        let members = db.get_group_members(group_id).unwrap();
        let bob_member = members.iter().find(|m| m.0 == bob).unwrap();
        assert_eq!(bob_member.3, "admin");
    }

    #[test]
    fn test_demote_member() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();
        db.add_group_member(group_id, bob).unwrap();
        db.promote_member(group_id, bob).unwrap();
        assert!(db.is_group_admin(group_id, bob).unwrap());

        db.demote_member(group_id, bob).unwrap();
        assert!(!db.is_group_admin(group_id, bob).unwrap());

        let members = db.get_group_members(group_id).unwrap();
        let bob_member = members.iter().find(|m| m.0 == bob).unwrap();
        assert_eq!(bob_member.3, "member");
    }

    #[test]
    fn test_is_group_admin() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();
        db.add_group_member(group_id, bob).unwrap();

        assert!(db.is_group_admin(group_id, alice).unwrap());
        assert!(!db.is_group_admin(group_id, bob).unwrap());

        // Non-member should return false.
        let charlie = db.create_user("charlie", "hash").unwrap();
        assert!(!db.is_group_admin(group_id, charlie).unwrap());
    }

    #[test]
    fn test_count_group_admins() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();

        assert_eq!(db.count_group_admins(group_id).unwrap(), 1);

        db.add_group_member(group_id, bob).unwrap();
        assert_eq!(db.count_group_admins(group_id).unwrap(), 1);

        db.promote_member(group_id, bob).unwrap();
        assert_eq!(db.count_group_admins(group_id).unwrap(), 2);

        db.demote_member(group_id, bob).unwrap();
        assert_eq!(db.count_group_admins(group_id).unwrap(), 1);
    }

    #[test]
    fn test_get_group_admins() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();
        db.add_group_member(group_id, bob).unwrap();

        let admins = db.get_group_admins(group_id).unwrap();
        assert_eq!(admins.len(), 1);
        assert_eq!(admins[0].0, alice);
        assert_eq!(admins[0].1, "alice");

        db.promote_member(group_id, bob).unwrap();
        let admins = db.get_group_admins(group_id).unwrap();
        assert_eq!(admins.len(), 2);
    }
}
