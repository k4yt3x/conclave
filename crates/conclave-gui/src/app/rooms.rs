use std::collections::HashSet;

use iced::Task;

use conclave_client::config::build_group_mapping;
use conclave_client::mls::MlsManager;
use conclave_client::operations;
use conclave_client::state::{DisplayMessage, Room};

use super::{Conclave, Message};

impl Conclave {
    // Result handlers

    pub(crate) fn handle_group_created(
        &mut self,
        result: Result<operations::GroupCreatedResult, String>,
    ) -> Task<Message> {
        match result {
            Ok(info) => {
                self.group_mapping
                    .insert(info.server_group_id, info.mls_group_id);

                self.active_room = Some(info.server_group_id);
                self.push_system_message(&format!("Group created ({})", info.server_group_id));

                self.load_rooms_task()
            }
            Err(e) => {
                self.push_system_message(&format!("Failed to create group: {e}"));
                Task::none()
            }
        }
    }

    pub(crate) fn handle_rooms_loaded(
        &mut self,
        result: Result<Vec<operations::RoomInfo>, String>,
    ) -> Task<Message> {
        match result {
            Ok(room_infos) => {
                self.group_mapping = build_group_mapping(&room_infos, &self.config.data_dir);
                let mut server_group_ids = HashSet::new();

                for info in room_infos {
                    server_group_ids.insert(info.group_id);

                    // Preserve sequence tracking from the existing room state
                    // so we don't re-fetch messages we've already seen.
                    let (existing_seq, existing_read) = self
                        .rooms
                        .get(&info.group_id)
                        .map(|r| (r.last_seen_seq, r.last_read_seq))
                        .unwrap_or((0, 0));

                    // Take the max of in-memory and persisted values so we
                    // never regress the sequence counters across restarts.
                    let (seq, read) = if let Some(store) = &self.msg_store {
                        let s = store.get_last_seen_seq(info.group_id);
                        let r = store.get_last_read_seq(info.group_id);
                        (s.max(existing_seq), r.max(existing_read))
                    } else {
                        (existing_seq, existing_read)
                    };

                    self.rooms.insert(
                        info.group_id,
                        Room {
                            server_group_id: info.group_id,
                            group_name: info.group_name,
                            alias: info.alias,
                            members: info.members.iter().map(|m| m.to_room_member()).collect(),
                            last_seen_seq: seq,
                            last_read_seq: read,
                            message_expiry_seconds: info.message_expiry_seconds,
                        },
                    );

                    // Load persisted message history for rooms we haven't
                    // loaded yet (first login, or rooms added since last load).
                    if let Some(store) = &self.msg_store
                        && let std::collections::hash_map::Entry::Vacant(entry) =
                            self.room_messages.entry(info.group_id)
                    {
                        let history = store.load_messages(info.group_id);
                        if !history.is_empty() {
                            entry.insert(history);
                        }
                    }
                }

                // Prune rooms that the server no longer returns.
                let stale_ids: Vec<i64> = self
                    .rooms
                    .keys()
                    .filter(|id| !server_group_ids.contains(id))
                    .copied()
                    .collect();

                if !stale_ids.is_empty() {
                    for id in &stale_ids {
                        self.rooms.remove(id);
                        self.group_mapping.remove(id);
                    }

                    if let Some(active) = self.active_room
                        && stale_ids.contains(&active)
                    {
                        self.active_room = None;
                    }
                }
            }
            Err(e) => {
                self.push_system_message(&format!("Failed to load rooms: {e}"));
                return Task::none();
            }
        }

        // On the very first load (startup catch-up), fetch missed messages
        // for all rooms. Subsequent calls (from /rooms, invite, kick, etc.)
        // skip this since SSE is already connected and last_seen_seq is
        // up to date.
        let was_loaded = self.rooms_loaded;
        self.rooms_loaded = true;

        // On first load, detect groups with no local MLS state (stale after data loss).
        if !was_loaded {
            let unmapped_count = self
                .rooms
                .keys()
                .filter(|gid| !self.group_mapping.contains_key(gid))
                .count();
            if unmapped_count > 0 {
                self.push_system_message(&format!(
                    "{unmapped_count} group(s) have no local encryption state. \
                     Run /reset to rejoin them with a new identity."
                ));
            }
        }

        // Deferred fetch: welcome processing triggered a rooms reload and
        // asked us to fetch missed messages once the rooms are present.
        if self.fetch_messages_on_rooms_load {
            self.fetch_messages_on_rooms_load = false;
            return self.fetch_all_missed_messages();
        }

        if was_loaded || self.rooms.is_empty() {
            return Task::none();
        }

        // Wait for pending welcomes to be processed before fetching.
        // Welcomes create group mappings needed for decryption.
        if !self.welcomes_processed {
            return Task::none();
        }

        self.fetch_all_missed_messages()
    }

