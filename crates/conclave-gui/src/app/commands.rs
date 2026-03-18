use iced::Task;
use uuid::Uuid;

use conclave_client::command::Command;
use conclave_client::mls::{format_fingerprint, normalize_fingerprint};
use conclave_client::operations;
use conclave_client::state::DisplayMessage;
use conclave_client::store::MessageStore;

use crate::screen;
use crate::screen::dashboard::PasswordChangeDialog;

use super::{Conclave, Message};

impl Conclave {
    pub(crate) fn handle_dashboard_message(
        &mut self,
        msg: screen::dashboard::Message,
    ) -> Task<Message> {
        match msg {
            screen::dashboard::Message::RoomSelected(room_id) => {
                self.select_room(room_id);
                Task::none()
            }
            screen::dashboard::Message::InputAction(action) => {
                if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                    dashboard.input_content.perform(action);
                }
                Task::none()
            }
            screen::dashboard::Message::InputSubmitted => {
                let text = if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                    let t = dashboard.input_content.text();
                    dashboard.input_content = iced::widget::text_editor::Content::new();
                    t
                } else {
                    return Task::none();
                };

                let text = text.trim_end_matches('\n').to_owned();

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
            screen::dashboard::Message::VerifyResult(result) => {
                match result {
                    Ok((status_update, msgs)) => {
                        if let Some((uid, status)) = status_update {
                            self.verification_status.insert(uid, status);
                        }
                        for msg in msgs {
                            self.add_message(None, msg);
                        }
                    }
                    Err(e) => self.push_system_message(&format!("Error: {e}")),
                }
                Task::none()
            }
            screen::dashboard::Message::PasswordDialogCurrentChanged(value) => {
                if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                    dashboard.password_dialog.current_password = value;
                }
                Task::none()
            }
            screen::dashboard::Message::PasswordDialogNewChanged(value) => {
                if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                    dashboard.password_dialog.new_password = value;
                }
                Task::none()
            }
            screen::dashboard::Message::PasswordDialogConfirmChanged(value) => {
                if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                    dashboard.password_dialog.confirm_password = value;
                }
                Task::none()
            }
            screen::dashboard::Message::PasswordDialogSubmit => {
                let (current, new, confirm) =
                    if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                        if dashboard.password_dialog.loading {
                            return Task::none();
                        }
                        (
                            dashboard.password_dialog.current_password.clone(),
                            dashboard.password_dialog.new_password.clone(),
                            dashboard.password_dialog.confirm_password.clone(),
                        )
                    } else {
                        return Task::none();
                    };

                if new != confirm {
                    if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                        dashboard.password_dialog.error =
                            Some("New passwords do not match.".to_string());
                    }
                    return Task::none();
                }

                if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                    dashboard.password_dialog.loading = true;
                    dashboard.password_dialog.error = None;
                }

                let params = self.api_params();
                Task::perform(
                    async move {
                        params
                            .into_client()
                            .change_password(&current, &new)
                            .await
                            .map_err(|e| e.to_string())
                    },
                    |result| {
                        Message::Dashboard(screen::dashboard::Message::PasswordDialogResult(result))
                    },
                )
            }
            screen::dashboard::Message::PasswordDialogCancel => {
                if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                    dashboard.show_password_dialog = false;
                    dashboard.password_dialog = PasswordChangeDialog::default();
                }
                Task::none()
            }
            screen::dashboard::Message::PasswordDialogResult(result) => {
                if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                    dashboard.password_dialog.loading = false;
                    match result {
                        Ok(()) => {
                            dashboard.show_password_dialog = false;
                            dashboard.password_dialog = PasswordChangeDialog::default();
                            self.push_system_message(
                                "Password changed successfully. Please log in again.",
                            );
                        }
                        Err(e) => {
                            dashboard.password_dialog.error = Some(e);
                        }
                    }
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
            Ok(Command::Rooms) => {
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
            Ok(Command::Members) => {
                if let Some(room_id) = self.active_room {
                    if let Some(room) = self.rooms.get(&room_id) {
                        let member_names: Vec<String> = room
                            .members
                            .iter()
                            .map(|m| {
                                let indicator = match self.verification_status.get(&m.user_id) {
                                    Some(conclave_client::state::VerificationStatus::Changed) => {
                                        "[!] "
                                    }
                                    Some(
                                        conclave_client::state::VerificationStatus::Unknown
                                        | conclave_client::state::VerificationStatus::Unverified,
                                    ) => "[?] ",
                                    Some(conclave_client::state::VerificationStatus::Verified)
                                    | None => "",
                                };
                                if m.role == "admin" {
                                    format!("{indicator}{} (admin)", m.display_name())
                                } else {
                                    format!("{indicator}{}", m.display_name())
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
            Ok(Command::Whois { username: None }) => {
                let own_fingerprint = self.mls.as_ref().map(|m| m.signing_key_fingerprint());
                let params = self.api_params();
                Task::perform(
                    async move {
                        let resp = params.into_client().me().await.map_err(|e| e.to_string())?;
                        let self_user_id = Uuid::from_slice(&resp.user_id)
                            .map(|id| id.to_string())
                            .unwrap_or_else(|_| "?".into());
                        let mut msgs = vec![DisplayMessage::system(&format!(
                            "User: {} (ID: {self_user_id})",
                            resp.username
                        ))];
                        if let Some(fp) = own_fingerprint {
                            msgs.push(DisplayMessage::system(&format!(
                                "Fingerprint: {}",
                                format_fingerprint(&fp)
                            )));
                        }
                        Ok(msgs)
                    },
                    Message::CommandResult,
                )
            }
            Ok(Command::Whois {
                username: Some(target),
            }) => {
                let params = self.api_params();
                let data_dir = self.config.data_dir.clone();
                Task::perform(
                    async move {
                        let api = params.into_client();
                        let resp = api
                            .get_user_by_username(&target)
                            .await
                            .map_err(|e| e.to_string())?;
                        let user_id = Uuid::from_slice(&resp.user_id)
                            .map_err(|e| format!("Invalid user ID: {e}"))?;
                        let mut msgs = vec![DisplayMessage::system(&format!(
                            "User: {} (ID: {})",
                            resp.username, user_id
                        ))];
                        if !resp.signing_key_fingerprint.is_empty() {
                            msgs.push(DisplayMessage::system(&format!(
                                "Fingerprint: {}",
                                format_fingerprint(&resp.signing_key_fingerprint)
                            )));
                            if let Ok(store) = MessageStore::open(&data_dir) {
                                let status = store.get_verification_status(
                                    user_id,
                                    &resp.signing_key_fingerprint,
                                );
                                let status_str = match status {
                                    conclave_client::state::VerificationStatus::Unknown => {
                                        "Unknown"
                                    }
                                    conclave_client::state::VerificationStatus::Unverified => {
                                        "Unverified"
                                    }
                                    conclave_client::state::VerificationStatus::Verified => {
                                        "Verified"
                                    }
                                    conclave_client::state::VerificationStatus::Changed => {
                                        "Changed (warning!)"
                                    }
                                };
                                msgs.push(DisplayMessage::system(&format!("Status: {status_str}")));
                            }
                        }
                        Ok(msgs)
                    },
                    Message::CommandResult,
                )
            }
            Ok(Command::Verify {
                username,
                fingerprint,
            }) => {
                let normalized = match normalize_fingerprint(&fingerprint) {
                    Ok(fp) => fp,
                    Err(e) => {
                        self.push_system_message(&format!("{e}"));
                        return Task::none();
                    }
                };
                let params = self.api_params();
                let data_dir = self.config.data_dir.clone();
                Task::perform(
                    async move {
                        let api = params.into_client();
                        let resp = api
                            .get_user_by_username(&username)
                            .await
                            .map_err(|e| e.to_string())?;
                        let user_id = Uuid::from_slice(&resp.user_id)
                            .map_err(|e| format!("Invalid user ID: {e}"))?;
                        let store = MessageStore::open(&data_dir)
                            .map_err(|e| format!("Message store not available: {e}"))?;
                        let stored = store.get_stored_fingerprint(user_id);
                        match stored {
                            Some((stored_fp, _)) => {
                                if stored_fp == normalized {
                                    store.verify_user(user_id);
                                    Ok((
                                        Some((
                                            user_id,
                                            conclave_client::state::VerificationStatus::Verified,
                                        )),
                                        vec![DisplayMessage::system(&format!(
                                            "Fingerprint verified for {username}."
                                        ))],
                                    ))
                                } else {
                                    Ok((
                                        None,
                                        vec![
                                            DisplayMessage::system(
                                                "Fingerprint does not match stored value!",
                                            ),
                                            DisplayMessage::system(&format!(
                                                "  Stored:   {}",
                                                format_fingerprint(&stored_fp)
                                            )),
                                            DisplayMessage::system(&format!(
                                                "  Provided: {}",
                                                format_fingerprint(&normalized)
                                            )),
                                        ],
                                    ))
                                }
                            }
                            None => Ok((
                                None,
                                vec![DisplayMessage::system(&format!(
                                    "No stored fingerprint for {username}. \
                                     Use /whois {username} first to establish initial trust."
                                ))],
                            )),
                        }
                    },
                    |result| {
                        Message::Dashboard(crate::screen::dashboard::Message::VerifyResult(result))
                    },
                )
            }
            Ok(Command::Unverify { username }) => {
                let params = self.api_params();
                let data_dir = self.config.data_dir.clone();
                Task::perform(
                    async move {
                        let api = params.into_client();
                        let resp = api
                            .get_user_by_username(&username)
                            .await
                            .map_err(|e| e.to_string())?;
                        let user_id = Uuid::from_slice(&resp.user_id)
                            .map_err(|e| format!("Invalid user ID: {e}"))?;
                        let store = MessageStore::open(&data_dir)
                            .map_err(|e| format!("Message store not available: {e}"))?;
                        if store.unverify_user(user_id) {
                            Ok((
                                Some((
                                    user_id,
                                    conclave_client::state::VerificationStatus::Unverified,
                                )),
                                vec![DisplayMessage::system(&format!(
                                    "Removed verification for {username}."
                                ))],
                            ))
                        } else {
                            Ok((
                                None,
                                vec![DisplayMessage::system(&format!(
                                    "No stored fingerprint for {username}."
                                ))],
                            ))
                        }
                    },
                    |result| {
                        Message::Dashboard(crate::screen::dashboard::Message::VerifyResult(result))
                    },
                )
            }
            Ok(Command::Trusted) => {
                let data_dir = self.config.data_dir.clone();
                let name_map: std::collections::HashMap<Uuid, String> = self
                    .rooms
                    .values()
                    .flat_map(|r| r.members.iter())
                    .map(|m| (m.user_id, m.display_name().to_string()))
                    .collect();
                Task::perform(
                    async move {
                        let store = MessageStore::open(&data_dir)
                            .map_err(|e| format!("Message store not available: {e}"))?;
                        let entries = store.get_all_known_fingerprints();
                        if entries.is_empty() {
                            return Ok(vec![DisplayMessage::system("No known fingerprints.")]);
                        }
                        let mut msgs = vec![DisplayMessage::system("Known fingerprints:")];
                        for (user_id, fingerprint, verified, key_changed) in &entries {
                            let display_name = name_map
                                .get(user_id)
                                .cloned()
                                .unwrap_or_else(|| format!("user#{user_id}"));
                            let status = if *key_changed {
                                "Changed"
                            } else if *verified {
                                "Verified"
                            } else {
                                "Unverified"
                            };
                            let formatted = format_fingerprint(fingerprint);
                            msgs.push(DisplayMessage::system(&format!(
                                "  {display_name}: [{status}] {formatted}"
                            )));
                        }
                        Ok(msgs)
                    },
                    Message::CommandResult,
                )
            }
            Ok(Command::Alias { alias }) => {
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
            Ok(Command::Passwd) => {
                if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                    dashboard.show_password_dialog = true;
                    dashboard.password_dialog = PasswordChangeDialog::default();
                }
                Task::none()
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
                        Ok(vec![DisplayMessage::system(&format!(
                            "Room alias set to: {topic}"
                        ))])
                    },
                    Message::CommandResult,
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

            // Retention policy
            Ok(Command::Expire { duration: None }) => {
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
                        let policy = api
                            .get_retention_policy(group_id)
                            .await
                            .map_err(|e| e.to_string())?;
                        let server = conclave_client::duration::format_duration(
                            policy.server_retention_seconds,
                        );
                        let group =
                            conclave_client::duration::format_duration(policy.group_expiry_seconds);
                        let effective = conclave_client::duration::compute_effective(
                            policy.server_retention_seconds,
                            policy.group_expiry_seconds,
                        );
                        Ok(vec![
                            DisplayMessage::system(&format!("Server retention: {server}")),
                            DisplayMessage::system(&format!("Room expiry: {group}")),
                            DisplayMessage::system(&format!(
                                "Effective: {}",
                                conclave_client::duration::format_duration(effective)
                            )),
                        ])
                    },
                    Message::CommandResult,
                )
            }
            Ok(Command::Expire {
                duration: Some(dur),
            }) => {
                let group_id = match self.active_room {
                    Some(id) => id,
                    None => {
                        self.push_system_message("No active room — use /join first");
                        return Task::none();
                    }
                };
                let seconds = match conclave_client::duration::parse_duration(&dur) {
                    Ok(s) => s,
                    Err(e) => {
                        self.push_system_message(&format!("{e}"));
                        return Task::none();
                    }
                };
                let formatted = conclave_client::duration::format_duration(seconds);
                let params = self.api_params();
                Task::perform(
                    async move {
                        let api = params.into_client();
                        api.set_group_expiry(group_id, seconds)
                            .await
                            .map_err(|e| e.to_string())?;
                        Ok(vec![DisplayMessage::system(&format!(
                            "Message expiry set to {formatted}"
                        ))])
                    },
                    Message::CommandResult,
                )
            }

            // Account deletion / group deletion
            Ok(Command::Expunge { password }) => self.expunge_account(password),
            Ok(Command::Delete) => self.delete_group(),

            // Leave / security / reset
            Ok(Command::Part) => self.leave_group(),
            Ok(Command::Rotate) => self.rotate_keys(),
            Ok(Command::Reset) => self.reset_account(),
            Err(e) => {
                self.push_system_message(&format!("{e}"));
                Task::none()
            }
        }
    }
}
