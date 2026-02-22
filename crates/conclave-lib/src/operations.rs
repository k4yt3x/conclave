use std::collections::HashMap;
use std::path::Path;

use prost::Message;

use crate::api::ApiClient;
use crate::config::generate_initial_key_packages;
use crate::error::{Error, Result};
use crate::mls::{DecryptedMessage, MlsManager};
use crate::state::RoomMember;

// ── Result types ─────────────────────────────────────────────────

/// Information about a room loaded from the server.
#[derive(Debug, Clone)]
pub struct RoomInfo {
    pub group_id: i64,
    pub group_name: String,
    pub alias: Option<String>,
    pub members: Vec<MemberInfo>,
    pub mls_group_id: Option<String>,
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
}

impl MemberInfo {
    pub fn display_name(&self) -> &str {
        self.alias
            .as_deref()
            .filter(|a| !a.is_empty())
            .unwrap_or(&self.username)
    }

    pub fn to_room_member(&self) -> RoomMember {
        RoomMember {
            user_id: self.user_id,
            username: self.username.clone(),
            alias: self.alias.clone(),
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
        removed_username: String,
    },
    IdentityReset {
        group_id: i64,
        username: String,
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
                removed_username: removed.removed_username,
            })
        }
        Some(conclave_proto::server_event::Event::IdentityReset(reset)) => {
            Ok(SseEvent::IdentityReset {
                group_id: reset.group_id,
                username: reset.username,
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
                })
                .collect(),
            mls_group_id: if group.mls_group_id.is_empty() {
                None
            } else {
                Some(group.mls_group_id)
            },
        })
        .collect())
}

// ── Message operations ───────────────────────────────────────────

/// Fetch messages after `after_sequence` and decrypt them via MLS.
///
/// Each message is classified into a `ProcessedMessage`:
/// - Application → user message
/// - Commit → system messages for adds/removes/key-rotation
/// - Failed → system error message
/// - None → skipped
pub async fn fetch_and_decrypt(
    api: &ApiClient,
    group_id: i64,
    after_sequence: u64,
    mls_group_id: &str,
    data_dir: &Path,
    user_id: i64,
    members: &[RoomMember],
) -> Result<FetchedMessages> {
    let response = api.get_messages(group_id, after_sequence as i64).await?;
    let mut messages = Vec::new();

    for stored_message in &response.messages {
        let data_dir = data_dir.to_path_buf();
        let mls_group_id = mls_group_id.to_string();
        let mls_bytes = stored_message.mls_message.clone();

        let (decrypted, epoch) =
            tokio::task::spawn_blocking(move || match MlsManager::new(&data_dir, user_id) {
                Ok(mls) => {
                    let result = mls.decrypt_message(&mls_group_id, &mls_bytes);
                    let epoch = mls.group_epoch(&mls_group_id).unwrap_or(0);
                    (result, epoch)
                }
                Err(e) => (Err(e), 0),
            })
            .await
            .map_err(|e| Error::Other(format!("task join error: {e}")))?;

        let sender_display = if !stored_message.sender_alias.is_empty() {
            stored_message.sender_alias.clone()
        } else {
            stored_message.sender_username.clone()
        };

        match decrypted {
            Ok(DecryptedMessage::Application(plaintext)) => {
                let text = String::from_utf8_lossy(&plaintext).to_string();
                messages.push(ProcessedMessage {
                    sender_id: stored_message.sender_id,
                    sender: sender_display,
                    content: text,
                    timestamp: stored_message.created_at as i64,
                    sequence_num: stored_message.sequence_num,
                    epoch,
                    is_system: false,
                });
            }
            Ok(DecryptedMessage::Commit(commit_info)) => {
                for added_uid in &commit_info.members_added {
                    let name = resolve_user_display_name(*added_uid, members);
                    messages.push(ProcessedMessage {
                        sender_id: 0,
                        sender: String::new(),
                        content: format!("{name} joined the group"),
                        timestamp: stored_message.created_at as i64,
                        sequence_num: stored_message.sequence_num,
                        epoch,
                        is_system: true,
                    });
                }
                for removed in &commit_info.members_removed {
                    messages.push(ProcessedMessage {
                        sender_id: 0,
                        sender: String::new(),
                        content: format!("{removed} was removed from the group"),
                        timestamp: stored_message.created_at as i64,
                        sequence_num: stored_message.sequence_num,
                        epoch,
                        is_system: true,
                    });
                }
                if commit_info.self_removed {
                    messages.push(ProcessedMessage {
                        sender_id: 0,
                        sender: String::new(),
                        content: "You were removed from this group".to_string(),
                        timestamp: stored_message.created_at as i64,
                        sequence_num: stored_message.sequence_num,
                        epoch,
                        is_system: true,
                    });
                }
                if commit_info.members_added.is_empty()
                    && commit_info.members_removed.is_empty()
                    && !commit_info.self_removed
                {
                    messages.push(ProcessedMessage {
                        sender_id: 0,
                        sender: String::new(),
                        content: "Group keys updated".to_string(),
                        timestamp: stored_message.created_at as i64,
                        sequence_num: stored_message.sequence_num,
                        epoch,
                        is_system: true,
                    });
                }
            }
            Ok(DecryptedMessage::Failed(reason)) => {
                messages.push(ProcessedMessage {
                    sender_id: 0,
                    sender: String::new(),
                    content: format!(
                        "Failed to decrypt message (seq {}): {reason}",
                        stored_message.sequence_num
                    ),
                    timestamp: stored_message.created_at as i64,
                    sequence_num: stored_message.sequence_num,
                    epoch,
                    is_system: true,
                });
            }
            Ok(DecryptedMessage::None) => {}
            Err(error) => {
                tracing::warn!(%error, seq = stored_message.sequence_num, "message decryption failed");
                messages.push(ProcessedMessage {
                    sender_id: 0,
                    sender: String::new(),
                    content: format!(
                        "Failed to decrypt message (seq {}): {error}",
                        stored_message.sequence_num
                    ),
                    timestamp: stored_message.created_at as i64,
                    sequence_num: stored_message.sequence_num,
                    epoch,
                    is_system: true,
                });
            }
        }
    }

    Ok(FetchedMessages { group_id, messages })
}