    pub(crate) fn handle_messages_fetched(
        &mut self,
        result: Result<operations::FetchedMessages, (i64, String)>,
    ) -> Task<Message> {
        match result {
            Ok(fetched) => {
                self.fetching_groups.remove(&fetched.group_id);

                // Decrypt and display each message, updating sequence tracking.
                for msg in &fetched.messages {
                    let mut display = if msg.is_system {
                        DisplayMessage::system(&msg.content)
                    } else {
                        DisplayMessage::user(
                            msg.sender_id,
                            &msg.sender,
                            &msg.content,
                            msg.timestamp,
                        )
                    };
                    display.sequence_num = Some(msg.sequence_num);
                    display.epoch = Some(msg.epoch);

                    self.add_message_to_room(fetched.group_id, display);

                    if let Some(room) = self.rooms.get_mut(&fetched.group_id) {
                        room.last_seen_seq = room.last_seen_seq.max(msg.sequence_num);
                        if let Some(store) = &self.msg_store {
                            store.set_last_seen_seq(fetched.group_id, room.last_seen_seq);
                        }
                    }
                }

                // Auto-mark as read if this is the active room.
                if self.active_room == Some(fetched.group_id)
                    && let Some(room) = self.rooms.get_mut(&fetched.group_id)
                {
                    room.last_read_seq = room.last_seen_seq;
                    if let Some(store) = &self.msg_store {
                        store.set_last_read_seq(fetched.group_id, room.last_read_seq);
                    }
                }

                // Clean up locally cached messages that exceed the group's
                // expiry policy (both SQLite store and in-memory display).
                if let Some(store) = &self.msg_store
                    && let Some(room) = self.rooms.get(&fetched.group_id)
                {
                    store.cleanup_expired_messages(
                        fetched.group_id,
                        room.message_expiry_seconds,
                    );
                }
                conclave_client::state::remove_expired_messages(
                    &self.rooms,
                    &mut self.room_messages,
                );

                // Desktop notification for messages received while unfocused.
                if !self.window_focused {
                    let last_msg = fetched.messages.iter().rev().find(|m| !m.is_system);
                    if let Some(msg) = last_msg {
                        let room_name = self
                            .rooms
                            .get(&fetched.group_id)
                            .map(|r| r.display_name())
                            .unwrap_or_else(|| "unknown".to_string());
                        crate::notification::send_notification(
                            &format!("#{room_name} - {}", msg.sender),
                            &msg.content,
                        );
                    }
                }
            }
            Err((group_id, error)) => {
                self.fetching_groups.remove(&group_id);
                self.push_system_message(&format!("Failed to fetch messages: {error}"));
            }
        }
        Task::none()
    }

