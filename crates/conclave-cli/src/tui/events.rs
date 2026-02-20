use std::path::Path;
use std::sync::Arc;

use tokio::sync::Mutex;

use conclave_lib::api::ApiClient;
use conclave_lib::config::save_group_mapping;
use conclave_lib::error::Result;
use conclave_lib::operations::{self, SseEvent};

use super::commands;
use super::state::{AppState, DisplayMessage};

/// Handle an SSE message (hex-encoded protobuf ServerEvent).
/// Returns a list of (group_id, DisplayMessage) pairs to render.
pub async fn handle_sse_message(
    hex_data: &str,
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    data_dir: &Path,
) -> Result<Vec<(String, DisplayMessage)>> {
    let event = operations::decode_sse_event(hex_data)?;

    match event {
        SseEvent::NewMessage { group_id } => {
            handle_new_message(&group_id, api, state, data_dir).await
        }
        SseEvent::Welcome {
            group_id,
            group_name,
        } => handle_welcome(&group_id, &group_name, api, state, data_dir).await,
        SseEvent::GroupUpdate {
            group_id,
            update_type,
        } => {
            let msg = DisplayMessage::system(&format!("Group {group_id} updated ({update_type})"));
            Ok(vec![(group_id, msg)])
        }
        SseEvent::MemberRemoved {
            group_id,
            removed_username,
        } => handle_member_removed(&group_id, &removed_username, api, state, data_dir).await,
    }
}

async fn handle_new_message(
    group_id: &str,
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    data_dir: &Path,
) -> Result<Vec<(String, DisplayMessage)>> {
    let last_seq = state
        .rooms
        .get(group_id)
        .map(|r| r.last_seen_seq)
        .unwrap_or(0);

    let mls_group_id = match state.group_mapping.get(group_id) {
        Some(id) => id.clone(),
        None => return Ok(vec![]),
    };

    let username = match &state.username {
        Some(u) => u.as_str(),
        None => return Ok(vec![]),
    };

    let fetched = {
        let api_guard = api.lock().await;
        operations::fetch_and_decrypt(
            &api_guard,
            group_id,
            last_seq,
            &mls_group_id,
            data_dir,
            username,
        )
        .await?
    };

    let mut results = Vec::new();
    for msg in &fetched.messages {
        let display_msg = if msg.is_system {
            DisplayMessage::system(&msg.content)
        } else {
            DisplayMessage::user(&msg.sender, &msg.content, msg.timestamp)
        };
        results.push((group_id.to_string(), display_msg));

        if let Some(room) = state.rooms.get_mut(group_id) {
            room.last_seen_seq = room.last_seen_seq.max(msg.sequence_num);
        }
    }

    Ok(results)
}

async fn handle_welcome(
    group_id: &str,
    group_name: &str,
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    data_dir: &Path,
) -> Result<Vec<(String, DisplayMessage)>> {
    let username = match &state.username {
        Some(u) => u.as_str(),
        None => return Ok(vec![]),
    };

    let results = {
        let api_guard = api.lock().await;
        operations::accept_welcomes(&api_guard, data_dir, username).await?
    };

    let mut display_results = Vec::new();

    for result in &results {
        state
            .group_mapping
            .insert(result.group_id.clone(), result.mls_group_id.clone());
    }

    if !results.is_empty() {
        save_group_mapping(data_dir, &state.group_mapping);
        commands::load_rooms(api, state).await?;

        // Advance last_seen_seq so fetch_missed_messages skips the initial
        // commit that was already processed as part of the welcome.
        for result in &results {
            let max_seq = match api.lock().await.get_messages(&result.group_id, 0).await {
                Ok(resp) => resp.messages.last().map(|m| m.sequence_num).unwrap_or(0),
                Err(_) => 0,
            };
            if let Some(room) = state.rooms.get_mut(&result.group_id) {
                room.last_seen_seq = room.last_seen_seq.max(max_seq);
            }
        }
    }

    let msg = DisplayMessage::system(&format!(
        "You have been invited to #{group_name} ({group_id})"
    ));
    display_results.push((group_id.to_string(), msg));

    Ok(display_results)
}

async fn handle_member_removed(
    group_id: &str,
    removed_username: &str,
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    data_dir: &Path,
) -> Result<Vec<(String, DisplayMessage)>> {
    let mut results = Vec::new();

    let our_username = state.username.as_deref().unwrap_or("");

    if removed_username == our_username {
        let room_name = state
            .rooms
            .get(group_id)
            .map(|r| r.name.clone())
            .unwrap_or_else(|| group_id.to_string());

        if let Some(mls_group_id) = state.group_mapping.get(group_id)
            && let Some(username) = &state.username
            && let Err(error) =
                operations::delete_mls_group_state(mls_group_id, data_dir, username).await
        {
            tracing::warn!(%error, "failed to delete MLS group state");
        }

        state.group_mapping.remove(group_id);
        save_group_mapping(data_dir, &state.group_mapping);
        state.rooms.remove(group_id);
        if state.active_room.as_deref() == Some(group_id) {
            state.active_room = None;
        }

        results.push((
            group_id.to_string(),
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

        if let (Some(mls_group_id), Some(username)) =
            (state.group_mapping.get(group_id), state.username.as_deref())
        {
            let fetched = {
                let api_guard = api.lock().await;
                operations::fetch_and_decrypt(
                    &api_guard,
                    group_id,
                    last_seq,
                    mls_group_id,
                    data_dir,
                    username,
                )
                .await
            };

            if let Ok(fetched) = fetched {
                for msg in &fetched.messages {
                    if let Some(room) = state.rooms.get_mut(group_id) {
                        room.last_seen_seq = room.last_seen_seq.max(msg.sequence_num);
                    }
                }
            }
        }

        results.push((
            group_id.to_string(),
            DisplayMessage::system(&format!("{removed_username} was removed from the group")),
        ));

        if let Some(room) = state.rooms.get_mut(group_id) {
            room.members.retain(|m| m != removed_username);
        }
    }

    Ok(results)
}