/// Resolve a user ID from an MLS credential to a display name using the room
/// member list. Falls back to the user ID as a string.
fn resolve_user_display_name(user_id: Option<i64>, members: &[RoomMember]) -> String {
    match user_id {
        Some(uid) => {
            if let Some(member) = members.iter().find(|m| m.user_id == uid) {
                member.display_name().to_string()
            } else {
                format!("user#{uid}")
            }
        }
        None => "<unknown>".to_string(),
    }
}

/// Encrypt a text message via MLS and send it to the server.
pub async fn send_message(
    api: &ApiClient,
    server_group_id: i64,
    mls_group_id: &str,
    text: &str,
    data_dir: &Path,
    user_id: i64,
) -> Result<MessageSentResult> {
    let data_dir = data_dir.to_path_buf();
    let mls_group_id = mls_group_id.to_string();
    let text_bytes = text.as_bytes().to_vec();

    let (encrypted, epoch) = tokio::task::spawn_blocking(move || {
        let mls = MlsManager::new(&data_dir, user_id)?;
        let ciphertext = mls.encrypt_message(&mls_group_id, &text_bytes)?;
        let epoch = mls.group_epoch(&mls_group_id).unwrap_or(0);
        Ok::<_, Error>((ciphertext, epoch))
    })
    .await
    .map_err(|e| Error::Other(format!("task join error: {e}")))??;

    let response = api.send_message(server_group_id, encrypted).await?;

    Ok(MessageSentResult {
        group_id: server_group_id,
        sequence_num: response.sequence_num,
        epoch,
    })
}

// ── Group management operations ──────────────────────────────────

