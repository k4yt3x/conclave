use std::collections::HashMap;

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

/// Central application state.
pub struct AppState {
    // Identity
    pub username: Option<String>,
    pub user_id: Option<u64>,
    pub logged_in: bool,

    // Rooms
    pub rooms: HashMap<String, Room>,
    /// Server group ID of the currently active room.
    pub active_room: Option<String>,
    /// Per-room display messages (keyed by server group ID).
    pub room_messages: HashMap<String, Vec<DisplayMessage>>,

    // Group mapping (server UUID -> MLS group ID hex).
    pub group_mapping: HashMap<String, String>,

    // Connection
    pub connection_status: ConnectionStatus,

    // Display
    pub scroll_offset: usize,
    pub terminal_rows: u16,
    pub terminal_cols: u16,

    // System messages (not tied to any room).
    pub system_messages: Vec<DisplayMessage>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            username: None,
            user_id: None,
            logged_in: false,
            rooms: HashMap::new(),
            active_room: None,
            room_messages: HashMap::new(),
            group_mapping: HashMap::new(),
            connection_status: ConnectionStatus::Disconnected,
            scroll_offset: 0,
            terminal_rows: 24,
            terminal_cols: 80,
            system_messages: Vec::new(),
        }
    }

    /// Get the display messages for the active room, or system messages if no room is active.
    pub fn active_messages(&self) -> &[DisplayMessage] {
        if let Some(room_id) = &self.active_room {
            self.room_messages
                .get(room_id)
                .map(|v| v.as_slice())
                .unwrap_or(&[])
        } else {
            &self.system_messages
        }
    }

    /// Get the active Room, if any.
    pub fn active_room_info(&self) -> Option<&Room> {
        self.active_room.as_ref().and_then(|id| self.rooms.get(id))
    }

    /// Find a room by name (case-insensitive prefix match).
    pub fn find_room_by_name(&self, name: &str) -> Option<&Room> {
        let lower = name.to_lowercase();
        self.rooms
            .values()
            .find(|r| r.name.to_lowercase() == lower)
            .or_else(|| {
                self.rooms
                    .values()
                    .find(|r| r.name.to_lowercase().starts_with(&lower))
            })
    }

    /// Push a message to a room's history.
    pub fn push_room_message(&mut self, group_id: &str, msg: DisplayMessage) {
        self.room_messages
            .entry(group_id.to_string())
            .or_default()
            .push(msg);
    }

    /// Push a system message.
    pub fn push_system_message(&mut self, msg: DisplayMessage) {
        self.system_messages.push(msg);
    }
}
