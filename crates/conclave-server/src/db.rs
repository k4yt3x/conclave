mod groups;
mod invites;
mod key_packages;
mod messages;
mod sessions;
mod users;
mod welcomes;

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::Result;

/// A user record from the `users` table.
#[derive(Debug)]
pub struct UserRow {
    pub user_id: Uuid,
    pub username: String,
    pub password_hash: String,
    pub alias: Option<String>,
    pub signing_key_fingerprint: Option<String>,
}

/// A user info record without the password hash, used for lookups and member lists.
#[derive(Debug)]
pub struct UserInfo {
    pub user_id: Uuid,
    pub username: String,
    pub alias: Option<String>,
    pub signing_key_fingerprint: Option<String>,
}

/// A pending welcome from the `pending_welcomes` table.
#[derive(Debug)]
pub struct PendingWelcomeRow {
    pub welcome_id: Uuid,
    pub group_id: Uuid,
    pub group_alias: Option<String>,
    pub welcome_data: Vec<u8>,
}

/// Result of `process_commit`: optional message sequence number.
#[derive(Debug)]
pub struct CommitResult {
    pub sequence_number: Option<i64>,
}

/// A pending invite held in escrow until the invitee accepts or declines.
#[derive(Debug)]
pub struct PendingInviteRow {
    pub invite_id: Uuid,
    pub group_id: Uuid,
    pub inviter_id: Uuid,
    pub invitee_id: Uuid,
    pub commit_message: Vec<u8>,
    pub welcome_data: Vec<u8>,
    pub group_info: Vec<u8>,
    pub created_at: i64,
}

/// Result of accepting a pending invite.
#[derive(Debug)]
pub struct AcceptedInvite {
    pub group_id: Uuid,
    pub inviter_id: Uuid,
    pub invitee_id: Uuid,
    pub invitee_username: String,
    pub group_alias: Option<String>,
    pub sequence_number: i64,
}

/// A row from the `groups` table joined with membership info.
pub struct UserGroupRow {
    pub group_id: Uuid,
    pub group_name: String,
    pub alias: Option<String>,
    pub mls_group_id: Option<String>,
    pub message_expiry_seconds: i64,
    pub visibility: i32,
}

/// A public group row for the discovery listing.
pub struct PublicGroupRow {
    pub group_id: Uuid,
    pub group_name: String,
    pub alias: Option<String>,
    pub member_count: u32,
}

/// A group member row from the `group_members` table joined with user info.
pub struct GroupMemberRow {
    pub user_id: Uuid,
    pub username: String,
    pub alias: Option<String>,
    pub role: String,
    pub signing_key_fingerprint: Option<String>,
}

