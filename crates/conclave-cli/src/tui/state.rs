use std::collections::HashMap;

pub use conclave_lib::state::{ConnectionStatus, DisplayMessage, Room};

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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_room(id: &str, name: &str) -> Room {
        Room {
            server_group_id: id.to_string(),
            name: name.to_string(),
            members: vec!["alice".to_string()],
            last_seen_seq: 0,
            last_read_seq: 0,
        }
    }

    #[test]
    fn test_new_defaults() {
        let state = AppState::new();
        assert!(!state.logged_in);
        assert!(state.active_room.is_none());
        assert!(state.rooms.is_empty());
    }

    #[test]
    fn test_active_messages_no_room() {
        let mut state = AppState::new();
        state.push_system_message(DisplayMessage::system("hello"));
        let msgs = state.active_messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "hello");
    }

    #[test]
    fn test_active_messages_with_room() {
        let mut state = AppState::new();
        state.active_room = Some("g1".to_string());
        state.push_room_message("g1", DisplayMessage::user("alice", "hi", 100));
        let msgs = state.active_messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "hi");
    }

    #[test]
    fn test_active_messages_empty_room() {
        let mut state = AppState::new();
        state.active_room = Some("g1".to_string());
        let msgs = state.active_messages();
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_find_room_by_name_exact() {
        let mut state = AppState::new();
        state
            .rooms
            .insert("g1".to_string(), make_room("g1", "general"));
        assert!(state.find_room_by_name("general").is_some());
    }

    #[test]
    fn test_find_room_by_name_case_insensitive() {
        let mut state = AppState::new();
        state
            .rooms
            .insert("g1".to_string(), make_room("g1", "general"));
        assert!(state.find_room_by_name("GENERAL").is_some());
    }

    #[test]
    fn test_find_room_by_name_prefix() {
        let mut state = AppState::new();
        state
            .rooms
            .insert("g1".to_string(), make_room("g1", "general"));
        assert!(state.find_room_by_name("gen").is_some());
    }

    #[test]
    fn test_find_room_by_name_not_found() {
        let mut state = AppState::new();
        state
            .rooms
            .insert("g1".to_string(), make_room("g1", "general"));
        assert!(state.find_room_by_name("xyz").is_none());
    }

    #[test]
    fn test_push_room_message() {
        let mut state = AppState::new();
        state.push_room_message("g1", DisplayMessage::user("alice", "hi", 100));
        assert_eq!(state.room_messages["g1"].len(), 1);
    }

    #[test]
    fn test_push_system_message() {
        let mut state = AppState::new();
        state.push_system_message(DisplayMessage::system("joined"));
        assert_eq!(state.system_messages.len(), 1);
    }

    #[test]
    fn test_active_room_info() {
        let mut state = AppState::new();
        state.active_room = Some("g1".to_string());
        state
            .rooms
            .insert("g1".to_string(), make_room("g1", "general"));
        assert!(state.active_room_info().is_some());
        assert_eq!(state.active_room_info().unwrap().name, "general");
    }

    #[test]
    fn test_active_room_info_none() {
        let state = AppState::new();
        assert!(state.active_room_info().is_none());
    }

    #[test]
    fn test_display_message_user() {
        let msg = DisplayMessage::user("alice", "hi", 123);
        assert!(!msg.is_system);
        assert_eq!(msg.sender, "alice");
        assert_eq!(msg.content, "hi");
        assert_eq!(msg.timestamp, 123);
    }

    #[test]
    fn test_display_message_system() {
        let msg = DisplayMessage::system("joined");
        assert!(msg.is_system);
        assert_eq!(msg.sender, "");
        assert_eq!(msg.content, "joined");
    }
}
