use iced::Task;
use iced::widget::operation::focus;

use conclave_lib::api::{ApiClient, normalize_server_url};
use conclave_lib::config::{SessionState, generate_initial_key_packages};
use conclave_lib::mls::MlsManager;
use conclave_lib::state::{ConnectionStatus, DisplayMessage};
use conclave_lib::store::MessageStore;

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

                if username.is_empty() || password.is_empty() {
                    login.status =
                        screen::login::Status::Error("Username and password required".into());
                    return Task::none();
                }

                login.status = screen::login::Status::Loading;

                // Both login and register are async — the result comes back
                // through Message::LoginResult / Message::RegisterResult.
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

                                conclave_lib::operations::initialize_mls_and_upload_key_packages(
                                    &auth_api, &data_dir, user_id,
                                )
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

                // Registration already generated and uploaded key packages,
                // so skip the duplicate keygen on the subsequent login.
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

    pub(crate) fn handle_register_result(
        &mut self,
        result: Result<LoginInfo, String>,
    ) -> Task<Message> {
        match result {
            Ok(info) => {
                // Registration already uploaded key packages, so tell
                // handle_login_result to skip the redundant keygen step.
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

    pub(crate) fn handle_keygen_done(
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
