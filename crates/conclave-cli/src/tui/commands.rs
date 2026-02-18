use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use tokio::sync::Mutex;

use conclave_lib::api::ApiClient;
pub use conclave_lib::command::Command;
use conclave_lib::config::{ClientConfig, SessionState};
use conclave_lib::mls::MlsManager;

pub use conclave_lib::command::parse;

use super::state::{AppState, DisplayMessage, Room};
use super::store::MessageStore;

type Result<T> = conclave_lib::error::Result<T>;
type Error = conclave_lib::error::Error;

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
    let mut msgs = Vec::new();
    let mut start_sse = false;

    match cmd {
        Command::Register {
            server,
            username,
            password,
        } => {
            let reg_api = ApiClient::new(&server, config.accept_invalid_certs);
            let resp = reg_api.register(&username, &password).await?;
            msgs.push(DisplayMessage::system(&format!(
                "Registered as user ID {} on {server}",
                resp.user_id
            )));
        }

        Command::Login {
            server,
            username,
            password,
        } => {
            let mut new_api = ApiClient::new(&server, config.accept_invalid_certs);
            let resp = new_api.login(&username, &password).await?;

            new_api.set_token(resp.token.clone());
            *api.lock().await = new_api;

            state.username = Some(username.clone());
            state.user_id = Some(resp.user_id);
            state.logged_in = true;

            // Save session.
            let session = SessionState {
                server_url: Some(server),
                token: Some(resp.token),
                user_id: Some(resp.user_id),
                username: Some(username.clone()),
            };
            session.save(&config.data_dir)?;

            // Initialize MLS identity.
            std::fs::create_dir_all(&config.data_dir)?;
            *mls = Some(MlsManager::new(&config.data_dir, &username)?);

            // Auto-generate and upload key packages (1 last-resort + 5 regular).
            if let Some(mls_mgr) = &*mls {
                match super::generate_initial_key_packages(mls_mgr) {
                    Ok(entries) => {
                        let count = entries.len();
                        if let Err(e) = api.lock().await.upload_key_packages(entries).await {
                            msgs.push(DisplayMessage::system(&format!(
                                "Warning: failed to upload key packages: {e}"
                            )));
                        } else {
                            msgs.push(DisplayMessage::system(&format!(
                                "{count} key packages uploaded."
                            )));
                        }
                    }
                    Err(e) => {
                        msgs.push(DisplayMessage::system(&format!(
                            "Warning: failed to generate key packages: {e}"
                        )));
                    }
                }
            }

            // Load group mapping.
            state.group_mapping = load_group_mapping(&config.data_dir);

            // Load rooms from server.
            load_rooms(api, state).await?;

            msgs.push(DisplayMessage::system(&format!(
                "Logged in as {username} (user ID {})",
                resp.user_id
            )));
            start_sse = true;
        }

        Command::Create { name, members } => {
            let mls_mgr = mls
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;

            let resp = api.lock().await.create_group(&name, members).await?;
            let server_group_id = resp.group_id.clone();

            let (mls_group_id, commit_bytes, welcome_map, group_info_bytes) =
                mls_mgr.create_group(&resp.member_key_packages)?;

            api.lock()
                .await
                .upload_commit(
                    &server_group_id,
                    commit_bytes,
                    welcome_map,
                    group_info_bytes,
                )
                .await?;

            // Save group mapping.
            state
                .group_mapping
                .insert(server_group_id.clone(), mls_group_id.clone());
            save_group_mapping(mls_mgr.data_dir(), &state.group_mapping);

            // Refresh rooms from server to get member list.
            load_rooms(api, state).await?;

            // Auto-switch to the new room.
            state.active_room = Some(server_group_id.clone());
            state.scroll_offset = 0;

            msgs.push(DisplayMessage::system(&format!(
                "Created and joined #{name} ({server_group_id})"
            )));
        }

        Command::Join { target: None } => {
            // Accept pending welcomes.
            let mls_mgr = mls
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;

            let resp = api.lock().await.list_pending_welcomes().await?;
            if resp.welcomes.is_empty() {
                msgs.push(DisplayMessage::system("No pending invitations."));
                return Ok((msgs, start_sse));
            }

            let mut last_group_id = None;
            let mut joined_group_ids = Vec::new();

            for welcome in &resp.welcomes {
                let mls_group_id = mls_mgr.join_group(&welcome.welcome_message)?;

                // Delete the welcome from the server so it is not re-processed.
                let _ = api.lock().await.accept_welcome(welcome.welcome_id).await;

                state
                    .group_mapping
                    .insert(welcome.group_id.clone(), mls_group_id);
                last_group_id = Some(welcome.group_id.clone());
                joined_group_ids.push(welcome.group_id.clone());

                msgs.push(DisplayMessage::system(&format!(
                    "Joined #{} ({})",
                    welcome.group_name, welcome.group_id
                )));
            }

            save_group_mapping(mls_mgr.data_dir(), &state.group_mapping);

            // Key packages are single-use (RFC 9420 §10); upload a fresh
            // replacement so we remain available for future group invitations.
            let kp = mls_mgr.generate_key_package()?;
            api.lock()
                .await
                .upload_key_packages(vec![(kp, false)])
                .await?;

            // Refresh rooms and auto-switch to the last joined room.
            load_rooms(api, state).await?;
            if let Some(gid) = last_group_id {
                state.active_room = Some(gid);
                state.scroll_offset = 0;
            }

            // Advance last_seen_seq for newly joined groups so that
            // fetch_missed_messages skips the initial commit (seq 1)
            // which was already processed as part of the welcome.
            for group_id in &joined_group_ids {
                if let Some(room) = state.rooms.get_mut(group_id) {
                    if room.last_seen_seq == 0 {
                        let max_seq = match api.lock().await.get_messages(group_id, 0).await {
                            Ok(resp) => resp.messages.last().map(|m| m.sequence_num).unwrap_or(0),
                            Err(_) => 0,
                        };
                        room.last_seen_seq = max_seq;
                    }
                }
            }
        }

        Command::Join {
            target: Some(target),
        } => {
            // Switch to a room by name or ID.
            let resolved_gid = if let Some(room) = state.find_room_by_name(&target) {
                Some(room.server_group_id.clone())
            } else if state.rooms.contains_key(&target) {
                Some(target.clone())
            } else {
                None
            };

            if let Some(gid) = resolved_gid {
                let name = state.rooms[&gid].name.clone();
                state.active_room = Some(gid.clone());
                state.scroll_offset = 0;
                // Mark all messages in this room as read.
                if let Some(room) = state.rooms.get_mut(&gid) {
                    room.last_read_seq = room.last_seen_seq;
                    if let Some(store) = msg_store {
                        store.set_last_read_seq(&gid, room.last_read_seq);
                    }
                }
                msgs.push(DisplayMessage::system(&format!("Switched to #{name}")));
            } else {
                msgs.push(DisplayMessage::system(&format!(
                    "Unknown room '{target}'. Use /rooms to list available rooms."
                )));
            }
        }

        Command::Invite { members } => {
            let mls_mgr = mls
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;

            let group_id = state
                .active_room
                .as_ref()
                .ok_or_else(|| Error::Other("no active room — use /join first".into()))?
                .clone();

            let resp = api.lock().await.invite_to_group(&group_id, members).await?;

            if resp.member_key_packages.is_empty() {
                msgs.push(DisplayMessage::system("No new members to invite."));
                return Ok((msgs, start_sse));
            }

            let mls_group_id = state
                .group_mapping
                .get(&group_id)
                .ok_or_else(|| Error::Other("group mapping not found".into()))?
                .clone();

            let (commit_bytes, welcome_map, group_info_bytes) =
                mls_mgr.invite_to_group(&mls_group_id, &resp.member_key_packages)?;

            api.lock()
                .await
                .upload_commit(&group_id, commit_bytes, welcome_map, group_info_bytes)
                .await?;

            let invited: Vec<String> = resp.member_key_packages.keys().cloned().collect();
            msgs.push(DisplayMessage::system(&format!(
                "Invited {} to the room",
                invited.join(", ")
            )));

            // Refresh room info.
            load_rooms(api, state).await?;
        }

        Command::Kick { username } => {
            let mls_mgr = mls
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;

            let group_id = state
                .active_room
                .as_ref()
                .ok_or_else(|| Error::Other("no active room — use /join first".into()))?
                .clone();

            let mls_group_id = state
                .group_mapping
                .get(&group_id)
                .ok_or_else(|| Error::Other("group mapping not found".into()))?
                .clone();

            let member_index = mls_mgr
                .find_member_index(&mls_group_id, &username)?
                .ok_or_else(|| {
                    Error::Other(format!("user '{username}' not found in MLS roster"))
                })?;

            let (commit_bytes, group_info_bytes) =
                mls_mgr.remove_member(&mls_group_id, member_index)?;

            api.lock()
                .await
                .remove_member(&group_id, &username, commit_bytes, group_info_bytes)
                .await?;

            // Refresh room info.
            load_rooms(api, state).await?;

            msgs.push(DisplayMessage::system(&format!(
                "Removed {username} from the room"
            )));
        }

        Command::Leave => {
            let group_id = state
                .active_room
                .as_ref()
                .ok_or_else(|| Error::Other("no active room — use /join first".into()))?
                .clone();

            let name = state
                .rooms
                .get(&group_id)
                .map(|r| r.name.clone())
                .unwrap_or_default();

            // Notify the server to remove us from the group.
            api.lock().await.leave_group(&group_id).await?;

            // Delete local MLS group state.
            if let Some(mls_mgr) = mls.as_ref() {
                if let Some(mls_group_id) = state.group_mapping.get(&group_id) {
                    let _ = mls_mgr.delete_group_state(mls_group_id);
                }
            }

            // Remove from local state.
            state.group_mapping.remove(&group_id);
            state.rooms.remove(&group_id);
            state.active_room = None;
            state.scroll_offset = 0;

            if let Some(mls_mgr) = mls.as_ref() {
                save_group_mapping(mls_mgr.data_dir(), &state.group_mapping);
            }

            msgs.push(DisplayMessage::system(&format!("Left #{name}")));
        }

        Command::Part => {
            if let Some(room_id) = state.active_room.take() {
                let name = state
                    .rooms
                    .get(&room_id)
                    .map(|r| r.name.clone())
                    .unwrap_or_default();
                state.scroll_offset = 0;
                msgs.push(DisplayMessage::system(&format!(
                    "Switched away from #{name} (use /leave to leave the group)"
                )));
            } else {
                msgs.push(DisplayMessage::system("No active room."));
            }
        }

        Command::Rotate => {
            let mls_mgr = mls
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;

            let group_id = state
                .active_room
                .as_ref()
                .ok_or_else(|| Error::Other("no active room — use /join first".into()))?
                .clone();

            let mls_group_id = state
                .group_mapping
                .get(&group_id)
                .ok_or_else(|| Error::Other("group mapping not found".into()))?
                .clone();

            let (commit_bytes, group_info_bytes) = mls_mgr.rotate_keys(&mls_group_id)?;

            api.lock()
                .await
                .upload_commit(
                    &group_id,
                    commit_bytes,
                    std::collections::HashMap::new(),
                    group_info_bytes,
                )
                .await?;

            msgs.push(DisplayMessage::system(
                "Keys rotated. Forward secrecy updated.",
            ));
        }

        Command::Reset => {
            let mls_mgr = mls
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;

            let username = state
                .username
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?
                .clone();

            // Collect groups to rejoin before wiping state.
            let groups_to_rejoin: Vec<(String, String)> = state
                .group_mapping
                .iter()
                .map(|(server_id, mls_id)| (server_id.clone(), mls_id.clone()))
                .collect();

            // Collect old leaf indices for each group before wiping.
            let mut old_indices: HashMap<String, Option<u32>> = HashMap::new();
            for (server_id, mls_id) in &groups_to_rejoin {
                let index = mls_mgr.find_member_index(mls_id, &username).ok().flatten();
                old_indices.insert(server_id.clone(), index);
            }

            // Notify server to clear our key packages.
            api.lock().await.reset_account().await?;

            // Wipe local MLS state.
            mls_mgr.wipe_local_state()?;

            // Regenerate identity.
            *mls = Some(MlsManager::new(&config.data_dir, &username)?);
            let mls_mgr = mls.as_ref().unwrap();

            // Upload new key packages (1 last-resort + 5 regular).
            let entries = super::generate_initial_key_packages(mls_mgr)?;
            api.lock().await.upload_key_packages(entries).await?;

            // Rejoin each group via external commit.
            state.group_mapping.clear();
            let mut rejoin_count = 0;

            for (server_id, _) in &groups_to_rejoin {
                let group_info_resp = match api.lock().await.get_group_info(server_id).await {
                    Ok(resp) => resp,
                    Err(e) => {
                        msgs.push(DisplayMessage::system(&format!(
                            "Failed to get group info for {server_id}: {e}"
                        )));
                        continue;
                    }
                };

                let old_index = old_indices.get(server_id).copied().flatten();

                match mls_mgr.external_rejoin_group(&group_info_resp.group_info, old_index) {
                    Ok((new_mls_id, commit_bytes)) => {
                        if let Err(e) = api
                            .lock()
                            .await
                            .external_join(server_id, commit_bytes)
                            .await
                        {
                            msgs.push(DisplayMessage::system(&format!(
                                "Failed to rejoin {server_id}: {e}"
                            )));
                            continue;
                        }
                        state.group_mapping.insert(server_id.clone(), new_mls_id);
                        rejoin_count += 1;
                    }
                    Err(e) => {
                        msgs.push(DisplayMessage::system(&format!(
                            "Failed external commit for {server_id}: {e}"
                        )));
                    }
                }
            }

            save_group_mapping(mls_mgr.data_dir(), &state.group_mapping);

            msgs.push(DisplayMessage::system(&format!(
                "Account reset complete. Rejoined {rejoin_count}/{} groups.",
                groups_to_rejoin.len()
            )));
        }

        Command::Info => {
            let mls_mgr = mls
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;

            let group_id = state
                .active_room
                .as_ref()
                .ok_or_else(|| Error::Other("no active room — use /join first".into()))?
                .clone();

            let mls_group_id = state
                .group_mapping
                .get(&group_id)
                .ok_or_else(|| Error::Other("group mapping not found".into()))?
                .clone();

            let details = mls_mgr.group_info_details(&mls_group_id)?;

            let room_name = state
                .rooms
                .get(&group_id)
                .map(|r| r.name.as_str())
                .unwrap_or("unknown");

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
            for (index, name) in &details.members {
                let marker = if *index == details.own_index {
                    " (you)"
                } else {
                    ""
                };
                msgs.push(DisplayMessage::system(&format!(
                    "    [{index}] {name}{marker}"
                )));
            }
        }

        Command::Rooms => {
            // Refresh from server.
            load_rooms(api, state).await?;

            if state.rooms.is_empty() {
                msgs.push(DisplayMessage::system("No rooms."));
            } else {
                msgs.push(DisplayMessage::system("Rooms:"));
                for room in state.rooms.values() {
                    let active = if state.active_room.as_ref() == Some(&room.server_group_id) {
                        " (active)"
                    } else {
                        ""
                    };
                    msgs.push(DisplayMessage::system(&format!(
                        "  #{} [{}] — {}{active}",
                        room.name,
                        room.members.join(", "),
                        room.server_group_id,
                    )));
                }
            }
        }

        Command::Who => {
            if let Some(room) = state.active_room_info() {
                let name = room.name.clone();
                let members = room.members.clone();
                msgs.push(DisplayMessage::system(&format!(
                    "Members of #{name}: {}",
                    members.join(", ")
                )));
            } else {
                msgs.push(DisplayMessage::system("No active room."));
            }
        }

        Command::Msg { room, text } => {
            let mls_mgr = mls
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;

            let group_id = if let Some(r) = state.find_room_by_name(&room) {
                r.server_group_id.clone()
            } else if state.rooms.contains_key(&room) {
                room.clone()
            } else {
                return Err(Error::Other(format!("Unknown room '{room}'")));
            };

            let mls_group_id = state
                .group_mapping
                .get(&group_id)
                .ok_or_else(|| Error::Other("group mapping not found".into()))?
                .clone();

            let encrypted = mls_mgr.encrypt_message(&mls_group_id, text.as_bytes())?;
            let resp = api.lock().await.send_message(&group_id, encrypted).await?;

            // Show in the target room's messages.
            let sender = state.username.clone().unwrap_or_default();
            let msg = DisplayMessage::user(&sender, &text, chrono::Local::now().timestamp());
            if let Some(store) = msg_store {
                store.push_message(&group_id, &msg);
            }
            state.push_room_message(&group_id, msg);

            // Update last_seen_seq so we don't re-display it from SSE.
            if let Some(room_state) = state.rooms.get_mut(&group_id) {
                room_state.last_seen_seq = room_state.last_seen_seq.max(resp.sequence_num);
                if let Some(store) = msg_store {
                    store.set_last_seen_seq(&group_id, room_state.last_seen_seq);
                }
            }

            msgs.push(DisplayMessage::system(&format!("Message sent to #{room}")));
        }

        Command::Unread => {
            if !state.logged_in {
                msgs.push(DisplayMessage::system("Not logged in."));
                return Ok((msgs, start_sse));
            }

            if state.rooms.is_empty() {
                msgs.push(DisplayMessage::system("No rooms."));
                return Ok((msgs, start_sse));
            }

            let mut any_unread = false;

            for room in state.rooms.values() {
                // Messages between last_read_seq and last_seen_seq have been
                // fetched/decrypted but not yet viewed by the user.
                let local_unread = room.last_seen_seq.saturating_sub(room.last_read_seq);

                // Also check the server for messages we haven't fetched yet.
                let server_unread = match api
                    .lock()
                    .await
                    .get_messages(&room.server_group_id, room.last_seen_seq as i64)
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
                        room.name,
                        if total == 1 { "" } else { "s" },
                    )));
                }
            }

            if !any_unread {
                msgs.push(DisplayMessage::system("No unread messages."));
            }
        }

        Command::Logout => {
            if !state.logged_in {
                msgs.push(DisplayMessage::system("Not logged in."));
                return Ok((msgs, start_sse));
            }

            // Revoke session on server.
            let _ = api.lock().await.logout().await;

            // Clear local session state.
            api.lock().await.set_token(String::new());
            state.logged_in = false;
            state.username = None;
            state.user_id = None;
            state.active_room = None;
            state.rooms.clear();
            state.group_mapping.clear();
            *mls = None;

            // Delete saved session file.
            let session_path = config.data_dir.join("session.toml");
            let _ = std::fs::remove_file(session_path);

            msgs.push(DisplayMessage::system("Logged out. Session revoked."));
        }

        Command::Me => {
            let resp = api.lock().await.me().await?;
            msgs.push(DisplayMessage::system(&format!(
                "User: {} (ID: {})",
                resp.username, resp.user_id
            )));
        }

        Command::Help => {
            let help = [
                "/register <server> <user> <pass>  Register a new account",
                "/login <server> <user> <pass>     Login to the server",
                "/create <name> <user1,user2>  Create a room with members",
                "/join                         Accept pending invitations",
                "/join <room>                  Switch to a room",
                "/invite <user1,user2>         Invite to the active room",
                "/kick <username>              Remove a member from the room",
                "/leave                        Leave the room (MLS removal)",
                "/part                         Switch away without leaving",
                "/rotate                       Rotate keys (forward secrecy)",
                "/reset                        Reset account and rejoin groups",
                "/info                         Show MLS group details",
                "/rooms                        List your rooms",
                "/unread                       Check rooms for new messages",
                "/logout                       Logout and revoke session",
                "/who                          List members of active room",
                "/msg <room> <text>            Send to a room without switching",
                "/me                           Show current user info",
                "/help                         Show this help",
                "/quit                         Exit",
                "",
                "Type text without / to send a message to the active room.",
            ];
            for line in help {
                msgs.push(DisplayMessage::system(line));
            }
        }

        Command::Quit => {
            // Handled by caller.
        }

        Command::Message { text } => {
            let mls_mgr = mls
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;

            let group_id = state
                .active_room
                .as_ref()
                .ok_or_else(|| Error::Other("no active room — use /join first".into()))?
                .clone();

            let mls_group_id = state
                .group_mapping
                .get(&group_id)
                .ok_or_else(|| Error::Other("group mapping not found".into()))?
                .clone();

            let encrypted = mls_mgr.encrypt_message(&mls_group_id, text.as_bytes())?;
            let resp = api.lock().await.send_message(&group_id, encrypted).await?;

            // Show our own message locally.
            let sender = state.username.clone().unwrap_or_default();
            let msg = DisplayMessage::user(&sender, &text, chrono::Local::now().timestamp());
            if let Some(store) = msg_store {
                store.push_message(&group_id, &msg);
            }
            state.push_room_message(&group_id, msg);

            // Update last_seen_seq so we don't re-display it from SSE.
            if let Some(room) = state.rooms.get_mut(&group_id) {
                room.last_seen_seq = room.last_seen_seq.max(resp.sequence_num);
                if let Some(store) = msg_store {
                    store.set_last_seen_seq(&group_id, room.last_seen_seq);
                }
            }
        }
    }

    Ok((msgs, start_sse))
}

