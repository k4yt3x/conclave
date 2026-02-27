use std::path::Path;

use crate::api::ApiClient;
use crate::error::{Error, Result};
use crate::mls::{DecryptedMessage, MlsManager};
use crate::state::RoomMember;

use super::{FetchedMessages, MessageSentResult, ProcessedMessage};

/// Fetch messages after `after_sequence` and decrypt them via MLS.
///
/// Each message is classified into a `ProcessedMessage`:
/// - Application -> user message
/// - Commit -> system messages for adds/removes/key-rotation
/// - Failed -> system error message
/// - None -> skipped
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
            .map_err(super::map_join_error)?;

        let sender_display = resolve_user_display_name(Some(stored_message.sender_id), members);

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
                process_commit_info(&commit_info, stored_message, epoch, members, &mut messages);
            }
            Ok(DecryptedMessage::Failed(reason)) => {
                messages.push(ProcessedMessage::system(
                    format!(
                        "Failed to decrypt message (seq {}): {reason}",
                        stored_message.sequence_num
                    ),
                    stored_message.created_at as i64,
                    stored_message.sequence_num,
                    epoch,
                ));
            }
            Ok(DecryptedMessage::None) => {}
            Err(error) => {
                tracing::warn!(%error, seq = stored_message.sequence_num, "message decryption failed");
                messages.push(ProcessedMessage::system(
                    format!(
                        "Failed to decrypt message (seq {}): {error}",
                        stored_message.sequence_num
                    ),
                    stored_message.created_at as i64,
                    stored_message.sequence_num,
                    epoch,
                ));
            }
        }
    }

    Ok(FetchedMessages { group_id, messages })
}

fn process_commit_info(
    commit_info: &crate::mls::CommitInfo,
    stored_message: &conclave_proto::StoredMessage,
    epoch: u64,
    members: &[RoomMember],
    messages: &mut Vec<ProcessedMessage>,
) {
    for added_uid in &commit_info.members_added {
        let name = resolve_user_display_name(*added_uid, members);
        messages.push(ProcessedMessage::system(
            format!("{name} joined the group"),
            stored_message.created_at as i64,
            stored_message.sequence_num,
            epoch,
        ));
    }
    for removed in &commit_info.members_removed {
        messages.push(ProcessedMessage::system(
            format!("{removed} was removed from the group"),
            stored_message.created_at as i64,
            stored_message.sequence_num,
            epoch,
        ));
    }
    if commit_info.self_removed {
        messages.push(ProcessedMessage::system(
            "You were removed from this group".to_string(),
            stored_message.created_at as i64,
            stored_message.sequence_num,
            epoch,
        ));
    }
    if commit_info.members_added.is_empty()
        && commit_info.members_removed.is_empty()
        && !commit_info.self_removed
    {
        messages.push(ProcessedMessage::system(
            "Group keys updated".to_string(),
            stored_message.created_at as i64,
            stored_message.sequence_num,
            epoch,
        ));
    }
}

/// Resolve a user ID from an MLS credential to a display name using the room
/// member list. Falls back to the user ID as a string.
pub fn resolve_user_display_name(user_id: Option<i64>, members: &[RoomMember]) -> String {
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
    .map_err(super::map_join_error)??;

    let response = api.send_message(server_group_id, encrypted).await?;

    Ok(MessageSentResult {
        group_id: server_group_id,
        sequence_num: response.sequence_num,
        epoch,
    })
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
    .map_err(super::map_join_error)??;

    api.upload_commit(server_group_id, commit_bytes, group_info_bytes, None)
        .await?;

    Ok(())
}
