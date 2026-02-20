use iced::widget::operation::{focus, focus_next};
use iced::{Subscription, Task, keyboard};
use std::collections::{HashMap, HashSet};

use conclave_lib::api::{normalize_server_url, ApiClient};
use conclave_lib::command::Command;
use conclave_lib::config::{
    ClientConfig, SessionState, generate_initial_key_packages, load_group_mapping,
    save_group_mapping,
};
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
    welcomes_processed: bool,
    /// Groups currently being fetched — prevents duplicate concurrent fetches.
    fetching_groups: HashSet<String>,
    /// Set when welcome processing triggers a rooms reload — defers the
    /// missed-message fetch until the rooms are actually in `self.rooms`.
    fetch_messages_on_rooms_load: bool,
    window_focused: bool,
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
    MessagesFetched(Result<FetchedMessages, (String, String)>),
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
    /// Account reset completed — update group mapping and MLS state.
    ResetComplete(Result<ResetCompleteInfo, String>),
    /// Tab key pressed (for login field navigation).
    TabPressed,
    /// Quit the application (e.g. Ctrl+Q).
    Quit,
    WindowFocused,
    WindowUnfocused,
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

#[derive(Debug, Clone)]
pub struct ResetCompleteInfo {
    pub new_group_mapping: HashMap<String, String>,
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
            welcomes_processed: false,
            fetching_groups: HashSet::new(),
            fetch_messages_on_rooms_load: false,
            window_focused: true,
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
            app.system_messages = vec![DisplayMessage::system(&format!(
                "Welcome back, {username}. Type /help for commands."
            ))];

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
                        generate_initial_key_packages(&mls).map_err(|e| e.to_string())
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

            let welcome_task = app.accept_welcomes();

