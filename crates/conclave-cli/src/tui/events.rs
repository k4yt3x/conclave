use std::path::PathBuf;
use std::sync::Arc;

use prost::Message;
use tokio::sync::Mutex;

use conclave_lib::api::ApiClient;
use conclave_lib::config::save_group_mapping;
use conclave_lib::error::{Error, Result};
use conclave_lib::mls::{DecryptedMessage, MlsManager};

use super::commands;
use super::state::{AppState, DisplayMessage};

/// Handle an SSE message (hex-encoded protobuf ServerEvent).
/// Returns a list of (group_id, DisplayMessage) pairs to render.
pub async fn handle_sse_message(
    hex_data: &str,
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    data_dir: &PathBuf,
) -> Result<Vec<(String, DisplayMessage)>> {
    let bytes =
        hex::decode(hex_data).map_err(|e| Error::Other(format!("SSE hex decode failed: {e}")))?;
    let event = conclave_proto::ServerEvent::decode(bytes.as_slice())?;

    match event.event {
        Some(conclave_proto::server_event::Event::NewMessage(msg_event)) => {
            handle_new_message(msg_event, api, state, data_dir).await
        }
        Some(conclave_proto::server_event::Event::Welcome(welcome_event)) => {
            handle_welcome(welcome_event, api, state, data_dir).await
        }
        Some(conclave_proto::server_event::Event::GroupUpdate(update_event)) => {
            handle_group_update(update_event, state).await
        }
        Some(conclave_proto::server_event::Event::MemberRemoved(removed_event)) => {
            handle_member_removed(removed_event, api, state, data_dir).await
        }
        None => Ok(vec![]),
    }
}

async fn handle_new_message(
    event: conclave_proto::NewMessageEvent,
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    data_dir: &PathBuf,
) -> Result<Vec<(String, DisplayMessage)>> {
    let group_id = event.group_id;
    let mut results = Vec::new();

    // Get the room's last seen sequence number.
    let last_seq = state
        .rooms
        .get(&group_id)
        .map(|r| r.last_seen_seq)
        .unwrap_or(0);

    // Fetch new messages from the server.
    let resp = api
        .lock()
        .await
        .get_messages(&group_id, last_seq as i64)
        .await?;

    // Get the MLS group ID for decryption.
    let mls_group_id = match state.group_mapping.get(&group_id) {
        Some(id) => id.clone(),
        None => return Ok(results),
    };

    let username = match &state.username {
        Some(u) => u.clone(),
        None => return Ok(results),
    };

    for stored_msg in &resp.messages {
        // Decrypt via spawn_blocking (MLS is sync).
        let data_dir_clone = data_dir.clone();
        let username_clone = username.clone();
        let mls_group_id_clone = mls_group_id.clone();
        let mls_bytes = stored_msg.mls_message.clone();

        let decrypted = tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir_clone, &username_clone)?;
            mls.decrypt_message(&mls_group_id_clone, &mls_bytes)
        })
        .await
        .map_err(|e| Error::Other(format!("task join error: {e}")))??;

        match decrypted {
            DecryptedMessage::Application(plaintext) => {
                let text = String::from_utf8_lossy(&plaintext).to_string();
                let msg = DisplayMessage::user(
                    &stored_msg.sender_username,
                    &text,
                    stored_msg.created_at as i64,
                );
                results.push((group_id.clone(), msg));
            }
            DecryptedMessage::Commit(commit_info) => {
                for added in &commit_info.members_added {
                    results.push((
                        group_id.clone(),
                        DisplayMessage::system(&format!("{added} joined the group")),
                    ));
                }
                for removed in &commit_info.members_removed {
                    results.push((
                        group_id.clone(),
                        DisplayMessage::system(&format!("{removed} was removed from the group")),
                    ));
                }
                if commit_info.self_removed {
                    results.push((
                        group_id.clone(),
                        DisplayMessage::system("You were removed from this group"),
                    ));
                }
                // If no adds/removes, it's likely a key rotation or empty commit.
                if commit_info.members_added.is_empty()
                    && commit_info.members_removed.is_empty()
                    && !commit_info.self_removed
                {
                    results.push((
                        group_id.clone(),
                        DisplayMessage::system("Group keys updated"),
                    ));
                }
            }
            DecryptedMessage::Failed(reason) => {
                results.push((
                    group_id.clone(),
                    DisplayMessage::system(&format!(
                        "Failed to decrypt message (seq {}): {reason}",
                        stored_msg.sequence_num
                    )),
                ));
            }
            DecryptedMessage::None => {}
        }

        // Update last seen sequence.
        if let Some(room) = state.rooms.get_mut(&group_id) {
            room.last_seen_seq = room.last_seen_seq.max(stored_msg.sequence_num);
        }
    }

    Ok(results)
}