/// Create a new MLS group: request key packages from the server, perform the
/// MLS group creation, and upload the commit + welcome messages.
pub async fn create_group(
    api: &ApiClient,
    alias: Option<&str>,
    group_name: &str,
    members: Vec<String>,
    data_dir: &Path,
    user_id: i64,
) -> Result<GroupCreatedResult> {
    let response = api.create_group(alias, group_name, members).await?;
    let server_group_id = response.group_id;
    let member_key_packages = response.member_key_packages;

    let data_dir = data_dir.to_path_buf();

    let result = tokio::task::spawn_blocking(move || {
        let mls = MlsManager::new(&data_dir, user_id)?;
        mls.create_group(&member_key_packages)
    })
    .await
    .map_err(|e| Error::Other(format!("task join error: {e}")))??;

    api.upload_commit(
        server_group_id,
        result.commit,
        result.welcomes,
        result.group_info,
        Some(&result.mls_group_id),
    )
    .await?;

    Ok(GroupCreatedResult {
        server_group_id,
        mls_group_id: result.mls_group_id,
    })
}

/// Invite members to an existing group: fetch their key packages, perform the
/// MLS invite, and upload the commit + welcome messages.
///
/// Returns the list of usernames that were actually invited (empty if all were
/// already members).
pub async fn invite_members(
    api: &ApiClient,
    server_group_id: i64,
    mls_group_id: &str,
    members: Vec<String>,
    data_dir: &Path,
    user_id: i64,
) -> Result<Vec<String>> {
    let response = api.invite_to_group(server_group_id, members).await?;

    if response.member_key_packages.is_empty() {
        return Ok(vec![]);
    }

    let invited: Vec<String> = response.member_key_packages.keys().cloned().collect();
    let member_key_packages = response.member_key_packages;

    let data_dir = data_dir.to_path_buf();
    let mls_group_id = mls_group_id.to_string();

    let result = tokio::task::spawn_blocking(move || {
        let mls = MlsManager::new(&data_dir, user_id)?;
        mls.invite_to_group(&mls_group_id, &member_key_packages)
    })
    .await
    .map_err(|e| Error::Other(format!("task join error: {e}")))??;

    api.upload_commit(
        server_group_id,
        result.commit,
        result.welcomes,
        result.group_info,
        None,
    )
    .await?;

    Ok(invited)
}

/// Remove a member from the group: find their MLS leaf index, produce a
/// removal commit, and notify the server.
pub async fn kick_member(
    api: &ApiClient,
    server_group_id: i64,
    mls_group_id: &str,
    target_username: &str,
    target_user_id: i64,
    data_dir: &Path,
    user_id: i64,
) -> Result<()> {
    let data_dir = data_dir.to_path_buf();
    let mls_group_id = mls_group_id.to_string();
    let target_username = target_username.to_string();

    let (commit_bytes, group_info_bytes) = tokio::task::spawn_blocking({
        let target = target_username.clone();
        move || {
            let mls = MlsManager::new(&data_dir, user_id)?;
            let member_index = mls
                .find_member_index(&mls_group_id, target_user_id)?
                .ok_or_else(|| Error::Other(format!("user '{target}' not found in MLS roster")))?;
            mls.remove_member(&mls_group_id, member_index)
        }
    })
    .await
    .map_err(|e| Error::Other(format!("task join error: {e}")))??;

    api.remove_member(
        server_group_id,
        &target_username,
        commit_bytes,
        group_info_bytes,
    )
    .await?;

    Ok(())
}

/// Rotate the MLS keys for the active group (epoch advancement for forward
/// secrecy) and upload the commit.
pub async fn rotate_keys(
    api: &ApiClient,
    server_group_id: i64,
    mls_group_id: &str,
    data_dir: &Path,
    user_id: i64,
) -> Result<()> {
    let data_dir = data_dir.to_path_buf();
    let mls_group_id = mls_group_id.to_string();

    let (commit_bytes, group_info_bytes) = tokio::task::spawn_blocking(move || {
        let mls = MlsManager::new(&data_dir, user_id)?;
        mls.rotate_keys(&mls_group_id)
    })
    .await
    .map_err(|e| Error::Other(format!("task join error: {e}")))??;

    api.upload_commit(
        server_group_id,
        commit_bytes,
        HashMap::new(),
        group_info_bytes,
        None,
    )
    .await?;

    Ok(())
}

// ── Lifecycle operations ─────────────────────────────────────────

