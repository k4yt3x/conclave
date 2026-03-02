use std::sync::Arc;

use tokio::sync::Mutex;

use conclave_client::api::ApiClient;
pub use conclave_client::command::Command;
use conclave_client::config::{ClientConfig, build_group_mapping};
use conclave_client::duration::{compute_effective, format_duration, parse_duration};
use conclave_client::mls::MlsManager;
use conclave_client::operations;

pub use conclave_client::command::parse;

use super::state::{AppState, DisplayMessage, Room};
use super::store::MessageStore;

type Result<T> = conclave_client::error::Result<T>;
type Error = conclave_client::error::Error;

/// Execute a command, updating state and returning messages to display.
/// Returns (messages_for_current_view, should_start_sse).
pub async fn execute(
    cmd: Command,
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    mls: &mut Option<MlsManager>,
    config: &ClientConfig,
    msg_store: &Option<MessageStore>,
) -> Result<(Vec<DisplayMessage>, bool)> {
    match cmd {
        Command::Register { .. }
        | Command::Login { .. }
        | Command::Logout
        | Command::Expunge { .. } => execute_account(cmd, api, state, mls, config, msg_store).await,
        Command::Create { .. }
        | Command::Rooms
        | Command::Join { .. }
        | Command::Close
        | Command::Part
        | Command::Unread
        | Command::Info
        | Command::Expire { .. }
        | Command::Delete => execute_room(cmd, api, state, mls, config, msg_store).await,
        Command::Invite { .. }
        | Command::Kick { .. }
        | Command::Promote { .. }
        | Command::Demote { .. }
        | Command::Admins
        | Command::Invited
        | Command::Uninvite { .. }
        | Command::Members => execute_member(cmd, api, state, config, msg_store).await,
        Command::Invites | Command::Accept { .. } | Command::Decline { .. } => {
            execute_invite(cmd, api, state, config, msg_store).await
        }
        Command::Message { .. } | Command::Rotate => {
            execute_messaging(cmd, api, state, config, msg_store).await
        }
        Command::Alias { .. }
        | Command::Topic { .. }
        | Command::Whois { .. }
        | Command::Verify { .. }
        | Command::Unverify { .. }
        | Command::Trusted
        | Command::Passwd { .. }
        | Command::Help
        | Command::Quit
        | Command::Reset => execute_profile(cmd, api, state, mls, config, msg_store).await,
    }
}

async fn execute_account(
    cmd: Command,
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    mls: &mut Option<MlsManager>,
    config: &ClientConfig,
    msg_store: &Option<MessageStore>,
) -> Result<(Vec<DisplayMessage>, bool)> {
    let mut msgs = Vec::new();
    let mut start_sse = false;

    match cmd {
        Command::Register {
            server,
            token,
            username,
            password,
        } => {
            let result = operations::register_and_login(
                &server,
                &username,
                &password,
                token.as_deref(),
                config.accept_invalid_certs,
                &config.data_dir,
            )
            .await?;

            tracing::debug!(
                count = result.key_packages_uploaded,
                "key packages uploaded"
            );

            *api.lock().await = result.into_api_client(config.accept_invalid_certs);
            state.username = Some(result.username.clone());
            state.user_id = Some(result.user_id);
            state.logged_in = true;

            result.save_session(&config.data_dir)?;

            *mls = Some(MlsManager::new(&config.data_dir, result.user_id)?);

            let room_infos = load_rooms(api, state, msg_store).await?;
            state.group_mapping = build_group_mapping(&room_infos, &config.data_dir);

            msgs.push(DisplayMessage::system(&format!(
                "Registered and logged in as {} (user ID {})",
                result.username, result.user_id
            )));
            start_sse = true;
        }

        Command::Login {
            server,
            username,
            password,
        } => {
            let result = operations::login(
                &server,
                &username,
                &password,
                config.accept_invalid_certs,
                &config.data_dir,
            )
            .await?;

            tracing::debug!(
                count = result.key_packages_uploaded,
                "key packages uploaded"
            );

            *api.lock().await = result.into_api_client(config.accept_invalid_certs);
            state.username = Some(result.username.clone());
            state.user_id = Some(result.user_id);
            state.logged_in = true;

            result.save_session(&config.data_dir)?;

            *mls = Some(MlsManager::new(&config.data_dir, result.user_id)?);

            let room_infos = load_rooms(api, state, msg_store).await?;
            state.group_mapping = build_group_mapping(&room_infos, &config.data_dir);

            let unmapped_count = state
                .rooms
                .keys()
                .filter(|gid| !state.group_mapping.contains_key(gid))
                .count();
            if unmapped_count > 0 {
                msgs.push(DisplayMessage::system(&format!(
                    "{unmapped_count} group(s) have no local encryption state. \
                     Run /reset to rejoin them with a new identity."
                )));
            }

            msgs.push(DisplayMessage::system(&format!(
                "Logged in as {} (user ID {})",
                result.username, result.user_id
            )));
            start_sse = true;
        }

        Command::Logout => {
            if !state.logged_in {
                msgs.push(DisplayMessage::system("Not logged in."));
                return Ok((msgs, start_sse));
            }

            if let Err(error) = api.lock().await.logout().await {
                tracing::warn!(%error, "server-side session revocation failed");
            }

            api.lock().await.set_token(String::new());
            state.logged_in = false;
            state.username = None;
            state.user_id = None;
            state.active_room = None;
            state.rooms.clear();
            state.group_mapping.clear();
            *mls = None;

            let session_path = config.data_dir.join("session.toml");
            if let Err(error) = std::fs::remove_file(&session_path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(%error, "failed to remove session file");
            }

            msgs.push(DisplayMessage::system("Logged out. Session revoked."));
        }

        Command::Expunge { password } => {
            if !state.logged_in {
                msgs.push(DisplayMessage::system("Not logged in."));
                return Ok((msgs, start_sse));
            }

            {
                let api_guard = api.lock().await;
                operations::delete_account(&api_guard, &password, &config.data_dir).await?;
            }

            api.lock().await.set_token(String::new());
            state.logged_in = false;
            state.username = None;
            state.user_id = None;
            state.active_room = None;
            state.rooms.clear();
            state.group_mapping.clear();
            *mls = None;

            msgs.push(DisplayMessage::system(
                "Account permanently deleted. All data has been wiped.",
            ));
        }

        _ => {}
    }

    Ok((msgs, start_sse))
}

