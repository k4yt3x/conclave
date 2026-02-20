use std::collections::HashMap;
use std::path::Path;

use prost::Message;

use crate::api::ApiClient;
use crate::config::generate_initial_key_packages;
use crate::error::{Error, Result};
use crate::mls::{DecryptedMessage, MlsManager};

// ── Result types ─────────────────────────────────────────────────

/// Information about a room loaded from the server.
#[derive(Debug, Clone)]
pub struct RoomInfo {
    pub group_id: String,
    pub name: String,
    pub members: Vec<String>,
}

/// A decrypted and classified message ready for display.
#[derive(Debug, Clone)]
pub struct ProcessedMessage {
    pub sender: String,
    pub content: String,
    pub timestamp: i64,
    pub sequence_num: u64,
    pub is_system: bool,
}

/// Result of fetching and decrypting messages for a group.
#[derive(Debug, Clone)]
pub struct FetchedMessages {
    pub group_id: String,
    pub messages: Vec<ProcessedMessage>,
}

/// Result of creating a group.
#[derive(Debug, Clone)]
pub struct GroupCreatedResult {
    pub server_group_id: String,
    pub mls_group_id: String,
}

/// Result of processing a welcome (joining a group via invitation).
#[derive(Debug, Clone)]
pub struct WelcomeJoinResult {
    pub group_id: String,
    pub group_name: String,
    pub mls_group_id: String,
}

/// Result of sending a message.
#[derive(Debug, Clone)]
pub struct MessageSentResult {
    pub group_id: String,
    pub sequence_num: u64,
}

/// Result of an account reset.
#[derive(Debug, Clone)]
pub struct ResetResult {
    pub new_group_mapping: HashMap<String, String>,
    pub rejoin_count: usize,
    pub total_groups: usize,
    pub errors: Vec<String>,
}

/// SSE server event decoded from hex+protobuf wire format.
#[derive(Debug, Clone)]
pub enum SseEvent {
    NewMessage {
        group_id: String,
    },
    Welcome {
        group_id: String,
        group_name: String,
    },
    GroupUpdate {
        group_id: String,
        update_type: String,
    },
    MemberRemoved {
        group_id: String,
        removed_username: String,
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
            group_name: welcome.group_name,
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
            name: group.name,
            members: group.members.into_iter().map(|m| m.username).collect(),
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
    group_id: &str,
    after_sequence: u64,
    mls_group_id: &str,
    data_dir: &Path,
    username: &str,
) -> Result<FetchedMessages> {
    let response = api.get_messages(group_id, after_sequence as i64).await?;
    let mut messages = Vec::new();

    for stored_message in &response.messages {
        let data_dir = data_dir.to_path_buf();
        let username = username.to_string();
        let mls_group_id = mls_group_id.to_string();
        let mls_bytes = stored_message.mls_message.clone();

        let decrypted = tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir, &username)?;
            mls.decrypt_message(&mls_group_id, &mls_bytes)
        })
        .await
        .map_err(|e| Error::Other(format!("task join error: {e}")))?;

        match decrypted {
            Ok(DecryptedMessage::Application(plaintext)) => {
                let text = String::from_utf8_lossy(&plaintext).to_string();
                messages.push(ProcessedMessage {
                    sender: stored_message.sender_username.clone(),
                    content: text,
                    timestamp: stored_message.created_at as i64,
                    sequence_num: stored_message.sequence_num,
                    is_system: false,
                });
            }
            Ok(DecryptedMessage::Commit(commit_info)) => {
                for added in &commit_info.members_added {
                    messages.push(ProcessedMessage {
                        sender: String::new(),
                        content: format!("{added} joined the group"),
                        timestamp: stored_message.created_at as i64,
                        sequence_num: stored_message.sequence_num,
                        is_system: true,
                    });
                }
                for removed in &commit_info.members_removed {
                    messages.push(ProcessedMessage {
                        sender: String::new(),
                        content: format!("{removed} was removed from the group"),
                        timestamp: stored_message.created_at as i64,
                        sequence_num: stored_message.sequence_num,
                        is_system: true,
                    });
                }
                if commit_info.self_removed {
                    messages.push(ProcessedMessage {
                        sender: String::new(),
                        content: "You were removed from this group".to_string(),
                        timestamp: stored_message.created_at as i64,
                        sequence_num: stored_message.sequence_num,
                        is_system: true,
                    });
                }
                if commit_info.members_added.is_empty()
                    && commit_info.members_removed.is_empty()
                    && !commit_info.self_removed
                {
                    messages.push(ProcessedMessage {
                        sender: String::new(),
                        content: "Group keys updated".to_string(),
                        timestamp: stored_message.created_at as i64,
                        sequence_num: stored_message.sequence_num,
                        is_system: true,
                    });
                }
            }
            Ok(DecryptedMessage::Failed(reason)) => {
                messages.push(ProcessedMessage {
                    sender: String::new(),
                    content: format!(
                        "Failed to decrypt message (seq {}): {reason}",
                        stored_message.sequence_num
                    ),
                    timestamp: stored_message.created_at as i64,
                    sequence_num: stored_message.sequence_num,
                    is_system: true,
                });
            }
            Ok(DecryptedMessage::None) => {}
            Err(error) => {
                tracing::warn!(%error, seq = stored_message.sequence_num, "message decryption failed");
                messages.push(ProcessedMessage {
                    sender: String::new(),
                    content: format!(
                        "Failed to decrypt message (seq {}): {error}",
                        stored_message.sequence_num
                    ),
                    timestamp: stored_message.created_at as i64,
                    sequence_num: stored_message.sequence_num,
                    is_system: true,
                });
            }
        }
    }

    Ok(FetchedMessages {
        group_id: group_id.to_string(),
        messages,
    })
}

