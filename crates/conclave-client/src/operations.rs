mod account;
mod groups;
mod messaging;

pub use account::*;
pub use groups::*;
pub use messaging::*;

use std::collections::HashMap;

use prost::Message;

use crate::api::ApiClient;
use crate::error::{Error, Result};

/// Map a `tokio::task::JoinError` (from `spawn_blocking`) into our error type.
fn map_join_error(error: tokio::task::JoinError) -> Error {
    Error::Other(format!("task join error: {error}"))
}

// ── Result types ─────────────────────────────────────────────────

/// Information about a room loaded from the server.
#[derive(Debug, Clone)]
pub struct RoomInfo {
    pub group_id: i64,
    pub group_name: String,
    pub alias: Option<String>,
    pub members: Vec<MemberInfo>,
    pub mls_group_id: Option<String>,
    pub message_expiry_seconds: i64,
}

impl RoomInfo {
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

/// Information about a group member from the server.
#[derive(Debug, Clone)]
pub struct MemberInfo {
    pub user_id: i64,
    pub username: String,
    pub alias: Option<String>,
    pub role: String,
    pub signing_key_fingerprint: Option<String>,
}

impl MemberInfo {
    pub fn display_name(&self) -> &str {
        self.alias
            .as_deref()
            .filter(|a| !a.is_empty())
            .unwrap_or(&self.username)
    }