async fn execute_room(
    cmd: Command,
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    mls: &mut Option<MlsManager>,
    config: &ClientConfig,
    msg_store: &Option<MessageStore>,
) -> Result<(Vec<DisplayMessage>, bool)> {
    let mut msgs = Vec::new();

    match cmd {
        Command::Create { name } => {
            let user_id = require_user_id(state)?;

            let result = {
                let api_guard = api.lock().await;
                operations::create_group(&api_guard, None, &name, &config.data_dir, user_id).await?
            };

            state
                .group_mapping
                .insert(result.server_group_id, result.mls_group_id);
            load_rooms(api, state, msg_store).await?;

            state.active_room = Some(result.server_group_id);
            state.scroll_offset = 0;
            msgs.push(DisplayMessage::system(&format!(
                "Created and joined #{name} ({})",
                result.server_group_id
            )));
        }

        Command::Join { target: None } => {
            let user_id = require_user_id(state)?;

            let results = {
                let api_guard = api.lock().await;
                operations::accept_welcomes(&api_guard, &config.data_dir, user_id).await?
            };

            if results.is_empty() {
                msgs.push(DisplayMessage::system("No pending invitations."));
                return Ok((msgs, false));
            }

            let mut last_group_id = None;
            let mut joined_group_ids = Vec::new();

            for result in &results {
                state
                    .group_mapping
                    .insert(result.group_id, result.mls_group_id.clone());
                last_group_id = Some(result.group_id);
                joined_group_ids.push(result.group_id);

                let id_string = result.group_id.to_string();
                let display = result.group_alias.as_deref().unwrap_or(&id_string);
                msgs.push(DisplayMessage::system(&format!(
                    "Joined #{display} ({})",
                    result.group_id
                )));
            }

            load_rooms(api, state, msg_store).await?;
            if let Some(gid) = last_group_id {
                state.active_room = Some(gid);
                state.scroll_offset = 0;
            }

            // Skip the initial commit (seq 1) that was already processed
            // as part of the welcome message.
            for group_id in &joined_group_ids {
                if let Some(room) = state.rooms.get_mut(group_id)
                    && room.last_seen_seq == 0
                {
                    let max_seq = match api.lock().await.get_messages(*group_id, 0).await {
                        Ok(resp) => resp.messages.last().map(|m| m.sequence_num).unwrap_or(0),
                        Err(_) => 0,
                    };
                    room.last_seen_seq = max_seq;
                }
            }
        }

        Command::Join {
            target: Some(target),
        } => {
            let resolved_gid = state.resolve_room(&target).map(|r| r.server_group_id);

            if let Some(gid) = resolved_gid {
                let name = state
                    .rooms
                    .get(&gid)
                    .map(|r| r.display_name())
                    .unwrap_or_default();

                state.active_room = Some(gid);
                state.scroll_offset = 0;

                if let Some(room) = state.rooms.get_mut(&gid) {
                    room.last_read_seq = room.last_seen_seq;
                    if let Some(store) = msg_store {
                        store.set_last_read_seq(gid, room.last_read_seq);
                    }
                }
                msgs.push(DisplayMessage::system(&format!("Switched to #{name}")));
            } else {
                msgs.push(DisplayMessage::system(&format!(
                    "Unknown room '{target}'. Use /rooms to list available rooms."
                )));
            }
        }

        Command::Close => {
            if let Some(room_id) = state.active_room.take() {
                let name = state
                    .rooms
                    .get(&room_id)
                    .map(|r| r.display_name())
                    .unwrap_or_default();

                state.scroll_offset = 0;
                msgs.push(DisplayMessage::system(&format!(
                    "Switched away from #{name} (use /part to leave the group)"
                )));
            } else {
                msgs.push(DisplayMessage::system("No active room."));
            }
        }

        Command::Part => {
            let user_id = require_user_id(state)?;

            let group_id = state
                .active_room
                .ok_or_else(|| Error::Other("no active room -- use /join first".into()))?;

            let name = state
                .rooms
                .get(&group_id)
                .map(|r| r.display_name())
                .unwrap_or_default();

            let mls_group_id = state.group_mapping.get(&group_id).cloned();

            {
                let api_guard = api.lock().await;
                operations::leave_group(
                    &api_guard,
                    group_id,
                    mls_group_id.as_deref(),
                    &config.data_dir,
                    user_id,
                )
                .await?;
            }

            state.group_mapping.remove(&group_id);
            state.rooms.remove(&group_id);
            state.active_room = None;
            state.scroll_offset = 0;

            msgs.push(DisplayMessage::system(&format!("Left #{name}")));
        }

        Command::Unread => {
            if !state.logged_in {
                msgs.push(DisplayMessage::system("Not logged in."));
                return Ok((msgs, false));
            }
            if state.rooms.is_empty() {
                msgs.push(DisplayMessage::system("No rooms."));
                return Ok((msgs, false));
            }

            let mut any_unread = false;

            for room in state.rooms.values() {
                let local_unread = room.last_seen_seq.saturating_sub(room.last_read_seq);

                let server_unread = match api
                    .lock()
                    .await
                    .get_messages(room.server_group_id, room.last_seen_seq as i64)
                    .await
                {
                    Ok(resp) => resp.messages.len() as u64,
                    Err(_) => 0,
                };

                let total = local_unread + server_unread;
                if total > 0 {
                    any_unread = true;
                    msgs.push(DisplayMessage::system(&format!(
                        "  #{}: {total} new message{}",
                        room.display_name(),
                        if total == 1 { "" } else { "s" },
                    )));
                }
            }

            if !any_unread {
                msgs.push(DisplayMessage::system("No unread messages."));
            }
        }

        Command::Info => {
            let mls_mgr = mls
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;
            let group_id = state
                .active_room
                .ok_or_else(|| Error::Other("no active room -- use /join first".into()))?;
            let mls_group_id = state
                .group_mapping
                .get(&group_id)
                .ok_or_else(|| Error::Other("group mapping not found".into()))?
                .clone();

            let details = mls_mgr.group_info_details(&mls_group_id)?;
            let room_name = state
                .rooms
                .get(&group_id)
                .map(|r| r.display_name())
                .unwrap_or_else(|| "unknown".to_string());

            msgs.push(DisplayMessage::system(&format!("Group: #{room_name}")));
            msgs.push(DisplayMessage::system(&format!("  Server ID: {group_id}")));
            msgs.push(DisplayMessage::system(&format!(
                "  MLS Group ID: {mls_group_id}"
            )));
            msgs.push(DisplayMessage::system(&format!(
                "  Epoch: {}",
                details.epoch
            )));
            msgs.push(DisplayMessage::system(&format!(
                "  Cipher Suite: {}",
                details.cipher_suite
            )));
            msgs.push(DisplayMessage::system(&format!(
                "  Members: {} (your index: {})",
                details.member_count, details.own_index
            )));

            let room_members = state
                .rooms
                .get(&group_id)
                .map(|r| &r.members[..])
                .unwrap_or(&[]);
            for (index, user_id) in &details.members {
                let marker = if *index == details.own_index {
                    " (you)"
                } else {
                    ""
                };
                let name = match user_id {
                    Some(uid) => room_members
                        .iter()
                        .find(|m| m.user_id == *uid)
                        .map(|m| m.display_name().to_string())
                        .unwrap_or_else(|| format!("user#{uid}")),
                    None => "<unknown>".to_string(),
                };
                msgs.push(DisplayMessage::system(&format!(
                    "    [{index}] {name}{marker}"
                )));
            }
        }

        Command::Expire { duration: None } => {
            let group_id = state
                .active_room
                .ok_or_else(|| Error::Other("no active room -- use /join first".into()))?;
            let policy = api.lock().await.get_retention_policy(group_id).await?;
            let server = format_duration(policy.server_retention_seconds);
            let group = format_duration(policy.group_expiry_seconds);
            msgs.push(DisplayMessage::system(&format!(
                "Server retention: {server}"
            )));
            msgs.push(DisplayMessage::system(&format!("Room expiry: {group}")));
            let effective =
                compute_effective(policy.server_retention_seconds, policy.group_expiry_seconds);
            msgs.push(DisplayMessage::system(&format!(
                "Effective: {}",
                format_duration(effective)
            )));
        }

        Command::Expire {
            duration: Some(dur),
        } => {
            let group_id = state
                .active_room
                .ok_or_else(|| Error::Other("no active room -- use /join first".into()))?;
            let seconds = parse_duration(&dur)?;
            api.lock().await.set_group_expiry(group_id, seconds).await?;
            msgs.push(DisplayMessage::system(&format!(
                "Message expiry set to {}",
                format_duration(seconds)
            )));
        }

        Command::Delete => {
            let group_id = state
                .active_room
                .ok_or_else(|| Error::Other("no active room -- use /join first".into()))?;
            let name = state
                .rooms
                .get(&group_id)
                .map(|r| r.display_name())
                .unwrap_or_default();

            api.lock().await.delete_group(group_id).await?;

            state.group_mapping.remove(&group_id);
            state.rooms.remove(&group_id);
            state.room_messages.remove(&group_id);
            state.active_room = None;
            state.scroll_offset = 0;

            msgs.push(DisplayMessage::system(&format!(
                "Room #{name} has been deleted"
            )));
        }

        Command::Rooms => {
            load_rooms(api, state, msg_store).await?;

            if state.rooms.is_empty() {
                msgs.push(DisplayMessage::system("No rooms."));
            } else {
                msgs.push(DisplayMessage::system("Rooms:"));
                for room in state.rooms.values() {
                    let active = if state.active_room == Some(room.server_group_id) {
                        " (active)"
                    } else {
                        ""
                    };
                    let member_display: Vec<&str> =
                        room.members.iter().map(|m| m.display_name()).collect();
                    msgs.push(DisplayMessage::system(&format!(
                        "  #{} [{}] -- {}{active}",
                        room.display_name(),
                        member_display.join(", "),
                        room.server_group_id,
                    )));
                }
            }
        }

        _ => {}
    }

    Ok((msgs, false))
}

