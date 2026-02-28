use iced::Task;

use conclave_client::command::Command;
use conclave_client::operations;
use conclave_client::state::DisplayMessage;

use crate::screen;

use super::{Conclave, Message};

impl Conclave {
    pub(crate) fn handle_dashboard_message(
        &mut self,
        msg: screen::dashboard::Message,
    ) -> Task<Message> {
        match msg {
            screen::dashboard::Message::RoomSelected(room_id) => {
                self.active_room = Some(room_id);

                if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                    dashboard.show_user_popover = false;
                }

                if let Some(room) = self.rooms.get_mut(&room_id) {
                    room.last_read_seq = room.last_seen_seq;
                    if let Some(store) = &self.msg_store {
                        store.set_last_read_seq(room_id, room.last_read_seq);
                    }
                }

                Task::none()
            }
            screen::dashboard::Message::InputChanged(value) => {
                if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                    dashboard.input_value = value;
                }
                Task::none()
            }
            screen::dashboard::Message::InputSubmitted => {
                let text = if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                    let t = dashboard.input_value.clone();
                    dashboard.input_value.clear();
                    t
                } else {
                    return Task::none();
                };

                if text.is_empty() {
                    return Task::none();
                }

                self.handle_input_text(text)
            }
            screen::dashboard::Message::ToggleUserPopover => {
                if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                    dashboard.show_user_popover = !dashboard.show_user_popover;
                }
                Task::none()
            }
            screen::dashboard::Message::CloseUserPopover => {
                if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                    dashboard.show_user_popover = false;
                }
                Task::none()
            }
            screen::dashboard::Message::ToggleMembersSidebar => {
                if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                    dashboard.show_members_sidebar = !dashboard.show_members_sidebar;
                }
                Task::none()
            }
            screen::dashboard::Message::SelectedText(fragments) => {
                if fragments.is_empty() {
                    return Task::none();
                }

                let mut sorted = fragments;
                sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

                let mut result = String::new();
                let mut last_y: Option<f32> = None;
                for (y_position, text) in &sorted {
                    match last_y {
                        Some(prev_y) if (*y_position - prev_y).abs() > 1.0 => {
                            result.push('\n');
                        }
                        Some(_) => {}
                        None => {}
                    }
                    result.push_str(text);
                    last_y = Some(*y_position);
                }

                if result.is_empty() {
                    return Task::none();
                }

                iced::clipboard::write(result)
            }
            screen::dashboard::Message::Logout => self.perform_logout(),
            screen::dashboard::Message::DragStarted(target) => {
                if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                    dashboard.dragging = Some(target);
                    dashboard.last_drag_x = 0.0;
                }
                Task::none()
            }
            screen::dashboard::Message::DragUpdate(cursor_x) => {
                if let screen::Screen::Dashboard(dashboard) = &mut self.screen
                    && dashboard.dragging.is_some()
                {
                    let last_x = dashboard.last_drag_x;
                    if last_x > 0.0 {
                        let delta = cursor_x - last_x;
                        match dashboard.dragging {
                            Some(screen::dashboard::DragTarget::LeftHandle) => {
                                dashboard.left_sidebar_width =
                                    (dashboard.left_sidebar_width + delta).clamp(
                                        screen::dashboard::SIDEBAR_MIN_WIDTH,
                                        screen::dashboard::SIDEBAR_MAX_WIDTH,
                                    );
                            }
                            Some(screen::dashboard::DragTarget::RightHandle) => {
                                dashboard.right_sidebar_width =
                                    (dashboard.right_sidebar_width - delta).clamp(
                                        screen::dashboard::SIDEBAR_MIN_WIDTH,
                                        screen::dashboard::SIDEBAR_MAX_WIDTH,
                                    );
                            }
                            None => {}
                        }
                    }
                    dashboard.last_drag_x = cursor_x;
                }
                Task::none()
            }
            screen::dashboard::Message::DragEnded => {
                if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                    dashboard.dragging = None;
                    dashboard.last_drag_x = 0.0;
                }
                Task::none()
            }
        }
    }

    fn handle_input_text(&mut self, text: String) -> Task<Message> {
        match conclave_client::command::parse(&text) {
            // General
            Ok(Command::Quit) => iced::exit(),
            Ok(Command::Message { text }) => self.send_message(text),
            Ok(Command::Help) => {
                self.show_help();
                Task::none()
            }

            // Rooms
            Ok(Command::List) => {
                let params = self.api_params();
                let active_room = self.active_room;
                Task::perform(
                    async move {
                        let api = params.into_client();
                        let rooms = operations::load_rooms(&api)
                            .await
                            .map_err(|e| e.to_string())?;
                        if rooms.is_empty() {
                            return Ok(vec![DisplayMessage::system("No rooms.")]);
                        }
                        let mut msgs = vec![DisplayMessage::system("Rooms:")];
                        for r in &rooms {
                            let active = if active_room == Some(r.group_id) {
                                " (active)"
                            } else {
                                ""
                            };
                            let member_names: Vec<&str> =
                                r.members.iter().map(|m| m.display_name()).collect();
                            msgs.push(DisplayMessage::system(&format!(
                                "  #{} [{}]{active}",
                                r.display_name(),
                                member_names.join(", "),
                            )));
                        }
                        Ok(msgs)
                    },
                    Message::RefreshRooms,
                )
            }
            Ok(Command::Join { target: None }) => self.accept_welcomes(),
            Ok(Command::Join {
                target: Some(target),
            }) => {
                self.switch_to_room(&target);
                Task::none()
            }

            // Members
            Ok(Command::Who) => {
                if let Some(room_id) = self.active_room {
                    if let Some(room) = self.rooms.get(&room_id) {
                        let member_names: Vec<String> = room
                            .members
                            .iter()
                            .map(|m| {
                                if m.role == "admin" {
                                    format!("{} (admin)", m.display_name())
                                } else {
                                    m.display_name().to_string()
                                }
                            })
                            .collect();
                        self.push_system_message(&format!(
                            "Members of #{}: {}",
                            room.display_name(),
                            member_names.join(", ")
                        ));
                    }
                } else {
                    self.push_system_message("No active room.");
                }
                Task::none()
            }
            Ok(Command::Info) => {
                self.show_group_info();
                Task::none()
            }
            Ok(Command::Close) => {
                if let Some(room_id) = self.active_room.take() {
                    let name = self
                        .rooms
                        .get(&room_id)
                        .map(|r| r.display_name())
                        .unwrap_or_default();
                    self.push_system_message(&format!(
                        "Switched away from #{name} (use /part to leave)"
                    ));
                }
                Task::none()
            }
            Ok(Command::Unread) => {
                self.show_unread();
                Task::none()
            }

            // Account
            Ok(Command::Whois) => {
                let params = self.api_params();
                Task::perform(
                    async move {
                        let resp = params.into_client().me().await.map_err(|e| e.to_string())?;
                        Ok(vec![DisplayMessage::system(&format!(
                            "User: {} (ID: {})",
                            resp.username, resp.user_id
                        ))])
                    },
                    Message::CommandResult,
                )
            }
            Ok(Command::Nick { alias }) => {
                let params = self.api_params();
                Task::perform(
                    async move {
                        params
                            .into_client()
                            .update_profile(&alias)
                            .await
                            .map_err(|e| e.to_string())?;
                        Ok(alias)
                    },
                    Message::NickResult,
                )
            }
            Ok(Command::Passwd { new_password }) => {
                let params = self.api_params();
                Task::perform(
                    async move {
                        params
                            .into_client()
                            .change_password(&new_password)
                            .await
                            .map_err(|e| e.to_string())?;
                        Ok(vec![DisplayMessage::system(
                            "Password changed successfully.",
                        )])
                    },
                    Message::CommandResult,
                )
            }

            // Rooms (continued)
            Ok(Command::Topic { topic }) => {
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
                        params
                            .into_client()
                            .update_group(group_id, Some(&topic))
                            .await
                            .map_err(|e| e.to_string())?;
                        Ok(topic)
                    },
                    Message::TopicResult,
                )
            }

            // Account (continued)
            Ok(Command::Login { .. }) | Ok(Command::Register { .. }) => {
                self.push_system_message("Already logged in. Use /logout first.");
                Task::none()
            }
            Ok(Command::Logout) => self.perform_logout(),

            // Group lifecycle
            Ok(Command::Create { name }) => self.create_group(name),

            // Members (continued)
            Ok(Command::Invite { members }) => self.invite_members(members),
            Ok(Command::Kick { username }) => self.kick_member(username),
            Ok(Command::Promote { username }) => self.promote_member(username),
            Ok(Command::Demote { username }) => self.demote_member(username),
            Ok(Command::Admins) => {
                let group_id = match self.active_room {
                    Some(id) => id,
                    None => {
                        self.push_system_message("No active room.");
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
                        let response =
                            api.list_admins(group_id).await.map_err(|e| e.to_string())?;
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
                        Ok(vec![DisplayMessage::system(&format!(
                            "Admins of #{room_name}: {}",
                            admin_names.join(", ")
                        ))])
                    },
                    Message::CommandResult,
                )
            }

            // Invites
            Ok(Command::Invited) => self.list_group_invites(),
            Ok(Command::Uninvite { username }) => self.cancel_invite(username),
            Ok(Command::Invites) => self.list_invites(),
            Ok(Command::Accept { invite_id }) => self.accept_invites(invite_id),
            Ok(Command::Decline { invite_id }) => self.decline_invite(invite_id),

            // Leave / messaging / reset
            Ok(Command::Part) => self.leave_group(),
            Ok(Command::Rotate) => self.rotate_keys(),
            Ok(Command::Reset) => self.reset_account(),
            Ok(Command::Msg { room, text }) => self.send_to_room(&room, &text),
            Err(e) => {
                self.push_system_message(&format!("{e}"));
                Task::none()
            }
        }
    }
}