    pub(crate) fn handle_message_sent(
        &mut self,
        result: Result<(operations::MessageSentResult, String), String>,
    ) -> Task<Message> {
        match result {
            Ok((info, text)) => {
                let sender_id = self.user_id.unwrap_or(0);
                let sender = self
                    .user_alias
                    .as_deref()
                    .filter(|a| !a.is_empty())
                    .unwrap_or_else(|| self.username.as_deref().unwrap_or_default())
                    .to_string();
                let mut msg = DisplayMessage::user(
                    sender_id,
                    &sender,
                    &text,
                    chrono::Local::now().timestamp(),
                );
                msg.sequence_num = Some(info.sequence_num);
                msg.epoch = Some(info.epoch);
                self.add_message_to_room(info.group_id, msg);

                if let Some(room) = self.rooms.get_mut(&info.group_id) {
                    room.last_seen_seq = room.last_seen_seq.max(info.sequence_num);
                    room.last_read_seq = room.last_seen_seq;
                    if let Some(store) = &self.msg_store {
                        store.set_last_seen_seq(info.group_id, room.last_seen_seq);
                        store.set_last_read_seq(info.group_id, room.last_read_seq);
                    }
                }
            }
            Err(e) => {
                self.push_system_message(&format!("Failed to send message: {e}"));
            }
        }
        Task::none()
    }

    pub(crate) fn handle_welcomes_processed(
        &mut self,
        result: Result<Vec<operations::WelcomeJoinResult>, String>,
    ) -> Task<Message> {
        let was_processed = self.welcomes_processed;
        self.welcomes_processed = true;

        match result {
            Ok(welcomes) => {
                for w in &welcomes {
                    self.group_mapping
                        .insert(w.group_id, w.mls_group_id.clone());
                    let group_id_str = w.group_id.to_string();
                    let display = w
                        .group_alias
                        .as_deref()
                        .filter(|a| !a.is_empty())
                        .unwrap_or(&group_id_str);
                    self.push_system_message(&format!("Joined #{display} ({})", w.group_id));
                }

                // Defer the missed-message fetch until rooms_task completes
                // so that newly joined groups are in self.rooms when
                // fetch_all_missed_messages iterates over them.
                if !was_processed {
                    self.fetch_messages_on_rooms_load = true;
                }
                self.load_rooms_task()
            }
            Err(e) => {
                self.push_system_message(&format!("Failed to process welcomes: {e}"));

                // Even on error, the initial fetch should proceed so
                // rooms that already have mappings get their messages.
                if !was_processed {
                    self.fetch_messages_on_rooms_load = true;
                    return self.load_rooms_task();
                }
                Task::none()
            }
        }
    }

    pub(crate) fn handle_reset_complete(
        &mut self,
        result: Result<operations::ResetResult, String>,
    ) -> Task<Message> {
        match result {
            Ok(info) => {
                self.group_mapping = info.new_group_mapping;

                // Re-initialize the MLS manager since reset_account wiped
                // and regenerated state internally.
                if let Some(uid) = self.user_id
                    && let Ok(mls) = MlsManager::new(&self.config.data_dir, uid)
                {
                    self.mls = Some(mls);
                }

                self.fetching_groups.clear();

                for error in &info.errors {
                    self.push_system_message(error);
                }
                self.push_system_message(&format!(
                    "Account reset complete. Rejoined {}/{} groups.",
                    info.rejoin_count, info.total_groups
                ));

                self.load_rooms_task()
            }
            Err(error) => {
                self.push_system_message(&format!("Reset failed: {error}"));
                Task::none()
            }
        }
    }

    // Message sending

    pub(crate) fn send_message(&mut self, text: String) -> Task<Message> {
        let group_id = match self.active_room {
            Some(id) => id,
            None => {
                self.push_system_message("No active room — use /join first");
                return Task::none();
            }
        };

        self.send_to_group(group_id, &text)
    }

    pub(crate) fn send_to_room(&mut self, room: &str, text: &str) -> Task<Message> {
        let group_id = if let Some(r) = self.find_room_by_name(room) {
            r.server_group_id
        } else if let Ok(id) = room.parse::<i64>() {
            if self.rooms.contains_key(&id) {
                id
            } else {
                self.push_system_message(&format!("Unknown room '{room}'"));
                return Task::none();
            }
        } else {
            self.push_system_message(&format!("Unknown room '{room}'"));
            return Task::none();
        };

        self.send_to_group(group_id, text)
    }

