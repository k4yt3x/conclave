use std::collections::HashMap;
use std::path::Path;

use uuid::Uuid;

use crate::api::ApiClient;
use crate::error::{Error, Result};
use crate::mls::MlsManager;

use super::{GroupCreatedResult, WelcomeJoinResult};

/// Create a new MLS group with only the creator, then upload the commit.
/// Members are added later via `/invite` through the escrow system.
pub async fn create_group(
    api: &ApiClient,
    alias: Option<&str>,
    group_name: &str,
    data_dir: &Path,
    user_id: Uuid,
) -> Result<GroupCreatedResult> {
    let response = api.create_group(alias, group_name).await?;
    let server_group_id = Uuid::from_slice(&response.group_id)?;

    let data_dir = data_dir.to_path_buf();

    let result = tokio::task::spawn_blocking(move || {
        let mls = MlsManager::new(&data_dir, user_id)?;
        mls.create_group(&HashMap::new())
    })
    .await
    .map_err(super::map_join_error)??;

    api.upload_commit(
        server_group_id,
        result.commit,
        result.group_info,
        Some(&result.mls_group_id),
    )
    .await?;

    Ok(GroupCreatedResult {
        server_group_id,
        mls_group_id: result.mls_group_id,
    })
}

/// Invite members to an existing group using the two-phase escrow system.
///
/// Each member is invited individually (one MLS commit per invite) so the
/// target can independently accept or decline. The admin's MLS state advances
/// one epoch per invite. The commit + welcome are held in escrow on the server
/// until the invitee accepts.
///
/// Returns the list of user IDs that were actually invited (empty if all were
/// already members or had no key packages).
pub async fn invite_members(
    api: &ApiClient,
    server_group_id: Uuid,
    mls_group_id: &str,
    member_ids: Vec<Uuid>,
    data_dir: &Path,
    user_id: Uuid,
) -> Result<Vec<Uuid>> {
    let mut invited = Vec::new();

    for &member_id in &member_ids {
        let response = api.invite_to_group(server_group_id, vec![member_id]).await;

        let response = match response {
            Ok(r) => r,
            Err(error) => {
                tracing::warn!(%error, member_id = %member_id, "failed to get key package for invite");
                continue;
            }
        };

        if response.member_key_packages.is_empty() {
            continue;
        }

        let member_key_packages: HashMap<Uuid, Vec<u8>> = {
            let mut map = HashMap::new();
            for entry in response.member_key_packages {
                match Uuid::from_slice(&entry.user_id) {
                    Ok(uid) => {
                        map.insert(uid, entry.key_package_data);
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to parse member key package UUID");
                        continue;
                    }
                }
            }
            map
        };
        let data_dir_clone = data_dir.to_path_buf();
        let mls_group_id_clone = mls_group_id.to_string();

        let result = tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir_clone, user_id)?;
            mls.invite_to_group(&mls_group_id_clone, &member_key_packages)
        })
        .await
        .map_err(super::map_join_error)?;

        let result = match result {
            Ok(r) => r,
            Err(error) => {
                tracing::warn!(%error, member_id = %member_id, "MLS invite failed");
                continue;
            }
        };

        // Extract the welcome for this specific user.
        let welcome_data = result.welcomes.get(&member_id).cloned().unwrap_or_default();

        api.escrow_invite(
            server_group_id,
            member_id,
            result.commit,
            welcome_data,
            result.group_info,
        )
        .await?;

        invited.push(member_id);
    }

    Ok(invited)
}

/// Fetch pending invites for the current user.
pub async fn list_pending_invites(api: &ApiClient) -> Result<Vec<conclave_proto::PendingInvite>> {
    let response = api.list_pending_invites().await?;
    Ok(response.invites)
}

/// Accept a pending invite: tells the server to finalize the invite, then
/// processes the resulting welcome to join the MLS group.
pub async fn accept_invite(
    api: &ApiClient,
    invite_id: Uuid,
    data_dir: &Path,
    user_id: Uuid,
) -> Result<Vec<WelcomeJoinResult>> {
    api.accept_pending_invite(invite_id).await?;

    // The server moves the welcome to pending_welcomes on accept.
    // Process welcomes to join the MLS group.
    accept_welcomes(api, data_dir, user_id).await
}

/// Fetch pending invites for a specific group (admin-only).
pub async fn list_group_pending_invites(
    api: &ApiClient,
    group_id: Uuid,
) -> Result<Vec<conclave_proto::PendingInvite>> {
    let response = api.list_group_pending_invites(group_id).await?;
    Ok(response.invites)
}

/// Cancel a pending invite by user ID.
pub async fn cancel_invite(api: &ApiClient, group_id: Uuid, user_id: Uuid) -> Result<()> {
    api.cancel_invite(group_id, user_id).await
}

/// Decline a pending invite.
pub async fn decline_invite(api: &ApiClient, invite_id: Uuid) -> Result<()> {
    api.decline_pending_invite(invite_id).await
}

/// Handle an invite decline by rotating keys to clean up the phantom MLS leaf.
pub async fn handle_invite_declined(
    api: &ApiClient,
    server_group_id: Uuid,
    mls_group_id: &str,
    data_dir: &Path,
    user_id: Uuid,
) -> Result<()> {
    super::messaging::rotate_keys(api, server_group_id, mls_group_id, data_dir, user_id).await
}

