/// Connection status for the SSE stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
}

/// A room the user has joined.
#[derive(Debug, Clone)]
pub struct Room {
    pub server_group_id: String,
    pub name: String,
    pub members: Vec<String>,
    /// Highest sequence number processed by MLS (fetched + decrypted).
    pub last_seen_seq: u64,
    /// Highest sequence number the user has actually viewed (room was active).
    pub last_read_seq: u64,
}

/// A single message for display.
#[derive(Debug, Clone)]
pub struct DisplayMessage {
    pub sender: String,
    pub content: String,
    pub timestamp: i64,
    /// System messages (joins, parts, errors) use a different format.
    pub is_system: bool,
}

impl DisplayMessage {
    pub fn user(sender: &str, content: &str, timestamp: i64) -> Self {
        Self {
            sender: sender.to_string(),
            content: content.to_string(),
            timestamp,
            is_system: false,
        }
    }

    pub fn system(content: &str) -> Self {
        Self {
            sender: String::new(),
            content: content.to_string(),
            timestamp: chrono::Local::now().timestamp(),
            is_system: true,
        }
    }
}