    fn send_to_group(&mut self, group_id: i64, text: &str) -> Task<Message> {
        let mls_group_id = match self.group_mapping.get(&group_id) {
            Some(id) => id.clone(),
            None => {
                self.push_system_message("Group mapping not found");
                return Task::none();
            }
        };

        let user_id = match self.user_id {
            Some(id) => id,
            None => {
                self.push_system_message("Not logged in");
                return Task::none();
            }
        };

        let params = self.api_params();
        let data_dir = self.config.data_dir.clone();
        let text = text.to_string();

        Task::perform(
            async move {
                let api = params.into_client();
                let result = operations::send_message(
                    &api,
                    group_id,
                    &mls_group_id,
                    &text,
                    &data_dir,
                    user_id,
                )
                .await
                .map_err(|e| e.to_string())?;
                Ok((result, text))
            },
            Message::MessageSent,
        )
    }

    // Group lifecycle

    pub(crate) fn create_group(&mut self, name: String) -> Task<Message> {
        let user_id = match self.user_id {
            Some(id) => id,
            None => {
                self.push_system_message("Not logged in");
                return Task::none();
            }
        };

        let params = self.api_params();
        let data_dir = self.config.data_dir.clone();

        Task::perform(
            async move {
                let api = params.into_client();
                operations::create_group(&api, None, &name, &data_dir, user_id)
                    .await
                    .map_err(|e| e.to_string())
            },
            Message::GroupCreated,
        )
    }

    pub(crate) fn invite_members(&mut self, members: Vec<String>) -> Task<Message> {
        let group_id = match self.active_room {
            Some(id) => id,
            None => {
                self.push_system_message("No active room — use /join first");
                return Task::none();
            }
        };

        let mls_group_id = match self.group_mapping.get(&group_id) {
            Some(id) => id.clone(),
            None => {
                self.push_system_message("Group mapping not found");
                return Task::none();
            }
        };

        let user_id = match self.user_id {
            Some(id) => id,
            None => {
                self.push_system_message("Not logged in");
                return Task::none();
            }
        };

        let params = self.api_params();
        let data_dir = self.config.data_dir.clone();

        Task::perform(
            async move {
                let api = params.into_client();

                // Resolve usernames to user IDs at the UI boundary.
                let mut member_ids = Vec::new();
                for username in &members {
                    let user_info = api
                        .get_user_by_username(username)
                        .await
                        .map_err(|e| e.to_string())?;
                    member_ids.push(user_info.user_id);
                }

                let invited = operations::invite_members(
                    &api,
                    group_id,
                    &mls_group_id,
                    member_ids.clone(),
                    &data_dir,
                    user_id,
                )
                .await
                .map_err(|e| e.to_string())?;

                if invited.is_empty() {
                    Ok(vec![DisplayMessage::system("No new members to invite.")])
                } else {
                    // Map invited user IDs back to usernames for display.
                    let invited_names: Vec<&str> = members
                        .iter()
                        .zip(member_ids.iter())
                        .filter(|(_, mid)| invited.contains(mid))
                        .map(|(name, _)| name.as_str())
                        .collect();

                    Ok(vec![DisplayMessage::system(&format!(
                        "Invited {} to the room",
                        invited_names.join(", ")
                    ))])
                }
            },
            Message::RefreshRooms,
        )
    }

    // Invites

    pub(crate) fn list_invites(&mut self) -> Task<Message> {
        let params = self.api_params();

        Task::perform(
            async move {
                let api = params.into_client();
                let invites = operations::list_pending_invites(&api)
                    .await
                    .map_err(|e| e.to_string())?;

                if invites.is_empty() {
                    Ok(vec![DisplayMessage::system("No pending invitations.")])
                } else {
                    let mut messages = Vec::new();
                    for invite in &invites {
                        let display = if invite.group_alias.is_empty() {
                            &invite.group_name
                        } else {
                            &invite.group_alias
                        };
                        messages.push(DisplayMessage::system(&format!(
                            "[{}] #{display} — invited by {}",
                            invite.invite_id, invite.inviter_username
                        )));
                    }
                    messages.push(DisplayMessage::system(
                        "Use /accept <id> or /decline <id> to respond.",
                    ));
                    Ok(messages)
                }
            },
            Message::CommandResult,
        )
    }