/// Remove a member from the group: find their MLS leaf index, produce a
/// removal commit, and notify the server.
pub async fn kick_member(
    api: &ApiClient,
    server_group_id: Uuid,
    mls_group_id: &str,
    target_user_id: Uuid,
    data_dir: &Path,
    user_id: Uuid,
) -> Result<()> {
    let data_dir = data_dir.to_path_buf();
    let mls_group_id = mls_group_id.to_string();

    let (commit_bytes, group_info_bytes) = tokio::task::spawn_blocking(move || {
        let mls = MlsManager::new(&data_dir, user_id)?;
        let member_index = mls
            .find_member_index(&mls_group_id, target_user_id)?
            .ok_or_else(|| {
                Error::Other(format!(
                    "user with ID {target_user_id} not found in MLS roster"
                ))
            })?;
        mls.remove_member(&mls_group_id, member_index)
    })
    .await
    .map_err(super::map_join_error)??;

    api.remove_member(
        server_group_id,
        target_user_id,
        commit_bytes,
        group_info_bytes,
    )
    .await?;

    Ok(())
}

/// Ban a member from a group: MLS removal + server-side ban list entry.
pub async fn ban_member(
    api: &ApiClient,
    server_group_id: Uuid,
    mls_group_id: &str,
    target_user_id: Uuid,
    data_dir: &Path,
    user_id: Uuid,
) -> Result<()> {
    let data_dir = data_dir.to_path_buf();
    let mls_group_id = mls_group_id.to_string();

    let (commit_bytes, group_info_bytes) = tokio::task::spawn_blocking(move || {
        let mls = MlsManager::new(&data_dir, user_id)?;
        let member_index = mls
            .find_member_index(&mls_group_id, target_user_id)?
            .ok_or_else(|| {
                Error::Other(format!(
                    "user with ID {target_user_id} not found in MLS roster"
                ))
            })?;
        mls.remove_member(&mls_group_id, member_index)
    })
    .await
    .map_err(super::map_join_error)??;

    api.ban_member(
        server_group_id,
        target_user_id,
        commit_bytes,
        group_info_bytes,
    )
    .await?;

    Ok(())
}

/// Leave a group: produce an MLS self-remove commit, notify the server, and
/// delete local MLS group state.
pub async fn leave_group(
    api: &ApiClient,
    server_group_id: Uuid,
    mls_group_id: Option<&str>,
    data_dir: &Path,
    user_id: Uuid,
) -> Result<()> {
    let (commit_bytes, group_info_bytes) = if let Some(mls_gid) = mls_group_id {
        let data_dir = data_dir.to_path_buf();
        let mls_gid = mls_gid.to_string();

        match tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir, user_id)?;
            mls.leave_group(&mls_gid)
        })
        .await
        .map_err(super::map_join_error)?
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
    user_id: Uuid,
) -> Result<()> {
    let data_dir = data_dir.to_path_buf();
    let mls_group_id = mls_group_id.to_string();

    tokio::task::spawn_blocking(move || {
        let mls = MlsManager::new(&data_dir, user_id)?;
        mls.delete_group_state(&mls_group_id)
    })
    .await
    .map_err(super::map_join_error)?
}

/// Delete local MLS group state for groups no longer present on the server.
pub async fn cleanup_orphaned_mls_state(
    data_dir: &Path,
    user_id: Uuid,
    valid_mls_group_ids: std::collections::HashSet<String>,
) -> Result<()> {
    let data_dir = data_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mls = MlsManager::new(&data_dir, user_id)?;
        mls.cleanup_orphaned_groups(&valid_mls_group_ids)
    })
    .await
    .map_err(super::map_join_error)?
}

/// Accept all pending welcome messages (group invitations): join each group
/// via MLS, acknowledge the welcome on the server, and replenish consumed key
/// packages per RFC 9420 §10.
pub async fn accept_welcomes(
    api: &ApiClient,
    data_dir: &Path,
    user_id: Uuid,
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
        .map_err(super::map_join_error)?;

        let mls_group_id = match mls_group_id {
            Ok(id) => id,
            Err(error) => {
                tracing::warn!(%error, "failed to join group via welcome");
                continue;
            }
        };

        let welcome_id = match Uuid::from_slice(&welcome.welcome_id) {
            Ok(id) => id,
            Err(error) => {
                tracing::warn!(%error, "failed to parse welcome_id UUID");
                continue;
            }
        };

        if let Err(error) = api.accept_welcome(welcome_id).await {
            tracing::warn!(%error, "failed to acknowledge welcome");
        }

        let group_id = match Uuid::from_slice(&welcome.group_id) {
            Ok(id) => id,
            Err(error) => {
                tracing::warn!(%error, "failed to parse welcome group_id UUID");
                continue;
            }
        };

        results.push(WelcomeJoinResult {
            group_id,
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
        .map_err(super::map_join_error)??;

        let entries: Vec<(Vec<u8>, bool)> =
            replacements.into_iter().map(|kp| (kp, false)).collect();
        if !entries.is_empty() {
            api.upload_key_packages(entries, "").await?;
        }
    }

    Ok(results)
}

/// Join a public room via MLS external commit.
pub async fn join_public_room(
    api: &ApiClient,
    data_dir: &Path,
    user_id: Uuid,
    group_id: Uuid,
) -> Result<GroupCreatedResult> {
    let response = api.join_public_group(group_id).await?;
    let group_info_bytes = response.group_info;

    let data_dir = data_dir.to_path_buf();
    let (mls_group_id, commit_message) = tokio::task::spawn_blocking(move || {
        let mls = MlsManager::new(&data_dir, user_id)?;
        mls.external_rejoin_group(&group_info_bytes, None)
    })
    .await
    .map_err(super::map_join_error)??;

    api.external_join(group_id, commit_message, &mls_group_id)
        .await?;

    Ok(GroupCreatedResult {
        server_group_id: group_id,
        mls_group_id,
    })
}
