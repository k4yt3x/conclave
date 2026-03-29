use std::collections::HashMap;

use uuid::Uuid;

pub use conclave_client::state::{ConnectionStatus, DisplayMessage, Room, VerificationStatus};

/// What the password prompt is being used for, and the stored fields
/// from the original command that triggered it.
pub enum PasswordPromptPurpose {
    ChangePassword,
    Register {
        server: String,
        username: String,
        token: Option<String>,
    },
    Login {
        server: String,
        username: String,
    },
    DeleteAccount,
}

pub enum PasswordPromptStage {
    Current,
    New,
    Confirm,
}

impl PasswordPromptStage {
    pub fn label(&self, purpose: &PasswordPromptPurpose) -> &'static str {
        match (self, purpose) {
            (PasswordPromptStage::New, PasswordPromptPurpose::Login { .. })
            | (PasswordPromptStage::New, PasswordPromptPurpose::DeleteAccount) => "Password: ",
            (PasswordPromptStage::Current, _) => "Current password: ",
            (PasswordPromptStage::New, _) => "New password: ",
            (PasswordPromptStage::Confirm, _) => "Confirm password: ",
        }
    }
}

pub enum InputMode {
    Normal,
    PasswordPrompt {
        purpose: PasswordPromptPurpose,
        stage: PasswordPromptStage,
        current_password: zeroize::Zeroizing<String>,
        new_password: zeroize::Zeroizing<String>,
    },
}

pub struct AppState {
    // Identity
    pub username: Option<String>,
    pub user_id: Option<Uuid>,
    pub logged_in: bool,

    // Rooms
    pub rooms: HashMap<Uuid, Room>,
    pub active_room: Option<Uuid>,
    pub room_messages: HashMap<Uuid, Vec<DisplayMessage>>,

    // Group mapping (server group ID -> MLS group ID hex).
    pub group_mapping: HashMap<Uuid, String>,

    // Connection
    pub connection_status: ConnectionStatus,

    // Display
    pub scroll_offset: usize,
    pub terminal_rows: u16,
    pub terminal_cols: u16,

    // Verification status per user (TOFU).
    pub verification_status: HashMap<Uuid, VerificationStatus>,

    // System messages (not tied to any room).
    pub system_messages: Vec<DisplayMessage>,

    // Whether to show indicators for verified users/rooms.
    pub show_verified_indicator: bool,

    // Input mode (normal or password prompt).
    pub input_mode: InputMode,
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
            verification_status: HashMap::new(),
            scroll_offset: 0,
            terminal_rows: 24,
            terminal_cols: 80,
            system_messages: Vec::new(),
            show_verified_indicator: false,
            input_mode: InputMode::Normal,
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
    /// Falls back to prefix match, then tries parsing as UUID for direct ID lookup.
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
        if let Ok(id) = Uuid::parse_str(name) {
            return self.rooms.get(&id);
        }

