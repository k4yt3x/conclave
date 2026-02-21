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
        let msg = DisplayMessage::user("alice", "hello", 1000);
        assert_eq!(msg.sender, "alice");
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.timestamp, 1000);
        assert!(!msg.is_system);
    }

    #[test]
    fn test_display_message_system() {
        let msg = DisplayMessage::system("user joined");
        assert!(msg.sender.is_empty());
        assert_eq!(msg.content, "user joined");
        assert!(msg.is_system);
        assert!(msg.timestamp > 0);
    }
}
