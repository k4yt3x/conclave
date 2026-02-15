use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::api::ApiClient;
use crate::config::{ClientConfig, SessionState};
use crate::error::{Error, Result};
use crate::mls::MlsManager;

use super::state::{AppState, DisplayMessage, Room};
use super::store::MessageStore;

/// Parsed command from user input.
pub enum Command {
    Register {
        username: String,
        password: String,
    },
    Login {
        username: String,
        password: String,
    },
    Keygen,
    Create {
        name: String,
        members: Vec<String>,
    },
    /// No args: accept pending welcomes. With arg: switch to room.
    Join {
        target: Option<String>,
    },
    Invite {
        members: Vec<String>,
    },
    Part,
    Rooms,
    Who,
    Msg {
        room: String,
        text: String,
    },
    Unread,
    Me,
    Help,
    Quit,
    Message {
        text: String,
    },
}

/// Parse user input into a Command.
pub fn parse(input: &str) -> Result<Command> {
    if !input.starts_with('/') {
        return Ok(Command::Message {
            text: input.to_string(),
        });
    }

    let parts: Vec<&str> = input.splitn(3, ' ').collect();
    let cmd = parts[0];

    match cmd {
        "/register" => {
            if parts.len() < 3 {
                return Err(Error::Other(
                    "Usage: /register <username> <password>".into(),
                ));
            }
            Ok(Command::Register {
                username: parts[1].to_string(),
                password: parts[2].to_string(),
            })
        }
        "/login" => {
            if parts.len() < 3 {
                return Err(Error::Other("Usage: /login <username> <password>".into()));
            }
            Ok(Command::Login {
                username: parts[1].to_string(),
                password: parts[2].to_string(),
            })
        }
        "/keygen" => Ok(Command::Keygen),
        "/create" => {
            if parts.len() < 3 {
                return Err(Error::Other(
                    "Usage: /create <name> <member1,member2,...>".into(),
                ));
            }
            let members = parts[2].split(',').map(|s| s.trim().to_string()).collect();
            Ok(Command::Create {
                name: parts[1].to_string(),
                members,
            })
        }
        "/join" => {
            let target = parts.get(1).map(|s| s.to_string());
            Ok(Command::Join { target })
        }
        "/invite" => {
            if parts.len() < 2 {
                return Err(Error::Other("Usage: /invite <member1,member2,...>".into()));
            }
            let members = parts[1].split(',').map(|s| s.trim().to_string()).collect();
            Ok(Command::Invite { members })
        }
        "/part" | "/leave" => Ok(Command::Part),
        "/rooms" | "/list" => Ok(Command::Rooms),
        "/who" => Ok(Command::Who),
        "/msg" => {
            if parts.len() < 3 {
                return Err(Error::Other("Usage: /msg <room> <message>".into()));
            }
            Ok(Command::Msg {
                room: parts[1].to_string(),
                text: parts[2].to_string(),
            })
        }
        "/unread" => Ok(Command::Unread),
        "/me" => Ok(Command::Me),
        "/help" | "/h" => Ok(Command::Help),
        "/quit" | "/exit" | "/q" => Ok(Command::Quit),
        _ => Err(Error::Other(format!(
            "Unknown command: {cmd}. Type /help for available commands."
        ))),
    }
}

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
        Command::Register { username, password } => {
            let resp = api.lock().await.register(&username, &password).await?;
            msgs.push(DisplayMessage::system(&format!(
                "Registered as user ID {}",
                resp.user_id
            )));
        }

        Command::Login { username, password } => {
            let resp = api.lock().await.login(&username, &password).await?;

            api.lock().await.set_token(resp.token.clone());

            state.username = Some(username.clone());
            state.user_id = Some(resp.user_id);
            state.logged_in = true;

            // Save session.
            let session = SessionState {
                token: Some(resp.token),
                user_id: Some(resp.user_id),
                username: Some(username.clone()),
            };
            session.save(&config.data_dir)?;

            // Initialize MLS identity.
            std::fs::create_dir_all(&config.data_dir)?;
            *mls = Some(MlsManager::new(&config.data_dir, &username)?);

            // Load group mapping.
            let udir = user_data_dir(&config.data_dir, &username);
            state.group_mapping = load_group_mapping(&udir);

            // Load rooms from server.
            load_rooms(api, state).await?;

            msgs.push(DisplayMessage::system(&format!(
                "Logged in as {username} (user ID {})",
                resp.user_id
            )));
            start_sse = true;
        }

        Command::Keygen => {
            let mls_mgr = mls
                .as_ref()
                .ok_or_else(|| Error::Other("not logged in".into()))?;
            let kp = mls_mgr.generate_key_package()?;
            api.lock().await.upload_key_package(kp).await?;
            msgs.push(DisplayMessage::system(
                "Key package generated and uploaded.",
            ));
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
            save_group_mapping(mls_mgr.user_data_dir(), &state.group_mapping);

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

            for welcome in &resp.welcomes {
                let mls_group_id = mls_mgr.join_group(&welcome.welcome_message)?;

                state
                    .group_mapping
                    .insert(welcome.group_id.clone(), mls_group_id);
                last_group_id = Some(welcome.group_id.clone());

                msgs.push(DisplayMessage::system(&format!(
                    "Joined #{} ({})",
                    welcome.group_name, welcome.group_id
                )));
            }

            save_group_mapping(mls_mgr.user_data_dir(), &state.group_mapping);

            // Refresh rooms and auto-switch to the last joined room.
            load_rooms(api, state).await?;
            if let Some(gid) = last_group_id {
                state.active_room = Some(gid);
                state.scroll_offset = 0;
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

        Command::Part => {
            if let Some(room_id) = state.active_room.take() {
                let name = state
                    .rooms
                    .get(&room_id)
                    .map(|r| r.name.clone())
                    .unwrap_or_default();
                state.scroll_offset = 0;
                msgs.push(DisplayMessage::system(&format!("Left #{name}")));
            } else {
                msgs.push(DisplayMessage::system("No active room."));
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

        Command::Me => {
            let resp = api.lock().await.me().await?;
            msgs.push(DisplayMessage::system(&format!(
                "User: {} (ID: {})",
                resp.username, resp.user_id
            )));
        }

        Command::Help => {
            let help = [
                "/register <user> <pass>       Register a new account",
                "/login <user> <pass>          Login to the server",
                "/keygen                       Generate and upload a key package",
                "/create <name> <user1,user2>  Create a room with members",
                "/join                         Accept pending invitations",
                "/join <room>                  Switch to a room",
                "/invite <user1,user2>         Invite to the active room",
                "/part                         Leave the active room",
                "/rooms                        List your rooms",
                "/unread                       Check rooms for new messages",
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

fn user_data_dir(data_dir: &Path, username: &str) -> std::path::PathBuf {
    data_dir.join("users").join(username)
}

pub fn load_group_mapping(user_dir: &Path) -> HashMap<String, String> {
    let path = user_dir.join("group_mapping.toml");
    if path.exists() {
        let contents = std::fs::read_to_string(&path).unwrap_or_default();
        toml::from_str(&contents).unwrap_or_default()
    } else {
        HashMap::new()
    }
}

pub fn save_group_mapping(user_dir: &Path, mapping: &HashMap<String, String>) {
    let path = user_dir.join("group_mapping.toml");
    if let Ok(contents) = toml::to_string_pretty(mapping) {
        let _ = std::fs::write(path, contents);
    }
}
