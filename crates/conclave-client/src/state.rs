use uuid::Uuid;

/// Connection status for the SSE stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
}

/// Hardcoded RGB colors for verification/trust indicators.
/// Defined here so both TUI (crossterm) and GUI (iced) use identical values.
pub const INDICATOR_COLOR_RISKY: (u8, u8, u8) = (0xcc, 0x44, 0x44);
pub const INDICATOR_COLOR_UNVERIFIED: (u8, u8, u8) = (0xcc, 0x99, 0x33);
pub const INDICATOR_COLOR_VERIFIED: (u8, u8, u8) = (0x44, 0x88, 0x44);

/// Verification status of a user's signing key fingerprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationStatus {
    /// No fingerprint available from server.
    Unknown,
    /// TOFU stored, not manually verified.
    Unverified,
    /// Manually verified via /verify.
    Verified,
    /// Fingerprint changed since last seen.
    Changed,
}

/// Aggregate trust level for an entire room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomTrustLevel {
    /// All members are verified.
    Verified,
    /// Some members are unverified or unknown.
    Unverified,
    /// A verified member has changed their signing key.
    Risky,
}

/// Compute the aggregate trust level for a room based on its members'
/// verification statuses.
///
/// Priority: any `Changed` → `Risky`, any non-`Verified` → `Unverified`,
/// all `Verified` → `Verified`.
pub fn room_trust_level(
    members: &[RoomMember],
    verification_status: &std::collections::HashMap<Uuid, VerificationStatus>,
) -> RoomTrustLevel {
    let mut all_verified = true;
    for member in members {
        match verification_status.get(&member.user_id) {
            Some(VerificationStatus::Changed) => return RoomTrustLevel::Risky,
            Some(VerificationStatus::Verified) => {}
            _ => all_verified = false,
        }
    }
    if all_verified {
        RoomTrustLevel::Verified
    } else {
        RoomTrustLevel::Unverified
    }
}

/// A member of a room.
#[derive(Debug, Clone)]
pub struct RoomMember {
    pub user_id: Uuid,
    pub username: String,
    pub alias: Option<String>,
    pub role: String,
    pub signing_key_fingerprint: Option<String>,
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
    pub server_group_id: Uuid,
    pub group_name: String,
    pub alias: Option<String>,
    pub members: Vec<RoomMember>,
    /// Highest sequence number processed by MLS (fetched + decrypted).
    pub last_seen_seq: u64,
    /// Highest sequence number the user has actually viewed (room was active).
    pub last_read_seq: u64,
    /// Message expiry policy: -1=disabled, 0=fetch-then-delete, >0=seconds.
    pub message_expiry_seconds: i64,
}

impl Room {
    /// Display name: alias if set, otherwise group_name.
    pub fn display_name(&self) -> String {
        if let Some(alias) = &self.alias
            && !alias.is_empty()
        {
            return alias.clone();
        }
        self.group_name.clone()
    }
}

