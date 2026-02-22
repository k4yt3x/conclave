use iced::widget::operation::{focus, focus_next};
use iced::{Subscription, Task, keyboard};
use std::collections::{HashMap, HashSet};

use conclave_lib::api::{ApiClient, normalize_server_url};
use conclave_lib::command::Command;
use conclave_lib::config::{
    ClientConfig, SessionState, build_group_mapping, generate_initial_key_packages,
};
use conclave_lib::mls::MlsManager;
use conclave_lib::operations;
use conclave_lib::state::{ConnectionStatus, DisplayMessage, Room};
use conclave_lib::store::MessageStore;

use crate::screen;
use crate::subscription::{self, SseUpdate};
use crate::widget::Element;

/// Snapshot of API connection parameters for use in async tasks.
/// Captures the minimum state needed to construct an [`ApiClient`] outside
/// of `&self` so the future can be `Send`.
struct ApiParams {
    server_url: String,
    accept_invalid_certs: bool,
    token: String,
}

impl ApiParams {
    fn into_client(self) -> ApiClient {
        let mut api = ApiClient::new(&self.server_url, self.accept_invalid_certs);
        api.set_token(self.token);
        api
    }
}

pub struct Conclave {
    screen: screen::Screen,
    theme: crate::theme::Theme,
    config: ClientConfig,
    // Core state
    server_url: Option<String>,
    api: Option<ApiClient>,
    mls: Option<MlsManager>,
    username: Option<String>,
    user_alias: Option<String>,
    user_id: Option<i64>,
    token: Option<String>,
    rooms: HashMap<i64, Room>,
    active_room: Option<i64>,
    room_messages: HashMap<i64, Vec<DisplayMessage>>,
    system_messages: Vec<DisplayMessage>,
    group_mapping: HashMap<i64, String>,
    connection_status: ConnectionStatus,
    msg_store: Option<MessageStore>,
    rooms_loaded: bool,
    welcomes_processed: bool,
    fetching_groups: HashSet<i64>,
    /// Set when welcome processing triggers a rooms reload — defers the
    /// missed-message fetch until the rooms are actually in `self.rooms`.
    fetch_messages_on_rooms_load: bool,
    window_focused: bool,
    skip_keygen: bool,
}

#[derive(Debug, Clone)]
#[allow(clippy::enum_variant_names)]
pub enum Message {
    // Screen messages
    Login(screen::login::Message),
    Dashboard(screen::dashboard::Message),
    // Async results
    LoginResult(Result<LoginInfo, String>),
    RegisterResult(Result<LoginInfo, String>),
    RoomsLoaded(Result<Vec<operations::RoomInfo>, String>),
    MessageSent(Result<(operations::MessageSentResult, String), String>),
    MessagesFetched(Result<operations::FetchedMessages, (i64, String)>),
    KeygenDone(Result<Vec<(Vec<u8>, bool)>, String>),
    KeyPackageUploaded(Result<(), String>),
    WelcomesProcessed(Result<Vec<operations::WelcomeJoinResult>, String>),
    // SSE
    SseEvent(SseUpdate),
    // Commands
    CommandResult(Result<Vec<DisplayMessage>, String>),
    /// A group was created or joined — update mapping and refresh rooms.
    GroupCreated(Result<operations::GroupCreatedResult, String>),
    /// A group operation completed that requires a room refresh.
    RefreshRooms(Result<Vec<DisplayMessage>, String>),
    /// Account reset completed — update group mapping and MLS state.
    ResetComplete(Result<operations::ResetResult, String>),
    /// User alias loaded from server.
    UserAliasLoaded(Result<Option<String>, String>),
    /// /nick command result — carries the new alias.
    NickResult(Result<String, String>),
    /// /topic command result — carries the new topic.
    TopicResult(Result<String, String>),
    /// Tab key pressed (for login field navigation).
    TabPressed,
    /// Escape key pressed (close popover, etc.).
    EscapePressed,
    /// Quit the application (e.g. Ctrl+Q).
    Quit,
    WindowFocused,
    WindowUnfocused,
}