async fn handle_welcome(
    event: conclave_proto::WelcomeEvent,
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    data_dir: &PathBuf,
) -> Result<Vec<(String, DisplayMessage)>> {
    let mut results = Vec::new();

    let username = match &state.username {
        Some(u) => u.clone(),
        None => return Ok(results),
    };

    // Fetch pending welcomes and join.
    let resp = api.lock().await.list_pending_welcomes().await?;

    for welcome in &resp.welcomes {
        let data_dir_clone = data_dir.clone();
        let username_clone = username.clone();
        let welcome_bytes = welcome.welcome_message.clone();

        let mls_group_id = tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir_clone, &username_clone)?;
            mls.join_group(&welcome_bytes)
        })
        .await
        .map_err(|e| Error::Other(format!("task join error: {e}")))??;

        // Delete the welcome from the server so it is not re-processed.
        if let Err(error) = api.lock().await.accept_welcome(welcome.welcome_id).await {
            tracing::warn!(%error, "failed to acknowledge welcome");
        }

        state
            .group_mapping
            .insert(welcome.group_id.clone(), mls_group_id);

        // Save updated mapping.
        save_group_mapping(data_dir, &state.group_mapping);

        // Key packages are single-use (RFC 9420 §10); upload a fresh
        // replacement so we remain available for future group invitations.
        {
            let data_dir_clone = data_dir.clone();
            let username_clone = username.clone();
            let kp = tokio::task::spawn_blocking(move || {
                let mls = MlsManager::new(&data_dir_clone, &username_clone)?;
                mls.generate_key_package()
            })
            .await
            .map_err(|e| Error::Other(format!("task join error: {e}")))??;
            api.lock()
                .await
                .upload_key_packages(vec![(kp, false)])
                .await?;
        }

        // Refresh rooms.
        commands::load_rooms(api, state).await?;

        // Advance last_seen_seq so fetch_missed_messages skips the initial
        // commit that was already processed as part of the welcome.
        {
            let max_seq = match api.lock().await.get_messages(&welcome.group_id, 0).await {
                Ok(resp) => resp.messages.last().map(|m| m.sequence_num).unwrap_or(0),
                Err(_) => 0,
            };
            if let Some(room) = state.rooms.get_mut(&welcome.group_id) {
                room.last_seen_seq = room.last_seen_seq.max(max_seq);
            }
        }

        let msg = DisplayMessage::system(&format!(
            "You have been invited to #{} ({})",
            event.group_name, event.group_id
        ));
        results.push((welcome.group_id.clone(), msg));
    }

    Ok(results)
}

async fn handle_group_update(
    event: conclave_proto::GroupUpdateEvent,
    _state: &mut AppState,
) -> Result<Vec<(String, DisplayMessage)>> {
    let msg = DisplayMessage::system(&format!(
        "Group {} updated ({})",
        event.group_id, event.update_type
    ));
    Ok(vec![(event.group_id, msg)])
}

async fn handle_member_removed(
    event: conclave_proto::MemberRemovedEvent,
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    data_dir: &PathBuf,
) -> Result<Vec<(String, DisplayMessage)>> {
    let mut results = Vec::new();

    let our_username = state.username.as_deref().unwrap_or("");
    let group_id = &event.group_id;
    let removed = &event.removed_username;

    if removed == our_username {
        // We were removed from the group.
        let room_name = state
            .rooms
            .get(group_id)
            .map(|r| r.name.clone())
            .unwrap_or_else(|| group_id.clone());

        // Delete local MLS group state.
        if let Some(mls_group_id) = state.group_mapping.get(group_id) {
            if let Some(username) = &state.username {
                let data_dir_clone = data_dir.clone();
                let username_clone = username.clone();
                let mls_group_id_clone = mls_group_id.clone();
                match tokio::task::spawn_blocking(move || {
                    let mls = MlsManager::new(&data_dir_clone, &username_clone)?;
                    mls.delete_group_state(&mls_group_id_clone)
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
        }

        // Remove from local state.
        state.group_mapping.remove(group_id);
        save_group_mapping(data_dir, &state.group_mapping);
        state.rooms.remove(group_id);
        if state.active_room.as_deref() == Some(group_id) {
            state.active_room = None;
        }

        results.push((
            group_id.clone(),
            DisplayMessage::system(&format!("You were removed from #{room_name}")),
        ));
    } else {
        // Someone else was removed — fetch the leave commit so our MLS
        // state advances the epoch and excludes the departed member.
        let last_seq = state
            .rooms
            .get(group_id)
            .map(|r| r.last_seen_seq)
            .unwrap_or(0);

        if let Ok(resp) = api
            .lock()
            .await
            .get_messages(group_id, last_seq as i64)
            .await
        {
            if let Some(mls_group_id) = state.group_mapping.get(group_id) {
                if let Some(username) = &state.username {
                    for stored_msg in &resp.messages {
                        let data_dir_clone = data_dir.clone();
                        let username_clone = username.clone();
                        let mls_group_id_clone = mls_group_id.clone();
                        let mls_bytes = stored_msg.mls_message.clone();

                        let decrypted = tokio::task::spawn_blocking(move || {
                            let mls = MlsManager::new(&data_dir_clone, &username_clone)?;
                            mls.decrypt_message(&mls_group_id_clone, &mls_bytes)
                        })
                        .await
                        .map_err(|e| Error::Other(format!("task join error: {e}")))?;

                        match decrypted {
                            Ok(DecryptedMessage::Commit(_)) | Ok(DecryptedMessage::None) => {}
                            Ok(DecryptedMessage::Application(plaintext)) => {
                                let text = String::from_utf8_lossy(&plaintext).to_string();
                                results.push((
                                    group_id.clone(),
                                    DisplayMessage::user(
                                        &stored_msg.sender_username,
                                        &text,
                                        stored_msg.created_at as i64,
                                    ),
                                ));
                            }
                            Ok(DecryptedMessage::Failed(error)) => {
                                tracing::warn!(error, "decryption failed during member removal");
                            }
                            Err(error) => {
                                tracing::warn!(%error, "failed to decrypt message during member removal");
                            }
                        }
                    }
                }

                // Update last seen sequence.
                if let Some(last) = resp.messages.last() {
                    if let Some(room) = state.rooms.get_mut(group_id) {
                        room.last_seen_seq = last.sequence_num as u64;
                    }
                }
            }
        }

        results.push((
            group_id.clone(),
            DisplayMessage::system(&format!("{removed} was removed from the group")),
        ));

        // Refresh member list for this room.
        if let Some(room) = state.rooms.get_mut(group_id) {
            room.members.retain(|m| m != removed);
        }
    }

    Ok(results)
}