    pub(crate) fn list_group_invites(&mut self) -> Task<Message> {
        let group_id = match self.active_room {
            Some(id) => id,
            None => {
                self.push_system_message("No active room — use /join first");
                return Task::none();
            }
        };

        let room_name = self
            .rooms
            .get(&group_id)
            .map(|r| r.display_name())
            .unwrap_or_default();

        let params = self.api_params();

        Task::perform(
            async move {
                let api = params.into_client();
                let invites = operations::list_group_pending_invites(&api, group_id)
                    .await
                    .map_err(|e| e.to_string())?;

                if invites.is_empty() {
                    Ok(vec![DisplayMessage::system(
                        "No pending invites for this room.",
                    )])
                } else {
                    let mut messages = vec![DisplayMessage::system(&format!(
                        "Pending invites for #{room_name}:"
                    ))];
                    for invite in &invites {
                        messages.push(DisplayMessage::system(&format!(
                            "  user#{} — invited by {}",
                            invite.invitee_id, invite.inviter_username
                        )));
                    }
                    Ok(messages)
                }
            },
            Message::CommandResult,
        )
    }

    pub(crate) fn cancel_invite(&mut self, username: String) -> Task<Message> {
        let group_id = match self.active_room {
            Some(id) => id,
            None => {
                self.push_system_message("No active room — use /join first");
                return Task::none();
            }
        };

        let params = self.api_params();

        Task::perform(
            async move {
                let api = params.into_client();
                // Resolve username → user_id at the UI boundary.
                let user_info = api
                    .get_user_by_username(&username)
                    .await
                    .map_err(|e| e.to_string())?;
                operations::cancel_invite(&api, group_id, user_info.user_id)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(vec![DisplayMessage::system(&format!(
                    "Cancelled invitation for {username}"
                ))])
            },
            Message::CommandResult,
        )
    }

    pub(crate) fn accept_invites(&mut self, invite_id: Option<i64>) -> Task<Message> {
        let user_id = match self.user_id {
            Some(id) => id,
            None => {
                self.push_system_message("Not logged in");
                return Task::none();
            }
        };

        let params = self.api_params();
        let data_dir = self.config.data_dir.clone();

        Task::perform(
            async move {
                let api = params.into_client();

                let invite_ids = if let Some(id) = invite_id {
                    vec![id]
                } else {
                    let pending = operations::list_pending_invites(&api)
                        .await
                        .map_err(|e| e.to_string())?;
                    pending.iter().map(|i| i.invite_id).collect()
                };

                if invite_ids.is_empty() {
                    return Ok(vec![DisplayMessage::system(
                        "No pending invitations to accept.",
                    )]);
                }

                let mut messages = Vec::new();
                for id in invite_ids {
                    let results = operations::accept_invite(&api, id, &data_dir, user_id)
                        .await
                        .map_err(|e| e.to_string())?;

                    for result in &results {
                        let id_string = result.group_id.to_string();
                        let display = result.group_alias.as_deref().unwrap_or(&id_string);
                        messages.push(DisplayMessage::system(&format!(
                            "Joined #{display} ({})",
                            result.group_id
                        )));
                    }
                }
                Ok(messages)
            },
            Message::RefreshRooms,
        )
    }

    pub(crate) fn decline_invite(&mut self, invite_id: i64) -> Task<Message> {
        let params = self.api_params();

        Task::perform(
            async move {
                let api = params.into_client();
                operations::decline_invite(&api, invite_id)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(vec![DisplayMessage::system("Invitation declined.")])
            },
            Message::CommandResult,
        )
    }

