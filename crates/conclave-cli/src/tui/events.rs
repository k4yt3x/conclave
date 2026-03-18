use std::path::Path;
use std::sync::Arc;

use tokio::sync::Mutex;
use uuid::Uuid;

use conclave_client::api::ApiClient;
use conclave_client::error::Result;
use conclave_client::operations::{self, SseEvent};

use super::commands;
use super::state::{AppState, DisplayMessage};
use super::store::MessageStore;

/// Handle an SSE message (hex-encoded protobuf ServerEvent).
/// Returns a list of (group_id, DisplayMessage) pairs to render.
/// `None` group_id means the message is not tied to a specific room
/// (e.g., invite notifications for groups the user hasn't joined yet).
pub async fn handle_sse_message(
    hex_data: &str,
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    data_dir: &Path,
    msg_store: &Option<MessageStore>,
) -> Result<Vec<(Option<Uuid>, DisplayMessage)>> {
    let event = operations::decode_sse_event(hex_data)?;

    match event {
        SseEvent::NewMessage { group_id } => {
            handle_new_message(group_id, api, state, data_dir).await
        }
        SseEvent::Welcome {
            group_id,
            group_alias,
        } => handle_welcome(group_id, &group_alias, api, state, data_dir, msg_store).await,
        SseEvent::GroupUpdate {
            group_id: _,
            update_type: _,
        } => {
            commands::load_rooms(api, state, msg_store).await?;
            Ok(vec![])
        }
        SseEvent::MemberRemoved {
            group_id,
            removed_user_id,
        } => handle_member_removed(group_id, removed_user_id, api, state, data_dir).await,
        SseEvent::IdentityReset { group_id, user_id } => {
            // Process the external commit to advance our MLS epoch state.
            let _ = handle_new_message(group_id, api, state, data_dir).await;

            // Refresh member list and fingerprints so TOFU check detects the change.
            commands::load_rooms(api, state, msg_store).await?;

            let display_name = state
                .rooms
                .get(&group_id)
                .and_then(|room| {
                    room.members
                        .iter()
                        .find(|m| m.user_id == user_id)
                        .map(|m| m.display_name().to_string())
                })
                .unwrap_or_else(|| format!("user#{user_id}"));

            Ok(vec![(
                Some(group_id),
                DisplayMessage::system(&format!(
                    "Caution: {display_name} has reset their encryption identity."
                )),
            )])
        }
        SseEvent::InviteReceived {
            invite_id,
            group_id: _,
            group_name,
            group_alias,
            inviter_id,
        } => {
            let display = if group_alias.is_empty() {
                &group_name
            } else {
                &group_alias
            };
            let inviter_name = format!("user#{inviter_id}");
            Ok(vec![(
                None,
                DisplayMessage::system(&format!(
                    "Invitation from {inviter_name} to join #{display}. \
                     Use /accept {invite_id} or /decline {invite_id}."
                )),
            )])
        }
        SseEvent::InviteCancelled { group_id: _ } => Ok(vec![(
            None,
            DisplayMessage::system("An invitation to this room was cancelled."),
        )]),
        SseEvent::GroupDeleted { group_id } => {
            handle_group_deleted(group_id, state, data_dir).await
        }
        SseEvent::InviteDeclined {
            group_id,
            declined_user_id,
        } => {
            // Auto-rotate keys to clean up the phantom MLS leaf.
            if let (Some(mls_group_id), Some(user_id)) =
                (state.group_mapping.get(&group_id).cloned(), state.user_id)
            {
                let api_guard = api.lock().await;
                if let Err(error) = operations::handle_invite_declined(
                    &api_guard,
                    group_id,
                    &mls_group_id,
                    data_dir,
                    user_id,
                )
                .await
                {
                    tracing::warn!(%error, "failed to rotate keys after invite decline");
                }
            }

            let declined_name = state
                .rooms
                .get(&group_id)
                .and_then(|room| {
                    room.members
                        .iter()
                        .find(|m| m.user_id == declined_user_id)
                        .map(|m| m.display_name().to_string())
                })
                .unwrap_or_else(|| format!("user#{declined_user_id}"));

            Ok(vec![(
                Some(group_id),
                DisplayMessage::system(&format!("{declined_name} declined the invitation.")),
            )])
        }
    }
}

async fn handle_group_deleted(
    group_id: Uuid,
    state: &mut AppState,
    data_dir: &Path,
) -> Result<Vec<(Option<Uuid>, DisplayMessage)>> {
    let room_name = state
        .rooms
        .get(&group_id)
        .map(|r| r.display_name())
        .unwrap_or_else(|| group_id.to_string());

    if let Some(mls_group_id) = state.group_mapping.get(&group_id)
        && let Some(user_id) = state.user_id
        && let Err(error) =
            operations::delete_mls_group_state(mls_group_id, data_dir, user_id).await
    {
        tracing::warn!(%error, "failed to delete MLS group state");
    }

    state.group_mapping.remove(&group_id);
    state.rooms.remove(&group_id);
    if state.active_room == Some(group_id) {
        state.active_room = None;
    }

    Ok(vec![(
        None,
        DisplayMessage::system(&format!("Room #{room_name} has been deleted")),
    )])
}

