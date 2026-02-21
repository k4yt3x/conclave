use std::collections::HashMap;

pub use conclave_lib::state::{ConnectionStatus, DisplayMessage, Room};

pub struct AppState {
    // Identity
    pub username: Option<String>,
    pub user_id: Option<i64>,
    pub logged_in: bool,

    // Rooms
    pub rooms: HashMap<i64, Room>,
    pub active_room: Option<i64>,
    pub room_messages: HashMap<i64, Vec<DisplayMessage>>,

    // Group mapping (server group ID -> MLS group ID hex).
    pub group_mapping: HashMap<i64, String>,

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

    pub fn active_room_info(&self) -> Option<&Room> {
        self.active_room.as_ref().and_then(|id| self.rooms.get(id))
    }

    /// Find a room by display name (alias or group_name), case-insensitive.
    /// Falls back to prefix match, then tries parsing as i64 for direct ID lookup.
    pub fn resolve_room(&self, name: &str) -> Option<&Room> {
        let lower = name.to_lowercase();

        // Exact match on display name.
        if let Some(room) = self
            .rooms
            .values()
            .find(|r| r.display_name().to_lowercase() == lower)
        {
            return Some(room);
        }

        // Prefix match on display name.
        if let Some(room) = self
            .rooms
            .values()
            .find(|r| r.display_name().to_lowercase().starts_with(&lower))
        {
            return Some(room);
        }

        // Direct ID lookup.
        if let Ok(id) = name.parse::<i64>() {
            return self.rooms.get(&id);
        }

        None
    }

    pub fn push_room_message(&mut self, group_id: i64, msg: DisplayMessage) {
        self.room_messages.entry(group_id).or_default().push(msg);
    }

    pub fn push_system_message(&mut self, msg: DisplayMessage) {
        self.system_messages.push(msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use conclave_lib::state::RoomMember;

    fn make_room(id: i64, name: &str) -> Room {
        Room {
            server_group_id: id,
            group_name: Some(name.to_string()),
            alias: None,
            members: vec![RoomMember {
                user_id: 1,
                username: "alice".to_string(),
                alias: None,
            }],
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
        state.active_room = Some(1);
        state.push_room_message(1, DisplayMessage::user(1, "alice", "hi", 100));
        let msgs = state.active_messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "hi");
    }

    #[test]
    fn test_active_messages_empty_room() {
        let mut state = AppState::new();
        state.active_room = Some(1);
        let msgs = state.active_messages();
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_resolve_room_exact() {
        let mut state = AppState::new();
        state.rooms.insert(1, make_room(1, "general"));
        assert!(state.resolve_room("general").is_some());
    }

    #[test]
    fn test_resolve_room_case_insensitive() {
        let mut state = AppState::new();
        state.rooms.insert(1, make_room(1, "general"));
        assert!(state.resolve_room("GENERAL").is_some());
    }

    #[test]
    fn test_resolve_room_prefix() {
        let mut state = AppState::new();
        state.rooms.insert(1, make_room(1, "general"));
        assert!(state.resolve_room("gen").is_some());
    }

    #[test]
    fn test_resolve_room_by_id() {
        let mut state = AppState::new();
        state.rooms.insert(42, make_room(42, "general"));
        assert!(state.resolve_room("42").is_some());
    }

    #[test]
    fn test_resolve_room_not_found() {
        let mut state = AppState::new();
        state.rooms.insert(1, make_room(1, "general"));
        assert!(state.resolve_room("xyz").is_none());
    }

    #[test]
    fn test_push_room_message() {
        let mut state = AppState::new();
        state.push_room_message(1, DisplayMessage::user(1, "alice", "hi", 100));
        assert_eq!(state.room_messages[&1].len(), 1);
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
        state.active_room = Some(1);
        state.rooms.insert(1, make_room(1, "general"));
        assert!(state.active_room_info().is_some());
        assert_eq!(
            state.active_room_info().map(|r| r.display_name()),
            Some("general".to_string())
        );
    }

    #[test]
    fn test_active_room_info_none() {
        let state = AppState::new();
        assert!(state.active_room_info().is_none());
    }

    #[test]
    fn test_display_message_user() {
        let msg = DisplayMessage::user(1, "alice", "hi", 123);
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

    #[test]
    fn test_resolve_room_empty_input() {
        let state = AppState::new();
        assert!(state.resolve_room("").is_none());
    }

    #[test]
    fn test_resolve_room_numeric_group_name_vs_id() {
        let mut state = AppState::new();
        state.rooms.insert(1, make_room(1, "room-one"));
        state.rooms.insert(2, make_room(2, "1"));

        // "1" matches group_name "1" (server_group_id=2) via exact name match,
        // which takes priority over the numeric ID fallback to server_group_id=1.
        let resolved = state.resolve_room("1");
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().server_group_id, 2);
    }

    #[test]
    fn test_resolve_room_both_alias_and_group_name_searchable() {
        let mut state = AppState::new();
        let mut room = make_room(1, "general");
        room.alias = Some("My Chat".to_string());
        state.rooms.insert(1, room);

        assert!(state.resolve_room("My Chat").is_some());
        assert!(state.resolve_room("general").is_none());
    }

    #[test]
    fn test_resolve_room_multiple_prefix_matches() {
        let mut state = AppState::new();
        state.rooms.insert(1, make_room(1, "general"));
        state.rooms.insert(2, make_room(2, "gen-admin"));

        let resolved = state.resolve_room("gen");
        assert!(resolved.is_some());
    }
}
