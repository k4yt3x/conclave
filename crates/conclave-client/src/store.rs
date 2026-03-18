use std::path::Path;

use rusqlite::{Connection, params};
use uuid::Uuid;

use crate::state::DisplayMessage;

/// Set restrictive permissions on a file (Unix only).
#[cfg(unix)]
fn set_file_permissions_0600(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(error) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        tracing::warn!(%error, path = %path.display(), "failed to set file permissions to 0600");
    }
}

/// Persistent message store backed by SQLite.
pub struct MessageStore {
    conn: Connection,
}

impl MessageStore {
    /// Open (or create) the message history database.
    pub fn open(data_dir: &Path) -> crate::error::Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let db_path = data_dir.join("message_history.db");
        let conn = Connection::open(&db_path)
            .map_err(|e| crate::error::Error::Other(format!("message store open failed: {e}")))?;

        #[cfg(unix)]
        set_file_permissions_0600(&db_path);

        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;

             CREATE TABLE IF NOT EXISTS messages (
                 id         INTEGER PRIMARY KEY AUTOINCREMENT,
                 group_id   TEXT NOT NULL,
                 sender     TEXT    NOT NULL,
                 content    TEXT    NOT NULL,
                 timestamp  INTEGER NOT NULL,
                 is_system  INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS idx_messages_group
                 ON messages(group_id);

             CREATE TABLE IF NOT EXISTS room_state (
                 group_id      TEXT PRIMARY KEY,
                 last_seen_seq INTEGER NOT NULL DEFAULT 0,
                 last_read_seq INTEGER NOT NULL DEFAULT 0
             );",
        )
        .map_err(|e| crate::error::Error::Other(format!("message store init failed: {e}")))?;

        // Migrate: add last_read_seq column if upgrading from older schema.
        if let Err(error) = conn.execute_batch(
            "ALTER TABLE room_state ADD COLUMN last_read_seq INTEGER NOT NULL DEFAULT 0;",
        ) {
            let msg = error.to_string();
            if !msg.contains("duplicate column") {
                return Err(crate::error::Error::Other(format!(
                    "migration failed: {msg}"
                )));
            }
        }

        // Migrate: add sender_id column if upgrading from older schema.
        if let Err(error) = conn
            .execute_batch("ALTER TABLE messages ADD COLUMN sender_id TEXT NOT NULL DEFAULT '';")
        {
            let msg = error.to_string();
            if !msg.contains("duplicate column") {
                return Err(crate::error::Error::Other(format!(
                    "migration failed: {msg}"
                )));
            }
        }

        // Migrate: add sequence_num column if upgrading from older schema.
        if let Err(error) = conn.execute_batch(
            "ALTER TABLE messages ADD COLUMN sequence_num INTEGER NOT NULL DEFAULT 0;",
        ) {
            let msg = error.to_string();
            if !msg.contains("duplicate column") {
                return Err(crate::error::Error::Other(format!(
                    "migration failed: {msg}"
                )));
            }
        }

        // Migrate: add epoch column if upgrading from older schema.
        if let Err(error) =
            conn.execute_batch("ALTER TABLE messages ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0;")
        {
            let msg = error.to_string();
            if !msg.contains("duplicate column") {
                return Err(crate::error::Error::Other(format!(
                    "migration failed: {msg}"
                )));
            }
        }