/// A single message for display.
#[derive(Debug, Clone)]
pub struct DisplayMessage {
    /// Sender's user ID. None for system messages.
    pub sender_id: Option<Uuid>,
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
    pub fn user(sender_id: Uuid, sender: &str, content: &str, timestamp: i64) -> Self {
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

/// Remove expired messages from in-memory `room_messages` based on each room's
/// expiry policy. Returns `true` if any messages were removed.
pub fn remove_expired_messages(
    rooms: &std::collections::HashMap<Uuid, Room>,
    room_messages: &mut std::collections::HashMap<Uuid, Vec<DisplayMessage>>,
) -> bool {
    let now = chrono::Utc::now().timestamp();
    let mut any_removed = false;

    for (group_id, room) in rooms {
        if room.message_expiry_seconds <= 0 {
            continue;
        }
        let cutoff = now - room.message_expiry_seconds;
        if let Some(messages) = room_messages.get_mut(group_id) {
            let before = messages.len();
            messages.retain(|msg| msg.is_system || msg.timestamp >= cutoff);
            if messages.len() < before {
                any_removed = true;
            }
        }
    }
    any_removed
}

/// Check whether any room has a positive `message_expiry_seconds`, meaning
/// periodic expiry cleanup is needed.
pub fn has_expiring_rooms(rooms: &std::collections::HashMap<Uuid, Room>) -> bool {
    rooms.values().any(|r| r.message_expiry_seconds > 0)
}

/// Resolve a message's sender display name from the room member list.
///
/// If `sender_id` is set and found in `members`, returns the member's display
/// name (alias if set, otherwise username). Falls back to the stored `sender`
/// string for system messages or when the sender is not in the member list.
pub fn resolve_sender_name(msg: &DisplayMessage, members: &[RoomMember]) -> String {
    if let Some(sid) = msg.sender_id
        && let Some(member) = members.iter().find(|m| m.user_id == sid)
    {
        return member.display_name().to_string();
    }
    msg.sender.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_uuid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn test_room_member_display_name_with_alias() {
        let member = RoomMember {
            user_id: test_uuid(1),
            username: "alice".into(),
            alias: Some("Alice W.".into()),
            role: "member".into(),
            signing_key_fingerprint: None,
        };
        assert_eq!(member.display_name(), "Alice W.");
    }

    #[test]
    fn test_room_member_display_name_empty_alias() {
        let member = RoomMember {
            user_id: test_uuid(1),
            username: "alice".into(),
            alias: Some(String::new()),
            role: "member".into(),
            signing_key_fingerprint: None,
        };
        assert_eq!(member.display_name(), "alice");
    }

    #[test]
    fn test_room_member_display_name_no_alias() {
        let member = RoomMember {
            user_id: test_uuid(1),
            username: "alice".into(),
            alias: None,
            role: "member".into(),
            signing_key_fingerprint: None,
        };
        assert_eq!(member.display_name(), "alice");
    }

    #[test]
    fn test_room_display_name_alias_priority() {
        let room = Room {
            server_group_id: test_uuid(42),
            group_name: "devs".into(),
            alias: Some("Dev Team".into()),
            members: vec![],
            last_seen_seq: 0,
            last_read_seq: 0,
            message_expiry_seconds: -1,
        };
        assert_eq!(room.display_name(), "Dev Team");
    }

    #[test]
    fn test_room_display_name_group_name_fallback() {
        let room = Room {
            server_group_id: test_uuid(42),
            group_name: "devs".into(),
            alias: None,
            members: vec![],
            last_seen_seq: 0,
            last_read_seq: 0,
            message_expiry_seconds: -1,
        };
        assert_eq!(room.display_name(), "devs");
    }

    #[test]
    fn test_room_display_name_empty_alias_falls_through() {
        let room = Room {
            server_group_id: test_uuid(42),
            group_name: "devs".into(),
            alias: Some(String::new()),
            members: vec![],
            last_seen_seq: 0,
            last_read_seq: 0,
            message_expiry_seconds: -1,
        };
        assert_eq!(room.display_name(), "devs");
    }

    #[test]
    fn test_display_message_user() {
        let msg = DisplayMessage::user(test_uuid(42), "alice", "hello", 1000);
        assert_eq!(msg.sender_id, Some(test_uuid(42)));
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
                user_id: test_uuid(1),
                username: "alice".into(),
                alias: Some("Alice W.".into()),
                role: "admin".into(),
                signing_key_fingerprint: None,
            },
            RoomMember {
                user_id: test_uuid(2),
                username: "bob".into(),
                alias: None,
                role: "member".into(),
                signing_key_fingerprint: None,
            },
        ];
        let msg = DisplayMessage::user(test_uuid(1), "old_name", "hi", 100);
        assert_eq!(resolve_sender_name(&msg, &members), "Alice W.");

        let msg2 = DisplayMessage::user(test_uuid(2), "old_name", "hi", 100);
        assert_eq!(resolve_sender_name(&msg2, &members), "bob");
    }