/// Load rooms from the server and update state.
pub async fn load_rooms(api: &Arc<Mutex<ApiClient>>, state: &mut AppState) -> Result<()> {
    let resp = api.lock().await.list_groups().await?;

    for g in &resp.groups {
        let members: Vec<String> = g.members.iter().map(|m| m.username.clone()).collect();

        let (existing_seq, existing_read) = state
            .rooms
            .get(&g.group_id)
            .map(|r| (r.last_seen_seq, r.last_read_seq))
            .unwrap_or((0, 0));

        state.rooms.insert(
            g.group_id.clone(),
            Room {
                server_group_id: g.group_id.clone(),
                name: g.name.clone(),
                members,
                last_seen_seq: existing_seq,
                last_read_seq: existing_read,
            },
        );
    }

    Ok(())
}

pub fn load_group_mapping(data_dir: &Path) -> HashMap<String, String> {
    let path = data_dir.join("group_mapping.toml");
    if path.exists() {
        let contents = std::fs::read_to_string(&path).unwrap_or_default();
        toml::from_str(&contents).unwrap_or_default()
    } else {
        HashMap::new()
    }
}

pub fn save_group_mapping(data_dir: &Path, mapping: &HashMap<String, String>) {
    let path = data_dir.join("group_mapping.toml");
    if let Ok(contents) = toml::to_string_pretty(mapping) {
        let _ = std::fs::write(&path, contents);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
    }
}
