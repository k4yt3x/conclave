use iced::Task;
use iced::widget::operation::focus;

use conclave_client::api::{ApiClient, normalize_server_url};
use conclave_client::mls::MlsManager;
use conclave_client::state::{ConnectionStatus, DisplayMessage};
use conclave_client::store::MessageStore;

use crate::screen;

use super::{Conclave, LoginInfo, Message};

impl Conclave {
    pub(crate) fn handle_login_message(&mut self, msg: screen::login::Message) -> Task<Message> {
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
                let data_dir = self.config.data_dir.clone();

                if username.is_empty() || password.is_empty() {
                    login.status =
                        screen::login::Status::Error("Username and password required".into());
                    return Task::none();
                }

                login.status = screen::login::Status::Loading;

                match mode {
                    screen::login::Mode::Login => Task::perform(
                        async move {
                            let result = conclave_client::operations::login(
                                &server_url,
                                &username,
                                &password,
                                accept_invalid_certs,
                                &data_dir,
                            )
                            .await
                            .map_err(|e| e.to_string())?;
                            Ok(LoginInfo {
                                server_url: result.server_url,
                                token: result.token,
                                user_id: result.user_id,
                                username: result.username,
                            })
                        },
                        Message::LoginResult,
                    ),
                    screen::login::Mode::Register => Task::perform(
                        async move {
                            let result = conclave_client::operations::register_and_login(
                                &server_url,
                                &username,
                                &password,
                                accept_invalid_certs,
                                &data_dir,
                            )
                            .await
                            .map_err(|e| e.to_string())?;
                            Ok(LoginInfo {
                                server_url: result.server_url,
                                token: result.token,
                                user_id: result.user_id,
                                username: result.username,
                            })
                        },
                        Message::RegisterResult,
                    ),
                }
            }
        }
    }

    pub(crate) fn handle_login_result(
        &mut self,
        result: Result<LoginInfo, String>,
    ) -> Task<Message> {
        match result {
            Ok(info) => {
                self.server_url = Some(normalize_server_url(&info.server_url));

                let mut api = ApiClient::new(&info.server_url, self.config.accept_invalid_certs);
                api.set_token(info.token.clone());
                self.api = Some(api);
                self.username = Some(info.username.clone());
                self.user_id = Some(info.user_id);
                self.token = Some(info.token.clone());

                if let Err(error) = (conclave_client::operations::AuthResult {
                    server_url: info.server_url.clone(),
                    token: info.token.clone(),
                    user_id: info.user_id,
                    username: info.username.clone(),
                    key_packages_uploaded: 0,
                })
                .save_session(&self.config.data_dir)
                {
                    tracing::warn!(%error, "failed to save session");
                }

                if let Err(error) = std::fs::create_dir_all(&self.config.data_dir) {
                    tracing::warn!(%error, "failed to create data directory");
                }
                if let Ok(mls) = MlsManager::new(&self.config.data_dir, info.user_id) {
                    self.mls = Some(mls);
                }

                if let Ok(store) = MessageStore::open(&self.config.data_dir) {
                    self.msg_store = Some(store);
                }

                self.screen = screen::Screen::Dashboard(screen::Dashboard::new());
                self.system_messages = vec![DisplayMessage::system(&format!(
                    "Logged in as {} (ID {}). Type /help for commands.",
                    info.username, info.user_id
                ))];

                let rooms_task = self.load_rooms_task();
                let alias_task = self.fetch_user_alias();
                Task::batch([rooms_task, alias_task])
            }
            Err(e) => {
                if let screen::Screen::Login(login) = &mut self.screen {
                    login.status = screen::login::Status::Error(e);
                }
                Task::none()
            }
        }
    }

    pub(crate) fn handle_register_result(
        &mut self,
        result: Result<LoginInfo, String>,
    ) -> Task<Message> {
        match result {
            Ok(info) => self.handle_login_result(Ok(info)),
            Err(e) => {
                if let screen::Screen::Login(login) = &mut self.screen {
                    login.status = screen::login::Status::Error(e);
                }
                Task::none()
            }
        }
    }

    pub(crate) fn perform_logout(&mut self) -> Task<Message> {
        // Capture before clearing so we can revoke the server-side token.
        let params = self.api_params();

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

        let session_path = self.config.data_dir.join("session.toml");
        if let Err(error) = std::fs::remove_file(session_path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(%error, "failed to remove session file");
        }

        self.screen = screen::Screen::Login(screen::Login::new(
            self.server_url.clone().unwrap_or_default(),
        ));

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