/// Leave a group: produce an MLS self-remove commit, notify the server, and
/// delete local MLS group state.
pub async fn leave_group(
    api: &ApiClient,
    server_group_id: i64,
    mls_group_id: Option<&str>,
    data_dir: &Path,
    user_id: i64,
) -> Result<()> {
    let (commit_bytes, group_info_bytes) = if let Some(mls_gid) = mls_group_id {
        let data_dir = data_dir.to_path_buf();
        let mls_gid = mls_gid.to_string();

        match tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir, user_id)?;
            mls.leave_group(&mls_gid)
        })
        .await
        .map_err(|e| Error::Other(format!("task join error: {e}")))?
        {
            Ok(Some(data)) => data,
            Ok(None) | Err(_) => (Vec::new(), Vec::new()),
        }
    } else {
        (Vec::new(), Vec::new())
    };

    api.leave_group(server_group_id, commit_bytes, group_info_bytes)
        .await?;

    // Delete local MLS group state after the server has been notified.
    if let Some(mls_gid) = mls_group_id {
        let data_dir = data_dir.to_path_buf();
        let mls_gid = mls_gid.to_string();

        match tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir, user_id)?;
            mls.delete_group_state(&mls_gid)
        })
        .await
        {
            Ok(Err(error)) => {
                tracing::warn!(%error, "failed to delete MLS group state");
            }
            Err(error) => {
                tracing::warn!(%error, "MLS group state deletion task panicked");
            }
            Ok(Ok(())) => {}
        }
    }

    Ok(())
}

/// Delete local MLS group state for a single group.
pub async fn delete_mls_group_state(
    mls_group_id: &str,
    data_dir: &Path,
    user_id: i64,
) -> Result<()> {
    let data_dir = data_dir.to_path_buf();
    let mls_group_id = mls_group_id.to_string();

    tokio::task::spawn_blocking(move || {
        let mls = MlsManager::new(&data_dir, user_id)?;
        mls.delete_group_state(&mls_group_id)
    })
    .await
    .map_err(|e| Error::Other(format!("task join error: {e}")))?
}

/// Accept all pending welcome messages (group invitations): join each group
/// via MLS, acknowledge the welcome on the server, and replenish consumed key
/// packages per RFC 9420 §10.
pub async fn accept_welcomes(
    api: &ApiClient,
    data_dir: &Path,
    user_id: i64,
) -> Result<Vec<WelcomeJoinResult>> {
    let response = api.list_pending_welcomes().await?;

    if response.welcomes.is_empty() {
        return Ok(vec![]);
    }

    let mut results = Vec::new();

    for welcome in &response.welcomes {
        let data_dir = data_dir.to_path_buf();
        let welcome_bytes = welcome.welcome_message.clone();

        let mls_group_id = tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir, user_id)?;
            mls.join_group(&welcome_bytes)
        })
        .await
        .map_err(|e| Error::Other(format!("task join error: {e}")))?;

        let mls_group_id = match mls_group_id {
            Ok(id) => id,
            Err(error) => {
                tracing::warn!(%error, "failed to join group via welcome");
                continue;
            }
        };

        if let Err(error) = api.accept_welcome(welcome.welcome_id).await {
            tracing::warn!(%error, "failed to acknowledge welcome");
        }

        results.push(WelcomeJoinResult {
            group_id: welcome.group_id,
            group_alias: if welcome.group_alias.is_empty() {
                None
            } else {
                Some(welcome.group_alias.clone())
            },
            mls_group_id,
        });
    }

    // Key packages are single-use (RFC 9420 §10); replenish one per welcome
    // consumed to maintain availability for future group invitations.
    let consumed_count = results.len();
    if consumed_count > 0 {
        let data_dir = data_dir.to_path_buf();

        let replacements = tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir, user_id)?;
            mls.generate_key_packages(consumed_count)
        })
        .await
        .map_err(|e| Error::Other(format!("task join error: {e}")))??;

        let entries: Vec<(Vec<u8>, bool)> =
            replacements.into_iter().map(|kp| (kp, false)).collect();
        if !entries.is_empty() {
            api.upload_key_packages(entries).await?;
        }
    }

    Ok(results)
}

