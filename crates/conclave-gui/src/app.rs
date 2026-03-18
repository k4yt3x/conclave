mod commands;
mod login;
mod rooms;
mod sse;

use iced::widget::operation::{focus, focus_next};
use iced::{Subscription, Task, keyboard};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use conclave_client::api::{ApiClient, normalize_server_url};
use conclave_client::config::{ClientConfig, SessionState};
use conclave_client::mls::MlsManager;
use conclave_client::operations;
use conclave_client::state::{ConnectionStatus, DisplayMessage, Room, VerificationStatus};
use conclave_client::store::MessageStore;

use crate::screen;
use crate::subscription::{self, SseUpdate};
use crate::widget::Element;

/// Snapshot of API connection parameters for use in async tasks.
/// Captures the minimum state needed to construct an [`ApiClient`] outside
/// of `&self` so the future can be `Send`.
pub(crate) struct ApiParams {
    pub(crate) server_url: String,
    pub(crate) accept_invalid_certs: bool,
    pub(crate) token: String,
    pub(crate) custom_headers: reqwest::header::HeaderMap,
}

impl ApiParams {
    pub(crate) fn into_client(self) -> ApiClient {
        let mut api = ApiClient::new(
            &self.server_url,
            self.accept_invalid_certs,
            self.custom_headers,
        );
        api.set_token(self.token);
        api
    }
}

pub struct Conclave {
    pub(crate) screen: screen::Screen,
    pub(crate) theme: crate::theme::Theme,
    pub(crate) config: ClientConfig,
    // Core state
    pub(crate) server_url: Option<String>,
    pub(crate) api: Option<ApiClient>,
    pub(crate) mls: Option<MlsManager>,
    pub(crate) username: Option<String>,
    pub(crate) user_alias: Option<String>,
    pub(crate) user_id: Option<Uuid>,
    pub(crate) token: Option<String>,
    pub(crate) rooms: HashMap<Uuid, Room>,
    pub(crate) active_room: Option<Uuid>,
    pub(crate) room_messages: HashMap<Uuid, Vec<DisplayMessage>>,
    pub(crate) system_messages: Vec<DisplayMessage>,
    pub(crate) group_mapping: HashMap<Uuid, String>,
    pub(crate) connection_status: ConnectionStatus,
    pub(crate) msg_store: Option<MessageStore>,
    pub(crate) rooms_loaded: bool,
    pub(crate) welcomes_processed: bool,
    pub(crate) fetching_groups: HashSet<Uuid>,
    /// Set when welcome processing triggers a rooms reload — defers the
    /// missed-message fetch until the rooms are actually in `self.rooms`.
    pub(crate) fetch_messages_on_rooms_load: bool,
    pub(crate) verification_status: HashMap<Uuid, VerificationStatus>,
    pub(crate) window_focused: bool,
}

#[derive(Debug, Clone)]
#[allow(clippy::enum_variant_names)]
pub enum Message {
    // Screen navigation
    Login(screen::login::Message),
    Dashboard(screen::dashboard::Message),

    // Authentication results
    LoginResult(Result<LoginInfo, String>),
    RegisterResult(Result<LoginInfo, String>),

    // Room / message async results
    RoomsLoaded(Result<Vec<operations::RoomInfo>, String>),
    MessageSent(Result<(operations::MessageSentResult, String), String>),
    MessagesFetched(Result<operations::FetchedMessages, (Uuid, String)>),
    KeyPackageUploaded(Result<(), String>),
    WelcomesProcessed(Result<Vec<operations::WelcomeJoinResult>, String>),

    // SSE
    SseEvent(SseUpdate),

    // Command results
    CommandResult(Result<Vec<DisplayMessage>, String>),
    GroupCreated(Result<operations::GroupCreatedResult, String>),
    RefreshRooms(Result<Vec<DisplayMessage>, String>),
    ResetComplete(Result<operations::ResetResult, String>),
    UserAliasLoaded(Result<Option<String>, String>),
    NickResult(Result<String, String>),
    ExpungeResult(Result<(), String>),