/// Encrypt a text message via MLS and send it to the server.
pub async fn send_message(
    api: &ApiClient,
    server_group_id: &str,
    mls_group_id: &str,
    text: &str,
    data_dir: &Path,
    username: &str,
) -> Result<MessageSentResult> {
    let data_dir = data_dir.to_path_buf();
    let username = username.to_string();
    let mls_group_id = mls_group_id.to_string();
    let text_bytes = text.as_bytes().to_vec();

    let encrypted = tokio::task::spawn_blocking(move || {
        let mls = MlsManager::new(&data_dir, &username)?;
        mls.encrypt_message(&mls_group_id, &text_bytes)
    })
    .await
    .map_err(|e| Error::Other(format!("task join error: {e}")))??;

    let response = api.send_message(server_group_id, encrypted).await?;

    Ok(MessageSentResult {
        group_id: server_group_id.to_string(),
        sequence_num: response.sequence_num,
    })
}

// ── Group management operations ──────────────────────────────────

/// Create a new MLS group: request key packages from the server, perform the
/// MLS group creation, and upload the commit + welcome messages.
pub async fn create_group(
    api: &ApiClient,
    name: &str,
    members: Vec<String>,
    data_dir: &Path,
    username: &str,
) -> Result<GroupCreatedResult> {
    let response = api.create_group(name, members).await?;
    let server_group_id = response.group_id.clone();
    let member_key_packages = response.member_key_packages;

    let data_dir = data_dir.to_path_buf();
    let username = username.to_string();

    let (mls_group_id, commit_bytes, welcome_map, group_info_bytes) =
        tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir, &username)?;
            mls.create_group(&member_key_packages)
        })
        .await
        .map_err(|e| Error::Other(format!("task join error: {e}")))??;

    api.upload_commit(
        &server_group_id,
        commit_bytes,
        welcome_map,
        group_info_bytes,
    )
    .await?;

    Ok(GroupCreatedResult {
        server_group_id,
        mls_group_id,
    })
}