    // Member management

    pub(crate) fn kick_member(&mut self, target: String) -> Task<Message> {
        let group_id = match self.active_room {
            Some(id) => id,
            None => {
                self.push_system_message("No active room — use /join first");
                return Task::none();
            }
        };

        let mls_group_id = match self.group_mapping.get(&group_id) {
            Some(id) => id.clone(),
            None => {
                self.push_system_message("Group mapping not found");
                return Task::none();
            }
        };

        let target_user_id = if let Some(room) = self.rooms.get(&group_id) {
            match room.members.iter().find(|m| m.username == target) {
                Some(member) => member.user_id,
                None => {
                    self.push_system_message(&format!("User '{target}' not found in room"));
                    return Task::none();
                }
            }
        } else {
            self.push_system_message("Room not found");
            return Task::none();
        };

        let user_id = match self.user_id {
            Some(id) => id,
            None => {
                self.push_system_message("Not logged in");
                return Task::none();
            }
        };

        let params = self.api_params();
        let data_dir = self.config.data_dir.clone();

        Task::perform(
            async move {
                let api = params.into_client();
                operations::kick_member(
                    &api,
                    group_id,
                    &mls_group_id,
                    target_user_id,
                    &data_dir,
                    user_id,
                )
                .await
                .map_err(|e| e.to_string())?;

                Ok(vec![DisplayMessage::system(&format!(
                    "Removed {target} from the room"
                ))])
            },
            Message::RefreshRooms,
        )
    }

    pub(crate) fn promote_member(&mut self, target: String) -> Task<Message> {
        let group_id = match self.active_room {
            Some(id) => id,
            None => {
                self.push_system_message("No active room — use /join first");
                return Task::none();
            }
        };

        // Resolve username → user_id from local member list.
        let target_user_id = if let Some(room) = self.rooms.get(&group_id) {
            match room.members.iter().find(|m| m.username == target) {
                Some(member) => member.user_id,
                None => {
                    self.push_system_message(&format!("User '{target}' not found in room"));
                    return Task::none();
                }
            }
        } else {
            self.push_system_message("Room not found");
            return Task::none();
        };

        let params = self.api_params();

        Task::perform(
            async move {
                let api = params.into_client();
                api.promote_member(group_id, target_user_id)
                    .await
                    .map_err(|e| e.to_string())?;

                Ok(vec![DisplayMessage::system(&format!(
                    "Promoted {target} to admin"
                ))])
            },
            Message::CommandResult,
        )
    }

    pub(crate) fn demote_member(&mut self, target: String) -> Task<Message> {
        let group_id = match self.active_room {
            Some(id) => id,
            None => {
                self.push_system_message("No active room — use /join first");
                return Task::none();
            }
        };

        // Resolve username → user_id from local member list.
        let target_user_id = if let Some(room) = self.rooms.get(&group_id) {
            match room.members.iter().find(|m| m.username == target) {
                Some(member) => member.user_id,
                None => {
                    self.push_system_message(&format!("User '{target}' not found in room"));
                    return Task::none();
                }
            }
        } else {
            self.push_system_message("Room not found");
            return Task::none();
        };

        let params = self.api_params();

        Task::perform(
            async move {
                let api = params.into_client();
                api.demote_member(group_id, target_user_id)
                    .await
                    .map_err(|e| e.to_string())?;

                Ok(vec![DisplayMessage::system(&format!(
                    "Demoted {target} to regular member"
                ))])
            },
            Message::CommandResult,
        )
    }

    // Leave / key rotation / reset

