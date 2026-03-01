use iced::Task;

use conclave_client::operations;
use conclave_client::state::{ConnectionStatus, DisplayMessage};

use crate::subscription::SseUpdate;

use super::{Conclave, Message};

impl Conclave {
    pub(crate) fn handle_sse_event(&mut self, update: SseUpdate) -> Task<Message> {
        match update {
            SseUpdate::Connected => {
                self.connection_status = ConnectionStatus::Connected;

                let rooms_task = self.load_rooms_task();
                let welcome_task = self.accept_welcomes();
                let fetch_task = self.fetch_all_missed_messages();
                Task::batch([rooms_task, welcome_task, fetch_task])
            }
            SseUpdate::Connecting => {
                self.connection_status = ConnectionStatus::Connecting;
                Task::none()
            }
            SseUpdate::Disconnected => {
                self.connection_status = ConnectionStatus::Disconnected;
                Task::none()
            }
            SseUpdate::NewMessage { group_id } => {
                // Deduplicate: skip if we're already fetching for this group.
                if self.fetching_groups.contains(&group_id) {
                    return Task::none();
                }

                let mls_group_id = match self.group_mapping.get(&group_id) {
                    Some(id) => id.clone(),
                    None => return Task::none(),
                };

                if self.username.is_none() {
                    return Task::none();
                }

                self.fetching_groups.insert(group_id);
                let last_seq = self
                    .rooms
                    .get(&group_id)
                    .map(|r| r.last_seen_seq)
                    .unwrap_or(0);

                self.fetch_messages_task(group_id, last_seq, mls_group_id)
            }
            SseUpdate::Welcome => self.accept_welcomes(),
            SseUpdate::GroupUpdate => self.load_rooms_task(),
            SseUpdate::IdentityReset { group_id, user_id } => {
                let display_name = self
                    .rooms
                    .get(&group_id)
                    .and_then(|room| {
                        room.members
                            .iter()
                            .find(|m| m.user_id == user_id)
                            .map(|m| m.display_name().to_string())
                    })
                    .unwrap_or_else(|| format!("user#{user_id}"));

                self.add_message_to_room(
                    group_id,
                    DisplayMessage::system(&format!(
                        "{display_name} has reset their encryption identity. \
                         New messages are secured with their new keys."
                    )),
                );
                let new_msg_task = self.handle_sse_event(SseUpdate::NewMessage { group_id });
                let rooms_task = self.load_rooms_task();
                Task::batch([new_msg_task, rooms_task])
            }
            SseUpdate::MemberRemoved {
                group_id,
                removed_user_id,
            } => {
                let is_self = self.user_id == Some(removed_user_id);

                if is_self {
                    let room_name = self
                        .rooms
                        .get(&group_id)
                        .map(|r| r.display_name())
                        .unwrap_or_else(|| group_id.to_string());
                    let mls_group_id = self.group_mapping.get(&group_id).cloned();

                    self.group_mapping.remove(&group_id);
                    self.rooms.remove(&group_id);
                    if self.active_room == Some(group_id) {
                        self.active_room = None;
                    }
                    self.push_system_message(&format!("You were removed from #{room_name}"));

                    // Clean up MLS group state for the group we were removed from.
                    if let (Some(mls_group_id), Some(our_user_id)) = (mls_group_id, self.user_id) {
                        let data_dir = self.config.data_dir.clone();
                        Task::perform(
                            async move {
                                if let Err(error) = operations::delete_mls_group_state(
                                    &mls_group_id,
                                    &data_dir,
                                    our_user_id,
                                )
                                .await
                                {
                                    tracing::warn!(%error, "failed to delete MLS group state");
                                }
                                Ok::<_, String>(vec![])
                            },
                            Message::CommandResult,
                        )
                    } else {
                        Task::none()
                    }
                } else {
                    let removed_name = self
                        .rooms
                        .get(&group_id)
                        .and_then(|room| {
                            room.members
                                .iter()
                                .find(|m| m.user_id == removed_user_id)
                                .map(|m| m.display_name().to_string())
                        })
                        .unwrap_or_else(|| format!("user#{removed_user_id}"));

                    if let Some(room) = self.rooms.get_mut(&group_id) {
                        room.members.retain(|m| m.user_id != removed_user_id);
                    }
                    self.add_message_to_room(
                        group_id,
                        DisplayMessage::system(&format!(
                            "{removed_name} was removed from the group"
                        )),
                    );

                    self.handle_sse_event(SseUpdate::NewMessage { group_id })
                }
            }
            SseUpdate::InviteReceived {
                invite_id,
                group_id,
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
                self.push_system_message(&format!(
                    "Invitation from {inviter_name} to join #{display} ({group_id}). \
                     Use /accept {invite_id} or /decline {invite_id}."
                ));
                Task::none()
            }
            SseUpdate::InviteCancelled => {
                self.push_system_message("An invitation to a room was cancelled.");
                Task::none()
            }
            SseUpdate::InviteDeclined {
                group_id,
                declined_user_id,
            } => {
                let declined_name = self
                    .rooms
                    .get(&group_id)
                    .and_then(|room| {
                        room.members
                            .iter()
                            .find(|m| m.user_id == declined_user_id)
                            .map(|m| m.display_name().to_string())
                    })
                    .unwrap_or_else(|| format!("user#{declined_user_id}"));

                self.push_system_message(&format!("{declined_name} declined the invitation."));

                // Auto-rotate keys to clean up the phantom MLS leaf.
                let user_id = match self.user_id {
                    Some(id) => id,
                    None => return Task::none(),
                };
                if let Some(mls_group_id) = self.group_mapping.get(&group_id).cloned() {
                    let params = self.api_params();
                    let data_dir = self.config.data_dir.clone();

                    Task::perform(
                        async move {
                            let api = params.into_client();
                            if let Err(error) = operations::handle_invite_declined(
                                &api,
                                group_id,
                                &mls_group_id,
                                &data_dir,
                                user_id,
                            )
                            .await
                            {
                                tracing::warn!(
                                    %error,
                                    "failed to rotate keys after invite decline"
                                );
                            }
                            Ok(vec![])
                        },
                        Message::CommandResult,
                    )
                } else {
                    Task::none()
                }
            }
        }
    }
}