    pub fn to_room_member(&self) -> crate::state::RoomMember {
        crate::state::RoomMember {
            user_id: self.user_id,
            username: self.username.clone(),
            alias: self.alias.clone(),
            role: self.role.clone(),
            signing_key_fingerprint: self.signing_key_fingerprint.clone(),
        }
    }
}

/// A decrypted and classified message ready for display.
#[derive(Debug, Clone)]
pub struct ProcessedMessage {
    /// Sender's user ID (0 for system messages).
    pub sender_id: i64,
    /// Fallback display name (alias or username from the server at fetch time).
    pub sender: String,
    pub content: String,
    pub timestamp: i64,
    pub sequence_num: u64,
    /// MLS epoch after processing this message.
    pub epoch: u64,
    pub is_system: bool,
}

impl ProcessedMessage {
    pub fn system(content: String, timestamp: i64, sequence_num: u64, epoch: u64) -> Self {
        Self {
            sender_id: 0,
            sender: String::new(),
            content,
            timestamp,
            sequence_num,
            epoch,
            is_system: true,
        }
    }
}

/// Result of fetching and decrypting messages for a group.
#[derive(Debug, Clone)]
pub struct FetchedMessages {
    pub group_id: i64,
    pub messages: Vec<ProcessedMessage>,
}

/// Result of creating a group.
#[derive(Debug, Clone)]
pub struct GroupCreatedResult {
    pub server_group_id: i64,
    pub mls_group_id: String,
}

/// Result of processing a welcome (joining a group via invitation).
#[derive(Debug, Clone)]
pub struct WelcomeJoinResult {
    pub group_id: i64,
    pub group_alias: Option<String>,
    pub mls_group_id: String,
}

/// Result of sending a message.
#[derive(Debug, Clone)]
pub struct MessageSentResult {
    pub group_id: i64,
    pub sequence_num: u64,
    /// MLS epoch at the time the message was sent.
    pub epoch: u64,
}

/// Result of an account reset.
#[derive(Debug, Clone)]
pub struct ResetResult {
    pub new_group_mapping: HashMap<i64, String>,
    pub rejoin_count: usize,
    pub total_groups: usize,
    pub errors: Vec<String>,
}

/// SSE server event decoded from hex+protobuf wire format.
#[derive(Debug, Clone)]
pub enum SseEvent {
    NewMessage {
        group_id: i64,
    },
    Welcome {
        group_id: i64,
        group_alias: String,
    },
    GroupUpdate {
        group_id: i64,
        update_type: String,
    },
    MemberRemoved {
        group_id: i64,
        removed_user_id: i64,
    },
    IdentityReset {
        group_id: i64,
        user_id: i64,
    },
    InviteReceived {
        invite_id: i64,
        group_id: i64,
        group_name: String,
        group_alias: String,
        inviter_id: i64,
    },
    InviteDeclined {
        group_id: i64,
        declined_user_id: i64,
    },
    InviteCancelled {
        group_id: i64,
    },
}

// ── SSE event decoding ───────────────────────────────────────────

/// Decode a hex-encoded protobuf SSE event into a typed `SseEvent`.
pub fn decode_sse_event(hex_data: &str) -> Result<SseEvent> {
    let bytes =
        hex::decode(hex_data).map_err(|e| Error::Other(format!("SSE hex decode failed: {e}")))?;
    let event = conclave_proto::ServerEvent::decode(bytes.as_slice())?;

    match event.event {
        Some(conclave_proto::server_event::Event::NewMessage(msg)) => Ok(SseEvent::NewMessage {
            group_id: msg.group_id,
        }),
        Some(conclave_proto::server_event::Event::Welcome(welcome)) => Ok(SseEvent::Welcome {
            group_id: welcome.group_id,
            group_alias: welcome.group_alias,
        }),
        Some(conclave_proto::server_event::Event::GroupUpdate(update)) => {
            Ok(SseEvent::GroupUpdate {
                group_id: update.group_id,
                update_type: update.update_type,
            })
        }
        Some(conclave_proto::server_event::Event::MemberRemoved(removed)) => {
            Ok(SseEvent::MemberRemoved {
                group_id: removed.group_id,
                removed_user_id: removed.removed_user_id,
            })
        }
        Some(conclave_proto::server_event::Event::IdentityReset(reset)) => {
            Ok(SseEvent::IdentityReset {
                group_id: reset.group_id,
                user_id: reset.user_id,
            })
        }
        Some(conclave_proto::server_event::Event::InviteReceived(invite)) => {
            Ok(SseEvent::InviteReceived {
                invite_id: invite.invite_id,
                group_id: invite.group_id,
                group_name: invite.group_name,
                group_alias: invite.group_alias,
                inviter_id: invite.inviter_id,
            })
        }
        Some(conclave_proto::server_event::Event::InviteDeclined(declined)) => {
            Ok(SseEvent::InviteDeclined {
                group_id: declined.group_id,
                declined_user_id: declined.declined_user_id,
            })
        }
        Some(conclave_proto::server_event::Event::InviteCancelled(cancelled)) => {
            Ok(SseEvent::InviteCancelled {
                group_id: cancelled.group_id,
            })
        }
        None => Err(Error::Other("empty SSE event".into())),
    }
}

// ── Room loading ─────────────────────────────────────────────────

/// Fetch the list of groups from the server and return them as `RoomInfo`.
pub async fn load_rooms(api: &ApiClient) -> Result<Vec<RoomInfo>> {
    let response = api.list_groups().await?;
    Ok(response
        .groups
        .into_iter()
        .map(|group| RoomInfo {
            group_id: group.group_id,
            group_name: group.group_name,
            alias: if group.alias.is_empty() {
                None
            } else {
                Some(group.alias)
            },
            members: group
                .members
                .into_iter()
                .map(|m| MemberInfo {
                    user_id: m.user_id,
                    username: m.username,
                    alias: if m.alias.is_empty() {
                        None
                    } else {
                        Some(m.alias)
                    },
                    role: m.role,
                    signing_key_fingerprint: if m.signing_key_fingerprint.is_empty() {
                        None
                    } else {
                        Some(m.signing_key_fingerprint)
                    },
                })
                .collect(),
            mls_group_id: if group.mls_group_id.is_empty() {
                None
            } else {
                Some(group.mls_group_id)
            },
            message_expiry_seconds: group.message_expiry_seconds,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RoomMember;

    #[test]
    fn test_room_info_display_name_alias() {
        let info = RoomInfo {
            group_id: 1,
            group_name: "devs".into(),
            alias: Some("Dev Team".into()),
            members: vec![],
            mls_group_id: None,
            message_expiry_seconds: -1,
        };
        assert_eq!(info.display_name(), "Dev Team");
    }

    #[test]
    fn test_room_info_display_name_group_name_fallback() {
        let info = RoomInfo {
            group_id: 1,
            group_name: "devs".into(),
            alias: None,
            members: vec![],
            mls_group_id: None,
            message_expiry_seconds: -1,
        };
        assert_eq!(info.display_name(), "devs");
    }

    #[test]
    fn test_room_info_display_name_empty_alias_falls_through() {
        let info = RoomInfo {
            group_id: 1,
            group_name: "devs".into(),
            alias: Some(String::new()),
            members: vec![],
            mls_group_id: None,
            message_expiry_seconds: -1,
        };
        assert_eq!(info.display_name(), "devs");
    }

    #[test]
    fn test_member_info_display_name_with_alias() {
        let info = MemberInfo {
            user_id: 1,
            username: "alice".into(),
            alias: Some("Alice W.".into()),
            role: "admin".into(),
            signing_key_fingerprint: None,
        };
        assert_eq!(info.display_name(), "Alice W.");
    }

    #[test]
    fn test_member_info_display_name_no_alias() {
        let info = MemberInfo {
            user_id: 1,
            username: "alice".into(),
            alias: None,
            role: "member".into(),
            signing_key_fingerprint: None,
        };
        assert_eq!(info.display_name(), "alice");
    }

    #[test]
    fn test_member_info_display_name_empty_alias() {
        let info = MemberInfo {
            user_id: 1,
            username: "alice".into(),
            alias: Some(String::new()),
            role: "member".into(),
            signing_key_fingerprint: None,
        };
        assert_eq!(info.display_name(), "alice");
    }

    #[test]
    fn test_member_info_to_room_member() {
        let info = MemberInfo {
            user_id: 42,
            username: "bob".into(),
            alias: Some("Bobby".into()),
            role: "admin".into(),
            signing_key_fingerprint: Some("abcd1234".into()),
        };
        let member = info.to_room_member();
        assert_eq!(member.user_id, 42);
        assert_eq!(member.username, "bob");
        assert_eq!(member.alias, Some("Bobby".into()));
        assert_eq!(member.role, "admin");
    }

    #[test]
    fn test_decode_sse_event_new_message() {
        let event = conclave_proto::ServerEvent {
            event: Some(conclave_proto::server_event::Event::NewMessage(
                conclave_proto::NewMessageEvent {
                    group_id: 5,
                    sequence_num: 10,
                    sender_id: 1,
                },
            )),
        };
        let mut buf = Vec::new();
        event.encode(&mut buf).unwrap();
        let hex_data = hex::encode(&buf);

        let SseEvent::NewMessage { group_id } = decode_sse_event(&hex_data).unwrap() else {
            panic!("expected NewMessage variant");
        };
        assert_eq!(group_id, 5);
    }

    #[test]
    fn test_decode_sse_event_welcome() {
        let event = conclave_proto::ServerEvent {
            event: Some(conclave_proto::server_event::Event::Welcome(
                conclave_proto::WelcomeEvent {
                    group_id: 3,
                    group_alias: "test-room".into(),
                },
            )),
        };
        let mut buf = Vec::new();
        event.encode(&mut buf).unwrap();
        let hex_data = hex::encode(&buf);

        let SseEvent::Welcome {
            group_id,
            group_alias,
        } = decode_sse_event(&hex_data).unwrap()
        else {
            panic!("expected Welcome variant");
        };
        assert_eq!(group_id, 3);
        assert_eq!(group_alias, "test-room");
    }

    #[test]
    fn test_decode_sse_event_group_update() {
        let event = conclave_proto::ServerEvent {
            event: Some(conclave_proto::server_event::Event::GroupUpdate(
                conclave_proto::GroupUpdateEvent {
                    group_id: 7,
                    update_type: "commit".into(),
                },
            )),
        };
        let mut buf = Vec::new();
        event.encode(&mut buf).unwrap();
        let hex_data = hex::encode(&buf);

        let SseEvent::GroupUpdate {
            group_id,
            update_type,
        } = decode_sse_event(&hex_data).unwrap()
        else {
            panic!("expected GroupUpdate variant");
        };
        assert_eq!(group_id, 7);
        assert_eq!(update_type, "commit");
    }

    #[test]
    fn test_decode_sse_event_member_removed() {
        let event = conclave_proto::ServerEvent {
            event: Some(conclave_proto::server_event::Event::MemberRemoved(
                conclave_proto::MemberRemovedEvent {
                    group_id: 2,
                    removed_user_id: 3,
                },
            )),
        };
        let mut buf = Vec::new();
        event.encode(&mut buf).unwrap();
        let hex_data = hex::encode(&buf);

        let SseEvent::MemberRemoved {
            group_id,
            removed_user_id,
        } = decode_sse_event(&hex_data).unwrap()
        else {
            panic!("expected MemberRemoved variant");
        };
        assert_eq!(group_id, 2);
        assert_eq!(removed_user_id, 3);
    }

    #[test]
    fn test_decode_sse_event_identity_reset() {
        let event = conclave_proto::ServerEvent {
            event: Some(conclave_proto::server_event::Event::IdentityReset(
                conclave_proto::IdentityResetEvent {
                    group_id: 7,
                    user_id: 1,
                },
            )),
        };
        let mut buf = Vec::new();
        event.encode(&mut buf).unwrap();
        let hex_data = hex::encode(&buf);

        let SseEvent::IdentityReset { group_id, user_id } = decode_sse_event(&hex_data).unwrap()
        else {
            panic!("expected IdentityReset variant");
        };
        assert_eq!(group_id, 7);
        assert_eq!(user_id, 1);
    }

    #[test]
    fn test_decode_sse_event_invalid_hex() {
        assert!(decode_sse_event("not-valid-hex!@#").is_err());
    }

    #[test]
    fn test_decode_sse_event_empty_event() {
        let event = conclave_proto::ServerEvent { event: None };
        let mut buf = Vec::new();
        event.encode(&mut buf).unwrap();
        let hex_data = hex::encode(&buf);

        assert!(decode_sse_event(&hex_data).is_err());
    }

    #[test]
    fn test_resolve_user_display_name_found_with_alias() {
        let members = vec![RoomMember {
            user_id: 1,
            username: "alice".into(),
            alias: Some("Alice W.".into()),
            role: "admin".into(),
            signing_key_fingerprint: None,
        }];
        assert_eq!(resolve_user_display_name(Some(1), &members), "Alice W.");
    }

    #[test]
    fn test_resolve_user_display_name_found_no_alias() {
        let members = vec![RoomMember {
            user_id: 1,
            username: "alice".into(),
            alias: None,
            role: "member".into(),
            signing_key_fingerprint: None,
        }];
        assert_eq!(resolve_user_display_name(Some(1), &members), "alice");
    }

    #[test]
    fn test_resolve_user_display_name_not_found() {
        let members = vec![RoomMember {
            user_id: 1,
            username: "alice".into(),
            alias: None,
            role: "member".into(),
            signing_key_fingerprint: None,
        }];
        assert_eq!(resolve_user_display_name(Some(99), &members), "user#99");
    }

    #[test]
    fn test_resolve_user_display_name_none() {
        assert_eq!(resolve_user_display_name(None, &[]), "<unknown>");
    }
}