        None
    }

    pub fn push_room_message(&mut self, group_id: Uuid, msg: DisplayMessage) {
        self.room_messages.entry(group_id).or_default().push(msg);
    }

    pub fn push_system_message(&mut self, msg: DisplayMessage) {
        self.system_messages.push(msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use conclave_client::state::RoomMember;

    fn make_room(id: Uuid, name: &str) -> Room {
        Room {
            server_group_id: id,
            group_name: name.to_string(),
            alias: None,
            members: vec![RoomMember {
                user_id: Uuid::from_u128(1),
                username: "alice".to_string(),
                alias: None,
                role: "admin".to_string(),
                signing_key_fingerprint: None,
            }],
            last_seen_seq: 0,
            last_read_seq: 0,
            message_expiry_seconds: -1,
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
        let room_id = Uuid::from_u128(1);
        let user_id = Uuid::from_u128(1);
        state.active_room = Some(room_id);
        state.push_room_message(room_id, DisplayMessage::user(user_id, "alice", "hi", 100));
        let msgs = state.active_messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "hi");
    }

    #[test]
    fn test_active_messages_empty_room() {
        let mut state = AppState::new();
        state.active_room = Some(Uuid::from_u128(1));
        let msgs = state.active_messages();
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_resolve_room_exact() {
        let mut state = AppState::new();
        let id = Uuid::from_u128(1);
        state.rooms.insert(id, make_room(id, "general"));
        assert!(state.resolve_room("general").is_some());
    }

    #[test]
    fn test_resolve_room_case_insensitive() {
        let mut state = AppState::new();
        let id = Uuid::from_u128(1);
        state.rooms.insert(id, make_room(id, "general"));
        assert!(state.resolve_room("GENERAL").is_some());
    }

    #[test]
    fn test_resolve_room_prefix() {
        let mut state = AppState::new();
        let id = Uuid::from_u128(1);
        state.rooms.insert(id, make_room(id, "general"));
        assert!(state.resolve_room("gen").is_some());
    }

    #[test]
    fn test_resolve_room_by_id() {
        let mut state = AppState::new();
        let id = Uuid::from_u128(42);
        state.rooms.insert(id, make_room(id, "general"));
        assert!(state.resolve_room(&id.to_string()).is_some());
    }

    #[test]
    fn test_resolve_room_not_found() {
        let mut state = AppState::new();
        let id = Uuid::from_u128(1);
        state.rooms.insert(id, make_room(id, "general"));
        assert!(state.resolve_room("xyz").is_none());
    }

    #[test]
    fn test_push_room_message() {
        let mut state = AppState::new();
        let room_id = Uuid::from_u128(1);
        let user_id = Uuid::from_u128(1);
        state.push_room_message(room_id, DisplayMessage::user(user_id, "alice", "hi", 100));
        assert_eq!(state.room_messages[&room_id].len(), 1);
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
        let id = Uuid::from_u128(1);
        state.active_room = Some(id);
        state.rooms.insert(id, make_room(id, "general"));
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
        let msg = DisplayMessage::user(Uuid::from_u128(1), "alice", "hi", 123);
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
    fn test_resolve_room_uuid_group_name_vs_id() {
        let mut state = AppState::new();
        let id1 = Uuid::from_u128(1);
        let id2 = Uuid::from_u128(2);
        state.rooms.insert(id1, make_room(id1, "room-one"));
        // Use id1's string representation as the group name for id2
        state.rooms.insert(id2, make_room(id2, &id1.to_string()));

        // The UUID string matches group_name (server_group_id=id2) via exact name match,
        // which takes priority over the UUID ID fallback to server_group_id=id1.
        let resolved = state.resolve_room(&id1.to_string());
        assert!(resolved.is_some());
        assert_eq!(resolved.expect("room should resolve").server_group_id, id2);
    }

    #[test]
    fn test_resolve_room_both_alias_and_group_name_searchable() {
        let mut state = AppState::new();
        let id = Uuid::from_u128(1);
        let mut room = make_room(id, "general");
        room.alias = Some("My Chat".to_string());
        state.rooms.insert(id, room);

        assert!(state.resolve_room("My Chat").is_some());
        assert!(state.resolve_room("general").is_none());
    }

    #[test]
    fn test_label_login_new() {
        let purpose = PasswordPromptPurpose::Login {
            server: "s".into(),
            username: "u".into(),
        };
        assert_eq!(PasswordPromptStage::New.label(&purpose), "Password: ");
    }

    #[test]
    fn test_label_delete_account_new() {
        assert_eq!(
            PasswordPromptStage::New.label(&PasswordPromptPurpose::DeleteAccount),
            "Password: "
        );
    }

    #[test]
    fn test_label_register_new() {
        let purpose = PasswordPromptPurpose::Register {
            server: "s".into(),
            username: "u".into(),
            token: None,
        };
        assert_eq!(PasswordPromptStage::New.label(&purpose), "New password: ");
    }

    #[test]
    fn test_label_change_password_new() {
        assert_eq!(
            PasswordPromptStage::New.label(&PasswordPromptPurpose::ChangePassword),
            "New password: "
        );
    }

    #[test]
    fn test_label_current() {
        assert_eq!(
            PasswordPromptStage::Current.label(&PasswordPromptPurpose::ChangePassword),
            "Current password: "
        );
    }

    #[test]
    fn test_label_confirm() {
        assert_eq!(
            PasswordPromptStage::Confirm.label(&PasswordPromptPurpose::ChangePassword),
            "Confirm password: "
        );
    }

    #[test]
    fn test_resolve_room_multiple_prefix_matches() {
        let mut state = AppState::new();
        let id1 = Uuid::from_u128(1);
        let id2 = Uuid::from_u128(2);
        state.rooms.insert(id1, make_room(id1, "general"));
        state.rooms.insert(id2, make_room(id2, "gen-admin"));

        let resolved = state.resolve_room("gen");
        assert!(resolved.is_some());
    }
}