async fn handle_new_message(
    group_id: Uuid,
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    data_dir: &Path,
) -> Result<Vec<(Option<Uuid>, DisplayMessage)>> {
    let last_seq = state
        .rooms
        .get(&group_id)
        .map(|r| r.last_seen_seq)
        .unwrap_or(0);

    let mls_group_id = match state.group_mapping.get(&group_id) {
        Some(id) => id.clone(),
        None => return Ok(vec![]),
    };

    let user_id = match state.user_id {
        Some(id) => id,
        None => return Ok(vec![]),
    };

    let members: Vec<conclave_client::state::RoomMember> = state
        .rooms
        .get(&group_id)
        .map(|r| r.members.clone())
        .unwrap_or_default();

    let fetched = {
        let api_guard = api.lock().await;
        operations::fetch_and_decrypt(
            &api_guard,
            group_id,
            last_seq,
            &mls_group_id,
            data_dir,
            user_id,
            &members,
        )
        .await?
    };

    let mut results = Vec::new();
    for msg in &fetched.messages {
        let mut display_msg = if msg.is_system {
            DisplayMessage::system(&msg.content)
        } else {
            DisplayMessage::user(
                msg.sender_id.unwrap_or(Uuid::nil()),
                &msg.sender,
                &msg.content,
                msg.timestamp,
            )
        };
        display_msg.sequence_num = Some(msg.sequence_num);
        display_msg.epoch = Some(msg.epoch);
        results.push((Some(group_id), display_msg));

        if let Some(room) = state.rooms.get_mut(&group_id) {
            room.last_seen_seq = room.last_seen_seq.max(msg.sequence_num);
        }
    }

    Ok(results)
}

async fn handle_welcome(
    group_id: Uuid,
    group_alias: &str,
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    data_dir: &Path,
    msg_store: &Option<MessageStore>,
) -> Result<Vec<(Option<Uuid>, DisplayMessage)>> {
    let user_id = match state.user_id {
        Some(id) => id,
        None => return Ok(vec![]),
    };

    let results = {
        let api_guard = api.lock().await;
        operations::accept_welcomes(&api_guard, data_dir, user_id).await?
    };

    let mut display_results = Vec::new();

    for result in &results {
        state
            .group_mapping
            .insert(result.group_id, result.mls_group_id.clone());
    }

    if !results.is_empty() {
        commands::load_rooms(api, state, msg_store).await?;

        // Advance last_seen_seq so fetch_missed_messages skips the initial
        // commit that was already processed as part of the welcome.
        for result in &results {
            let max_seq = match api.lock().await.get_messages(result.group_id, 0).await {
                Ok(resp) => resp.messages.last().map(|m| m.sequence_num).unwrap_or(0),
                Err(_) => 0,
            };
            if let Some(room) = state.rooms.get_mut(&result.group_id) {
                room.last_seen_seq = room.last_seen_seq.max(max_seq);
            }
        }
    }

    let display = if group_alias.is_empty() {
        group_id.to_string()
    } else {
        group_alias.to_string()
    };
    let msg = DisplayMessage::system(&format!("You have been invited to #{display} ({group_id})"));
    display_results.push((Some(group_id), msg));

    Ok(display_results)
}

async fn handle_member_removed(
    group_id: Uuid,
    removed_user_id: Uuid,
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    data_dir: &Path,
) -> Result<Vec<(Option<Uuid>, DisplayMessage)>> {
    let mut results = Vec::new();

    let is_self = state.user_id == Some(removed_user_id);

    if is_self {
        let room_name = state
            .rooms
            .get(&group_id)
            .map(|r| r.display_name())
            .unwrap_or_else(|| group_id.to_string());

        let user_id = state.user_id;

        if let Some(mls_group_id) = state.group_mapping.get(&group_id)
            && let Some(user_id) = user_id
            && let Err(error) =
                operations::delete_mls_group_state(mls_group_id, data_dir, user_id).await
        {
            tracing::warn!(%error, "failed to delete MLS group state");
        }

        state.group_mapping.remove(&group_id);
        state.rooms.remove(&group_id);
        if state.active_room == Some(group_id) {
            state.active_room = None;
        }

        results.push((
            Some(group_id),
            DisplayMessage::system(&format!("You were removed from #{room_name}")),
        ));
    } else {
        // Resolve display name from local member list before removal.
        let removed_name = state
            .rooms
            .get(&group_id)
            .and_then(|room| {
                room.members
                    .iter()
                    .find(|m| m.user_id == removed_user_id)
                    .map(|m| m.display_name().to_string())
            })
            .unwrap_or_else(|| format!("user#{removed_user_id}"));

        // Someone else was removed -- fetch the leave commit so our MLS
        // state advances the epoch and excludes the departed member.
        let last_seq = state
            .rooms
            .get(&group_id)
            .map(|r| r.last_seen_seq)
            .unwrap_or(0);

        let user_id = state.user_id;

        if let (Some(mls_group_id), Some(user_id)) = (state.group_mapping.get(&group_id), user_id) {
            let members: Vec<conclave_client::state::RoomMember> = state
                .rooms
                .get(&group_id)
                .map(|r| r.members.clone())
                .unwrap_or_default();

            let fetched = {
                let api_guard = api.lock().await;
                operations::fetch_and_decrypt(
                    &api_guard,
                    group_id,
                    last_seq,
                    mls_group_id,
                    data_dir,
                    user_id,
                    &members,
                )
                .await
            };

            if let Ok(fetched) = fetched {
                for msg in &fetched.messages {
                    if let Some(room) = state.rooms.get_mut(&group_id) {
                        room.last_seen_seq = room.last_seen_seq.max(msg.sequence_num);
                    }
                }
            }
        }

        results.push((
            Some(group_id),
            DisplayMessage::system(&format!("{removed_name} was removed from the group")),
        ));

        if let Some(room) = state.rooms.get_mut(&group_id) {
            room.members.retain(|m| m.user_id != removed_user_id);
        }
    }

    Ok(results)
}