/// Invite members to an existing group: fetch their key packages, perform the
/// MLS invite, and upload the commit + welcome messages.
///
/// Returns the list of usernames that were actually invited (empty if all were
/// already members).
pub async fn invite_members(
    api: &ApiClient,
    server_group_id: &str,
    mls_group_id: &str,
    members: Vec<String>,
    data_dir: &Path,
    username: &str,
) -> Result<Vec<String>> {
    let response = api.invite_to_group(server_group_id, members).await?;

    if response.member_key_packages.is_empty() {
        return Ok(vec![]);
    }

    let invited: Vec<String> = response.member_key_packages.keys().cloned().collect();
    let member_key_packages = response.member_key_packages;

    let data_dir = data_dir.to_path_buf();
    let username = username.to_string();
    let mls_group_id = mls_group_id.to_string();
    let server_group_id = server_group_id.to_string();

    let (commit_bytes, welcome_map, group_info_bytes) = tokio::task::spawn_blocking(move || {
        let mls = MlsManager::new(&data_dir, &username)?;
        mls.invite_to_group(&mls_group_id, &member_key_packages)
    })
    .await
    .map_err(|e| Error::Other(format!("task join error: {e}")))??;

    api.upload_commit(
        &server_group_id,
        commit_bytes,
        welcome_map,
        group_info_bytes,
    )
    .await?;

    Ok(invited)
}