    #[test]
    fn test_resolve_sender_name_fallback() {
        let msg = DisplayMessage::user(test_uuid(999), "fallback_name", "hi", 100);
        assert_eq!(resolve_sender_name(&msg, &[]), "fallback_name");
    }

    #[test]
    fn test_resolve_sender_name_system_message() {
        let msg = DisplayMessage::system("joined");
        assert_eq!(resolve_sender_name(&msg, &[]), "");
    }

    fn make_room(group_id: u128, expiry: i64) -> Room {
        Room {
            server_group_id: test_uuid(group_id),
            group_name: format!("room{group_id}"),
            alias: None,
            members: vec![],
            last_seen_seq: 0,
            last_read_seq: 0,
            message_expiry_seconds: expiry,
        }
    }

    #[test]
    fn test_has_expiring_rooms_none() {
        let mut rooms = std::collections::HashMap::new();
        rooms.insert(test_uuid(1), make_room(1, -1));
        rooms.insert(test_uuid(2), make_room(2, 0));
        assert!(!has_expiring_rooms(&rooms));
    }

    #[test]
    fn test_has_expiring_rooms_some() {
        let mut rooms = std::collections::HashMap::new();
        rooms.insert(test_uuid(1), make_room(1, -1));
        rooms.insert(test_uuid(2), make_room(2, 60));
        assert!(has_expiring_rooms(&rooms));
    }

    #[test]
    fn test_remove_expired_messages_removes_old() {
        let now = chrono::Utc::now().timestamp();
        let mut rooms = std::collections::HashMap::new();
        rooms.insert(test_uuid(1), make_room(1, 10));

        let mut room_messages = std::collections::HashMap::new();
        room_messages.insert(
            test_uuid(1),
            vec![
                DisplayMessage::user(test_uuid(1), "alice", "old", now - 20),
                DisplayMessage::system("old system msg"),
                DisplayMessage::user(test_uuid(1), "alice", "recent", now - 5),
                DisplayMessage::user(test_uuid(1), "alice", "new", now),
            ],
        );

        // Backdate the system message so it would be expired if not protected.
        room_messages.get_mut(&test_uuid(1)).unwrap()[1].timestamp = now - 20;

        let removed = remove_expired_messages(&rooms, &mut room_messages);
        assert!(removed);
        assert_eq!(room_messages[&test_uuid(1)].len(), 3);
        assert!(room_messages[&test_uuid(1)][0].is_system);
        assert_eq!(room_messages[&test_uuid(1)][0].content, "old system msg");
        assert_eq!(room_messages[&test_uuid(1)][1].content, "recent");
        assert_eq!(room_messages[&test_uuid(1)][2].content, "new");
    }

    #[test]
    fn test_remove_expired_messages_preserves_system() {
        let now = chrono::Utc::now().timestamp();
        let mut rooms = std::collections::HashMap::new();
        rooms.insert(test_uuid(1), make_room(1, 10));

        let mut room_messages = std::collections::HashMap::new();
        let mut sys_msg = DisplayMessage::system("help output");
        sys_msg.timestamp = now - 100;
        room_messages.insert(test_uuid(1), vec![sys_msg]);

        let removed = remove_expired_messages(&rooms, &mut room_messages);
        assert!(!removed);
        assert_eq!(room_messages[&test_uuid(1)].len(), 1);
        assert!(room_messages[&test_uuid(1)][0].is_system);
    }

    #[test]
    fn test_remove_expired_messages_skips_disabled() {
        let now = chrono::Utc::now().timestamp();
        let mut rooms = std::collections::HashMap::new();
        rooms.insert(test_uuid(1), make_room(1, -1));

        let mut room_messages = std::collections::HashMap::new();
        room_messages.insert(
            test_uuid(1),
            vec![DisplayMessage::user(
                test_uuid(1),
                "alice",
                "old",
                now - 9999,
            )],
        );

        let removed = remove_expired_messages(&rooms, &mut room_messages);
        assert!(!removed);
        assert_eq!(room_messages[&test_uuid(1)].len(), 1);
    }

