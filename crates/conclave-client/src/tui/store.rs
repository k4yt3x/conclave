use std::path::Path;

use rusqlite::{Connection, params};

use super::state::DisplayMessage;

/// Set restrictive permissions on a file (Unix only).
#[cfg(unix)]
fn set_file_permissions_0600(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
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
                 group_id   TEXT    NOT NULL,
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
        let _ = conn.execute_batch(
            "ALTER TABLE room_state ADD COLUMN last_read_seq INTEGER NOT NULL DEFAULT 0;",
        );

        Ok(Self { conn })
    }

    /// Append a message to the history for a room.
    pub fn push_message(&self, group_id: &str, msg: &DisplayMessage) {
        let _ = self.conn.execute(
            "INSERT INTO messages (group_id, sender, content, timestamp, is_system)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                group_id,
                msg.sender,
                msg.content,
                msg.timestamp,
                msg.is_system as i32,
            ],
        );
    }

    /// Load recent messages for a room (last 1000), ordered chronologically.
    pub fn load_messages(&self, group_id: &str) -> Vec<DisplayMessage> {
        let mut stmt = match self.conn.prepare(
            "SELECT sender, content, timestamp, is_system
             FROM (SELECT * FROM messages WHERE group_id = ?1 ORDER BY id DESC LIMIT 1000)
             ORDER BY id ASC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let rows = match stmt.query_map(params![group_id], |row| {
            Ok(DisplayMessage {
                sender: row.get(0)?,
                content: row.get(1)?,
                timestamp: row.get(2)?,
                is_system: row.get::<_, i32>(3)? != 0,
            })
        }) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        rows.filter_map(|r| r.ok()).collect()
    }

    /// Get the last seen sequence number for a room.
    pub fn get_last_seen_seq(&self, group_id: &str) -> u64 {
        self.conn
            .query_row(
                "SELECT last_seen_seq FROM room_state WHERE group_id = ?1",
                params![group_id],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    /// Update the last seen sequence number for a room.
    pub fn set_last_seen_seq(&self, group_id: &str, seq: u64) {
        let _ = self.conn.execute(
            "INSERT INTO room_state (group_id, last_seen_seq) VALUES (?1, ?2)
             ON CONFLICT(group_id) DO UPDATE SET last_seen_seq = excluded.last_seen_seq",
            params![group_id, seq],
        );
    }

    /// Get the last read sequence number for a room.
    pub fn get_last_read_seq(&self, group_id: &str) -> u64 {
        self.conn
            .query_row(
                "SELECT last_read_seq FROM room_state WHERE group_id = ?1",
                params![group_id],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    /// Update the last read sequence number for a room.
    pub fn set_last_read_seq(&self, group_id: &str, seq: u64) {
        let _ = self.conn.execute(
            "INSERT INTO room_state (group_id, last_read_seq) VALUES (?1, ?2)
             ON CONFLICT(group_id) DO UPDATE SET last_read_seq = excluded.last_read_seq",
            params![group_id, seq],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::state::DisplayMessage;
    use super::*;
    use tempfile::TempDir;

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
        store.push_message("g1", &DisplayMessage::user("alice", "hello", 1));
        store.push_message("g1", &DisplayMessage::user("bob", "world", 2));
        let msgs = store.load_messages("g1");
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
        let msgs = store.load_messages("nonexistent");
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_load_messages_limit() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        for i in 0..1001 {
            store.push_message(
                "g1",
                &DisplayMessage::user("alice", &format!("msg{i}"), i as i64),
            );
        }
        let msgs = store.load_messages("g1");
        assert_eq!(msgs.len(), 1000);
    }

    #[test]
    fn test_last_seen_seq_default() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        assert_eq!(store.get_last_seen_seq("unknown"), 0);
    }

    #[test]
    fn test_set_and_get_last_seen_seq() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.set_last_seen_seq("g1", 42);
        assert_eq!(store.get_last_seen_seq("g1"), 42);
    }

    #[test]
    fn test_last_read_seq_default() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        assert_eq!(store.get_last_read_seq("unknown"), 0);
    }

    #[test]
    fn test_set_and_get_last_read_seq() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.set_last_read_seq("g1", 10);
        assert_eq!(store.get_last_read_seq("g1"), 10);
    }

    #[test]
    fn test_set_last_seen_seq_upsert() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.set_last_seen_seq("g1", 5);
        store.set_last_seen_seq("g1", 10);
        assert_eq!(store.get_last_seen_seq("g1"), 10);
    }

    #[test]
    fn test_messages_isolated_by_group() {
        let dir = TempDir::new().unwrap();
        let store = MessageStore::open(dir.path()).unwrap();
        store.push_message("g1", &DisplayMessage::user("alice", "for g1", 1));
        store.push_message("g2", &DisplayMessage::user("bob", "for g2", 2));
        let g1_msgs = store.load_messages("g1");
        assert_eq!(g1_msgs.len(), 1);
        assert_eq!(g1_msgs[0].content, "for g1");
        let g2_msgs = store.load_messages("g2");
        assert_eq!(g2_msgs.len(), 1);
        assert_eq!(g2_msgs[0].content, "for g2");
    }
}