/// A row from the `messages` table.
pub struct StoredMessageRow {
    pub sequence_num: i64,
    pub sender_id: Uuid,
    pub mls_message: Vec<u8>,
    pub created_at: i64,
}

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
    fn lock_conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|error| {
            tracing::warn!("database mutex was poisoned, recovering");
            error.into_inner()
        })
    }

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
        let conn = self.lock_conn();
        conn.execute_batch("PRAGMA journal_mode = WAL;")?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                username TEXT UNIQUE NOT NULL,
                password_hash TEXT NOT NULL,
                alias TEXT
            );

            CREATE TABLE IF NOT EXISTS sessions (
                token TEXT PRIMARY KEY,
                user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                expires_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS key_packages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                key_package_data BLOB NOT NULL,
                is_last_resort INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE TABLE IF NOT EXISTS groups (
                id TEXT PRIMARY KEY,
                group_name TEXT UNIQUE NOT NULL,
                alias TEXT,
                mls_group_id TEXT,
                message_expiry_seconds INTEGER NOT NULL DEFAULT -1,
                visibility INTEGER NOT NULL DEFAULT 1
            );

            CREATE TABLE IF NOT EXISTS group_members (
                group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                role TEXT NOT NULL DEFAULT 'member',
                PRIMARY KEY (group_id, user_id)
            );

            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                sender_id TEXT NOT NULL REFERENCES users(id),
                mls_message BLOB NOT NULL,
                sequence_num INTEGER NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                UNIQUE(group_id, sequence_num)
            );

            CREATE TABLE IF NOT EXISTS pending_welcomes (
                id TEXT PRIMARY KEY,
                group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                group_alias TEXT,
                user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
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

            CREATE TABLE IF NOT EXISTS pending_invites (
                id TEXT PRIMARY KEY,
                group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                inviter_id TEXT NOT NULL REFERENCES users(id),
                invitee_id TEXT NOT NULL REFERENCES users(id),
                commit_message BLOB NOT NULL,
                welcome_data BLOB NOT NULL,
                group_info BLOB NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                UNIQUE(group_id, invitee_id)
            );
            CREATE INDEX IF NOT EXISTS idx_pending_invites_invitee
                ON pending_invites(invitee_id);

            CREATE TABLE IF NOT EXISTS message_fetch_watermarks (
                group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                last_fetched_seq INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (group_id, user_id)
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

        // Migration: add message_expiry_seconds column to existing databases.
        if let Err(error) = conn.execute_batch(
            "ALTER TABLE groups ADD COLUMN message_expiry_seconds INTEGER NOT NULL DEFAULT -1;",
        ) {
            let message = error.to_string();
            if !message.contains("duplicate column") {
                tracing::warn!(%error, "migration failed: add message_expiry_seconds column");
            }
        }

        // Migration: add signing_key_fingerprint column to existing databases.
        if let Err(error) =
            conn.execute_batch("ALTER TABLE users ADD COLUMN signing_key_fingerprint TEXT;")
        {
            let message = error.to_string();
            if !message.contains("duplicate column") {
                tracing::warn!(%error, "migration failed: add signing_key_fingerprint column");
            }
        }

        // Migration: add visibility column to existing databases.
        if let Err(error) = conn
            .execute_batch("ALTER TABLE groups ADD COLUMN visibility INTEGER NOT NULL DEFAULT 1;")
        {
            let message = error.to_string();
            if !message.contains("duplicate column") {
                tracing::warn!(%error, "migration failed: add visibility column");
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::{OptionalExtension, params};
    use uuid::Uuid;

    use super::*;
    use crate::validation::validate_alias;

    // Users

    #[test]
    fn test_user_crud() {
        let db = Database::open_in_memory().unwrap();

        let id = db.create_user("alice", "hash123").unwrap();

        let user = db.get_user_by_username("alice").unwrap().unwrap();
        assert_eq!(user.user_id, id);
        assert_eq!(user.username, "alice");
        assert_eq!(user.password_hash, "hash123");
        assert!(user.alias.is_none());

        // Duplicate username should fail.
        let result = db.create_user("alice", "hash456");
        assert!(result.is_err());
    }

    // Key packages

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

    // Groups and messages

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
        assert_eq!(user.user_id, id);
        assert_eq!(user.username, "alice");
        assert!(user.alias.is_none());
    }

    #[test]
    fn test_get_user_by_id_nonexistent() {
        let db = Database::open_in_memory().unwrap();
        let result = db.get_user_by_id(Uuid::new_v4()).unwrap();
        assert!(result.is_none());
    }

    // Sessions

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

    // Key packages (continued)

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
        let result = db.consume_key_package(Uuid::new_v4()).unwrap();
        assert!(result.is_none());
    }

    // Groups and membership

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
        assert_eq!(members[0].user_id, alice);
        assert_eq!(members[0].username, "alice");
    }

    // Messages

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

    // Welcomes

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

    // Group info

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
        let result = db.get_group_info(Uuid::new_v4()).unwrap();
        assert!(result.is_none());
    }

    // Key packages (cleanup)

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

    // Commits

    #[test]
    fn test_process_commit_empty_commit_message() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();

        let result = db
            .process_commit(group_id, alice, b"group_info", b"")
            .unwrap();

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

        db.process_commit(group_id, alice, b"", b"commit_msg")
            .unwrap();

        let info = db.get_group_info(group_id).unwrap();
        assert!(info.is_none());
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
        assert!(!db.group_exists(Uuid::new_v4()).unwrap());
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
        let found: Option<String> = conn
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
        let found: Option<String> = conn
            .prepare("SELECT user_id FROM sessions WHERE token = ?1")
            .unwrap()
            .query_row(params![token_hash], |row| row.get(0))
            .optional()
            .unwrap();
        assert_eq!(found, Some(uid.to_string()));
    }

    // Aliases

    #[test]
    fn test_user_alias() {
        let db = Database::open_in_memory().unwrap();
        let uid = db.create_user("alice", "hash").unwrap();

        // Initially no alias.
        let user = db.get_user_by_id(uid).unwrap().unwrap();
        assert!(user.alias.is_none());

        // Set alias.
        db.update_user_alias(uid, Some("Alice Wonderland")).unwrap();
        let user = db.get_user_by_id(uid).unwrap().unwrap();
        assert_eq!(user.alias, Some("Alice Wonderland".to_string()));

        // Clear alias.
        db.update_user_alias(uid, None).unwrap();
        let user = db.get_user_by_id(uid).unwrap().unwrap();
        assert!(user.alias.is_none());
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
        assert_eq!(user.alias, Some("Alice".to_string()));

        db.update_user_alias(uid, None).unwrap();
        let user = db.get_user_by_id(uid).unwrap().unwrap();
        assert!(user.alias.is_none());
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

    // Admin roles

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
        assert_eq!(members[0].role, "admin");
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
        let bob_member = members.iter().find(|m| m.user_id == bob).unwrap();
        assert_eq!(bob_member.role, "member");
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
        let bob_member = members.iter().find(|m| m.user_id == bob).unwrap();
        assert_eq!(bob_member.role, "admin");
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
        let bob_member = members.iter().find(|m| m.user_id == bob).unwrap();
        assert_eq!(bob_member.role, "member");
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
        assert_eq!(admins[0].user_id, alice);
        assert_eq!(admins[0].username, "alice");

        db.promote_member(group_id, bob).unwrap();
        let admins = db.get_group_admins(group_id).unwrap();
        assert_eq!(admins.len(), 2);
    }

    // Invites

    #[test]
    fn test_create_and_get_pending_invite() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("test_group"), alice)
            .unwrap();

        let invite_id = db
            .create_pending_invite(group_id, alice, bob, b"commit", b"welcome", b"ginfo")
            .unwrap();

        let invite = db.get_pending_invite(invite_id).unwrap().unwrap();
        assert_eq!(invite.group_id, group_id);
        assert_eq!(invite.inviter_id, alice);
        assert_eq!(invite.invitee_id, bob);
        assert_eq!(invite.commit_message, b"commit");
        assert_eq!(invite.welcome_data, b"welcome");
        assert_eq!(invite.group_info, b"ginfo");
    }

    #[test]
    fn test_list_pending_invites_for_user() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();

        let group1 = db.create_group("group1", None, alice).unwrap();
        let group2 = db.create_group("group2", None, alice).unwrap();

        db.create_pending_invite(group1, alice, bob, b"c1", b"w1", b"g1")
            .unwrap();
        db.create_pending_invite(group2, alice, bob, b"c2", b"w2", b"g2")
            .unwrap();

        let invites = db.list_pending_invites_for_user(bob).unwrap();
        assert_eq!(invites.len(), 2);

        // Alice should have no pending invites.
        let invites = db.list_pending_invites_for_user(alice).unwrap();
        assert!(invites.is_empty());
    }

    #[test]
    fn test_pending_invite_unique_constraint() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();

        let group_id = db.create_group("test_group", None, alice).unwrap();

        db.create_pending_invite(group_id, alice, bob, b"c1", b"w1", b"g1")
            .unwrap();

        // Duplicate invite for same group+invitee should fail.
        let result = db.create_pending_invite(group_id, alice, bob, b"c2", b"w2", b"g2");
        assert!(result.is_err());
    }

    #[test]
    fn test_accept_pending_invite() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();

        let group_id = db
            .create_group("test_group", Some("Test Group"), alice)
            .unwrap();

        let invite_id = db
            .create_pending_invite(
                group_id,
                alice,
                bob,
                b"commit_data",
                b"welcome_data",
                b"ginfo",
            )
            .unwrap();

        let result = db.accept_pending_invite(invite_id).unwrap();
        assert_eq!(result.group_id, group_id);
        assert_eq!(result.inviter_id, alice);
        assert_eq!(result.invitee_id, bob);
        assert_eq!(result.invitee_username, "bob");
        assert_eq!(result.group_alias, Some("Test Group".to_string()));
        assert_eq!(result.sequence_number, 1);

        // Bob should now be a group member.
        assert!(db.is_group_member(group_id, bob).unwrap());

        // The pending invite should be gone.
        assert!(db.get_pending_invite(invite_id).unwrap().is_none());

        // There should be a pending welcome for Bob.
        let welcomes = db.get_pending_welcomes(bob).unwrap();
        assert_eq!(welcomes.len(), 1);
        assert_eq!(welcomes[0].welcome_data, b"welcome_data");

        // There should be a message in the group.
        let messages = db.get_messages(group_id, 0, 100).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].mls_message, b"commit_data");
    }

    #[test]
    fn test_delete_pending_invite() {
        let db = Database::open_in_memory().unwrap();
        let alice = db.create_user("alice", "hash").unwrap();
        let bob = db.create_user("bob", "hash").unwrap();

        let group_id = db.create_group("test_group", None, alice).unwrap();
        let invite_id = db
            .create_pending_invite(group_id, alice, bob, b"c", b"w", b"g")
            .unwrap();

        db.delete_pending_invite(invite_id).unwrap();
        assert!(db.get_pending_invite(invite_id).unwrap().is_none());
    }

    #[test]
    fn test_accept_nonexistent_invite() {
        let db = Database::open_in_memory().unwrap();
        let result = db.accept_pending_invite(Uuid::new_v4());
        assert!(result.is_err());
    }
}