/// Reset the account: wipe all local MLS state, regenerate identity and key
/// packages, then rejoin each group via external commit.
///
/// Groups are discovered from the server (not from local state), so this works
/// even when the user has lost their local data directory.
pub async fn reset_account(api: &ApiClient, data_dir: &Path, user_id: i64) -> Result<ResetResult> {
    // Step 1: Fetch group list from the server.
    let rooms = load_rooms(api).await?;
    let groups_to_rejoin: Vec<i64> = rooms.iter().map(|r| r.group_id).collect();
    let total_groups = groups_to_rejoin.len();

    // Step 2: Collect old leaf indices before wiping state (best-effort;
    // the mapping and MLS state may be missing after data loss).
    let old_indices: HashMap<i64, Option<u32>> = {
        let data_dir = data_dir.to_path_buf();
        let groups = groups_to_rejoin.clone();
        let group_mapping = crate::config::build_group_mapping(&rooms, &data_dir);

        tokio::task::spawn_blocking(move || {
            let mls = match MlsManager::new(&data_dir, user_id) {
                Ok(mls) => mls,
                Err(_) => return Ok(HashMap::new()),
            };
            let mut indices = HashMap::new();
            for server_id in &groups {
                if let Some(mls_id) = group_mapping.get(server_id) {
                    let index = mls.find_member_index(mls_id, user_id).ok().flatten();
                    indices.insert(*server_id, index);
                }
            }
            Ok::<_, Error>(indices)
        })
        .await
        .map_err(|e| Error::Other(format!("task join error: {e}")))?
    }?;

    // Step 3: Notify server to clear our key packages.
    api.reset_account().await?;

    // Step 4: Wipe local MLS state.
    {
        let data_dir = data_dir.to_path_buf();

        tokio::task::spawn_blocking(move || {
            let mls = match MlsManager::new(&data_dir, user_id) {
                Ok(mls) => mls,
                Err(_) => return Ok(()),
            };
            mls.wipe_local_state()
        })
        .await
        .map_err(|e| Error::Other(format!("task join error: {e}")))?
    }?;

    // Step 5: Regenerate identity and upload new key packages.
    {
        let data_dir = data_dir.to_path_buf();

        let entries = tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir, user_id)?;
            generate_initial_key_packages(&mls)
        })
        .await
        .map_err(|e| Error::Other(format!("task join error: {e}")))??;

        api.upload_key_packages(entries).await?;
    }

    // Step 6: Rejoin each group via external commit.
    let mut new_group_mapping = HashMap::new();
    let mut errors = Vec::new();
    let mut rejoin_count = 0;

    for server_group_id in &groups_to_rejoin {
        let server_group_id = *server_group_id;

        let group_info_response = match api.get_group_info(server_group_id).await {
            Ok(response) => response,
            Err(error) => {
                errors.push(format!(
                    "Failed to get group info for {server_group_id}: {error}"
                ));
                continue;
            }
        };

        let old_index = old_indices.get(&server_group_id).copied().flatten();
        let group_info_bytes = group_info_response.group_info.clone();

        let data_dir = data_dir.to_path_buf();

        let rejoin_result = tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir, user_id)?;
            mls.external_rejoin_group(&group_info_bytes, old_index)
        })
        .await
        .map_err(|e| Error::Other(format!("task join error: {e}")))?;

        match rejoin_result {
            Ok((new_mls_id, commit_bytes)) => {
                if let Err(error) = api
                    .external_join(server_group_id, commit_bytes, &new_mls_id)
                    .await
                {
                    errors.push(format!("Failed to rejoin {server_group_id}: {error}"));
                    continue;
                }
                new_group_mapping.insert(server_group_id, new_mls_id);
                rejoin_count += 1;
            }
            Err(error) => {
                errors.push(format!(
                    "Failed external commit for {server_group_id}: {error}"
                ));
            }
        }
    }

    Ok(ResetResult {
        new_group_mapping,
        rejoin_count,
        total_groups,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_room_info_display_name_alias() {
        let info = RoomInfo {
            group_id: 1,
            group_name: "devs".into(),
            alias: Some("Dev Team".into()),
            members: vec![],
            mls_group_id: None,
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
        };
        assert_eq!(info.display_name(), "devs");
    }

    #[test]
    fn test_member_info_display_name_with_alias() {
        let info = MemberInfo {
            user_id: 1,
            username: "alice".into(),
            alias: Some("Alice W.".into()),
        };
        assert_eq!(info.display_name(), "Alice W.");
    }

    #[test]
    fn test_member_info_display_name_no_alias() {
        let info = MemberInfo {
            user_id: 1,
            username: "alice".into(),
            alias: None,
        };
        assert_eq!(info.display_name(), "alice");
    }

    #[test]
    fn test_member_info_display_name_empty_alias() {
        let info = MemberInfo {
            user_id: 1,
            username: "alice".into(),
            alias: Some(String::new()),
        };
        assert_eq!(info.display_name(), "alice");
    }

    #[test]
    fn test_member_info_to_room_member() {
        let info = MemberInfo {
            user_id: 42,
            username: "bob".into(),
            alias: Some("Bobby".into()),
        };
        let member = info.to_room_member();
        assert_eq!(member.user_id, 42);
        assert_eq!(member.username, "bob");
        assert_eq!(member.alias, Some("Bobby".into()));
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

        match decode_sse_event(&hex_data).unwrap() {
            SseEvent::NewMessage { group_id } => assert_eq!(group_id, 5),
            other => panic!("expected NewMessage, got {other:?}"),
        }
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

        match decode_sse_event(&hex_data).unwrap() {
            SseEvent::Welcome {
                group_id,
                group_alias,
            } => {
                assert_eq!(group_id, 3);
                assert_eq!(group_alias, "test-room");
            }
            other => panic!("expected Welcome, got {other:?}"),
        }
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

        match decode_sse_event(&hex_data).unwrap() {
            SseEvent::GroupUpdate {
                group_id,
                update_type,
            } => {
                assert_eq!(group_id, 7);
                assert_eq!(update_type, "commit");
            }
            other => panic!("expected GroupUpdate, got {other:?}"),
        }
    }

    #[test]
    fn test_decode_sse_event_member_removed() {
        let event = conclave_proto::ServerEvent {
            event: Some(conclave_proto::server_event::Event::MemberRemoved(
                conclave_proto::MemberRemovedEvent {
                    group_id: 2,
                    removed_username: "charlie".into(),
                },
            )),
        };
        let mut buf = Vec::new();
        event.encode(&mut buf).unwrap();
        let hex_data = hex::encode(&buf);

        match decode_sse_event(&hex_data).unwrap() {
            SseEvent::MemberRemoved {
                group_id,
                removed_username,
            } => {
                assert_eq!(group_id, 2);
                assert_eq!(removed_username, "charlie");
            }
            other => panic!("expected MemberRemoved, got {other:?}"),
        }
    }

    #[test]
    fn test_decode_sse_event_identity_reset() {
        let event = conclave_proto::ServerEvent {
            event: Some(conclave_proto::server_event::Event::IdentityReset(
                conclave_proto::IdentityResetEvent {
                    group_id: 7,
                    username: "alice".into(),
                },
            )),
        };
        let mut buf = Vec::new();
        event.encode(&mut buf).unwrap();
        let hex_data = hex::encode(&buf);

        match decode_sse_event(&hex_data).unwrap() {
            SseEvent::IdentityReset { group_id, username } => {
                assert_eq!(group_id, 7);
                assert_eq!(username, "alice");
            }
            other => panic!("expected IdentityReset, got {other:?}"),
        }
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
        }];
        assert_eq!(resolve_user_display_name(Some(1), &members), "Alice W.");
    }

    #[test]
    fn test_resolve_user_display_name_found_no_alias() {
        let members = vec![RoomMember {
            user_id: 1,
            username: "alice".into(),
            alias: None,
        }];
        assert_eq!(resolve_user_display_name(Some(1), &members), "alice");
    }

    #[test]
    fn test_resolve_user_display_name_not_found() {
        let members = vec![RoomMember {
            user_id: 1,
            username: "alice".into(),
            alias: None,
        }];
        assert_eq!(resolve_user_display_name(Some(99), &members), "user#99");
    }

    #[test]
    fn test_resolve_user_display_name_none() {
        assert_eq!(resolve_user_display_name(None, &[]), "<unknown>");
    }
}
