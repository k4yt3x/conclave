use iced::widget::operation::{focus, focus_next};
use iced::{Subscription, Task, keyboard};
use std::collections::HashMap;

use conclave_lib::api::ApiClient;
use conclave_lib::command::Command;
use conclave_lib::config::{ClientConfig, SessionState};
use conclave_lib::mls::MlsManager;
use conclave_lib::state::{ConnectionStatus, DisplayMessage, Room};
use conclave_lib::store::MessageStore;

use crate::screen;
use crate::subscription::{self, SseUpdate};
use crate::widget::Element;

pub struct Conclave {
    screen: screen::Screen,
    theme: crate::theme::Theme,
    config: ClientConfig,
    // Core state
    server_url: Option<String>,
    api: Option<ApiClient>,
    mls: Option<MlsManager>,
    username: Option<String>,
    user_id: Option<u64>,
    token: Option<String>,
    rooms: HashMap<String, Room>,
    active_room: Option<String>,
    room_messages: HashMap<String, Vec<DisplayMessage>>,
    system_messages: Vec<DisplayMessage>,
    group_mapping: HashMap<String, String>,
    connection_status: ConnectionStatus,
    msg_store: Option<MessageStore>,
    rooms_loaded: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    // Screen messages
    Login(screen::login::Message),
    Dashboard(screen::dashboard::Message),
    // Async results
    LoginResult(Result<LoginInfo, String>),
    RegisterResult(Result<RegisterInfo, String>),
    RoomsLoaded(Result<Vec<RoomInfo>, String>),
    MessageSent(Result<MessageSentInfo, String>),
    MessagesFetched(Result<FetchedMessages, String>),
    KeygenDone(Result<Vec<(Vec<u8>, bool)>, String>),
    KeyPackageUploaded(Result<(), String>),
    WelcomesProcessed(Result<Vec<WelcomeResult>, String>),
    // SSE
    SseEvent(SseUpdate),
    // Commands
    CommandResult(Result<Vec<DisplayMessage>, String>),
    /// A group was created or joined — update mapping and refresh rooms.
    GroupCreated(Result<GroupCreatedInfo, String>),
    /// A group operation completed that requires a room refresh.
    RefreshRooms(Result<Vec<DisplayMessage>, String>),
    /// Tab key pressed (for login field navigation).
    TabPressed,
    /// Quit the application (e.g. Ctrl+Q).
    Quit,
}

#[derive(Debug, Clone)]
pub struct LoginInfo {
    pub server_url: String,
    pub token: String,
    pub user_id: u64,
    pub username: String,
}

#[derive(Debug, Clone)]
pub struct RegisterInfo {
    pub user_id: u64,
}