    // Keyboard / window events
    TabPressed,
    EscapePressed,
    CopySelection,
    Quit,
    PreviousRoom,
    NextRoom,
    GoToRoom(u8),
    WindowFocused,
    WindowUnfocused,

    // Periodic expiry cleanup
    ExpiryTick,
}

#[derive(Debug, Clone)]
pub struct LoginInfo {
    pub server_url: String,
    pub token: String,
    pub user_id: Uuid,
    pub username: String,
}

impl Conclave {
    pub fn new() -> (Self, Task<Message>) {
        let config = ClientConfig::load();

        // Try to restore session
        let session = SessionState::load(&config.data_dir);
        let initial_server_url = session.server_url.clone();

        // Load theme with user overrides from config.toml [theme] section
        let theme_config = crate::theme::config::ThemeConfig::load(&config.config_dir);
        let theme = theme_config.apply(crate::theme::Theme::default());

        let mut app = Self {
            screen: screen::Screen::Login(screen::Login::new(
                initial_server_url.clone().unwrap_or_default(),
            )),
            theme,
            config,
            server_url: initial_server_url,
            api: None,
            mls: None,
            username: None,
            user_alias: None,
            user_id: None,
            token: None,
            rooms: HashMap::new(),
            active_room: None,
            room_messages: HashMap::new(),
            system_messages: vec![DisplayMessage::system(
                "Welcome to Conclave. Login or register to get started.",
            )],
            group_mapping: HashMap::new(),
            connection_status: ConnectionStatus::Disconnected,
            msg_store: None,
            rooms_loaded: false,
            welcomes_processed: false,
            fetching_groups: HashSet::new(),
            fetch_messages_on_rooms_load: false,
            verification_status: HashMap::new(),
            window_focused: true,
        };

        if let (Some(server_url), Some(token), Some(username), Some(user_id)) = (
            session.server_url,
            session.token,
            session.username,
            session.user_id,
        ) {
            let custom_headers =
                conclave_client::api::parse_custom_headers(&app.config.custom_headers);
            let mut api = ApiClient::new(
                &server_url,
                app.config.accept_invalid_certs,
                custom_headers.clone(),
            );
            api.set_token(token.clone());
            app.api = Some(api);
            app.server_url = Some(normalize_server_url(&server_url));
            app.username = Some(username.clone());
            app.user_id = Some(user_id);
            app.token = Some(token.clone());

            if let Ok(mls) = MlsManager::new(&app.config.data_dir, user_id) {
                app.mls = Some(mls);
            }

            // Open message store
            if let Ok(store) = MessageStore::open(&app.config.data_dir) {
                app.msg_store = Some(store);
            }

            // Transition to dashboard
            app.screen = screen::Screen::Dashboard(screen::Dashboard::new());
            app.system_messages = vec![DisplayMessage::system(&format!(
                "Welcome back, {username}. Type /help for commands."
            ))];

            let data_dir = app.config.data_dir.clone();
            let accept_invalid_certs = app.config.accept_invalid_certs;
            let server_url = app.server_url.clone().unwrap_or_default();
            let token_clone = token.clone();
            let keygen_task = Task::perform(
                async move {
                    let mut api = ApiClient::new(&server_url, accept_invalid_certs, custom_headers);
                    api.set_token(token_clone);
                    operations::initialize_mls_and_upload_key_packages(&api, &data_dir, user_id)
                        .await
                        .map_err(|e| e.to_string())
                        .map(|_| ())
                },
                Message::KeyPackageUploaded,
            );

            let rooms_task = app.load_rooms_task();
            let welcome_task = app.accept_welcomes();
            let alias_task = app.fetch_user_alias();

            return (
                app,
                Task::batch([keygen_task, rooms_task, welcome_task, alias_task]),
            );
        }

        (app, Task::none())
    }

    pub fn title(&self) -> String {
        "Conclave".to_string()
    }

    pub fn theme(&self) -> crate::theme::Theme {
        self.theme.clone()
    }