            return (app, Task::batch([keygen_task, rooms_task, welcome_task]));
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
            Message::WindowFocused => {
                self.window_focused = true;
                Task::none()
            }
            Message::WindowUnfocused => {
                self.window_focused = false;
                Task::none()
            }
            Message::ResetComplete(result) => self.handle_reset_complete(result),
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
                    &self.server_url,
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
            iced::Event::Window(iced::window::Event::Focused) => Some(Message::WindowFocused),
            iced::Event::Window(iced::window::Event::Unfocused) => Some(Message::WindowUnfocused),
            _ => None,
        });

        if let (Some(token), true) = (&self.token, self.rooms_loaded) {
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
                            generate_initial_key_packages(&mls).map_err(|e| e.to_string())
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
            Ok(Command::Logout) => self.perform_logout(),
            Ok(Command::Create { name, members }) => self.create_group(name, members),
            Ok(Command::Invite { members }) => self.invite_members(members),
            Ok(Command::Kick { username }) => self.kick_member(username),
            Ok(Command::Leave) => self.leave_group(),
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

                // Accept any pending welcomes (invites received while
                // disconnected) and fetch missed messages for all rooms.
                let welcome_task = self.accept_welcomes();
                let fetch_task = self.fetch_all_missed_messages();
                Task::batch([welcome_task, fetch_task])
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
                // Skip if we're already fetching this group to prevent
                // concurrent MLS operations on the same group state.
                if self.fetching_groups.contains(&group_id) {
                    return Task::none();
                }
                self.fetching_groups.insert(group_id.clone());

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
                        match fetch_and_decrypt(
                            &api,
                            &group_id_clone,
                            last_seq,
                            mls_group_id.as_deref(),
                            username.as_deref(),
                            &data_dir,
                        )
                        .await
                        {
                            Ok(fetched) => Ok(fetched),
                            Err(error) => Err((group_id_clone, error)),
                        }
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
                    // We were removed — delete MLS group state
                    let room_name = self
                        .rooms
                        .get(&group_id)
                        .map(|r| r.name.clone())
                        .unwrap_or_else(|| group_id.clone());
                    let mls_group_id = self.group_mapping.get(&group_id).cloned();

                    self.group_mapping.remove(&group_id);
                    save_group_mapping(&self.config.data_dir, &self.group_mapping);
                    self.rooms.remove(&group_id);
                    if self.active_room.as_deref() == Some(&group_id) {
                        self.active_room = None;
                    }
                    self.push_system_message(&format!("You were removed from #{room_name}"));

                    if let Some(mls_group_id) = mls_group_id {
                        let data_dir = self.config.data_dir.clone();
                        let our_username = self.username.clone().unwrap_or_default();
                        Task::perform(
                            async move {
                                match tokio::task::spawn_blocking(move || {
                                    let mls = MlsManager::new(&data_dir, &our_username)?;
                                    mls.delete_group_state(&mls_group_id)
                                })
                                .await
                                {
                                    Ok(Err(error)) => {
                                        tracing::warn!(%error, "failed to delete MLS group state");
                                    }
                                    Err(error) => {
                                        tracing::warn!(%error, "MLS group state deletion task panicked");
                                    }
                                    Ok(Ok(())) => {}
                                }
                                Ok::<_, String>(vec![])
                            },
                            Message::CommandResult,
                        )
                    } else {
                        Task::none()
                    }
                } else {
                    // Someone else was removed — fetch the leave commit so our
                    // MLS state advances the epoch.
                    if let Some(room) = self.rooms.get_mut(&group_id) {
                        room.members.retain(|m| m != &username);
                    }
                    self.add_message(
                        Some(&group_id),
                        DisplayMessage::system(&format!("{username} was removed from the group")),
                    );

                    // Trigger a message fetch to process the MLS removal commit
                    self.handle_sse_event(SseUpdate::NewMessage { group_id })
                }
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
        result: Result<FetchedMessages, (String, String)>,
    ) -> Task<Message> {
        match result {
            Ok(fetched) => {
                // Allow new fetches for this group now that we're done.
                self.fetching_groups.remove(&fetched.group_id);

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

                if !self.window_focused {
                    let last_msg = fetched
                        .messages
                        .iter()
                        .rev()
                        .find(|m| !m.is_system);
                    if let Some(msg) = last_msg {
                        let room_name = self
                            .rooms
                            .get(&fetched.group_id)
                            .map(|r| r.name.as_str())
                            .unwrap_or("unknown");
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
        let was_processed = self.welcomes_processed;
        self.welcomes_processed = true;

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

                // Reload rooms (picks up newly joined groups).
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

                // Defer the missed-message fetch until rooms_task completes
                // so that newly joined groups are in self.rooms when
                // fetch_all_missed_messages iterates over them.
                if !was_processed {
                    self.fetch_messages_on_rooms_load = true;
                }
                rooms_task
            }
            Err(e) => {
                self.push_system_message(&format!("Failed to process welcomes: {e}"));

                // Even on error, the initial fetch should proceed so
                // rooms that already have mappings get their messages.
                if !was_processed {
                    self.fetch_messages_on_rooms_load = true;
                    let server_url = self.server_url.clone().unwrap_or_default();
                    let accept_invalid_certs = self.config.accept_invalid_certs;
                    let token = self.token.clone().unwrap_or_default();
                    return Task::perform(
                        async move {
                            let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                            api.set_token(token);
                            load_rooms_async(&api).await
                        },
                        Message::RoomsLoaded,
                    );
                }
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

        let mls_group_id = self.group_mapping.get(&group_id).cloned();

        let room_name = self
            .rooms
            .get(&group_id)
            .map(|r| r.name.clone())
            .unwrap_or_default();

        let server_url = self.server_url.clone().unwrap_or_default();
        let accept_invalid_certs = self.config.accept_invalid_certs;
        let token = self.token.clone().unwrap_or_default();
        let data_dir = self.config.data_dir.clone();
        let username = self.username.clone().unwrap_or_default();

        // Clean up local UI state immediately
        self.group_mapping.remove(&group_id);
        save_group_mapping(&self.config.data_dir, &self.group_mapping);
        self.rooms.remove(&group_id);
        self.active_room = None;
        self.push_system_message(&format!("Left #{room_name}"));

        Task::perform(
            async move {
                // Produce an MLS self-remove commit so remaining members can
                // advance their epoch and exclude us from future messages.
                // This must happen BEFORE deleting the MLS group state.
                let (commit_bytes, group_info_bytes) = if let Some(mls_gid) = &mls_group_id {
                    match tokio::task::spawn_blocking({
                        let data_dir = data_dir.clone();
                        let username = username.clone();
                        let mls_gid = mls_gid.clone();
                        move || {
                            let mls =
                                MlsManager::new(&data_dir, &username).map_err(|e| e.to_string())?;
                            mls.leave_group(&mls_gid).map_err(|e| e.to_string())
                        }
                    })
                    .await
                    {
                        Ok(Ok(Some(data))) => data,
                        _ => (Vec::new(), Vec::new()),
                    }
                } else {
                    (Vec::new(), Vec::new())
                };

                // Delete MLS group state after commit is produced
                if let Some(mls_gid) = &mls_group_id {
                    match tokio::task::spawn_blocking({
                        let data_dir = data_dir.clone();
                        let username = username.clone();
                        let mls_gid = mls_gid.clone();
                        move || {
                            let mls = MlsManager::new(&data_dir, &username)?;
                            mls.delete_group_state(&mls_gid)
                        }
                    })
                    .await
                    {
                        Ok(Err(error)) => {
                            tracing::warn!(%error, "failed to delete MLS group state");
                        }
                        Err(error) => {
                            tracing::warn!(%error, "MLS group state deletion task panicked");
                        }
                        Ok(Ok(())) => {}
                    }
                }

                let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                api.set_token(token);
                api.leave_group(&group_id, commit_bytes, group_info_bytes)
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

    fn reset_account(&mut self) -> Task<Message> {
        let username = match &self.username {
            Some(username) => username.clone(),
            None => {
                self.push_system_message("Not logged in.");
                return Task::none();
            }
        };

        let server_url = self.server_url.clone().unwrap_or_default();
        let accept_invalid_certs = self.config.accept_invalid_certs;
        let token = self.token.clone().unwrap_or_default();
        let data_dir = self.config.data_dir.clone();
        let group_mapping = self.group_mapping.clone();

        self.push_system_message("Resetting account...");

        Task::perform(
            async move {
                let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                api.set_token(token);

                // Step 1: Collect groups and find old leaf indices before wiping
                let groups_to_rejoin: Vec<(String, String)> = group_mapping
                    .iter()
                    .map(|(server_id, mls_id)| (server_id.clone(), mls_id.clone()))
                    .collect();

                let old_indices: HashMap<String, Option<u32>> = tokio::task::spawn_blocking({
                    let data_dir = data_dir.clone();
                    let username = username.clone();
                    let groups = groups_to_rejoin.clone();
                    move || {
                        let mls =
                            MlsManager::new(&data_dir, &username).map_err(|e| e.to_string())?;
                        let mut indices = HashMap::new();
                        for (server_id, mls_id) in &groups {
                            let index =
                                mls.find_member_index(mls_id, &username).ok().flatten();
                            indices.insert(server_id.clone(), index);
                        }
                        Ok::<_, String>(indices)
                    }
                })
                .await
                .map_err(|e| e.to_string())??;

                // Step 2: Notify server to clear our key packages
                api.reset_account().await.map_err(|e| e.to_string())?;

                // Step 3: Wipe local MLS state
                tokio::task::spawn_blocking({
                    let data_dir = data_dir.clone();
                    let username = username.clone();
                    move || {
                        let mls =
                            MlsManager::new(&data_dir, &username).map_err(|e| e.to_string())?;
                        mls.wipe_local_state().map_err(|e| e.to_string())
                    }
                })
                .await
                .map_err(|e| e.to_string())??;

                // Step 4: Regenerate identity and upload new key packages
                let entries = tokio::task::spawn_blocking({
                    let data_dir = data_dir.clone();
                    let username = username.clone();
                    move || {
                        let mls =
                            MlsManager::new(&data_dir, &username).map_err(|e| e.to_string())?;
                        generate_initial_key_packages(&mls).map_err(|e| e.to_string())
                    }
                })
                .await
                .map_err(|e| e.to_string())??;

                api.upload_key_packages(entries)
                    .await
                    .map_err(|e| e.to_string())?;

                // Step 5: Rejoin each group via external commit
                let mut new_mapping = HashMap::new();
                let mut messages = Vec::new();
                let mut rejoin_count = 0;

                for (server_id, _) in &groups_to_rejoin {
                    let group_info_response = match api.get_group_info(server_id).await {
                        Ok(response) => response,
                        Err(error) => {
                            messages.push(DisplayMessage::system(&format!(
                                "Failed to get group info for {server_id}: {error}"
                            )));
                            continue;
                        }
                    };

                    let old_index = old_indices.get(server_id).copied().flatten();
                    let group_info_bytes = group_info_response.group_info.clone();

                    let rejoin_result = tokio::task::spawn_blocking({
                        let data_dir = data_dir.clone();
                        let username = username.clone();
                        move || {
                            let mls = MlsManager::new(&data_dir, &username)
                                .map_err(|e| e.to_string())?;
                            mls.external_rejoin_group(&group_info_bytes, old_index)
                                .map_err(|e| e.to_string())
                        }
                    })
                    .await
                    .map_err(|e| e.to_string())?;

                    match rejoin_result {
                        Ok((new_mls_id, commit_bytes)) => {
                            if let Err(error) =
                                api.external_join(server_id, commit_bytes).await
                            {
                                messages.push(DisplayMessage::system(&format!(
                                    "Failed to rejoin {server_id}: {error}"
                                )));
                                continue;
                            }
                            new_mapping.insert(server_id.clone(), new_mls_id);
                            rejoin_count += 1;
                        }
                        Err(error) => {
                            messages.push(DisplayMessage::system(&format!(
                                "Failed external commit for {server_id}: {error}"
                            )));
                        }
                    }
                }

                messages.push(DisplayMessage::system(&format!(
                    "Account reset complete. Rejoined {rejoin_count}/{} groups.",
                    groups_to_rejoin.len()
                )));

                Ok(ResetCompleteInfo {
                    new_group_mapping: new_mapping,
                    messages,
                })
            },
            Message::ResetComplete,
        )
    }

    fn handle_reset_complete(
        &mut self,
        result: Result<ResetCompleteInfo, String>,
    ) -> Task<Message> {
        match result {
            Ok(info) => {
                self.group_mapping = info.new_group_mapping;
                save_group_mapping(&self.config.data_dir, &self.group_mapping);

                // Reinitialize MLS with the new identity
                if let Some(username) = &self.username {
                    if let Ok(mls) = MlsManager::new(&self.config.data_dir, username) {
                        self.mls = Some(mls);
                    }
                }

                // Clear stale fetch state from pre-reset MLS groups
                self.fetching_groups.clear();

                for message in info.messages {
                    self.add_message(None, message);
                }

                // Reload rooms to pick up any membership changes
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
            Err(error) => {
                self.push_system_message(&format!("Reset failed: {error}"));
                Task::none()
            }
        }
    }

    /// Fetch missed messages for all rooms that have a group mapping.
    fn fetch_all_missed_messages(&mut self) -> Task<Message> {
        let tasks: Vec<_> = self
            .rooms
            .keys()
            .filter(|group_id| {
                // Skip groups that are already being fetched to prevent
                // concurrent MLS operations on the same group state.
                self.group_mapping.contains_key(*group_id)
                    && !self.fetching_groups.contains(*group_id)
            })
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|group_id| {
                let mls_group_id = self.group_mapping.get(&group_id)?.clone();
                self.fetching_groups.insert(group_id.clone());
                let server_url = self.server_url.clone().unwrap_or_default();
                let accept_invalid_certs = self.config.accept_invalid_certs;
                let token = self.token.clone().unwrap_or_default();
                let last_seq = self
                    .rooms
                    .get(&group_id)
                    .map(|r| r.last_seen_seq)
                    .unwrap_or(0);
                let mls_group_id = Some(mls_group_id);
                let username = self.username.clone();
                let data_dir = self.config.data_dir.clone();

                Some(Task::perform(
                    async move {
                        let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                        api.set_token(token);
                        match fetch_and_decrypt(
                            &api,
                            &group_id,
                            last_seq,
                            mls_group_id.as_deref(),
                            username.as_deref(),
                            &data_dir,
                        )
                        .await
                        {
                            Ok(fetched) => Ok(fetched),
                            Err(error) => Err((group_id, error)),
                        }
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

                    // Delete the welcome from the server so it is not re-processed.
                    if let Err(error) = api.accept_welcome(welcome.welcome_id).await {
                        tracing::warn!(%error, "failed to acknowledge welcome");
                    }

                    results.push(WelcomeResult {
                        group_id,
                        group_name,
                        mls_group_id,
                    });
                }

                // Key packages are single-use (RFC 9420 §10); replenish one
                // per welcome consumed to maintain 5 regular packages.
                let consumed_count = results.len();
                let replacements = tokio::task::spawn_blocking({
                    let data_dir = data_dir.clone();
                    let username = username.clone();
                    move || {
                        let mls =
                            MlsManager::new(&data_dir, &username).map_err(|e| e.to_string())?;
                        mls.generate_key_packages(consumed_count)
                            .map_err(|e| e.to_string())
                    }
                })
                .await
                .map_err(|e| e.to_string())??;
                let entries: Vec<(Vec<u8>, bool)> =
                    replacements.into_iter().map(|kp| (kp, false)).collect();
                if !entries.is_empty() {
                    api.upload_key_packages(entries)
                        .await
                        .map_err(|e| e.to_string())?;
                }

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
            "/reset                        Reset account and rejoin groups",
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

    fn perform_logout(&mut self) -> Task<Message> {
        // Capture server info before clearing state so we can revoke the
        // server-side token asynchronously.
        let server_url = self.server_url.clone().unwrap_or_default();
        let accept_invalid_certs = self.config.accept_invalid_certs;
        let token = self.token.clone().unwrap_or_default();

        // Clear local state.
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
        self.welcomes_processed = false;
        self.connection_status = ConnectionStatus::Disconnected;

        // Delete session file.
        let session_path = self.config.data_dir.join("session.toml");
        if let Err(error) = std::fs::remove_file(session_path) {
            tracing::warn!(%error, "failed to remove session file");
        }

        // Go back to login screen.
        self.screen = screen::Screen::Login(screen::Login::new(
            self.server_url.clone().unwrap_or_default(),
        ));

        // Revoke the server-side token in the background.
        if !token.is_empty() {
            Task::perform(
                async move {
                    let mut api = ApiClient::new(&server_url, accept_invalid_certs);
                    api.set_token(token);
                    if let Err(error) = api.logout().await {
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
            conclave_lib::mls::DecryptedMessage::Failed(reason) => {
                messages.push(DecryptedMsg {
                    sender: String::new(),
                    content: format!(
                        "Failed to decrypt message (seq {}): {reason}",
                        stored_msg.sequence_num
                    ),
                    timestamp: stored_msg.created_at as i64,
                    sequence_num: stored_msg.sequence_num,
                    is_system: true,
                });
            }
            conclave_lib::mls::DecryptedMessage::None => {}
        }
    }

    Ok(FetchedMessages {
        group_id: group_id.to_string(),
        messages,
    })
}