async fn execute_member(
    cmd: Command,
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    config: &ClientConfig,
    msg_store: &Option<MessageStore>,
) -> Result<(Vec<DisplayMessage>, bool)> {
    let mut msgs = Vec::new();

    match cmd {
        Command::Invite { members } => {
            let user_id = require_user_id(state)?;
            let group_id = state
                .active_room
                .ok_or_else(|| Error::Other("no active room -- use /join first".into()))?;
            let mls_group_id = state
                .group_mapping
                .get(&group_id)
                .ok_or_else(|| Error::Other("group mapping not found".into()))?
                .clone();

            // Resolve usernames to user IDs at the UI boundary.
            let mut member_ids = Vec::new();
            let mut member_names = Vec::new();
            for username in &members {
                let user_info = api.lock().await.get_user_by_username(username).await?;
                member_ids.push(user_info.user_id);
                member_names.push(username.clone());
            }

            let invited = {
                let api_guard = api.lock().await;
                operations::invite_members(
                    &api_guard,
                    group_id,
                    &mls_group_id,
                    member_ids.clone(),
                    &config.data_dir,
                    user_id,
                )
                .await?
            };

            if invited.is_empty() {
                msgs.push(DisplayMessage::system("No new members to invite."));
            } else {
                // Map invited user IDs back to usernames for display.
                let display_names: Vec<&str> = invited
                    .iter()
                    .filter_map(|id| {
                        member_ids
                            .iter()
                            .zip(member_names.iter())
                            .find(|(mid, _)| *mid == id)
                            .map(|(_, name)| name.as_str())
                    })
                    .collect();
                msgs.push(DisplayMessage::system(&format!(
                    "Invited {} to the room",
                    display_names.join(", ")
                )));
            }

            load_rooms(api, state, msg_store).await?;
        }

        Command::Kick { username: target } => {
            let user_id = require_user_id(state)?;
            let group_id = state
                .active_room
                .ok_or_else(|| Error::Other("no active room -- use /join first".into()))?;
            let mls_group_id = state
                .group_mapping
                .get(&group_id)
                .ok_or_else(|| Error::Other("group mapping not found".into()))?
                .clone();
            let target_user_id = state
                .rooms
                .get(&group_id)
                .and_then(|room| {
                    room.members
                        .iter()
                        .find(|m| m.username == target)
                        .map(|m| m.user_id)
                })
                .ok_or_else(|| {
                    Error::Other(format!("user '{target}' not found in the room member list"))
                })?;

            {
                let api_guard = api.lock().await;
                operations::kick_member(
                    &api_guard,
                    group_id,
                    &mls_group_id,
                    target_user_id,
                    &config.data_dir,
                    user_id,
                )
                .await?;
            }

            load_rooms(api, state, msg_store).await?;

            msgs.push(DisplayMessage::system(&format!(
                "Removed {target} from the room"
            )));
        }

        Command::Promote { username: target } => {
            let group_id = state
                .active_room
                .ok_or_else(|| Error::Other("no active room -- use /join first".into()))?;
            let target_user_id = state
                .rooms
                .get(&group_id)
                .and_then(|room| {
                    room.members
                        .iter()
                        .find(|m| m.username == target)
                        .map(|m| m.user_id)
                })
                .ok_or_else(|| {
                    Error::Other(format!("user '{target}' not found in the room member list"))
                })?;

            api.lock()
                .await
                .promote_member(group_id, target_user_id)
                .await?;
            msgs.push(DisplayMessage::system(&format!(
                "Promoted {target} to admin"
            )));
        }

        Command::Demote { username: target } => {
            let group_id = state
                .active_room
                .ok_or_else(|| Error::Other("no active room -- use /join first".into()))?;
            let target_user_id = state
                .rooms
                .get(&group_id)
                .and_then(|room| {
                    room.members
                        .iter()
                        .find(|m| m.username == target)
                        .map(|m| m.user_id)
                })
                .ok_or_else(|| {
                    Error::Other(format!("user '{target}' not found in the room member list"))
                })?;

            api.lock()
                .await
                .demote_member(group_id, target_user_id)
                .await?;
            msgs.push(DisplayMessage::system(&format!(
                "Demoted {target} to regular member"
            )));
        }

        Command::Admins => {
            let group_id = state
                .active_room
                .ok_or_else(|| Error::Other("no active room -- use /join first".into()))?;
            let response = api.lock().await.list_admins(group_id).await?;
            let admin_names: Vec<String> = response
                .admins
                .iter()
                .map(|a| {
                    if a.alias.is_empty() {
                        a.username.clone()
                    } else {
                        format!("{} (@{})", a.alias, a.username)
                    }
                })
                .collect();
            let room_name = state
                .rooms
                .get(&group_id)
                .map(|r| r.display_name())
                .unwrap_or_default();

            msgs.push(DisplayMessage::system(&format!(
                "Admins of #{room_name}: {}",
                admin_names.join(", ")
            )));
        }

        Command::Invited => {
            let group_id = state
                .active_room
                .ok_or_else(|| Error::Other("no active room -- use /join first".into()))?;

            let invites = {
                let api_guard = api.lock().await;
                operations::list_group_pending_invites(&api_guard, group_id).await?
            };

            if invites.is_empty() {
                msgs.push(DisplayMessage::system("No pending invites for this room."));
            } else {
                let room_name = state
                    .rooms
                    .get(&group_id)
                    .map(|r| r.display_name())
                    .unwrap_or_default();
                msgs.push(DisplayMessage::system(&format!(
                    "Pending invites for #{room_name}:"
                )));
                for invite in &invites {
                    let invitee_name = state
                        .rooms
                        .get(&group_id)
                        .and_then(|room| {
                            room.members
                                .iter()
                                .find(|m| m.user_id == invite.invitee_id)
                                .map(|m| m.display_name().to_string())
                        })
                        .unwrap_or_else(|| format!("user#{}", invite.invitee_id));
                    msgs.push(DisplayMessage::system(&format!(
                        "  {invitee_name} — invited by {}",
                        invite.inviter_username
                    )));
                }
            }
        }

        Command::Uninvite {
            username: target_username,
        } => {
            let group_id = state
                .active_room
                .ok_or_else(|| Error::Other("no active room -- use /join first".into()))?;

            // Resolve username → user_id via API lookup.
            let user_info = api
                .lock()
                .await
                .get_user_by_username(&target_username)
                .await?;

            {
                let api_guard = api.lock().await;
                operations::cancel_invite(&api_guard, group_id, user_info.user_id).await?;
            }

            msgs.push(DisplayMessage::system(&format!(
                "Cancelled invitation for {target_username}"
            )));
        }

        Command::Members => {
            if let Some(room) = state.active_room_info() {
                let name = room.display_name();
                let member_display: Vec<String> = room
                    .members
                    .iter()
                    .map(|m| {
                        let verification = msg_store
                            .as_ref()
                            .and_then(|store| {
                                m.signing_key_fingerprint
                                    .as_ref()
                                    .map(|fp| store.get_verification_status(m.user_id, fp))
                            })
                            .unwrap_or(conclave_client::state::VerificationStatus::Unknown);

                        let indicator = match verification {
                            conclave_client::state::VerificationStatus::Changed => "[!] ",
                            conclave_client::state::VerificationStatus::Unknown
                            | conclave_client::state::VerificationStatus::Unverified => "[?] ",
                            conclave_client::state::VerificationStatus::Verified => "",
                        };

                        if m.role == "admin" {
                            format!("{indicator}{} (admin)", m.display_name())
                        } else {
                            format!("{indicator}{}", m.display_name())
                        }
                    })
                    .collect();
                msgs.push(DisplayMessage::system(&format!(
                    "Members of #{name}: {}",
                    member_display.join(", ")
                )));
            } else {
                msgs.push(DisplayMessage::system("No active room."));
            }
        }

        _ => {}
    }

    Ok((msgs, false))
}