    pub(crate) fn leave_group(&mut self) -> Task<Message> {
        let group_id = match self.active_room {
            Some(id) => id,
            None => {
                self.push_system_message("No active room — use /join first");
                return Task::none();
            }
        };

        let mls_group_id = self.group_mapping.get(&group_id).cloned();

        let room_name = self
            .rooms
            .get(&group_id)
            .map(|r| r.display_name())
            .unwrap_or_default();

        let user_id = match self.user_id {
            Some(id) => id,
            None => {
                self.push_system_message("Not logged in");
                return Task::none();
            }
        };

        let params = self.api_params();
        let data_dir = self.config.data_dir.clone();

        self.group_mapping.remove(&group_id);
        self.rooms.remove(&group_id);
        self.active_room = None;
        self.push_system_message(&format!("Left #{room_name}"));

        Task::perform(
            async move {
                let api = params.into_client();
                operations::leave_group(
                    &api,
                    group_id,
                    mls_group_id.as_deref(),
                    &data_dir,
                    user_id,
                )
                .await
                .map_err(|e| e.to_string())?;
                Ok(vec![])
            },
            Message::CommandResult,
        )
    }

    pub(crate) fn rotate_keys(&mut self) -> Task<Message> {
        let group_id = match self.active_room {
            Some(id) => id,
            None => {
                self.push_system_message("No active room — use /join first");
                return Task::none();
            }
        };

        let mls_group_id = match self.group_mapping.get(&group_id) {
            Some(id) => id.clone(),
            None => {
                self.push_system_message("Group mapping not found");
                return Task::none();
            }
        };

        let user_id = match self.user_id {
            Some(id) => id,
            None => {
                self.push_system_message("Not logged in");
                return Task::none();
            }
        };

        let params = self.api_params();
        let data_dir = self.config.data_dir.clone();

        Task::perform(
            async move {
                let api = params.into_client();
                operations::rotate_keys(&api, group_id, &mls_group_id, &data_dir, user_id)
                    .await
                    .map_err(|e| e.to_string())?;

                Ok(vec![DisplayMessage::system(
                    "Keys rotated. Forward secrecy updated.",
                )])
            },
            Message::CommandResult,
        )
    }

    pub(crate) fn reset_account(&mut self) -> Task<Message> {
        let user_id = match self.user_id {
            Some(id) => id,
            None => {
                self.push_system_message("Not logged in");
                return Task::none();
            }
        };

        let params = self.api_params();
        let data_dir = self.config.data_dir.clone();

        self.push_system_message("Resetting account...");

        Task::perform(
            async move {
                let api = params.into_client();
                operations::reset_account(&api, &data_dir, user_id)
                    .await
                    .map_err(|e| e.to_string())
            },
            Message::ResetComplete,
        )
    }

    // Message fetching

    pub(crate) fn fetch_all_missed_messages(&mut self) -> Task<Message> {
        // Collect eligible group IDs first, then create fetch tasks.
        // Two-phase iteration avoids borrowing self immutably and mutably
        // at the same time (fetching_groups.insert needs &mut self).
        let tasks: Vec<_> = self
            .rooms
            .keys()
            .filter(|group_id| {
                self.group_mapping.contains_key(*group_id)
                    && !self.fetching_groups.contains(*group_id)
            })
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|group_id| {
                let mls_group_id = self.group_mapping.get(&group_id)?.clone();
                self.fetching_groups.insert(group_id);
                let last_seq = self
                    .rooms
                    .get(&group_id)
                    .map(|r| r.last_seen_seq)
                    .unwrap_or(0);

                Some(self.fetch_messages_task(group_id, last_seq, mls_group_id))
            })
            .collect();

        if tasks.is_empty() {
            Task::none()
        } else {
            Task::batch(tasks)
        }
    }

    pub(crate) fn accept_welcomes(&mut self) -> Task<Message> {
        let user_id = match self.user_id {
            Some(id) => id,
            None => return Task::none(),
        };

        let params = self.api_params();
        let data_dir = self.config.data_dir.clone();

        Task::perform(
            async move {
                let api = params.into_client();
                operations::accept_welcomes(&api, &data_dir, user_id)
                    .await
                    .map_err(|e| e.to_string())
            },
            Message::WelcomesProcessed,
        )
    }
}