    #[test]
    fn test_remove_expired_messages_skips_fetch_then_delete() {
        let now = chrono::Utc::now().timestamp();
        let mut rooms = std::collections::HashMap::new();
        rooms.insert(test_uuid(1), make_room(1, 0));

        let mut room_messages = std::collections::HashMap::new();
        room_messages.insert(
            test_uuid(1),
            vec![DisplayMessage::user(
                test_uuid(1),
                "alice",
                "old",
                now - 9999,
            )],
        );

        let removed = remove_expired_messages(&rooms, &mut room_messages);
        assert!(!removed);
        assert_eq!(room_messages[&test_uuid(1)].len(), 1);
    }

    #[test]
    fn test_remove_expired_messages_nothing_to_remove() {
        let now = chrono::Utc::now().timestamp();
        let mut rooms = std::collections::HashMap::new();
        rooms.insert(test_uuid(1), make_room(1, 60));

        let mut room_messages = std::collections::HashMap::new();
        room_messages.insert(
            test_uuid(1),
            vec![DisplayMessage::user(test_uuid(1), "alice", "recent", now)],
        );

        let removed = remove_expired_messages(&rooms, &mut room_messages);
        assert!(!removed);
        assert_eq!(room_messages[&test_uuid(1)].len(), 1);
    }

    fn make_member(id: u128) -> RoomMember {
        RoomMember {
            user_id: test_uuid(id),
            username: format!("user{id}"),
            alias: None,
            role: "member".into(),
            signing_key_fingerprint: None,
        }
    }

    #[test]
    fn test_room_trust_level_all_verified() {
        let members = vec![make_member(1), make_member(2), make_member(3)];
        let mut status = std::collections::HashMap::new();
        status.insert(test_uuid(1), VerificationStatus::Verified);
        status.insert(test_uuid(2), VerificationStatus::Verified);
        status.insert(test_uuid(3), VerificationStatus::Verified);
        assert_eq!(
            room_trust_level(&members, &status),
            RoomTrustLevel::Verified
        );
    }

    #[test]
    fn test_room_trust_level_one_unverified() {
        let members = vec![make_member(1), make_member(2)];
        let mut status = std::collections::HashMap::new();
        status.insert(test_uuid(1), VerificationStatus::Verified);
        status.insert(test_uuid(2), VerificationStatus::Unverified);
        assert_eq!(
            room_trust_level(&members, &status),
            RoomTrustLevel::Unverified
        );
    }

    #[test]
    fn test_room_trust_level_one_unknown() {
        let members = vec![make_member(1), make_member(2)];
        let mut status = std::collections::HashMap::new();
        status.insert(test_uuid(1), VerificationStatus::Verified);
        status.insert(test_uuid(2), VerificationStatus::Unknown);
        assert_eq!(
            room_trust_level(&members, &status),
            RoomTrustLevel::Unverified
        );
    }

    #[test]
    fn test_room_trust_level_one_changed() {
        let members = vec![make_member(1), make_member(2), make_member(3)];
        let mut status = std::collections::HashMap::new();
        status.insert(test_uuid(1), VerificationStatus::Verified);
        status.insert(test_uuid(2), VerificationStatus::Changed);
        status.insert(test_uuid(3), VerificationStatus::Verified);
        assert_eq!(room_trust_level(&members, &status), RoomTrustLevel::Risky);
    }

    #[test]
    fn test_room_trust_level_missing_entry() {
        let members = vec![make_member(1), make_member(2)];
        let mut status = std::collections::HashMap::new();
        status.insert(test_uuid(1), VerificationStatus::Verified);
        assert_eq!(
            room_trust_level(&members, &status),
            RoomTrustLevel::Unverified
        );
    }

    #[test]
    fn test_room_trust_level_empty_members() {
        let status = std::collections::HashMap::new();
        assert_eq!(room_trust_level(&[], &status), RoomTrustLevel::Verified);
    }
}