async fn execute_invite(
    cmd: Command,
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    config: &ClientConfig,
    msg_store: &Option<MessageStore>,
) -> Result<(Vec<DisplayMessage>, bool)> {
    let mut msgs = Vec::new();

    match cmd {
        Command::Invites => {
            let invites = {
                let api_guard = api.lock().await;
                operations::list_pending_invites(&api_guard).await?
            };

            if invites.is_empty() {
                msgs.push(DisplayMessage::system("No pending invitations."));
            } else {
                for invite in &invites {
                    let display = if invite.group_alias.is_empty() {
                        &invite.group_name
                    } else {
                        &invite.group_alias
                    };
                    msgs.push(DisplayMessage::system(&format!(
                        "[{}] #{display} — invited by {}",
                        invite.invite_id, invite.inviter_username
                    )));
                }
                msgs.push(DisplayMessage::system(
                    "Use /accept <id> or /decline <id> to respond.",
                ));
            }
        }

        Command::Accept { invite_id } => {
            let user_id = require_user_id(state)?;
            let invites = if let Some(id) = invite_id {
                vec![id]
            } else {
                let pending = {
                    let api_guard = api.lock().await;
                    operations::list_pending_invites(&api_guard).await?
                };
                pending.iter().map(|i| i.invite_id).collect()
            };

            if invites.is_empty() {
                msgs.push(DisplayMessage::system("No pending invitations to accept."));
                return Ok((msgs, false));
            }

            for id in &invites {
                let results = {
                    let api_guard = api.lock().await;
                    operations::accept_invite(&api_guard, *id, &config.data_dir, user_id).await?
                };

                for result in &results {
                    state
                        .group_mapping
                        .insert(result.group_id, result.mls_group_id.clone());
                    let id_string = result.group_id.to_string();
                    let display = result.group_alias.as_deref().unwrap_or(&id_string);
                    msgs.push(DisplayMessage::system(&format!(
                        "Accepted invitation to #{display} ({})",
                        result.group_id
                    )));
                }
            }

            load_rooms(api, state, msg_store).await?;

            for group_id in state.group_mapping.keys() {
                if let Some(room) = state.rooms.get_mut(group_id)
                    && room.last_seen_seq == 0
                {
                    let max_seq = match api.lock().await.get_messages(*group_id, 0).await {
                        Ok(resp) => resp.messages.last().map(|m| m.sequence_num).unwrap_or(0),
                        Err(_) => 0,
                    };
                    room.last_seen_seq = max_seq;
                }
            }
        }

        Command::Decline { invite_id } => {
            {
                let api_guard = api.lock().await;
                operations::decline_invite(&api_guard, invite_id).await?;
            }
            msgs.push(DisplayMessage::system("Invitation declined."));
        }

        _ => {}
    }

    Ok((msgs, false))
}