/// Remove a member from the group: find their MLS leaf index, produce a
/// removal commit, and notify the server.
pub async fn kick_member(
    api: &ApiClient,
    server_group_id: &str,
    mls_group_id: &str,
    target_username: &str,
    data_dir: &Path,
    username: &str,
) -> Result<()> {
    let data_dir = data_dir.to_path_buf();
    let username = username.to_string();
    let mls_group_id = mls_group_id.to_string();
    let target_username = target_username.to_string();

    let (commit_bytes, group_info_bytes) = tokio::task::spawn_blocking({
        let target = target_username.clone();
        move || {
            let mls = MlsManager::new(&data_dir, &username)?;
            let member_index = mls
                .find_member_index(&mls_group_id, &target)?
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
    server_group_id: &str,
    mls_group_id: &str,
    data_dir: &Path,
    username: &str,
) -> Result<()> {
    let data_dir = data_dir.to_path_buf();
    let username = username.to_string();
    let mls_group_id = mls_group_id.to_string();

    let (commit_bytes, group_info_bytes) = tokio::task::spawn_blocking(move || {
        let mls = MlsManager::new(&data_dir, &username)?;
        mls.rotate_keys(&mls_group_id)
    })
    .await
    .map_err(|e| Error::Other(format!("task join error: {e}")))??;

    api.upload_commit(
        server_group_id,
        commit_bytes,
        HashMap::new(),
        group_info_bytes,
    )
    .await?;

    Ok(())
}

// ── Lifecycle operations ─────────────────────────────────────────

/// Leave a group: produce an MLS self-remove commit, notify the server, and
/// delete local MLS group state.
pub async fn leave_group(
    api: &ApiClient,
    server_group_id: &str,
    mls_group_id: Option<&str>,
    data_dir: &Path,
    username: &str,
) -> Result<()> {
    let (commit_bytes, group_info_bytes) = if let Some(mls_gid) = mls_group_id {
        let data_dir = data_dir.to_path_buf();
        let username = username.to_string();
        let mls_gid = mls_gid.to_string();

        match tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir, &username)?;
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
        let username = username.to_string();
        let mls_gid = mls_gid.to_string();

        match tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir, &username)?;
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
    username: &str,
) -> Result<()> {
    let data_dir = data_dir.to_path_buf();
    let username = username.to_string();
    let mls_group_id = mls_group_id.to_string();

    tokio::task::spawn_blocking(move || {
        let mls = MlsManager::new(&data_dir, &username)?;
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
    username: &str,
) -> Result<Vec<WelcomeJoinResult>> {
    let response = api.list_pending_welcomes().await?;

    if response.welcomes.is_empty() {
        return Ok(vec![]);
    }

    let mut results = Vec::new();

    for welcome in &response.welcomes {
        let data_dir = data_dir.to_path_buf();
        let username = username.to_string();
        let welcome_bytes = welcome.welcome_message.clone();

        let mls_group_id = tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir, &username)?;
            mls.join_group(&welcome_bytes)
        })
        .await
        .map_err(|e| Error::Other(format!("task join error: {e}")))??;

        if let Err(error) = api.accept_welcome(welcome.welcome_id).await {
            tracing::warn!(%error, "failed to acknowledge welcome");
        }

        results.push(WelcomeJoinResult {
            group_id: welcome.group_id.clone(),
            group_name: welcome.group_name.clone(),
            mls_group_id,
        });
    }

    // Key packages are single-use (RFC 9420 §10); replenish one per welcome
    // consumed to maintain availability for future group invitations.
    let consumed_count = results.len();
    if consumed_count > 0 {
        let data_dir = data_dir.to_path_buf();
        let username = username.to_string();

        let replacements = tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir, &username)?;
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
pub async fn reset_account(
    api: &ApiClient,
    group_mapping: &HashMap<String, String>,
    data_dir: &Path,
    username: &str,
) -> Result<ResetResult> {
    let groups_to_rejoin: Vec<(String, String)> = group_mapping
        .iter()
        .map(|(server_id, mls_id)| (server_id.clone(), mls_id.clone()))
        .collect();
    let total_groups = groups_to_rejoin.len();

    // Step 1: Collect old leaf indices before wiping state.
    let old_indices: HashMap<String, Option<u32>> = {
        let data_dir = data_dir.to_path_buf();
        let username = username.to_string();
        let groups = groups_to_rejoin.clone();

        tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir, &username)?;
            let mut indices = HashMap::new();
            for (server_id, mls_id) in &groups {
                let index = mls.find_member_index(mls_id, &username).ok().flatten();
                indices.insert(server_id.clone(), index);
            }
            Ok::<_, Error>(indices)
        })
        .await
        .map_err(|e| Error::Other(format!("task join error: {e}")))?
    }?;

    // Step 2: Notify server to clear our key packages.
    api.reset_account().await?;

    // Step 3: Wipe local MLS state.
    {
        let data_dir = data_dir.to_path_buf();
        let username = username.to_string();

        tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir, &username)?;
            mls.wipe_local_state()
        })
        .await
        .map_err(|e| Error::Other(format!("task join error: {e}")))?
    }?;

    // Step 4: Regenerate identity and upload new key packages.
    {
        let data_dir = data_dir.to_path_buf();
        let username = username.to_string();

        let entries = tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir, &username)?;
            generate_initial_key_packages(&mls)
        })
        .await
        .map_err(|e| Error::Other(format!("task join error: {e}")))??;

        api.upload_key_packages(entries).await?;
    }

    // Step 5: Rejoin each group via external commit.
    let mut new_group_mapping = HashMap::new();
    let mut errors = Vec::new();
    let mut rejoin_count = 0;

    for (server_id, _) in &groups_to_rejoin {
        let group_info_response = match api.get_group_info(server_id).await {
            Ok(response) => response,
            Err(error) => {
                errors.push(format!("Failed to get group info for {server_id}: {error}"));
                continue;
            }
        };

        let old_index = old_indices.get(server_id).copied().flatten();
        let group_info_bytes = group_info_response.group_info.clone();

        let data_dir = data_dir.to_path_buf();
        let username = username.to_string();

        let rejoin_result = tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir, &username)?;
            mls.external_rejoin_group(&group_info_bytes, old_index)
        })
        .await
        .map_err(|e| Error::Other(format!("task join error: {e}")))?;

        match rejoin_result {
            Ok((new_mls_id, commit_bytes)) => {
                if let Err(error) = api.external_join(server_id, commit_bytes).await {
                    errors.push(format!("Failed to rejoin {server_id}: {error}"));
                    continue;
                }
                new_group_mapping.insert(server_id.clone(), new_mls_id);
                rejoin_count += 1;
            }
            Err(error) => {
                errors.push(format!("Failed external commit for {server_id}: {error}"));
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