        // Create the known_fingerprints table for TOFU verification.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS known_fingerprints (
                 user_id        TEXT PRIMARY KEY,
                 fingerprint    TEXT NOT NULL,
                 verified       INTEGER NOT NULL DEFAULT 0,
                 first_seen_at  INTEGER NOT NULL DEFAULT (unixepoch()),
                 verified_at    INTEGER
             );",
        )
        .map_err(|e| {
            crate::error::Error::Other(format!("known_fingerprints table creation failed: {e}"))
        })?;

        // Migrate: add key_changed column for persistent Changed status.
        if let Err(error) = conn.execute_batch(
            "ALTER TABLE known_fingerprints ADD COLUMN key_changed INTEGER NOT NULL DEFAULT 0;",
        ) {
            let msg = error.to_string();
            if !msg.contains("duplicate column") {
                return Err(crate::error::Error::Other(format!(
                    "migration failed: {msg}"
                )));
            }
        }

        Ok(Self { conn })
    }

    /// Append a message to the history for a room.
    pub fn push_message(&self, group_id: Uuid, msg: &DisplayMessage) {
        if let Err(error) = self.conn.execute(
            "INSERT INTO messages (group_id, sender, content, timestamp, is_system, sender_id, sequence_num, epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                group_id.to_string(),
                msg.sender,
                msg.content,
                msg.timestamp,
                msg.is_system as i32,
                msg.sender_id.map(|id| id.to_string()).unwrap_or_default(),
                msg.sequence_num.unwrap_or(0),
                msg.epoch.unwrap_or(0),
            ],
        ) {
            tracing::trace!(%error, "failed to persist message to local store");
        }
    }

    /// Load recent messages for a room (last 1000), ordered chronologically.
    pub fn load_messages(&self, group_id: Uuid) -> Vec<DisplayMessage> {
        let mut stmt = match self.conn.prepare(
            "SELECT sender, content, timestamp, is_system, sender_id, sequence_num, epoch
             FROM (SELECT * FROM messages WHERE group_id = ?1 ORDER BY id DESC LIMIT 1000)
             ORDER BY id ASC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let rows = match stmt.query_map(params![group_id.to_string()], |row| {
            let sender_id_raw: String = row.get(4)?;
            let seq_raw: u64 = row.get(5)?;
            let epoch_raw: u64 = row.get(6)?;
            Ok(DisplayMessage {
                sender_id: if sender_id_raw.is_empty() {
                    None
                } else {
                    Uuid::parse_str(&sender_id_raw).ok()
                },
                sender: row.get(0)?,
                content: row.get(1)?,
                timestamp: row.get(2)?,
                sequence_num: if seq_raw == 0 { None } else { Some(seq_raw) },
                epoch: if epoch_raw == 0 {
                    None
                } else {
                    Some(epoch_raw)
                },
                is_system: row.get::<_, i32>(3)? != 0,
            })
        }) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        rows.filter_map(|r| r.ok()).collect()
    }

    /// Get the last seen sequence number for a room.
    pub fn get_last_seen_seq(&self, group_id: Uuid) -> u64 {
        self.conn
            .query_row(
                "SELECT last_seen_seq FROM room_state WHERE group_id = ?1",
                params![group_id.to_string()],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    /// Update the last seen sequence number for a room.
    pub fn set_last_seen_seq(&self, group_id: Uuid, sequence_number: u64) {
        if let Err(error) = self.conn.execute(
            "INSERT INTO room_state (group_id, last_seen_seq, last_read_seq) VALUES (?1, ?2, 0)
             ON CONFLICT(group_id) DO UPDATE SET last_seen_seq = excluded.last_seen_seq",
            params![group_id.to_string(), sequence_number],
        ) {
            tracing::trace!(%error, "failed to update last_seen_seq");
        }
    }

    /// Get the last read sequence number for a room.
    pub fn get_last_read_seq(&self, group_id: Uuid) -> u64 {
        self.conn
            .query_row(
                "SELECT last_read_seq FROM room_state WHERE group_id = ?1",
                params![group_id.to_string()],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    /// Delete locally cached messages for a group based on its expiry policy.
    pub fn cleanup_expired_messages(&self, group_id: Uuid, expiry_seconds: i64) {
        if expiry_seconds == 0 {
            if let Err(error) = self.conn.execute(
                "DELETE FROM messages WHERE group_id = ?1 AND is_system = 0",
                params![group_id.to_string()],
            ) {
                tracing::trace!(%error, "failed to cleanup expired messages from local store");
            }
        } else if expiry_seconds > 0 {
            let cutoff = chrono::Utc::now().timestamp() - expiry_seconds;
            if let Err(error) = self.conn.execute(
                "DELETE FROM messages WHERE group_id = ?1 AND timestamp < ?2 AND is_system = 0",
                params![group_id.to_string(), cutoff],
            ) {
                tracing::trace!(%error, "failed to cleanup expired messages from local store");
            }
        }
    }

    /// Check and update the TOFU store for a user's fingerprint.
    ///
    /// Returns the verification status based on the current fingerprint
    /// and stored TOFU state.
    pub fn get_verification_status(
        &self,
        user_id: Uuid,
        current_fingerprint: &str,
    ) -> crate::state::VerificationStatus {
        use crate::state::VerificationStatus;

        if current_fingerprint.is_empty() {
            return VerificationStatus::Unknown;
        }

        let existing: Option<(String, bool, bool)> = self
            .conn
            .query_row(
                "SELECT fingerprint, verified, key_changed FROM known_fingerprints WHERE user_id = ?1",
                params![user_id.to_string()],
                |row| {
                    let fp: String = row.get(0)?;
                    let verified: bool = row.get::<_, i32>(1)? != 0;
                    let key_changed: bool = row.get::<_, i32>(2)? != 0;
                    Ok((fp, verified, key_changed))
                },
            )
            .ok();

        match existing {
            None => {
                if let Err(error) = self.conn.execute(
                    "INSERT INTO known_fingerprints (user_id, fingerprint) VALUES (?1, ?2)",
                    params![user_id.to_string(), current_fingerprint],
                ) {
                    tracing::warn!(%error, "failed to store TOFU fingerprint");
                }
                VerificationStatus::Unverified
            }
            Some((stored_fp, verified, key_changed)) => {
                if stored_fp == current_fingerprint {
                    if key_changed {
                        VerificationStatus::Changed
                    } else if verified {
                        VerificationStatus::Verified
                    } else {
                        VerificationStatus::Unverified
                    }
                } else {
                    if let Err(error) = self.conn.execute(
                        "UPDATE known_fingerprints SET fingerprint = ?2, verified = 0, verified_at = NULL, key_changed = 1 WHERE user_id = ?1",
                        params![user_id.to_string(), current_fingerprint],
                    ) {
                        tracing::warn!(%error, "failed to update TOFU fingerprint");
                    }
                    VerificationStatus::Changed
                }
            }
        }
    }

    /// Mark a user's fingerprint as manually verified.
    ///
    /// Returns false if no TOFU entry exists for this user.
    pub fn verify_user(&self, user_id: Uuid) -> bool {
        match self.conn.execute(
            "UPDATE known_fingerprints SET verified = 1, key_changed = 0, verified_at = unixepoch() WHERE user_id = ?1",
            params![user_id.to_string()],
        ) {
            Ok(count) => count > 0,
            Err(error) => {
                tracing::warn!(%error, "failed to verify user fingerprint");
                false
            }
        }
    }

    /// Remove manual verification for a user's fingerprint.
    ///
    /// Returns false if no TOFU entry exists for this user.
    pub fn unverify_user(&self, user_id: Uuid) -> bool {
        match self.conn.execute(
            "UPDATE known_fingerprints SET verified = 0, key_changed = 0, verified_at = NULL WHERE user_id = ?1",
            params![user_id.to_string()],
        ) {
            Ok(count) => count > 0,
            Err(error) => {
                tracing::warn!(%error, "failed to unverify user fingerprint");
                false
            }
        }
    }

    /// Get the stored fingerprint and verification status for a user.
    pub fn get_stored_fingerprint(&self, user_id: Uuid) -> Option<(String, bool)> {
        self.conn
            .query_row(
                "SELECT fingerprint, verified FROM known_fingerprints WHERE user_id = ?1",
                params![user_id.to_string()],
                |row| {
                    let fp: String = row.get(0)?;
                    let verified: bool = row.get::<_, i32>(1)? != 0;
                    Ok((fp, verified))
                },
            )
            .ok()
    }

    /// Get all known fingerprints from the TOFU store.
    ///
    /// Returns `(user_id, fingerprint, verified, key_changed)` tuples ordered by user ID.
    pub fn get_all_known_fingerprints(&self) -> Vec<(Uuid, String, bool, bool)> {
        let mut stmt = match self.conn.prepare(
            "SELECT user_id, fingerprint, verified, key_changed FROM known_fingerprints ORDER BY user_id",
        ) {
            Ok(s) => s,
            Err(error) => {
                tracing::warn!(%error, "failed to query known fingerprints");
                return Vec::new();
            }
        };
        stmt.query_map([], |row| {
            let user_id_raw: String = row.get(0)?;
            Ok((
                user_id_raw,
                row.get::<_, String>(1)?,
                row.get::<_, i32>(2)? != 0,
                row.get::<_, i32>(3)? != 0,
            ))
        })
        .map(|rows| {
            rows.filter_map(|r| r.ok())
                .filter_map(|(uid_str, fp, verified, changed)| {
                    Uuid::parse_str(&uid_str)
                        .ok()
                        .map(|uid| (uid, fp, verified, changed))
                })
                .collect()
        })
        .unwrap_or_default()
    }

    /// Update the last read sequence number for a room.
    pub fn set_last_read_seq(&self, group_id: Uuid, sequence_number: u64) {
        if let Err(error) = self.conn.execute(
            "INSERT INTO room_state (group_id, last_seen_seq, last_read_seq) VALUES (?1, 0, ?2)
             ON CONFLICT(group_id) DO UPDATE SET last_read_seq = excluded.last_read_seq",
            params![group_id.to_string(), sequence_number],
        ) {
            tracing::trace!(%error, "failed to update last_read_seq");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_uuid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn test_open_creates_db() {
        let dir = TempDir::new().unwrap();
        let _store = MessageStore::open(dir.path()).unwrap();
        assert!(dir.path().join("message_history.db").exists());
    }

    #[test]
    fn test_push_and_load_messages() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.push_message(
            test_uuid(1),
            &DisplayMessage::user(test_uuid(1), "alice", "hello", 1),
        );
        store.push_message(
            test_uuid(1),
            &DisplayMessage::user(test_uuid(2), "bob", "world", 2),
        );
        let msgs = store.load_messages(test_uuid(1));
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].sender, "alice");
        assert_eq!(msgs[0].content, "hello");
        assert_eq!(msgs[1].sender, "bob");
        assert_eq!(msgs[1].content, "world");
    }

    #[test]
    fn test_load_messages_empty() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        let msgs = store.load_messages(test_uuid(999));
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_load_messages_limit() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        for i in 0..1001 {
            store.push_message(
                test_uuid(1),
                &DisplayMessage::user(test_uuid(1), "alice", &format!("msg{i}"), i as i64),
            );
        }
        let msgs = store.load_messages(test_uuid(1));
        assert_eq!(msgs.len(), 1000);
    }

    #[test]
    fn test_last_seen_seq_default() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        assert_eq!(store.get_last_seen_seq(test_uuid(999)), 0);
    }

    #[test]
    fn test_set_and_get_last_seen_seq() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.set_last_seen_seq(test_uuid(1), 42);
        assert_eq!(store.get_last_seen_seq(test_uuid(1)), 42);
    }

    #[test]
    fn test_last_read_seq_default() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        assert_eq!(store.get_last_read_seq(test_uuid(999)), 0);
    }

    #[test]
    fn test_set_and_get_last_read_seq() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.set_last_read_seq(test_uuid(1), 10);
        assert_eq!(store.get_last_read_seq(test_uuid(1)), 10);
    }

    #[test]
    fn test_set_last_seen_seq_upsert() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.set_last_seen_seq(test_uuid(1), 5);
        store.set_last_seen_seq(test_uuid(1), 10);
        assert_eq!(store.get_last_seen_seq(test_uuid(1)), 10);
    }

    #[test]
    fn test_set_last_read_seq_does_not_clobber_last_seen_seq() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.set_last_seen_seq(test_uuid(1), 42);
        store.set_last_read_seq(test_uuid(1), 10);
        assert_eq!(store.get_last_seen_seq(test_uuid(1)), 42);
        assert_eq!(store.get_last_read_seq(test_uuid(1)), 10);
    }

    #[test]
    fn test_set_last_seen_seq_does_not_clobber_last_read_seq() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.set_last_read_seq(test_uuid(1), 10);
        store.set_last_seen_seq(test_uuid(1), 42);
        assert_eq!(store.get_last_read_seq(test_uuid(1)), 10);
        assert_eq!(store.get_last_seen_seq(test_uuid(1)), 42);
    }

    #[test]
    fn test_messages_isolated_by_group() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.push_message(
            test_uuid(1),
            &DisplayMessage::user(test_uuid(1), "alice", "for g1", 1),
        );
        store.push_message(
            test_uuid(2),
            &DisplayMessage::user(test_uuid(2), "bob", "for g2", 2),
        );
        let g1_msgs = store.load_messages(test_uuid(1));
        assert_eq!(g1_msgs.len(), 1);
        assert_eq!(g1_msgs[0].content, "for g1");
        let g2_msgs = store.load_messages(test_uuid(2));
        assert_eq!(g2_msgs.len(), 1);
        assert_eq!(g2_msgs[0].content, "for g2");
    }

    #[test]
    fn test_system_messages_stored_correctly() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.push_message(
            test_uuid(1),
            &DisplayMessage::system("alice joined the group"),
        );
        let msgs = store.load_messages(test_uuid(1));
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].is_system);
        assert_eq!(msgs[0].content, "alice joined the group");
    }

    #[test]
    fn test_mixed_user_and_system_messages() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.push_message(test_uuid(1), &DisplayMessage::system("group created"));
        store.push_message(
            test_uuid(1),
            &DisplayMessage::user(test_uuid(1), "alice", "hello!", 100),
        );
        store.push_message(test_uuid(1), &DisplayMessage::system("bob joined"));
        store.push_message(
            test_uuid(1),
            &DisplayMessage::user(test_uuid(2), "bob", "hi!", 200),
        );

        let msgs = store.load_messages(test_uuid(1));
        assert_eq!(msgs.len(), 4);
        assert!(msgs[0].is_system);
        assert!(!msgs[1].is_system);
        assert!(msgs[2].is_system);
        assert!(!msgs[3].is_system);
    }

    #[test]
    fn test_reopen_persistence() {
        let dir = TempDir::new().unwrap();
        {
            let store = MessageStore::open(dir.path()).unwrap();
            store.push_message(
                test_uuid(1),
                &DisplayMessage::user(test_uuid(1), "alice", "persisted", 1),
            );
            store.set_last_seen_seq(test_uuid(1), 99);
            store.set_last_read_seq(test_uuid(1), 50);
        }
        let store = MessageStore::open(dir.path()).unwrap();
        let msgs = store.load_messages(test_uuid(1));
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "persisted");
        assert_eq!(store.get_last_seen_seq(test_uuid(1)), 99);
        assert_eq!(store.get_last_read_seq(test_uuid(1)), 50);
    }

    #[test]
    fn test_sequence_state_persistence() {
        let dir = TempDir::new().unwrap();
        {
            let store = MessageStore::open(dir.path()).unwrap();
            store.set_last_seen_seq(test_uuid(1), 100);
            store.set_last_read_seq(test_uuid(1), 75);
        }
        let store = MessageStore::open(dir.path()).unwrap();
        assert_eq!(store.get_last_seen_seq(test_uuid(1)), 100);
        assert_eq!(store.get_last_read_seq(test_uuid(1)), 75);
    }

    #[test]
    fn test_multiple_group_sequences() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.set_last_seen_seq(test_uuid(1), 10);
        store.set_last_seen_seq(test_uuid(2), 20);
        store.set_last_seen_seq(test_uuid(3), 30);
        assert_eq!(store.get_last_seen_seq(test_uuid(1)), 10);
        assert_eq!(store.get_last_seen_seq(test_uuid(2)), 20);
        assert_eq!(store.get_last_seen_seq(test_uuid(3)), 30);
    }

    #[test]
    fn test_messages_ordered_chronologically() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.push_message(
            test_uuid(1),
            &DisplayMessage::user(test_uuid(1), "alice", "first", 300),
        );
        store.push_message(
            test_uuid(1),
            &DisplayMessage::user(test_uuid(2), "bob", "second", 100),
        );
        store.push_message(
            test_uuid(1),
            &DisplayMessage::user(test_uuid(3), "carol", "third", 200),
        );
        let msgs = store.load_messages(test_uuid(1));
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].content, "first");
        assert_eq!(msgs[1].content, "second");
        assert_eq!(msgs[2].content, "third");
    }

    #[test]
    fn test_load_messages_returns_most_recent_1000() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        for i in 0..1100 {
            store.push_message(
                test_uuid(1),
                &DisplayMessage::user(test_uuid(1), "alice", &format!("msg{i}"), i as i64),
            );
        }
        let msgs = store.load_messages(test_uuid(1));
        assert_eq!(msgs.len(), 1000);
        assert_eq!(msgs[0].content, "msg100");
        assert_eq!(msgs[999].content, "msg1099");
    }

    #[test]
    fn test_set_last_seen_seq_creates_room_state_entry() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.set_last_seen_seq(test_uuid(1), 50);
        assert_eq!(store.get_last_read_seq(test_uuid(1)), 0);
    }

    #[test]
    fn test_set_last_read_seq_creates_room_state_entry() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.set_last_read_seq(test_uuid(1), 50);
        assert_eq!(store.get_last_seen_seq(test_uuid(1)), 0);
    }

    #[test]
    fn test_reopen_preserves_room_state_independently() {
        let dir = TempDir::new().unwrap();
        {
            let store = MessageStore::open(dir.path()).unwrap();
            store.set_last_seen_seq(test_uuid(1), 10);
            store.set_last_read_seq(test_uuid(1), 5);
            store.set_last_seen_seq(test_uuid(2), 200);
            store.set_last_read_seq(test_uuid(2), 150);
        }
        let store = MessageStore::open(dir.path()).unwrap();
        assert_eq!(store.get_last_seen_seq(test_uuid(1)), 10);
        assert_eq!(store.get_last_read_seq(test_uuid(1)), 5);
        assert_eq!(store.get_last_seen_seq(test_uuid(2)), 200);
        assert_eq!(store.get_last_read_seq(test_uuid(2)), 150);
    }

    #[test]
    fn test_empty_content_message() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.push_message(
            test_uuid(1),
            &DisplayMessage::user(test_uuid(1), "alice", "", 1),
        );
        let msgs = store.load_messages(test_uuid(1));
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "");
    }

    #[test]
    fn test_unicode_content() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        let unicode_content =
            "\u{1F600}\u{1F389} \u{4F60}\u{597D}\u{4E16}\u{754C} \u{00E9}\u{00E8}\u{00EA}";
        store.push_message(
            test_uuid(1),
            &DisplayMessage::user(test_uuid(1), "alice", unicode_content, 1),
        );
        let msgs = store.load_messages(test_uuid(1));
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, unicode_content);
    }

    #[test]
    fn test_large_message_content() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        let large_content = "A".repeat(100 * 1024);
        store.push_message(
            test_uuid(1),
            &DisplayMessage::user(test_uuid(1), "alice", &large_content, 1),
        );
        let msgs = store.load_messages(test_uuid(1));
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, large_content);
    }

    #[test]
    fn test_system_and_user_message_counts() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.push_message(test_uuid(1), &DisplayMessage::system("system msg 1"));
        store.push_message(test_uuid(1), &DisplayMessage::system("system msg 2"));
        store.push_message(test_uuid(1), &DisplayMessage::system("system msg 3"));
        store.push_message(
            test_uuid(1),
            &DisplayMessage::user(test_uuid(1), "alice", "user msg 1", 100),
        );
        store.push_message(
            test_uuid(1),
            &DisplayMessage::user(test_uuid(2), "bob", "user msg 2", 200),
        );
        let msgs = store.load_messages(test_uuid(1));
        assert_eq!(msgs.len(), 5);
        assert!(msgs[0].is_system);
        assert!(msgs[1].is_system);
        assert!(msgs[2].is_system);
        assert!(!msgs[3].is_system);
        assert!(!msgs[4].is_system);
    }

    #[test]
    fn test_sequence_numbers_isolated_between_groups() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.set_last_seen_seq(test_uuid(1), 100);
        store.set_last_seen_seq(test_uuid(2), 200);
        assert_eq!(store.get_last_seen_seq(test_uuid(1)), 100);
        assert_eq!(store.get_last_seen_seq(test_uuid(2)), 200);
    }

    #[test]
    fn test_tofu_first_seen_returns_unverified() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        let status = store.get_verification_status(test_uuid(1), "aabbccdd");
        assert_eq!(status, crate::state::VerificationStatus::Unverified);
    }

    #[test]
    fn test_tofu_same_fingerprint_returns_unverified() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.get_verification_status(test_uuid(1), "aabbccdd");
        let status = store.get_verification_status(test_uuid(1), "aabbccdd");
        assert_eq!(status, crate::state::VerificationStatus::Unverified);
    }

    #[test]
    fn test_tofu_verified_returns_verified() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.get_verification_status(test_uuid(1), "aabbccdd");
        store.verify_user(test_uuid(1));
        let status = store.get_verification_status(test_uuid(1), "aabbccdd");
        assert_eq!(status, crate::state::VerificationStatus::Verified);
    }

    #[test]
    fn test_tofu_changed_fingerprint_returns_changed() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.get_verification_status(test_uuid(1), "aabbccdd");
        let status = store.get_verification_status(test_uuid(1), "newfingerprint");
        assert_eq!(status, crate::state::VerificationStatus::Changed);
    }

    #[test]
    fn test_tofu_changed_is_persistent() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.get_verification_status(test_uuid(1), "aabbccdd");
        store.get_verification_status(test_uuid(1), "newfingerprint");
        // Second call with same new fingerprint should still return Changed.
        let status = store.get_verification_status(test_uuid(1), "newfingerprint");
        assert_eq!(status, crate::state::VerificationStatus::Changed);
    }

    #[test]
    fn test_tofu_changed_persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        {
            let store = MessageStore::open(dir.path()).unwrap();
            store.get_verification_status(test_uuid(1), "aabbccdd");
            store.get_verification_status(test_uuid(1), "newfingerprint");
        }
        let store = MessageStore::open(dir.path()).unwrap();
        let status = store.get_verification_status(test_uuid(1), "newfingerprint");
        assert_eq!(status, crate::state::VerificationStatus::Changed);
    }

    #[test]
    fn test_tofu_verify_clears_changed() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.get_verification_status(test_uuid(1), "aabbccdd");
        store.verify_user(test_uuid(1));
        store.get_verification_status(test_uuid(1), "newfingerprint");
        assert_eq!(
            store.get_verification_status(test_uuid(1), "newfingerprint"),
            crate::state::VerificationStatus::Changed,
        );
        store.verify_user(test_uuid(1));
        let status = store.get_verification_status(test_uuid(1), "newfingerprint");
        assert_eq!(status, crate::state::VerificationStatus::Verified);
    }

    #[test]
    fn test_tofu_unverify_clears_changed() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.get_verification_status(test_uuid(1), "aabbccdd");
        store.get_verification_status(test_uuid(1), "newfingerprint");
        store.unverify_user(test_uuid(1));
        let status = store.get_verification_status(test_uuid(1), "newfingerprint");
        assert_eq!(status, crate::state::VerificationStatus::Unverified);
    }

    #[test]
    fn test_tofu_empty_fingerprint_returns_unknown() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        let status = store.get_verification_status(test_uuid(1), "");
        assert_eq!(status, crate::state::VerificationStatus::Unknown);
    }

    #[test]
    fn test_tofu_verify_nonexistent_returns_false() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        assert!(!store.verify_user(test_uuid(999)));
    }

    #[test]
    fn test_tofu_unverify_nonexistent_returns_false() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        assert!(!store.unverify_user(test_uuid(999)));
    }

    #[test]
    fn test_tofu_get_all_includes_key_changed() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.get_verification_status(test_uuid(1), "fp1");
        store.get_verification_status(test_uuid(2), "fp2");
        store.get_verification_status(test_uuid(2), "fp2_new");
        let entries = store.get_all_known_fingerprints();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], (test_uuid(1), "fp1".to_string(), false, false));
        assert_eq!(
            entries[1],
            (test_uuid(2), "fp2_new".to_string(), false, true)
        );
    }
}