async fn execute_messaging(
    cmd: Command,
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    config: &ClientConfig,
    msg_store: &Option<MessageStore>,
) -> Result<(Vec<DisplayMessage>, bool)> {
    let mut msgs = Vec::new();

    match cmd {
        Command::Message { text } => {
            let group_id = state
                .active_room
                .ok_or_else(|| Error::Other("no active room -- use /join first".into()))?;
            send_to_group(api, state, config, msg_store, group_id, &text).await?;
        }

        Command::Rotate => {
            let user_id = require_user_id(state)?;
            let group_id = state
                .active_room
                .ok_or_else(|| Error::Other("no active room -- use /join first".into()))?;
            let mls_group_id = state
                .group_mapping
                .get(&group_id)
                .ok_or_else(|| Error::Other("group mapping not found".into()))?
                .clone();

            {
                let api_guard = api.lock().await;
                operations::rotate_keys(
                    &api_guard,
                    group_id,
                    &mls_group_id,
                    &config.data_dir,
                    user_id,
                )
                .await?;
            }

            msgs.push(DisplayMessage::system(
                "Keys rotated. Forward secrecy updated.",
            ));
        }

        _ => {}
    }

    Ok((msgs, false))
}

async fn execute_profile(
    cmd: Command,
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    mls: &mut Option<MlsManager>,
    config: &ClientConfig,
    msg_store: &Option<MessageStore>,
) -> Result<(Vec<DisplayMessage>, bool)> {
    let mut msgs = Vec::new();

    match cmd {
        Command::Alias { alias } => {
            api.lock().await.update_profile(&alias).await?;
            msgs.push(DisplayMessage::system(&format!("Alias set to: {alias}")));
        }

        Command::Topic { topic } => {
            let group_id = state
                .active_room
                .ok_or_else(|| Error::Other("no active room -- use /join first".into()))?;

            api.lock()
                .await
                .update_group(group_id, Some(&topic))
                .await?;
            msgs.push(DisplayMessage::system(&format!(
                "Room alias set to: {topic}"
            )));
        }

        Command::Whois { username: None } => {
            let resp = api.lock().await.me().await?;
            msgs.push(DisplayMessage::system(&format!(
                "User: {} (ID: {})",
                resp.username, resp.user_id
            )));
            if let Some(mls_mgr) = mls {
                let fp = mls_mgr.signing_key_fingerprint();
                let formatted = conclave_client::mls::format_fingerprint(&fp);
                msgs.push(DisplayMessage::system(&format!("Fingerprint: {formatted}")));
            }
        }

        Command::Whois {
            username: Some(target),
        } => {
            let resp = api.lock().await.get_user_by_username(&target).await?;
            msgs.push(DisplayMessage::system(&format!(
                "User: {} (ID: {})",
                resp.username, resp.user_id
            )));
            if !resp.signing_key_fingerprint.is_empty() {
                let formatted =
                    conclave_client::mls::format_fingerprint(&resp.signing_key_fingerprint);
                msgs.push(DisplayMessage::system(&format!("Fingerprint: {formatted}")));
                if let Some(store) = msg_store {
                    let status =
                        store.get_verification_status(resp.user_id, &resp.signing_key_fingerprint);
                    let label = match status {
                        conclave_client::state::VerificationStatus::Unknown => "Unknown",
                        conclave_client::state::VerificationStatus::Unverified => "Unverified",
                        conclave_client::state::VerificationStatus::Verified => "Verified",
                        conclave_client::state::VerificationStatus::Changed => "Changed (warning!)",
                    };
                    msgs.push(DisplayMessage::system(&format!("Status: {label}")));
                }
            } else {
                msgs.push(DisplayMessage::system("Fingerprint: not available"));
            }
        }

        Command::Verify {
            username,
            fingerprint,
        } => {
            let normalized = conclave_client::mls::normalize_fingerprint(&fingerprint)?;
            let resp = api.lock().await.get_user_by_username(&username).await?;

            if let Some(store) = msg_store {
                let stored = store.get_stored_fingerprint(resp.user_id);
                match stored {
                    Some((stored_fp, _)) if stored_fp == normalized => {
                        store.verify_user(resp.user_id);
                        state.verification_status.insert(
                            resp.user_id,
                            conclave_client::state::VerificationStatus::Verified,
                        );
                        msgs.push(DisplayMessage::system(&format!(
                            "Verified {username}'s signing key fingerprint."
                        )));
                    }
                    Some((stored_fp, _)) => {
                        let stored_fmt = conclave_client::mls::format_fingerprint(&stored_fp);
                        let provided_fmt = conclave_client::mls::format_fingerprint(&normalized);
                        msgs.push(DisplayMessage::system("Fingerprint mismatch!"));
                        msgs.push(DisplayMessage::system(&format!("  Stored:   {stored_fmt}")));
                        msgs.push(DisplayMessage::system(&format!(
                            "  Provided: {provided_fmt}"
                        )));
                    }
                    None => {
                        msgs.push(DisplayMessage::system(&format!(
                            "No stored fingerprint for {username}. Use /whois {username} first."
                        )));
                    }
                }
            } else {
                msgs.push(DisplayMessage::system(
                    "Message store not available for verification.",
                ));
            }
        }

        Command::Unverify { username } => {
            let resp = api.lock().await.get_user_by_username(&username).await?;
            if let Some(store) = msg_store {
                if store.unverify_user(resp.user_id) {
                    state.verification_status.insert(
                        resp.user_id,
                        conclave_client::state::VerificationStatus::Unverified,
                    );
                    msgs.push(DisplayMessage::system(&format!(
                        "Removed verification for {username}."
                    )));
                } else {
                    msgs.push(DisplayMessage::system(&format!(
                        "No stored fingerprint for {username}."
                    )));
                }
            } else {
                msgs.push(DisplayMessage::system("Message store not available."));
            }
        }

        Command::Trusted => {
            if let Some(store) = msg_store {
                let entries = store.get_all_known_fingerprints();
                if entries.is_empty() {
                    msgs.push(DisplayMessage::system("No known fingerprints."));
                } else {
                    msgs.push(DisplayMessage::system("Known fingerprints:"));
                    for (user_id, fingerprint, verified, key_changed) in &entries {
                        let display_name = resolve_display_name(*user_id, state);
                        let status = if *key_changed {
                            "Changed"
                        } else if *verified {
                            "Verified"
                        } else {
                            "Unverified"
                        };
                        let formatted = conclave_client::mls::format_fingerprint(fingerprint);
                        msgs.push(DisplayMessage::system(&format!(
                            "  {display_name}: [{status}] {formatted}"
                        )));
                    }
                }
            } else {
                msgs.push(DisplayMessage::system("Message store not available."));
            }
        }

        Command::Passwd { new_password } => {
            api.lock().await.change_password(&new_password).await?;
            msgs.push(DisplayMessage::system("Password changed successfully."));
        }

        Command::Help => {
            for line in conclave_client::command::format_help_lines() {
                msgs.push(DisplayMessage::system(&line));
            }
        }

        Command::Quit => {
            // Handled by caller.
        }

        Command::Reset => {
            let user_id = require_user_id(state)?;

            let result = {
                let api_guard = api.lock().await;
                operations::reset_account(&api_guard, &config.data_dir, user_id).await?
            };

            // Re-initialize because reset_account wiped and regenerated MLS state.
            *mls = Some(MlsManager::new(&config.data_dir, user_id)?);
            state.group_mapping = result.new_group_mapping;

            for error in &result.errors {
                msgs.push(DisplayMessage::system(error));
            }

            msgs.push(DisplayMessage::system(&format!(
                "Account reset complete. Rejoined {}/{} groups.",
                result.rejoin_count, result.total_groups
            )));
        }

        _ => {}
    }

    Ok((msgs, false))
}