#[derive(Debug, Clone)]
pub struct RoomInfo {
    pub group_id: String,
    pub name: String,
    pub members: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct MessageSentInfo {
    pub group_id: String,
    pub sequence_num: u64,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct FetchedMessages {
    pub group_id: String,
    pub messages: Vec<DecryptedMsg>,
}

#[derive(Debug, Clone)]
pub struct DecryptedMsg {
    pub sender: String,
    pub content: String,
    pub timestamp: i64,
    pub sequence_num: u64,
    pub is_system: bool,
}

#[derive(Debug, Clone)]
pub struct WelcomeResult {
    pub group_id: String,
    pub group_name: String,
    pub mls_group_id: String,
}

#[derive(Debug, Clone)]
pub struct GroupCreatedInfo {
    pub server_group_id: String,
    pub mls_group_id: String,
    pub messages: Vec<DisplayMessage>,
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
            app.server_url = Some(server_url.clone());
            app.username = Some(username.clone());
            app.user_id = Some(user_id);
            app.token = Some(token);

            // Initialize MLS
            if let Ok(mls) = MlsManager::new(&app.config.data_dir, &username) {
                app.mls = Some(mls);
            }

            // Load group mapping
            app.group_mapping = load_group_mapping(&app.config.data_dir);

            // Open message store
            if let Ok(store) = MessageStore::open(&app.config.data_dir) {
                app.msg_store = Some(store);
            }

            // Transition to dashboard
            app.screen = screen::Screen::Dashboard(screen::Dashboard::new());

            // Generate key packages and load rooms from server
            let accept_invalid_certs = app.config.accept_invalid_certs;
            let token = app.token.clone().unwrap_or_default();

            let data_dir = app.config.data_dir.clone();
            let keygen_username = username.clone();
            let keygen_task = Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        let mls = MlsManager::new(&data_dir, &keygen_username)
                            .map_err(|e| e.to_string())?;
                        generate_initial_key_packages(&mls)
                    })
                    .await
                    .map_err(|e| e.to_string())?
                },
                Message::KeygenDone,
            );

            let rooms_task = Task::perform(
                async move {
                    let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                    api.set_token(token);
                    load_rooms_async(&api).await
                },
                Message::RoomsLoaded,
            );

            return (app, Task::batch([keygen_task, rooms_task]));
        }

        (app, Task::none())
    }

    pub fn title(&self) -> String {
        "Conclave".to_string()
    }

    pub fn theme(&self) -> crate::theme::Theme {
        self.theme.clone()
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
            Message::Quit => iced::exit(),
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
                // Trigger a room reload
                let server_url = self.server_url.clone().unwrap_or_default();
                let accept_invalid_certs = self.config.accept_invalid_certs;
                let token = self.token.clone().unwrap_or_default();
                Task::perform(
                    async move {
                        let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                        api.set_token(token);
                        load_rooms_async(&api).await
                    },
                    Message::RoomsLoaded,
                )
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
                )
                .map(Message::Dashboard),
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let kbd = iced::event::listen_with(|event, _status, _window| match event {
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Character(c),
                modifiers,
                ..
            }) if modifiers.command() && c.as_ref() == "q" => Some(Message::Quit),
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Tab),
                ..
            }) => Some(Message::TabPressed),
            _ => None,
        });

        if let (Some(token), true) = (&self.token, self.rooms_loaded) {
            let sse = subscription::sse(
                self.server_url.clone().unwrap_or_default(),
                token.clone(),
                self.config.accept_invalid_certs,
            )
            .map(Message::SseEvent);
            Subscription::batch([sse, kbd])
        } else {
            kbd
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
                            Ok(LoginInfo {
                                server_url,
                                token: resp.token,
                                user_id: resp.user_id,
                                username,
                            })
                        },
                        Message::LoginResult,
                    ),
                    screen::login::Mode::Register => Task::perform(
                        async move {
                            let api = ApiClient::new(&server_url, accept_invalid_certs);
                            let resp = api
                                .register(&username, &password)
                                .await
                                .map_err(|e| e.to_string())?;
                            Ok(RegisterInfo {
                                user_id: resp.user_id,
                            })
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
                self.server_url = Some(info.server_url.clone());

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
                let _ = session.save(&self.config.data_dir);

                // Initialize MLS
                let _ = std::fs::create_dir_all(&self.config.data_dir);
                if let Ok(mls) = MlsManager::new(&self.config.data_dir, &info.username) {
                    self.mls = Some(mls);
                }

                // Load group mapping
                self.group_mapping = load_group_mapping(&self.config.data_dir);

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

                // Generate key packages and load rooms
                let data_dir = self.config.data_dir.clone();
                let username = info.username;
                let keygen_task = Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let mls =
                                MlsManager::new(&data_dir, &username).map_err(|e| e.to_string())?;
                            generate_initial_key_packages(&mls)
                        })
                        .await
                        .map_err(|e| e.to_string())?
                    },
                    Message::KeygenDone,
                );

                let server_url = self.server_url.clone().unwrap_or_default();
                let accept_invalid_certs = self.config.accept_invalid_certs;
                let token = self.token.clone().unwrap_or_default();
                let rooms_task = Task::perform(
                    async move {
                        let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                        api.set_token(token);
                        load_rooms_async(&api).await
                    },
                    Message::RoomsLoaded,
                );

                Task::batch([keygen_task, rooms_task])
            }
            Err(e) => {
                if let screen::Screen::Login(login) = &mut self.screen {
                    login.status = screen::login::Status::Error(e);
                }
                Task::none()
            }
        }
    }

    fn handle_register_result(&mut self, result: Result<RegisterInfo, String>) -> Task<Message> {
        if let screen::Screen::Login(login) = &mut self.screen {
            match result {
                Ok(info) => {
                    login.status = screen::login::Status::Success(format!(
                        "Registered as user ID {}. You can now login.",
                        info.user_id
                    ));
                    login.mode = screen::login::Mode::Login;
                }
                Err(e) => {
                    login.status = screen::login::Status::Error(e);
                }
            }
        }
        Task::none()
    }

    fn handle_keygen_done(
        &mut self,
        result: Result<Vec<(Vec<u8>, bool)>, String>,
    ) -> Task<Message> {
        match result {
            Ok(entries) => {
                let server_url = self.server_url.clone().unwrap_or_default();
                let accept_invalid_certs = self.config.accept_invalid_certs;
                let token = self.token.clone().unwrap_or_default();
                Task::perform(
                    async move {
                        let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                        api.set_token(token);
                        api.upload_key_packages(entries)
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
                self.active_room = Some(room_id.clone());

                // Mark messages as read
                if let Some(room) = self.rooms.get_mut(&room_id) {
                    room.last_read_seq = room.last_seen_seq;
                    if let Some(store) = &self.msg_store {
                        store.set_last_read_seq(&room_id, room.last_read_seq);
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
            screen::dashboard::Message::Logout => {
                self.perform_logout();
                Task::none()
            }
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
            Ok(Command::Rooms) => {
                let server_url = self.server_url.clone().unwrap_or_default();
                let accept_invalid_certs = self.config.accept_invalid_certs;
                let token = self.token.clone().unwrap_or_default();
                let active_room = self.active_room.clone();
                Task::perform(
                    async move {
                        let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                        api.set_token(token);
                        let rooms = load_rooms_async(&api).await.map_err(|e| e.to_string())?;
                        if rooms.is_empty() {
                            return Ok(vec![DisplayMessage::system("No rooms.")]);
                        }
                        let mut msgs = vec![DisplayMessage::system("Rooms:")];
                        for r in &rooms {
                            let active = if active_room.as_deref() == Some(&r.group_id) {
                                " (active)"
                            } else {
                                ""
                            };
                            msgs.push(DisplayMessage::system(&format!(
                                "  #{} [{}]{active}",
                                r.name,
                                r.members.join(", "),
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
                if let Some(room_id) = &self.active_room {
                    if let Some(room) = self.rooms.get(room_id) {
                        self.push_system_message(&format!(
                            "Members of #{}: {}",
                            room.name,
                            room.members.join(", ")
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
            Ok(Command::Part) => {
                if let Some(room_id) = self.active_room.take() {
                    let name = self
                        .rooms
                        .get(&room_id)
                        .map(|r| r.name.clone())
                        .unwrap_or_default();
                    self.push_system_message(&format!(
                        "Switched away from #{name} (use /leave to leave)"
                    ));
                }
                Task::none()
            }
            Ok(Command::Unread) => {
                self.show_unread();
                Task::none()
            }
            Ok(Command::Me) => {
                let server_url = self.server_url.clone().unwrap_or_default();
                let accept_invalid_certs = self.config.accept_invalid_certs;
                let token = self.token.clone().unwrap_or_default();
                Task::perform(
                    async move {
                        let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                        api.set_token(token);
                        let resp = api.me().await.map_err(|e| e.to_string())?;
                        Ok(vec![DisplayMessage::system(&format!(
                            "User: {} (ID: {})",
                            resp.username, resp.user_id
                        ))])
                    },
                    Message::CommandResult,
                )
            }
            Ok(Command::Login { .. }) | Ok(Command::Register { .. }) => {
                self.push_system_message("Already logged in. Use /logout first.");
                Task::none()
            }
            Ok(Command::Logout) => {
                self.perform_logout();
                Task::none()
            }
            Ok(Command::Create { name, members }) => self.create_group(name, members),
            Ok(Command::Invite { members }) => self.invite_members(members),
            Ok(Command::Kick { username }) => self.kick_member(username),
            Ok(Command::Leave) => self.leave_group(),
            Ok(Command::Rotate) => self.rotate_keys(),
            Ok(Command::Reset) => {
                self.push_system_message("Reset not yet supported in GUI. Use CLI.");
                Task::none()
            }
            Ok(Command::Keygen) => {
                let data_dir = self.config.data_dir.clone();
                let username = self.username.clone().unwrap_or_default();
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let mls =
                                MlsManager::new(&data_dir, &username).map_err(|e| e.to_string())?;
                            generate_initial_key_packages(&mls)
                        })
                        .await
                        .map_err(|e| e.to_string())?
                    },
                    Message::KeygenDone,
                )
            }
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

                // Fetch missed messages for all rooms on reconnect
                let tasks: Vec<_> = self
                    .rooms
                    .keys()
                    .map(|group_id| {
                        let server_url = self.server_url.clone().unwrap_or_default();
                        let accept_invalid_certs = self.config.accept_invalid_certs;
                        let token = self.token.clone().unwrap_or_default();
                        let last_seq = self
                            .rooms
                            .get(group_id)
                            .map(|r| r.last_seen_seq)
                            .unwrap_or(0);
                        let mls_group_id = self.group_mapping.get(group_id).cloned();
                        let username = self.username.clone();
                        let data_dir = self.config.data_dir.clone();
                        let group_id = group_id.clone();

                        Task::perform(
                            async move {
                                let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                                api.set_token(token);
                                fetch_and_decrypt(
                                    &api,
                                    &group_id,
                                    last_seq,
                                    mls_group_id.as_deref(),
                                    username.as_deref(),
                                    &data_dir,
                                )
                                .await
                            },
                            Message::MessagesFetched,
                        )
                    })
                    .collect();

                Task::batch(tasks)
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
                // Fetch and decrypt new messages
                let server_url = self.server_url.clone().unwrap_or_default();
                let accept_invalid_certs = self.config.accept_invalid_certs;
                let token = self.token.clone().unwrap_or_default();
                let last_seq = self
                    .rooms
                    .get(&group_id)
                    .map(|r| r.last_seen_seq)
                    .unwrap_or(0);
                let mls_group_id = self.group_mapping.get(&group_id).cloned();
                let username = self.username.clone();
                let data_dir = self.config.data_dir.clone();
                let group_id_clone = group_id.clone();

                Task::perform(
                    async move {
                        let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                        api.set_token(token);
                        fetch_and_decrypt(
                            &api,
                            &group_id_clone,
                            last_seq,
                            mls_group_id.as_deref(),
                            username.as_deref(),
                            &data_dir,
                        )
                        .await
                    },
                    Message::MessagesFetched,
                )
            }
            SseUpdate::Welcome => self.accept_welcomes(),
            SseUpdate::GroupUpdate => {
                // Reload rooms
                let server_url = self.server_url.clone().unwrap_or_default();
                let accept_invalid_certs = self.config.accept_invalid_certs;
                let token = self.token.clone().unwrap_or_default();
                Task::perform(
                    async move {
                        let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                        api.set_token(token);
                        load_rooms_async(&api).await
                    },
                    Message::RoomsLoaded,
                )
            }
            SseUpdate::MemberRemoved { group_id, username } => {
                let is_self = self.username.as_deref() == Some(&username);
                if is_self {
                    // We were removed
                    let room_name = self
                        .rooms
                        .get(&group_id)
                        .map(|r| r.name.clone())
                        .unwrap_or_else(|| group_id.clone());
                    self.group_mapping.remove(&group_id);
                    save_group_mapping(&self.config.data_dir, &self.group_mapping);
                    self.rooms.remove(&group_id);
                    if self.active_room.as_deref() == Some(&group_id) {
                        self.active_room = None;
                    }
                    self.push_system_message(&format!("You were removed from #{room_name}"));
                } else {
                    if let Some(room) = self.rooms.get_mut(&group_id) {
                        room.members.retain(|m| m != &username);
                    }
                    self.add_message(
                        Some(&group_id),
                        DisplayMessage::system(&format!("{username} was removed from the group")),
                    );
                }
                Task::none()
            }
        }
    }

    // ── Rooms ─────────────────────────────────────────────────────

    fn handle_group_created(&mut self, result: Result<GroupCreatedInfo, String>) -> Task<Message> {
        match result {
            Ok(info) => {
                // Update group mapping
                self.group_mapping
                    .insert(info.server_group_id.clone(), info.mls_group_id);
                save_group_mapping(&self.config.data_dir, &self.group_mapping);

                // Auto-switch to the new room
                self.active_room = Some(info.server_group_id.clone());

                for msg in info.messages {
                    self.add_message(None, msg);
                }

                // Reload rooms
                let server_url = self.server_url.clone().unwrap_or_default();
                let accept_invalid_certs = self.config.accept_invalid_certs;
                let token = self.token.clone().unwrap_or_default();
                Task::perform(
                    async move {
                        let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                        api.set_token(token);
                        load_rooms_async(&api).await
                    },
                    Message::RoomsLoaded,
                )
            }
            Err(e) => {
                self.push_system_message(&format!("Failed to create group: {e}"));
                Task::none()
            }
        }
    }

    fn handle_rooms_loaded(&mut self, result: Result<Vec<RoomInfo>, String>) -> Task<Message> {
        match result {
            Ok(room_infos) => {
                for info in room_infos {
                    let (existing_seq, existing_read) = self
                        .rooms
                        .get(&info.group_id)
                        .map(|r| (r.last_seen_seq, r.last_read_seq))
                        .unwrap_or((0, 0));

                    // Restore persisted seq values from store
                    let (seq, read) = if let Some(store) = &self.msg_store {
                        let s = store.get_last_seen_seq(&info.group_id);
                        let r = store.get_last_read_seq(&info.group_id);
                        (s.max(existing_seq), r.max(existing_read))
                    } else {
                        (existing_seq, existing_read)
                    };

                    self.rooms.insert(
                        info.group_id.clone(),
                        Room {
                            server_group_id: info.group_id.clone(),
                            name: info.name,
                            members: info.members,
                            last_seen_seq: seq,
                            last_read_seq: read,
                        },
                    );

                    // Load persisted messages from store
                    if let Some(store) = &self.msg_store {
                        if !self.room_messages.contains_key(&info.group_id) {
                            let history = store.load_messages(&info.group_id);
                            if !history.is_empty() {
                                self.room_messages.insert(info.group_id, history);
                            }
                        }
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

        if was_loaded || self.rooms.is_empty() {
            return Task::none();
        }

        // Fetch missed messages for all rooms
        let tasks: Vec<_> = self
            .rooms
            .keys()
            .filter_map(|group_id| {
                let mls_group_id = self.group_mapping.get(group_id)?;
                let server_url = self.server_url.clone().unwrap_or_default();
                let accept_invalid_certs = self.config.accept_invalid_certs;
                let token = self.token.clone().unwrap_or_default();
                let last_seq = self
                    .rooms
                    .get(group_id)
                    .map(|r| r.last_seen_seq)
                    .unwrap_or(0);
                let mls_group_id = Some(mls_group_id.clone());
                let username = self.username.clone();
                let data_dir = self.config.data_dir.clone();
                let group_id = group_id.clone();

                Some(Task::perform(
                    async move {
                        let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                        api.set_token(token);
                        fetch_and_decrypt(
                            &api,
                            &group_id,
                            last_seq,
                            mls_group_id.as_deref(),
                            username.as_deref(),
                            &data_dir,
                        )
                        .await
                    },
                    Message::MessagesFetched,
                ))
            })
            .collect();

        if tasks.is_empty() {
            Task::none()
        } else {
            Task::batch(tasks)
        }
    }

    fn handle_messages_fetched(
        &mut self,
        result: Result<FetchedMessages, String>,
    ) -> Task<Message> {
        match result {
            Ok(fetched) => {
                for msg in &fetched.messages {
                    let display = if msg.is_system {
                        DisplayMessage::system(&msg.content)
                    } else {
                        DisplayMessage::user(&msg.sender, &msg.content, msg.timestamp)
                    };

                    self.add_message(Some(&fetched.group_id), display);

                    // Update last_seen_seq
                    if let Some(room) = self.rooms.get_mut(&fetched.group_id) {
                        room.last_seen_seq = room.last_seen_seq.max(msg.sequence_num);
                        if let Some(store) = &self.msg_store {
                            store.set_last_seen_seq(&fetched.group_id, room.last_seen_seq);
                        }
                    }
                }

                // Mark as read if viewing this room
                if self.active_room.as_deref() == Some(fetched.group_id.as_str()) {
                    if let Some(room) = self.rooms.get_mut(&fetched.group_id) {
                        room.last_read_seq = room.last_seen_seq;
                        if let Some(store) = &self.msg_store {
                            store.set_last_read_seq(&fetched.group_id, room.last_read_seq);
                        }
                    }
                }
            }
            Err(e) => {
                self.push_system_message(&format!("Failed to fetch messages: {e}"));
            }
        }
        Task::none()
    }

    fn handle_message_sent(&mut self, result: Result<MessageSentInfo, String>) -> Task<Message> {
        match result {
            Ok(info) => {
                let sender = self.username.clone().unwrap_or_default();
                let msg =
                    DisplayMessage::user(&sender, &info.text, chrono::Local::now().timestamp());
                self.add_message(Some(&info.group_id), msg);

                if let Some(room) = self.rooms.get_mut(&info.group_id) {
                    room.last_seen_seq = room.last_seen_seq.max(info.sequence_num);
                    room.last_read_seq = room.last_seen_seq;
                    if let Some(store) = &self.msg_store {
                        store.set_last_seen_seq(&info.group_id, room.last_seen_seq);
                        store.set_last_read_seq(&info.group_id, room.last_read_seq);
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
        result: Result<Vec<WelcomeResult>, String>,
    ) -> Task<Message> {
        match result {
            Ok(welcomes) => {
                for w in &welcomes {
                    self.group_mapping
                        .insert(w.group_id.clone(), w.mls_group_id.clone());
                    self.push_system_message(&format!("Joined #{} ({})", w.group_name, w.group_id));
                }
                save_group_mapping(&self.config.data_dir, &self.group_mapping);

                // Auto-switch to last joined room
                if let Some(last) = welcomes.last() {
                    self.active_room = Some(last.group_id.clone());
                }

                // Reload rooms
                let server_url = self.server_url.clone().unwrap_or_default();
                let accept_invalid_certs = self.config.accept_invalid_certs;
                let token = self.token.clone().unwrap_or_default();
                Task::perform(
                    async move {
                        let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                        api.set_token(token);
                        load_rooms_async(&api).await
                    },
                    Message::RoomsLoaded,
                )
            }
            Err(e) => {
                self.push_system_message(&format!("Failed to process welcomes: {e}"));
                Task::none()
            }
        }
    }

    // ── Message sending ───────────────────────────────────────────

    fn send_message(&mut self, text: String) -> Task<Message> {
        let group_id = match &self.active_room {
            Some(id) => id.clone(),
            None => {
                self.push_system_message("No active room — use /join first");
                return Task::none();
            }
        };

        self.send_to_group(&group_id, &text)
    }

    fn send_to_room(&mut self, room: &str, text: &str) -> Task<Message> {
        // Find room by name or ID
        let group_id = if let Some(r) = self.find_room_by_name(room) {
            r.server_group_id.clone()
        } else if self.rooms.contains_key(room) {
            room.to_string()
        } else {
            self.push_system_message(&format!("Unknown room '{room}'"));
            return Task::none();
        };

        self.send_to_group(&group_id, text)
    }

    fn send_to_group(&mut self, group_id: &str, text: &str) -> Task<Message> {
        let mls_group_id = match self.group_mapping.get(group_id) {
            Some(id) => id.clone(),
            None => {
                self.push_system_message("Group mapping not found");
                return Task::none();
            }
        };

        let data_dir = self.config.data_dir.clone();
        let username = self.username.clone().unwrap_or_default();
        let server_url = self.server_url.clone().unwrap_or_default();
        let accept_invalid_certs = self.config.accept_invalid_certs;
        let token = self.token.clone().unwrap_or_default();
        let group_id = group_id.to_string();
        let text = text.to_string();

        Task::perform(
            async move {
                let encrypted = tokio::task::spawn_blocking({
                    let data_dir = data_dir.clone();
                    let username = username.clone();
                    let mls_group_id = mls_group_id.clone();
                    let text = text.clone();
                    move || {
                        let mls =
                            MlsManager::new(&data_dir, &username).map_err(|e| e.to_string())?;
                        mls.encrypt_message(&mls_group_id, text.as_bytes())
                            .map_err(|e| e.to_string())
                    }
                })
                .await
                .map_err(|e| e.to_string())??;

                let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                api.set_token(token);
                let resp = api
                    .send_message(&group_id, encrypted)
                    .await
                    .map_err(|e| e.to_string())?;

                Ok(MessageSentInfo {
                    group_id,
                    sequence_num: resp.sequence_num,
                    text,
                })
            },
            Message::MessageSent,
        )
    }

    // ── Group operations ──────────────────────────────────────────

    fn create_group(&mut self, name: String, members: Vec<String>) -> Task<Message> {
        let server_url = self.server_url.clone().unwrap_or_default();
        let accept_invalid_certs = self.config.accept_invalid_certs;
        let token = self.token.clone().unwrap_or_default();
        let data_dir = self.config.data_dir.clone();
        let username = self.username.clone().unwrap_or_default();

        Task::perform(
            async move {
                let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                api.set_token(token.clone());

                let resp = api
                    .create_group(&name, members)
                    .await
                    .map_err(|e| e.to_string())?;
                let server_group_id = resp.group_id.clone();

                let (mls_group_id, commit_bytes, welcome_map, group_info_bytes) =
                    tokio::task::spawn_blocking({
                        let data_dir = data_dir.clone();
                        let username = username.clone();
                        let member_kps = resp.member_key_packages.clone();
                        move || {
                            let mls =
                                MlsManager::new(&data_dir, &username).map_err(|e| e.to_string())?;
                            mls.create_group(&member_kps).map_err(|e| e.to_string())
                        }
                    })
                    .await
                    .map_err(|e| e.to_string())??;

                api.upload_commit(
                    &server_group_id,
                    commit_bytes,
                    welcome_map,
                    group_info_bytes,
                )
                .await
                .map_err(|e| e.to_string())?;

                Ok(GroupCreatedInfo {
                    server_group_id: server_group_id.clone(),
                    mls_group_id,
                    messages: vec![DisplayMessage::system(&format!(
                        "Created and joined #{name} ({server_group_id})"
                    ))],
                })
            },
            Message::GroupCreated,
        )
    }

    fn invite_members(&mut self, members: Vec<String>) -> Task<Message> {
        let group_id = match &self.active_room {
            Some(id) => id.clone(),
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

        let server_url = self.server_url.clone().unwrap_or_default();
        let accept_invalid_certs = self.config.accept_invalid_certs;
        let token = self.token.clone().unwrap_or_default();
        let data_dir = self.config.data_dir.clone();
        let username = self.username.clone().unwrap_or_default();

        Task::perform(
            async move {
                let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                api.set_token(token);

                let resp = api
                    .invite_to_group(&group_id, members)
                    .await
                    .map_err(|e| e.to_string())?;

                if resp.member_key_packages.is_empty() {
                    return Ok(vec![DisplayMessage::system("No new members to invite.")]);
                }

                let invited: Vec<String> = resp.member_key_packages.keys().cloned().collect();

                let (commit_bytes, welcome_map, group_info_bytes) = tokio::task::spawn_blocking({
                    let data_dir = data_dir.clone();
                    let username = username.clone();
                    let mls_group_id = mls_group_id.clone();
                    let member_kps = resp.member_key_packages.clone();
                    move || {
                        let mls =
                            MlsManager::new(&data_dir, &username).map_err(|e| e.to_string())?;
                        mls.invite_to_group(&mls_group_id, &member_kps)
                            .map_err(|e| e.to_string())
                    }
                })
                .await
                .map_err(|e| e.to_string())??;

                api.upload_commit(&group_id, commit_bytes, welcome_map, group_info_bytes)
                    .await
                    .map_err(|e| e.to_string())?;

                Ok(vec![DisplayMessage::system(&format!(
                    "Invited {} to the room",
                    invited.join(", ")
                ))])
            },
            Message::RefreshRooms,
        )
    }

    fn kick_member(&mut self, target: String) -> Task<Message> {
        let group_id = match &self.active_room {
            Some(id) => id.clone(),
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

        let server_url = self.server_url.clone().unwrap_or_default();
        let accept_invalid_certs = self.config.accept_invalid_certs;
        let token = self.token.clone().unwrap_or_default();
        let data_dir = self.config.data_dir.clone();
        let username = self.username.clone().unwrap_or_default();

        Task::perform(
            async move {
                let mls = tokio::task::spawn_blocking({
                    let data_dir = data_dir.clone();
                    let username = username.clone();
                    let mls_group_id = mls_group_id.clone();
                    let target = target.clone();
                    move || {
                        let mls =
                            MlsManager::new(&data_dir, &username).map_err(|e| e.to_string())?;
                        let member_index = mls
                            .find_member_index(&mls_group_id, &target)
                            .map_err(|e| e.to_string())?
                            .ok_or_else(|| format!("user '{target}' not found in MLS roster"))?;
                        let (commit_bytes, group_info_bytes) = mls
                            .remove_member(&mls_group_id, member_index)
                            .map_err(|e| e.to_string())?;
                        Ok::<_, String>((commit_bytes, group_info_bytes))
                    }
                })
                .await
                .map_err(|e| e.to_string())??;

                let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                api.set_token(token);
                api.remove_member(&group_id, &target, mls.0, mls.1)
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
        let group_id = match &self.active_room {
            Some(id) => id.clone(),
            None => {
                self.push_system_message("No active room — use /join first");
                return Task::none();
            }
        };

        let room_name = self
            .rooms
            .get(&group_id)
            .map(|r| r.name.clone())
            .unwrap_or_default();

        let server_url = self.server_url.clone().unwrap_or_default();
        let accept_invalid_certs = self.config.accept_invalid_certs;
        let token = self.token.clone().unwrap_or_default();

        // Clean up local state immediately
        if let Some(mls_group_id) = self.group_mapping.get(&group_id).cloned() {
            if let Some(mls) = &self.mls {
                let _ = mls.delete_group_state(&mls_group_id);
            }
        }
        self.group_mapping.remove(&group_id);
        save_group_mapping(&self.config.data_dir, &self.group_mapping);
        self.rooms.remove(&group_id);
        self.active_room = None;
        self.push_system_message(&format!("Left #{room_name}"));

        Task::perform(
            async move {
                let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                api.set_token(token);
                api.leave_group(&group_id)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(vec![])
            },
            Message::CommandResult,
        )
    }

    fn rotate_keys(&mut self) -> Task<Message> {
        let group_id = match &self.active_room {
            Some(id) => id.clone(),
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

        let server_url = self.server_url.clone().unwrap_or_default();
        let accept_invalid_certs = self.config.accept_invalid_certs;
        let token = self.token.clone().unwrap_or_default();
        let data_dir = self.config.data_dir.clone();
        let username = self.username.clone().unwrap_or_default();

        Task::perform(
            async move {
                let (commit_bytes, group_info_bytes) = tokio::task::spawn_blocking(move || {
                    let mls = MlsManager::new(&data_dir, &username).map_err(|e| e.to_string())?;
                    mls.rotate_keys(&mls_group_id).map_err(|e| e.to_string())
                })
                .await
                .map_err(|e| e.to_string())??;

                let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                api.set_token(token);
                api.upload_commit(&group_id, commit_bytes, HashMap::new(), group_info_bytes)
                    .await
                    .map_err(|e| e.to_string())?;

                Ok(vec![DisplayMessage::system(
                    "Keys rotated. Forward secrecy updated.",
                )])
            },
            Message::CommandResult,
        )
    }

    fn accept_welcomes(&mut self) -> Task<Message> {
        let server_url = self.server_url.clone().unwrap_or_default();
        let accept_invalid_certs = self.config.accept_invalid_certs;
        let token = self.token.clone().unwrap_or_default();
        let data_dir = self.config.data_dir.clone();
        let username = self.username.clone().unwrap_or_default();

        Task::perform(
            async move {
                let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                api.set_token(token);
                let resp = api
                    .list_pending_welcomes()
                    .await
                    .map_err(|e| e.to_string())?;

                if resp.welcomes.is_empty() {
                    return Ok(vec![]);
                }

                let mut results = Vec::new();
                for welcome in &resp.welcomes {
                    let data_dir = data_dir.clone();
                    let username = username.clone();
                    let welcome_bytes = welcome.welcome_message.clone();
                    let group_id = welcome.group_id.clone();
                    let group_name = welcome.group_name.clone();

                    let mls_group_id = tokio::task::spawn_blocking(move || {
                        let mls =
                            MlsManager::new(&data_dir, &username).map_err(|e| e.to_string())?;
                        mls.join_group(&welcome_bytes).map_err(|e| e.to_string())
                    })
                    .await
                    .map_err(|e| e.to_string())??;

                    results.push(WelcomeResult {
                        group_id,
                        group_name,
                        mls_group_id,
                    });
                }

                // Key packages are single-use (RFC 9420 §10); upload a fresh
                // replacement so we remain available for future group invitations.
                let kp = tokio::task::spawn_blocking({
                    let data_dir = data_dir.clone();
                    let username = username.clone();
                    move || {
                        let mls =
                            MlsManager::new(&data_dir, &username).map_err(|e| e.to_string())?;
                        mls.generate_key_package().map_err(|e| e.to_string())
                    }
                })
                .await
                .map_err(|e| e.to_string())??;
                api.upload_key_packages(vec![(kp, false)])
                    .await
                    .map_err(|e| e.to_string())?;

                Ok(results)
            },
            Message::WelcomesProcessed,
        )
    }

    // ── Helpers ───────────────────────────────────────────────────

    fn push_system_message(&mut self, content: &str) {
        let msg = DisplayMessage::system(content);
        self.add_message(None, msg);
    }

    fn add_message(&mut self, group_id: Option<&str>, msg: DisplayMessage) {
        let effective_gid = group_id
            .map(|s| s.to_string())
            .or_else(|| self.active_room.clone());

        // Persist to store
        if let (Some(gid), Some(store)) = (&effective_gid, &self.msg_store) {
            if group_id.is_some() {
                store.push_message(gid, &msg);
            }
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

    fn switch_to_room(&mut self, target: &str) {
        let resolved_gid = if let Some(room) = self.find_room_by_name(target) {
            Some(room.server_group_id.clone())
        } else if self.rooms.contains_key(target) {
            Some(target.to_string())
        } else {
            None
        };

        if let Some(gid) = resolved_gid {
            let name = self.rooms[&gid].name.clone();
            self.active_room = Some(gid.clone());

            // Mark as read
            if let Some(room) = self.rooms.get_mut(&gid) {
                room.last_read_seq = room.last_seen_seq;
                if let Some(store) = &self.msg_store {
                    store.set_last_read_seq(&gid, room.last_read_seq);
                }
            }

            self.push_system_message(&format!("Switched to #{name}"));
        } else {
            self.push_system_message(&format!(
                "Unknown room '{target}'. Use /rooms to list available rooms."
            ));
        }
    }

    fn find_room_by_name(&self, name: &str) -> Option<&Room> {
        let lower = name.to_lowercase();
        // Exact match
        if let Some(room) = self.rooms.values().find(|r| r.name.to_lowercase() == lower) {
            return Some(room);
        }
        // Prefix match
        let matches: Vec<_> = self
            .rooms
            .values()
            .filter(|r| r.name.to_lowercase().starts_with(&lower))
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
            "/leave                        Leave the room",
            "/part                         Switch away without leaving",
            "/rotate                       Rotate keys (forward secrecy)",
            "/keygen                       Generate and upload a key package",
            "/info                         Show MLS group details",
            "/rooms                        List your rooms",
            "/unread                       Check rooms for new messages",
            "/who                          List members of active room",
            "/msg <room> <text>            Send to a room without switching",
            "/me                           Show current user info",
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
        let group_id = match &self.active_room {
            Some(id) => id.clone(),
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
            .map(|r| r.name.as_str())
            .unwrap_or("unknown");

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
                    for (index, name) in &details.members {
                        let marker = if *index == details.own_index {
                            " (you)"
                        } else {
                            ""
                        };
                        self.push_system_message(&format!("    [{index}] {name}{marker}"));
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
                        room.name,
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

    fn perform_logout(&mut self) {
        // Clear API token
        self.token = None;
        self.api = None;
        self.mls = None;
        self.username = None;
        self.user_id = None;
        self.active_room = None;
        self.rooms.clear();
        self.group_mapping.clear();
        self.room_messages.clear();
        self.system_messages.clear();
        self.msg_store = None;
        self.rooms_loaded = false;
        self.connection_status = ConnectionStatus::Disconnected;

        // Delete session file
        let session_path = self.config.data_dir.join("session.toml");
        let _ = std::fs::remove_file(session_path);

        // Go back to login screen
        self.screen = screen::Screen::Login(screen::Login::new(
            self.server_url.clone().unwrap_or_default(),
        ));
    }
}

// ── Free functions ────────────────────────────────────────────────

async fn load_rooms_async(api: &ApiClient) -> Result<Vec<RoomInfo>, String> {
    let resp = api.list_groups().await.map_err(|e| e.to_string())?;
    Ok(resp
        .groups
        .into_iter()
        .map(|g| RoomInfo {
            group_id: g.group_id,
            name: g.name,
            members: g.members.into_iter().map(|m| m.username).collect(),
        })
        .collect())
}

async fn fetch_and_decrypt(
    api: &ApiClient,
    group_id: &str,
    last_seq: u64,
    mls_group_id: Option<&str>,
    username: Option<&str>,
    data_dir: &std::path::Path,
) -> Result<FetchedMessages, String> {
    let mls_group_id = mls_group_id
        .ok_or_else(|| "group mapping not found".to_string())?
        .to_string();
    let username = username
        .ok_or_else(|| "not logged in".to_string())?
        .to_string();

    let resp = api
        .get_messages(group_id, last_seq as i64)
        .await
        .map_err(|e| e.to_string())?;

    let mut messages = Vec::new();

    for stored_msg in &resp.messages {
        let data_dir = data_dir.to_path_buf();
        let username = username.clone();
        let mls_group_id = mls_group_id.clone();
        let mls_bytes = stored_msg.mls_message.clone();

        let decrypted = match tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir, &username).map_err(|e| e.to_string())?;
            mls.decrypt_message(&mls_group_id, &mls_bytes)
                .map_err(|e| e.to_string())
        })
        .await
        {
            Ok(Ok(d)) => d,
            _ => continue,
        };

        match decrypted {
            conclave_lib::mls::DecryptedMessage::Application(plaintext) => {
                let text = String::from_utf8_lossy(&plaintext).to_string();
                messages.push(DecryptedMsg {
                    sender: stored_msg.sender_username.clone(),
                    content: text,
                    timestamp: stored_msg.created_at as i64,
                    sequence_num: stored_msg.sequence_num,
                    is_system: false,
                });
            }
            conclave_lib::mls::DecryptedMessage::Commit(commit_info) => {
                for added in &commit_info.members_added {
                    messages.push(DecryptedMsg {
                        sender: String::new(),
                        content: format!("{added} joined the group"),
                        timestamp: stored_msg.created_at as i64,
                        sequence_num: stored_msg.sequence_num,
                        is_system: true,
                    });
                }
                for removed in &commit_info.members_removed {
                    messages.push(DecryptedMsg {
                        sender: String::new(),
                        content: format!("{removed} was removed from the group"),
                        timestamp: stored_msg.created_at as i64,
                        sequence_num: stored_msg.sequence_num,
                        is_system: true,
                    });
                }
                if commit_info.members_added.is_empty()
                    && commit_info.members_removed.is_empty()
                    && !commit_info.self_removed
                {
                    messages.push(DecryptedMsg {
                        sender: String::new(),
                        content: "Group keys updated".to_string(),
                        timestamp: stored_msg.created_at as i64,
                        sequence_num: stored_msg.sequence_num,
                        is_system: true,
                    });
                }
            }
            conclave_lib::mls::DecryptedMessage::None => {}
        }
    }

    Ok(FetchedMessages {
        group_id: group_id.to_string(),
        messages,
    })
}

/// Generate initial key packages: 1 last-resort + 5 regular.
fn generate_initial_key_packages(mls: &MlsManager) -> Result<Vec<(Vec<u8>, bool)>, String> {
    let mut entries = Vec::with_capacity(6);
    let last_resort = mls
        .generate_last_resort_key_package()
        .map_err(|e| e.to_string())?;
    entries.push((last_resort, true));
    for kp in mls.generate_key_packages(5).map_err(|e| e.to_string())? {
        entries.push((kp, false));
    }
    Ok(entries)
}

fn load_group_mapping(data_dir: &std::path::Path) -> HashMap<String, String> {
    let path = data_dir.join("group_mapping.toml");
    if path.exists() {
        let contents = std::fs::read_to_string(&path).unwrap_or_default();
        toml::from_str(&contents).unwrap_or_default()
    } else {
        HashMap::new()
    }
}

fn save_group_mapping(data_dir: &std::path::Path, mapping: &HashMap<String, String>) {
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
