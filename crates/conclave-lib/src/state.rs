/// Connection status for the SSE stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
}

/// A member of a room.
#[derive(Debug, Clone)]
pub struct RoomMember {
    pub user_id: i64,
    pub username: String,
    pub alias: Option<String>,
}

impl RoomMember {
    /// Display name: alias if set, otherwise username.
    pub fn display_name(&self) -> &str {
        self.alias
            .as_deref()
            .filter(|a| !a.is_empty())
            .unwrap_or(&self.username)
    }
}

/// A room the user has joined.
#[derive(Debug, Clone)]
pub struct Room {
    pub server_group_id: i64,
    pub group_name: Option<String>,
    pub alias: Option<String>,
    pub members: Vec<RoomMember>,
    /// Highest sequence number processed by MLS (fetched + decrypted).
    pub last_seen_seq: u64,
    /// Highest sequence number the user has actually viewed (room was active).
    pub last_read_seq: u64,
}

impl Room {
    /// Display name: alias > group_name > id as string.
    pub fn display_name(&self) -> String {
        if let Some(alias) = &self.alias
            && !alias.is_empty()
        {
            return alias.clone();
        }
        if let Some(group_name) = &self.group_name
            && !group_name.is_empty()
        {
            return group_name.clone();
        }
        self.server_group_id.to_string()
    }
}

/// A single message for display.
#[derive(Debug, Clone)]
pub struct DisplayMessage {
    /// Sender's user ID. None for system messages.
    pub sender_id: Option<i64>,
    /// Fallback display name (used when sender can't be resolved from members).
    pub sender: String,
    pub content: String,
    pub timestamp: i64,
    /// Server-assigned sequence number within the group (None for local-only messages).
    pub sequence_num: Option<u64>,
    /// MLS epoch at the time this message was processed (None for local-only messages).
    pub epoch: Option<u64>,
    /// System messages (joins, parts, errors) use a different format.
    pub is_system: bool,
}

impl DisplayMessage {
    pub fn user(sender_id: i64, sender: &str, content: &str, timestamp: i64) -> Self {
        Self {
            sender_id: Some(sender_id),
            sender: sender.to_string(),
            content: content.to_string(),
            timestamp,
            sequence_num: None,
            epoch: None,
            is_system: false,
        }
    }

    pub fn system(content: &str) -> Self {
        Self {
            sender_id: None,
            sender: String::new(),
            content: content.to_string(),
            timestamp: chrono::Local::now().timestamp(),
            sequence_num: None,
            epoch: None,
            is_system: true,
        }
    }
}

/// Resolve a message's sender display name from the room member list.
///
/// If `sender_id` is set and found in `members`, returns the member's display
/// name (alias if set, otherwise username). Falls back to the stored `sender`
/// string for system messages or when the sender is not in the member list.
pub fn resolve_sender_name(msg: &DisplayMessage, members: &[RoomMember]) -> String {
    if let Some(sid) = msg.sender_id {
        if let Some(member) = members.iter().find(|m| m.user_id == sid) {
            return member.display_name().to_string();
        }
    }
    msg.sender.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_room_member_display_name_with_alias() {
        let member = RoomMember {
            user_id: 1,
            username: "alice".into(),
            alias: Some("Alice W.".into()),
        };
        assert_eq!(member.display_name(), "Alice W.");
    }

    #[test]
    fn test_room_member_display_name_empty_alias() {
        let member = RoomMember {
            user_id: 1,
            username: "alice".into(),
            alias: Some(String::new()),
        };
        assert_eq!(member.display_name(), "alice");
    }

    #[test]
    fn test_room_member_display_name_no_alias() {
        let member = RoomMember {
            user_id: 1,
            username: "alice".into(),
            alias: None,
        };
        assert_eq!(member.display_name(), "alice");
    }

    #[test]
    fn test_room_display_name_alias_priority() {
        let room = Room {
            server_group_id: 42,
            group_name: Some("devs".into()),
            alias: Some("Dev Team".into()),
            members: vec![],
            last_seen_seq: 0,
            last_read_seq: 0,
        };
        assert_eq!(room.display_name(), "Dev Team");
    }

    #[test]
    fn test_room_display_name_group_name_fallback() {
        let room = Room {
            server_group_id: 42,
            group_name: Some("devs".into()),
            alias: None,
            members: vec![],
            last_seen_seq: 0,
            last_read_seq: 0,
        };
        assert_eq!(room.display_name(), "devs");
    }

    #[test]
    fn test_room_display_name_id_fallback() {
        let room = Room {
            server_group_id: 42,
            group_name: None,
            alias: None,
            members: vec![],
            last_seen_seq: 0,
            last_read_seq: 0,
        };
        assert_eq!(room.display_name(), "42");
    }

    #[test]
    fn test_room_display_name_empty_alias_falls_through() {
        let room = Room {
            server_group_id: 42,
            group_name: Some("devs".into()),
            alias: Some(String::new()),
            members: vec![],
            last_seen_seq: 0,
            last_read_seq: 0,
        };
        assert_eq!(room.display_name(), "devs");
    }

    #[test]
    fn test_room_display_name_empty_group_name_falls_through() {
        let room = Room {
            server_group_id: 42,
            group_name: Some(String::new()),
            alias: None,
            members: vec![],
            last_seen_seq: 0,
            last_read_seq: 0,
        };
        assert_eq!(room.display_name(), "42");
    }

    #[test]
    fn test_display_message_user() {
        let msg = DisplayMessage::user(42, "alice", "hello", 1000);
        assert_eq!(msg.sender_id, Some(42));
        assert_eq!(msg.sender, "alice");
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.timestamp, 1000);
        assert!(!msg.is_system);
    }

    #[test]
    fn test_display_message_system() {
        let msg = DisplayMessage::system("user joined");
        assert_eq!(msg.sender_id, None);
        assert!(msg.sender.is_empty());
        assert_eq!(msg.content, "user joined");
        assert!(msg.is_system);
        assert!(msg.timestamp > 0);
    }

    #[test]
    fn test_resolve_sender_name_from_members() {
        let members = vec![
            RoomMember {
                user_id: 1,
                username: "alice".into(),
                alias: Some("Alice W.".into()),
            },
            RoomMember {
                user_id: 2,
                username: "bob".into(),
                alias: None,
            },
        ];
        let msg = DisplayMessage::user(1, "old_name", "hi", 100);
        assert_eq!(resolve_sender_name(&msg, &members), "Alice W.");

        let msg2 = DisplayMessage::user(2, "old_name", "hi", 100);
        assert_eq!(resolve_sender_name(&msg2, &members), "bob");
    }

    #[test]
    fn test_resolve_sender_name_fallback() {
        let msg = DisplayMessage::user(999, "fallback_name", "hi", 100);
        assert_eq!(resolve_sender_name(&msg, &[]), "fallback_name");
    }

    #[test]
    fn test_resolve_sender_name_system_message() {
        let msg = DisplayMessage::system("joined");
        assert_eq!(resolve_sender_name(&msg, &[]), "");
    }
}