    pub(crate) fn api_params(&self) -> ApiParams {
        ApiParams {
            server_url: self.server_url.clone().unwrap_or_default(),
            accept_invalid_certs: self.config.accept_invalid_certs,
            token: self.token.clone().unwrap_or_default(),
            custom_headers: conclave_client::api::parse_custom_headers(&self.config.custom_headers),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let on_dashboard = matches!(self.screen, screen::Screen::Dashboard(_));

        let task = match message {
            // Screen navigation
            Message::Login(msg) => self.handle_login_message(msg),
            Message::Dashboard(msg) => self.handle_dashboard_message(msg),

            // Authentication results
            Message::LoginResult(result) => self.handle_login_result(result),
            Message::RegisterResult(result) => self.handle_register_result(result),

            // Room / message async results
            Message::RoomsLoaded(result) => self.handle_rooms_loaded(result),
            Message::MessageSent(result) => self.handle_message_sent(result),
            Message::MessagesFetched(result) => self.handle_messages_fetched(result),
            Message::KeyPackageUploaded(result) => {
                if let Err(e) = result {
                    if let Some(task) = self.check_unauthorized(&e) {
                        return task;
                    }
                    self.push_system_message(&format!("Key package upload failed: {e}"));
                }
                Task::none()
            }
            Message::WelcomesProcessed(result) => self.handle_welcomes_processed(result),

            // SSE
            Message::SseEvent(update) => self.handle_sse_event(update),

            // Command results
            Message::CommandResult(result) => {
                match result {
                    Ok(msgs) => {
                        for msg in msgs {
                            self.add_message(None, msg);
                        }
                    }
                    Err(e) => {
                        if let Some(task) = self.check_unauthorized(&e) {
                            return task;
                        }
                        self.push_system_message(&format!("Error: {e}"));
                    }
                }
                Task::none()
            }
            Message::GroupCreated(result) => self.handle_group_created(result),
            Message::RefreshRooms(result) => {
                match result {
                    Ok(msgs) => {
                        for msg in msgs {
                            self.add_message(None, msg);
                        }
                    }
                    Err(e) => {
                        if let Some(task) = self.check_unauthorized(&e) {
                            return task;
                        }
                        self.push_system_message(&format!("Error: {e}"));
                    }
                }
                self.load_rooms_task()
            }
            Message::ResetComplete(result) => self.handle_reset_complete(result),
            Message::UserAliasLoaded(result) => {
                if let Ok(alias) = result {
                    self.user_alias = alias;
                }
                Task::none()
            }
            Message::NickResult(result) => match result {
                Ok(alias) => {
                    self.user_alias = Some(alias.clone());
                    self.push_system_message(&format!("Alias set to: {alias}"));
                    Task::none()
                }
                Err(e) => {
                    self.push_system_message(&format!("Failed to set alias: {e}"));
                    Task::none()
                }
            },
            Message::ExpungeResult(result) => match result {
                Ok(()) => {
                    let logout_task = self.perform_logout();
                    self.push_system_message(
                        "Account permanently deleted. All data has been wiped.",
                    );
                    logout_task
                }
                Err(e) => {
                    self.push_system_message(&format!("Failed to delete account: {e}"));
                    Task::none()
                }
            },

            // Keyboard / window events
            Message::TabPressed => {
                if matches!(self.screen, screen::Screen::Login(_)) {
                    focus_next()
                } else {
                    Task::none()
                }
            }
            Message::CopySelection => {
                if matches!(self.screen, screen::Screen::Dashboard(_)) {
                    return crate::widget::selectable_rich_text::selected(|fragments| {
                        Message::Dashboard(screen::dashboard::Message::SelectedText(fragments))
                    });
                }
                Task::none()
            }
            Message::EscapePressed => {
                if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                    if dashboard.show_user_popover {
                        dashboard.show_user_popover = false;
                    } else if let Some(room_id) = self.active_room.take() {
                        dashboard.show_members_sidebar = false;
                        let name = self
                            .rooms
                            .get(&room_id)
                            .map(|r| r.display_name())
                            .unwrap_or_default();
                        self.push_system_message(&format!(
                            "Switched away from #{name} (use /part to leave)"
                        ));
                    }
                }
                Task::none()
            }
            Message::Quit => iced::exit(),
            Message::PreviousRoom | Message::NextRoom => {
                if matches!(self.screen, screen::Screen::Dashboard(_)) {
                    let sorted = self.sorted_room_ids();
                    if !sorted.is_empty() {
                        let current_idx = self
                            .active_room
                            .and_then(|id| sorted.iter().position(|&r| r == id));
                        let target_idx = match message {
                            Message::PreviousRoom => match current_idx {
                                Some(0) | None => sorted.len() - 1,
                                Some(i) => i - 1,
                            },
                            _ => match current_idx {
                                Some(i) if i + 1 < sorted.len() => i + 1,
                                _ => 0,
                            },
                        };
                        self.select_room(sorted[target_idx]);
                    }
                }
                Task::none()
            }
            Message::GoToRoom(n) => {
                if matches!(self.screen, screen::Screen::Dashboard(_)) {
                    let sorted = self.sorted_room_ids();
                    if !sorted.is_empty() {
                        let index = if n == 9 {
                            sorted.len() - 1
                        } else {
                            (n as usize).saturating_sub(1).min(sorted.len() - 1)
                        };
                        self.select_room(sorted[index]);
                    }
                }
                Task::none()
            }
            Message::WindowFocused => {
                self.window_focused = true;
                Task::none()
            }
            Message::WindowUnfocused => {
                self.window_focused = false;
                Task::none()
            }
            Message::ExpiryTick => {
                conclave_client::state::remove_expired_messages(
                    &self.rooms,
                    &mut self.room_messages,
                );
                if let Some(store) = &self.msg_store {
                    for (group_id, room) in &self.rooms {
                        if room.message_expiry_seconds > 0 {
                            store.cleanup_expired_messages(*group_id, room.message_expiry_seconds);
                        }
                    }
                }
                Task::none()
            }
        };

        let dialog_open = matches!(
            &self.screen,
            screen::Screen::Dashboard(d) if d.show_password_dialog
        );
        if on_dashboard && !dialog_open {
            Task::batch([task, focus("chat_input")])
        } else {
            task
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        match &self.screen {
            screen::Screen::Login(login) => login.view().map(Message::Login),
            screen::Screen::Dashboard(dashboard) => dashboard
                .view(
                    &self.rooms,
                    &self.active_room,
                    &self.room_messages,
                    &self.system_messages,
                    &self.connection_status,
                    &self.username,
                    &self.user_alias,
                    &self.user_id,
                    &self.server_url,
                    self.config.accept_invalid_certs,
                    &self.theme,
                    &self.verification_status,
                    self.config.show_verified_indicator,
                )
                .map(Message::Dashboard),
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let events = iced::event::listen_with(|event, _status, _window| match event {
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Character(c),
                modifiers,
                ..
            }) if modifiers.command() && c.as_ref() == "q" => Some(Message::Quit),
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Character(c),
                modifiers,
                ..
            }) if modifiers.command() && c.as_ref() == "c" => Some(Message::CopySelection),
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Tab),
                ..
            }) => Some(Message::TabPressed),
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            }) => Some(Message::EscapePressed),
            // Ctrl+PageUp or Alt+Up: previous room
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::PageUp),
                modifiers,
                ..
            }) if modifiers.control() => Some(Message::PreviousRoom),
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::ArrowUp),
                modifiers,
                ..
            }) if modifiers.alt() => Some(Message::PreviousRoom),
            // Ctrl+PageDown or Alt+Down: next room
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::PageDown),
                modifiers,
                ..
            }) if modifiers.control() => Some(Message::NextRoom),
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::ArrowDown),
                modifiers,
                ..
            }) if modifiers.alt() => Some(Message::NextRoom),
            // Alt+1..9: jump to room by position
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Character(c),
                modifiers,
                ..
            }) if modifiers.alt() && c.len() == 1 && matches!(c.as_bytes()[0], b'1'..=b'9') => {
                Some(Message::GoToRoom(c.as_bytes()[0] - b'0'))
            }
            iced::Event::Window(iced::window::Event::Focused) => Some(Message::WindowFocused),
            iced::Event::Window(iced::window::Event::Unfocused) => Some(Message::WindowUnfocused),
            _ => None,
        });

        let mut subs = vec![events];

        if let Some(token) = &self.token {
            subs.push(
                subscription::sse(
                    self.server_url.clone().unwrap_or_default(),
                    token.clone(),
                    self.config.accept_invalid_certs,
                    conclave_client::api::parse_custom_headers(&self.config.custom_headers),
                )
                .map(Message::SseEvent),
            );
        }

        if conclave_client::state::has_expiring_rooms(&self.rooms) {
            subs.push(
                iced::time::every(std::time::Duration::from_secs(1)).map(|_| Message::ExpiryTick),
            );
        }

        Subscription::batch(subs)
    }

    /// Check if an error string indicates a 401 Unauthorized response.
    /// If so, perform auto-logout and return the logout task.
    fn check_unauthorized(&mut self, error: &str) -> Option<Task<Message>> {
        if error.contains("server error (401)") {
            let task = self.perform_logout();
            self.push_system_message("Session expired. Please log in again.");
            Some(task)
        } else {
            None
        }
    }

    // ── Helpers ───────────────────────────────────────────────────

    /// Return room IDs sorted by display name (matches sidebar order).
    pub(crate) fn sorted_room_ids(&self) -> Vec<Uuid> {
        let mut rooms: Vec<_> = self.rooms.values().collect();
        rooms.sort_by_key(|r| r.display_name());
        rooms.iter().map(|r| r.server_group_id).collect()
    }

    /// Select a room: set active, close popover, mark read.
    pub(crate) fn select_room(&mut self, room_id: Uuid) {
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
    }

    pub(crate) fn load_rooms_task(&self) -> Task<Message> {
        let params = self.api_params();
        Task::perform(
            async move {
                operations::load_rooms(&params.into_client())
                    .await
                    .map_err(|e| e.to_string())
            },
            Message::RoomsLoaded,
        )
    }

    pub(crate) fn fetch_user_alias(&self) -> Task<Message> {
        let params = self.api_params();
        Task::perform(
            async move {
                let resp = params.into_client().me().await.map_err(|e| e.to_string())?;
                let alias = if resp.alias.is_empty() {
                    None
                } else {
                    Some(resp.alias)
                };
                Ok(alias)
            },
            Message::UserAliasLoaded,
        )
    }

    pub(crate) fn fetch_messages_task(
        &self,
        group_id: Uuid,
        last_seq: u64,
        mls_group_id: String,
    ) -> Task<Message> {
        let user_id = match self.user_id {
            Some(id) => id,
            None => return Task::none(),
        };

        let params = self.api_params();
        let data_dir = self.config.data_dir.clone();
        let members: Vec<_> = self
            .rooms
            .get(&group_id)
            .map(|r| r.members.clone())
            .unwrap_or_default();
        Task::perform(
            async move {
                let api = params.into_client();
                operations::fetch_and_decrypt(
                    &api,
                    group_id,
                    last_seq,
                    &mls_group_id,
                    &data_dir,
                    user_id,
                    &members,
                )
                .await
                .map_err(|e| (group_id, e.to_string()))
            },
            Message::MessagesFetched,
        )
    }

    pub(crate) fn push_system_message(&mut self, content: &str) {
        let msg = DisplayMessage::system(content);
        self.add_message(None, msg);
    }

    pub(crate) fn add_message(&mut self, group_id: Option<Uuid>, msg: DisplayMessage) {
        // Fall back to the active room for system messages that have no group_id.
        let effective_gid = group_id.or(self.active_room);

        // Only persist to the store when the caller provided an explicit group_id
        // (system messages routed via active_room are transient).
        if let (Some(gid), Some(store)) = (effective_gid, &self.msg_store)
            && group_id.is_some()
        {
            store.push_message(gid, &msg);
        }

        match effective_gid {
            Some(gid) => {
                self.room_messages.entry(gid).or_default().push(msg);
            }
            None => {
                self.system_messages.push(msg);
            }
        }
    }

    pub(crate) fn add_message_to_room(&mut self, group_id: Uuid, msg: DisplayMessage) {
        if let Some(store) = &self.msg_store {
            store.push_message(group_id, &msg);
        }
        self.room_messages.entry(group_id).or_default().push(msg);
    }

    pub(crate) fn switch_to_room(&mut self, target: &str) {
        let resolved_gid = if let Some(room) = self.find_room_by_name(target) {
            Some(room.server_group_id)
        } else if let Ok(id) = Uuid::parse_str(target) {
            if self.rooms.contains_key(&id) {
                Some(id)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(gid) = resolved_gid {
            let name = self.rooms[&gid].display_name();
            self.active_room = Some(gid);

            if let Some(room) = self.rooms.get_mut(&gid) {
                room.last_read_seq = room.last_seen_seq;
                if let Some(store) = &self.msg_store {
                    store.set_last_read_seq(gid, room.last_read_seq);
                }
            }

            self.push_system_message(&format!("Switched to #{name}"));
        } else {
            self.push_system_message(&format!(
                "Unknown room '{target}'. Use /rooms to list available rooms."
            ));
        }
    }

    pub(crate) fn find_room_by_name(&self, name: &str) -> Option<&Room> {
        let lower = name.to_lowercase();

        // Exact match first.
        if let Some(room) = self
            .rooms
            .values()
            .find(|r| r.display_name().to_lowercase() == lower)
        {
            return Some(room);
        }

        // Fall back to unique prefix match.
        let matches: Vec<_> = self
            .rooms
            .values()
            .filter(|r| r.display_name().to_lowercase().starts_with(&lower))
            .collect();
        if matches.len() == 1 {
            return Some(matches[0]);
        }
        None
    }

    pub(crate) fn show_help(&mut self) {
        for line in conclave_client::command::format_help_lines() {
            self.push_system_message(&line);
        }
    }

    pub(crate) fn show_group_info(&mut self) {
        let group_id = match self.active_room {
            Some(id) => id,
            None => {
                self.push_system_message("No active room — use /join first");
                return;
            }
        };

        let mls_group_id = match self.group_mapping.get(&group_id) {
            Some(id) => id.clone(),
            None => {
                self.push_system_message("Group mapping not found");
                return;
            }
        };

        let room_name = self
            .rooms
            .get(&group_id)
            .map(|r| r.display_name())
            .unwrap_or_else(|| "unknown".to_string());

        if let Some(mls) = &self.mls {
            match mls.group_info_details(&mls_group_id) {
                Ok(details) => {
                    self.push_system_message(&format!("Group: #{room_name}"));
                    self.push_system_message(&format!("  Server ID: {group_id}"));
                    self.push_system_message(&format!("  MLS Group ID: {mls_group_id}"));
                    self.push_system_message(&format!("  Epoch: {}", details.epoch));
                    self.push_system_message(&format!("  Cipher Suite: {}", details.cipher_suite));
                    self.push_system_message(&format!(
                        "  Members: {} (your index: {})",
                        details.member_count, details.own_index
                    ));

                    for (index, user_id) in &details.members {
                        let marker = if *index == details.own_index {
                            " (you)"
                        } else {
                            ""
                        };
                        let display = match user_id {
                            Some(uid) => format!("user#{uid}"),
                            None => "<unknown>".to_string(),
                        };
                        self.push_system_message(&format!("    [{index}] {display}{marker}"));
                    }
                }
                Err(e) => {
                    self.push_system_message(&format!("Failed to get group info: {e}"));
                }
            }
        }
    }

    pub(crate) fn show_unread(&mut self) {
        if self.rooms.is_empty() {
            self.push_system_message("No rooms.");
            return;
        }

        let unread_lines: Vec<_> = self
            .rooms
            .values()
            .filter_map(|room| {
                // Messages between last_read_seq and last_seen_seq have been
                // fetched/decrypted but not yet viewed by the user.
                let unread = room.last_seen_seq.saturating_sub(room.last_read_seq);

                if unread > 0 {
                    Some(format!(
                        "  #{}: {unread} new message{}",
                        room.display_name(),
                        if unread == 1 { "" } else { "s" },
                    ))
                } else {
                    None
                }
            })
            .collect();

        if unread_lines.is_empty() {
            self.push_system_message("No unread messages.");
        } else {
            for line in unread_lines {
                self.push_system_message(&line);
            }
        }
    }
}