#[derive(Debug, Clone)]
pub struct LoginInfo {
    pub server_url: String,
    pub token: String,
    pub user_id: i64,
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
            window_focused: true,
            skip_keygen: false,
        };

        if let (Some(server_url), Some(token), Some(username), Some(user_id)) = (
            session.server_url,
            session.token,
            session.username,
            session.user_id,
        ) {
            let mut api = ApiClient::new(&server_url, app.config.accept_invalid_certs);
            api.set_token(token.clone());
            app.api = Some(api);
            app.server_url = Some(normalize_server_url(&server_url));
            app.username = Some(username.clone());
            app.user_id = Some(user_id);
            app.token = Some(token);

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
            let keygen_task = Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        let mls = MlsManager::new(&data_dir, user_id).map_err(|e| e.to_string())?;
                        generate_initial_key_packages(&mls).map_err(|e| e.to_string())
                    })
                    .await
                    .map_err(|e| e.to_string())?
                },
                Message::KeygenDone,
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

    fn api_params(&self) -> ApiParams {
        ApiParams {
            server_url: self.server_url.clone().unwrap_or_default(),
            accept_invalid_certs: self.config.accept_invalid_certs,
            token: self.token.clone().unwrap_or_default(),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Login(msg) => self.handle_login_message(msg),
            Message::Dashboard(msg) => self.handle_dashboard_message(msg),
            Message::LoginResult(result) => self.handle_login_result(result),
            Message::RegisterResult(result) => self.handle_register_result(result),
            Message::RoomsLoaded(result) => self.handle_rooms_loaded(result),
            Message::MessageSent(result) => self.handle_message_sent(result),
            Message::MessagesFetched(result) => self.handle_messages_fetched(result),
            Message::KeygenDone(result) => self.handle_keygen_done(result),
            Message::KeyPackageUploaded(result) => {
                if let Err(e) = result {
                    self.push_system_message(&format!("Key package upload failed: {e}"));
                }
                Task::none()
            }
            Message::WelcomesProcessed(result) => self.handle_welcomes_processed(result),
            Message::SseEvent(update) => self.handle_sse_event(update),
            Message::CommandResult(result) => {
                match result {
                    Ok(msgs) => {
                        for msg in msgs {
                            self.add_message(None, msg);
                        }
                    }
                    Err(e) => self.push_system_message(&format!("Error: {e}")),
                }
                Task::none()
            }
            Message::TabPressed => {
                if matches!(self.screen, screen::Screen::Login(_)) {
                    focus_next()
                } else {
                    Task::none()
                }
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
            Message::WindowFocused => {
                self.window_focused = true;
                Task::none()
            }
            Message::WindowUnfocused => {
                self.window_focused = false;
                Task::none()
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
                    self.load_rooms_task()
                }
                Err(e) => {
                    self.push_system_message(&format!("Failed to set alias: {e}"));
                    Task::none()
                }
            },
            Message::TopicResult(result) => match result {
                Ok(topic) => {
                    self.push_system_message(&format!("Room alias set to: {topic}"));
                    self.load_rooms_task()
                }
                Err(e) => {
                    self.push_system_message(&format!("Failed to set topic: {e}"));
                    Task::none()
                }
            },
            Message::GroupCreated(result) => self.handle_group_created(result),
            Message::RefreshRooms(result) => {
                match result {
                    Ok(msgs) => {
                        for msg in msgs {
                            self.add_message(None, msg);
                        }
                    }
                    Err(e) => self.push_system_message(&format!("Error: {e}")),
                }
                self.load_rooms_task()
            }
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
                key: keyboard::Key::Named(keyboard::key::Named::Tab),
                ..
            }) => Some(Message::TabPressed),
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            }) => Some(Message::EscapePressed),
            iced::Event::Window(iced::window::Event::Focused) => Some(Message::WindowFocused),
            iced::Event::Window(iced::window::Event::Unfocused) => Some(Message::WindowUnfocused),
            _ => None,
        });

        if let Some(token) = &self.token {
            let sse = subscription::sse(
                self.server_url.clone().unwrap_or_default(),
                token.clone(),
                self.config.accept_invalid_certs,
            )
            .map(Message::SseEvent);
            Subscription::batch([sse, events])
        } else {
            events
        }
    }

    // ── Login handling ────────────────────────────────────────────

    fn handle_login_message(&mut self, msg: screen::login::Message) -> Task<Message> {
        let screen::Screen::Login(login) = &mut self.screen else {
            return Task::none();
        };

        match msg {
            screen::login::Message::ServerUrlChanged(url) => {
                login.server_url = url;
                Task::none()
            }
            screen::login::Message::UsernameChanged(name) => {
                login.username = name;
                Task::none()
            }
            screen::login::Message::PasswordChanged(pw) => {
                login.password = pw;
                Task::none()
            }
            screen::login::Message::FocusUsername => focus("login_username"),
            screen::login::Message::FocusPassword => focus("login_password"),
            screen::login::Message::ToggleMode => {
                login.mode = match login.mode {
                    screen::login::Mode::Login => screen::login::Mode::Register,
                    screen::login::Mode::Register => screen::login::Mode::Login,
                };
                login.status = screen::login::Status::Idle;
                Task::none()
            }
            screen::login::Message::Submit => {
                let server_url = login.server_url.clone();
                let accept_invalid_certs = self.config.accept_invalid_certs;
                let username = login.username.clone();
                let password = login.password.clone();
                let mode = login.mode.clone();

                if username.is_empty() || password.is_empty() {
                    login.status =
                        screen::login::Status::Error("Username and password required".into());
                    return Task::none();
                }

                login.status = screen::login::Status::Loading;

                match mode {
                    screen::login::Mode::Login => Task::perform(
                        async move {
                            let api = ApiClient::new(&server_url, accept_invalid_certs);
                            let resp = api
                                .login(&username, &password)
                                .await
                                .map_err(|e| e.to_string())?;
                            let canonical_username = if resp.username.is_empty() {
                                username
                            } else {
                                resp.username
                            };
                            Ok(LoginInfo {
                                server_url,
                                token: resp.token,
                                user_id: resp.user_id,
                                username: canonical_username,
                            })
                        },
                        Message::LoginResult,
                    ),
                    screen::login::Mode::Register => Task::perform(
                        {
                            let data_dir = self.config.data_dir.clone();
                            async move {
                                let api = ApiClient::new(&server_url, accept_invalid_certs);
                                let resp = api
                                    .register(&username, &password, None)
                                    .await
                                    .map_err(|e| e.to_string())?;
                                let user_id = resp.user_id;

                                let login_resp = api
                                    .login(&username, &password)
                                    .await
                                    .map_err(|e| e.to_string())?;
                                let canonical_username = if login_resp.username.is_empty() {
                                    username
                                } else {
                                    login_resp.username
                                };
                                let token = login_resp.token;

                                let mut auth_api =
                                    ApiClient::new(&server_url, accept_invalid_certs);
                                auth_api.set_token(token.clone());

                                std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
                                let entries = tokio::task::spawn_blocking({
                                    let data_dir = data_dir.clone();
                                    move || {
                                        let mls = MlsManager::new(&data_dir, user_id)
                                            .map_err(|e| e.to_string())?;
                                        generate_initial_key_packages(&mls)
                                            .map_err(|e| e.to_string())
                                    }
                                })
                                .await
                                .map_err(|e| e.to_string())??;

                                auth_api
                                    .upload_key_packages(entries)
                                    .await
                                    .map_err(|e| e.to_string())?;

                                Ok(LoginInfo {
                                    server_url,
                                    token,
                                    user_id,
                                    username: canonical_username,
                                })
                            }
                        },
                        Message::RegisterResult,
                    ),
                }
            }
        }
    }

    fn handle_login_result(&mut self, result: Result<LoginInfo, String>) -> Task<Message> {
        match result {
            Ok(info) => {
                // Store the server URL the user actually connected to
                self.server_url = Some(normalize_server_url(&info.server_url));

                // Set up API client
                let mut api = ApiClient::new(&info.server_url, self.config.accept_invalid_certs);
                api.set_token(info.token.clone());
                self.api = Some(api);
                self.username = Some(info.username.clone());
                self.user_id = Some(info.user_id);
                self.token = Some(info.token.clone());

                // Save session
                let session = SessionState {
                    server_url: Some(info.server_url),
                    token: Some(info.token),
                    user_id: Some(info.user_id),
                    username: Some(info.username.clone()),
                };
                if let Err(error) = session.save(&self.config.data_dir) {
                    tracing::warn!(%error, "failed to save session");
                }

                if let Err(error) = std::fs::create_dir_all(&self.config.data_dir) {
                    tracing::warn!(%error, "failed to create data directory");
                }
                if let Ok(mls) = MlsManager::new(&self.config.data_dir, info.user_id) {
                    self.mls = Some(mls);
                }

                // Open message store
                if let Ok(store) = MessageStore::open(&self.config.data_dir) {
                    self.msg_store = Some(store);
                }

                // Transition to dashboard
                self.screen = screen::Screen::Dashboard(screen::Dashboard::new());
                self.system_messages = vec![DisplayMessage::system(&format!(
                    "Logged in as {} (ID {}). Type /help for commands.",
                    info.username, info.user_id
                ))];

                let rooms_task = self.load_rooms_task();
                let alias_task = self.fetch_user_alias();

                if self.skip_keygen {
                    self.skip_keygen = false;
                    Task::batch([rooms_task, alias_task])
                } else {
                    let data_dir = self.config.data_dir.clone();
                    let keygen_user_id = info.user_id;
                    let keygen_task = Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || {
                                let mls = MlsManager::new(&data_dir, keygen_user_id)
                                    .map_err(|e| e.to_string())?;
                                generate_initial_key_packages(&mls).map_err(|e| e.to_string())
                            })
                            .await
                            .map_err(|e| e.to_string())?
                        },
                        Message::KeygenDone,
                    );
                    Task::batch([keygen_task, rooms_task, alias_task])
                }
            }
            Err(e) => {
                if let screen::Screen::Login(login) = &mut self.screen {
                    login.status = screen::login::Status::Error(e);
                }
                Task::none()
            }
        }
    }

    fn handle_register_result(&mut self, result: Result<LoginInfo, String>) -> Task<Message> {
        match result {
            Ok(info) => {
                self.skip_keygen = true;
                self.handle_login_result(Ok(info))
            }
            Err(e) => {
                if let screen::Screen::Login(login) = &mut self.screen {
                    login.status = screen::login::Status::Error(e);
                }
                Task::none()
            }
        }
    }

    fn handle_keygen_done(
        &mut self,
        result: Result<Vec<(Vec<u8>, bool)>, String>,
    ) -> Task<Message> {
        match result {
            Ok(entries) => {
                let params = self.api_params();
                Task::perform(
                    async move {
                        params
                            .into_client()
                            .upload_key_packages(entries)
                            .await
                            .map_err(|e| e.to_string())
                    },
                    Message::KeyPackageUploaded,
                )
            }
            Err(e) => {
                self.push_system_message(&format!("Key generation failed: {e}"));
                Task::none()
            }
        }
    }

    // ── Dashboard handling ────────────────────────────────────────

    fn handle_dashboard_message(&mut self, msg: screen::dashboard::Message) -> Task<Message> {
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
            screen::dashboard::Message::CopyText(text) => {
                if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                    dashboard.toast = Some("Copied to clipboard".into());
                }
                Task::batch([
                    iced::clipboard::write(text),
                    Task::perform(
                        async {
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        },
                        |_| Message::Dashboard(screen::dashboard::Message::DismissToast),
                    ),
                ])
            }
            screen::dashboard::Message::DismissToast => {
                if let screen::Screen::Dashboard(dashboard) = &mut self.screen {
                    dashboard.toast = None;
                }
                Task::none()
            }
            screen::dashboard::Message::Logout => self.perform_logout(),
        }
    }

    fn handle_input_text(&mut self, text: String) -> Task<Message> {
        match conclave_lib::command::parse(&text) {
            Ok(Command::Quit) => iced::exit(),
            Ok(Command::Message { text }) => self.send_message(text),
            Ok(Command::Help) => {
                self.show_help();
                Task::none()
            }
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
            Ok(Command::Who) => {
                if let Some(room_id) = self.active_room {
                    if let Some(room) = self.rooms.get(&room_id) {
                        let member_names: Vec<&str> =
                            room.members.iter().map(|m| m.display_name()).collect();
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
            Ok(Command::Login { .. }) | Ok(Command::Register { .. }) => {
                self.push_system_message("Already logged in. Use /logout first.");
                Task::none()
            }
            Ok(Command::Logout) => self.perform_logout(),
            Ok(Command::Create { name, members }) => self.create_group(name, members),
            Ok(Command::Invite { members }) => self.invite_members(members),
            Ok(Command::Kick { username }) => self.kick_member(username),
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

    // ── SSE handling ──────────────────────────────────────────────

    fn handle_sse_event(&mut self, update: SseUpdate) -> Task<Message> {
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
        }
    }

    // ── Rooms ─────────────────────────────────────────────────────

    fn handle_group_created(
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

    fn handle_rooms_loaded(
        &mut self,
        result: Result<Vec<operations::RoomInfo>, String>,
    ) -> Task<Message> {
        match result {
            Ok(room_infos) => {
                // Build group mapping from server-provided MLS group IDs.
                self.group_mapping = build_group_mapping(&room_infos, &self.config.data_dir);

                let mut server_group_ids = HashSet::new();

                for info in room_infos {
                    server_group_ids.insert(info.group_id);

                    let (existing_seq, existing_read) = self
                        .rooms
                        .get(&info.group_id)
                        .map(|r| (r.last_seen_seq, r.last_read_seq))
                        .unwrap_or((0, 0));

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
                        },
                    );

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

    fn handle_messages_fetched(
        &mut self,
        result: Result<operations::FetchedMessages, (i64, String)>,
    ) -> Task<Message> {
        match result {
            Ok(fetched) => {
                self.fetching_groups.remove(&fetched.group_id);

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

                if self.active_room == Some(fetched.group_id)
                    && let Some(room) = self.rooms.get_mut(&fetched.group_id)
                {
                    room.last_read_seq = room.last_seen_seq;
                    if let Some(store) = &self.msg_store {
                        store.set_last_read_seq(fetched.group_id, room.last_read_seq);
                    }
                }

                if !self.window_focused {
                    let last_msg = fetched.messages.iter().rev().find(|m| !m.is_system);
                    if let Some(msg) = last_msg {
                        let room_name = self
                            .rooms
                            .get(&fetched.group_id)
                            .map(|r| r.display_name())
                            .unwrap_or_else(|| "unknown".to_string());
                        conclave_lib::notification::send_notification(
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

    fn handle_message_sent(
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

    fn handle_welcomes_processed(
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

                if let Some(last) = welcomes.last() {
                    self.active_room = Some(last.group_id);
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

    // ── Message sending ───────────────────────────────────────────

    fn send_message(&mut self, text: String) -> Task<Message> {
        let group_id = match self.active_room {
            Some(id) => id,
            None => {
                self.push_system_message("No active room — use /join first");
                return Task::none();
            }
        };

        self.send_to_group(group_id, &text)
    }

    fn send_to_room(&mut self, room: &str, text: &str) -> Task<Message> {
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

        let params = self.api_params();
        let data_dir = self.config.data_dir.clone();
        let user_id = self.user_id.unwrap_or(0);
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

    // ── Group operations ──────────────────────────────────────────

    fn create_group(&mut self, name: String, members: Vec<String>) -> Task<Message> {
        let params = self.api_params();
        let data_dir = self.config.data_dir.clone();
        let user_id = self.user_id.unwrap_or(0);

        Task::perform(
            async move {
                let api = params.into_client();
                operations::create_group(&api, None, Some(&name), members, &data_dir, user_id)
                    .await
                    .map_err(|e| e.to_string())
            },
            Message::GroupCreated,
        )
    }

    fn invite_members(&mut self, members: Vec<String>) -> Task<Message> {
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

        let params = self.api_params();
        let data_dir = self.config.data_dir.clone();
        let user_id = self.user_id.unwrap_or(0);

        Task::perform(
            async move {
                let api = params.into_client();
                let invited = operations::invite_members(
                    &api,
                    group_id,
                    &mls_group_id,
                    members,
                    &data_dir,
                    user_id,
                )
                .await
                .map_err(|e| e.to_string())?;

                if invited.is_empty() {
                    Ok(vec![DisplayMessage::system("No new members to invite.")])
                } else {
                    Ok(vec![DisplayMessage::system(&format!(
                        "Invited {} to the room",
                        invited.join(", ")
                    ))])
                }
            },
            Message::RefreshRooms,
        )
    }

    fn kick_member(&mut self, target: String) -> Task<Message> {
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

        let params = self.api_params();
        let data_dir = self.config.data_dir.clone();
        let user_id = self.user_id.unwrap_or(0);

        Task::perform(
            async move {
                let api = params.into_client();
                operations::kick_member(
                    &api,
                    group_id,
                    &mls_group_id,
                    &target,
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

    fn leave_group(&mut self) -> Task<Message> {
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

        let params = self.api_params();
        let data_dir = self.config.data_dir.clone();
        let user_id = self.user_id.unwrap_or(0);

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

    fn rotate_keys(&mut self) -> Task<Message> {
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

        let params = self.api_params();
        let data_dir = self.config.data_dir.clone();
        let user_id = self.user_id.unwrap_or(0);

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

    fn reset_account(&mut self) -> Task<Message> {
        if self.username.is_none() {
            self.push_system_message("Not logged in.");
            return Task::none();
        }

        let params = self.api_params();
        let data_dir = self.config.data_dir.clone();
        let user_id = self.user_id.unwrap_or(0);

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

    fn handle_reset_complete(
        &mut self,
        result: Result<operations::ResetResult, String>,
    ) -> Task<Message> {
        match result {
            Ok(info) => {
                self.group_mapping = info.new_group_mapping;

                if let Some(uid) = self.user_id
                    && let Ok(mls) = MlsManager::new(&self.config.data_dir, uid)
                {
                    self.mls = Some(mls);
                }

                // Clear stale fetch state from pre-reset MLS groups
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

    fn fetch_all_missed_messages(&mut self) -> Task<Message> {
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

    fn accept_welcomes(&mut self) -> Task<Message> {
        let params = self.api_params();
        let data_dir = self.config.data_dir.clone();
        let user_id = self.user_id.unwrap_or(0);

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

    // ── Helpers ───────────────────────────────────────────────────

    fn load_rooms_task(&self) -> Task<Message> {
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

    fn fetch_user_alias(&self) -> Task<Message> {
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

    fn fetch_messages_task(
        &self,
        group_id: i64,
        last_seq: u64,
        mls_group_id: String,
    ) -> Task<Message> {
        let params = self.api_params();
        let data_dir = self.config.data_dir.clone();
        let user_id = self.user_id.unwrap_or(0);
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

    fn push_system_message(&mut self, content: &str) {
        let msg = DisplayMessage::system(content);
        self.add_message(None, msg);
    }

    fn add_message(&mut self, group_id: Option<i64>, msg: DisplayMessage) {
        let effective_gid = group_id.or(self.active_room);

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

    fn add_message_to_room(&mut self, group_id: i64, msg: DisplayMessage) {
        if let Some(store) = &self.msg_store {
            store.push_message(group_id, &msg);
        }
        self.room_messages.entry(group_id).or_default().push(msg);
    }

    fn switch_to_room(&mut self, target: &str) {
        let resolved_gid = if let Some(room) = self.find_room_by_name(target) {
            Some(room.server_group_id)
        } else if let Ok(id) = target.parse::<i64>() {
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
                "Unknown room '{target}'. Use /list to list available rooms."
            ));
        }
    }

    fn find_room_by_name(&self, name: &str) -> Option<&Room> {
        let lower = name.to_lowercase();
        if let Some(room) = self
            .rooms
            .values()
            .find(|r| r.display_name().to_lowercase() == lower)
        {
            return Some(room);
        }
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

    fn show_help(&mut self) {
        let help = [
            "/create <name> <user1,user2>  Create a room with members",
            "/join                         Accept pending invitations",
            "/join <room>                  Switch to a room",
            "/invite <user1,user2>         Invite to the active room",
            "/kick <username>              Remove a member from the room",
            "/nick <alias>                 Set your display name",
            "/topic <text>                 Set active room's display alias",
            "/part                         Leave the room (MLS removal)",
            "/close                        Switch away without leaving",
            "/rotate                       Rotate keys (forward secrecy)",
            "/reset                        Reset account and rejoin groups",
            "/info                         Show MLS group details",
            "/list                         List your rooms",
            "/unread                       Check rooms for new messages",
            "/who                          List members of active room",
            "/msg <room> <text>            Send to a room without switching",
            "/whois                        Show current user info",
            "/logout                       Logout and revoke session",
            "/help                         Show this help",
            "/quit                         Exit",
            "",
            "Type text without / to send a message to the active room.",
        ];
        for line in help {
            self.push_system_message(line);
        }
    }

    fn show_group_info(&mut self) {
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

    fn show_unread(&mut self) {
        if self.rooms.is_empty() {
            self.push_system_message("No rooms.");
            return;
        }

        let unread_lines: Vec<_> = self
            .rooms
            .values()
            .filter_map(|room| {
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

    fn perform_logout(&mut self) -> Task<Message> {
        // Capture server info before clearing state so we can revoke the
        // server-side token asynchronously.
        let params = self.api_params();

        // Clear local state.
        self.token = None;
        self.api = None;
        self.mls = None;
        self.username = None;
        self.user_alias = None;
        self.user_id = None;
        self.active_room = None;
        self.rooms.clear();
        self.group_mapping.clear();
        self.room_messages.clear();
        self.system_messages.clear();
        self.msg_store = None;
        self.rooms_loaded = false;
        self.welcomes_processed = false;
        self.connection_status = ConnectionStatus::Disconnected;

        // Delete session file.
        let session_path = self.config.data_dir.join("session.toml");
        if let Err(error) = std::fs::remove_file(session_path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(%error, "failed to remove session file");
        }

        // Go back to login screen.
        self.screen = screen::Screen::Login(screen::Login::new(
            self.server_url.clone().unwrap_or_default(),
        ));

        // Revoke the server-side token in the background.
        if !params.token.is_empty() {
            Task::perform(
                async move {
                    if let Err(error) = params.into_client().logout().await {
                        tracing::warn!(%error, "server-side token revocation failed");
                    }
                    Ok::<_, String>(vec![])
                },
                Message::CommandResult,
            )
        } else {
            Task::none()
        }
    }
}