fn require_user_id(state: &AppState) -> Result<i64> {
    state
        .user_id
        .ok_or_else(|| Error::Other("not logged in".into()))
}

fn resolve_display_name(user_id: i64, state: &AppState) -> String {
    for room in state.rooms.values() {
        if let Some(member) = room.members.iter().find(|m| m.user_id == user_id) {
            return member.display_name().to_string();
        }
    }
    format!("user#{user_id}")
}

pub async fn load_rooms(
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    msg_store: &Option<MessageStore>,
) -> Result<Vec<operations::RoomInfo>> {
    let rooms = {
        let api_guard = api.lock().await;
        operations::load_rooms(&api_guard).await?
    };

    let mut server_group_ids = std::collections::HashSet::new();

    for room_info in &rooms {
        server_group_ids.insert(room_info.group_id);

        // Preserve existing sequence counters so we don't re-fetch old messages.
        let (existing_seq, existing_read) = state
            .rooms
            .get(&room_info.group_id)
            .map(|r| (r.last_seen_seq, r.last_read_seq))
            .unwrap_or((0, 0));

        state.rooms.insert(
            room_info.group_id,
            Room {
                server_group_id: room_info.group_id,
                group_name: room_info.group_name.clone(),
                alias: room_info.alias.clone(),
                members: room_info
                    .members
                    .iter()
                    .map(|m| m.to_room_member())
                    .collect(),
                last_seen_seq: existing_seq,
                last_read_seq: existing_read,
                message_expiry_seconds: room_info.message_expiry_seconds,
            },
        );
    }

    let stale_ids: Vec<i64> = state
        .rooms
        .keys()
        .filter(|id| !server_group_ids.contains(id))
        .copied()
        .collect();

    if !stale_ids.is_empty() {
        for id in &stale_ids {
            state.rooms.remove(id);
            state.group_mapping.remove(id);
        }

        if let Some(active) = state.active_room
            && stale_ids.contains(&active)
        {
            state.active_room = None;
        }
    }

    // Update TOFU verification status for all members.
    if let Some(store) = msg_store {
        for room in state.rooms.values() {
            for member in &room.members {
                // Implicitly trust our own signing key.
                if Some(member.user_id) == state.user_id {
                    state.verification_status.insert(
                        member.user_id,
                        conclave_client::state::VerificationStatus::Verified,
                    );
                    continue;
                }
                if let Some(fp) = &member.signing_key_fingerprint {
                    let status = store.get_verification_status(member.user_id, fp);
                    state.verification_status.insert(member.user_id, status);
                }
            }
        }
    }

    Ok(rooms)
}

async fn send_to_group(
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    config: &ClientConfig,
    msg_store: &Option<MessageStore>,
    group_id: i64,
    text: &str,
) -> Result<()> {
    let user_id = require_user_id(state)?;
    let mls_group_id = state
        .group_mapping
        .get(&group_id)
        .ok_or_else(|| Error::Other("group mapping not found".into()))?
        .clone();

    let result = {
        let api_guard = api.lock().await;
        operations::send_message(
            &api_guard,
            group_id,
            &mls_group_id,
            text,
            &config.data_dir,
            user_id,
        )
        .await?
    };

    let sender = state.username.clone().unwrap_or_default();
    let mut msg = DisplayMessage::user(user_id, &sender, text, chrono::Local::now().timestamp());
    msg.sequence_num = Some(result.sequence_num);
    msg.epoch = Some(result.epoch);
    if let Some(store) = msg_store {
        store.push_message(group_id, &msg);
    }
    state.push_room_message(group_id, msg);

    if let Some(room) = state.rooms.get_mut(&group_id) {
        room.last_seen_seq = room.last_seen_seq.max(result.sequence_num);
        if let Some(store) = msg_store {
            store.set_last_seen_seq(group_id, room.last_seen_seq);
        }
    }

    Ok(())
}
