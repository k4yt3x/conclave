use std::path::Path;

use rusqlite::{Connection, params};

use super::state::DisplayMessage;

/// Persistent message store backed by SQLite.
pub struct MessageStore {
    conn: Connection,
}

impl MessageStore {
    /// Open (or create) the message history database for a user.
    pub fn open(user_data_dir: &Path) -> crate::error::Result<Self> {
        std::fs::create_dir_all(user_data_dir)?;
        let db_path = user_data_dir.join("message_history.db");
        let conn = Connection::open(&db_path)
            .map_err(|e| crate::error::Error::Other(format!("message store open failed: {e}")))?;

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

    /// Load all messages for a room, ordered chronologically.
    pub fn load_messages(&self, group_id: &str) -> Vec<DisplayMessage> {
        let mut stmt = match self.conn.prepare(
            "SELECT sender, content, timestamp, is_system
             FROM messages WHERE group_id = ?1
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
