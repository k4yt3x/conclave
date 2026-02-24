use iced::Task;

use conclave_lib::operations;
use conclave_lib::state::{ConnectionStatus, DisplayMessage};

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
            SseUpdate::IdentityReset { group_id, username } => {
                self.add_message_to_room(
                    group_id,
                    DisplayMessage::system(&format!(
                        "{username} has reset their encryption identity. \
                         New messages are secured with their new keys."
                    )),
                );
                self.handle_sse_event(SseUpdate::NewMessage { group_id })
            }
            SseUpdate::MemberRemoved { group_id, username } => {
                let is_self = self.username.as_deref() == Some(&username);

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
                    if let Some(room) = self.rooms.get_mut(&group_id) {
                        room.members.retain(|m| m.username != username);
                    }
                    self.add_message_to_room(
                        group_id,
                        DisplayMessage::system(&format!("{username} was removed from the group")),
                    );

                    self.handle_sse_event(SseUpdate::NewMessage { group_id })
                }
            }
            SseUpdate::InviteReceived {
                invite_id,
                group_id,
                group_name,
                group_alias,
                inviter_username,
            } => {
                let display = if group_alias.is_empty() {
                    &group_name
                } else {
                    &group_alias
                };
                self.push_system_message(&format!(
                    "Invitation from {inviter_username} to join #{display} ({group_id}). \
                     Use /accept {invite_id} or /decline {invite_id}."
                ));
                Task::none()
            }
            SseUpdate::InviteDeclined {
                group_id,
                declined_username,
            } => {
                self.push_system_message(&format!("{declined_username} declined the invitation."));

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
