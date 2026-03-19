mod commands;
mod events;
mod input;
mod render;
mod state;
mod store;

use std::io::Write;
use std::sync::Arc;

use crossterm::{
    cursor,
    event::{
        Event as CtEvent, EventStream, KeyCode, KeyModifiers, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::StreamExt;
use reqwest_eventsource::{Event as EsEvent, EventSource};
use tokio::sync::Mutex;
use uuid::Uuid;

use conclave_client::api::ApiClient;
use conclave_client::config::{
    ClientConfig, SessionState, build_group_mapping, generate_initial_key_packages,
};

use crate::NotificationMethod;
use conclave_client::mls::MlsManager;
use conclave_client::operations;

use self::commands::Command;
use self::input::InputLine;
use self::state::{
    AppState, ConnectionStatus, DisplayMessage, InputMode, PasswordPromptPurpose,
    PasswordPromptStage,
};
use self::store::MessageStore;

/// Run the interactive TUI.
pub async fn run(
    config: &ClientConfig,
    notifications: &NotificationMethod,
) -> crate::error::Result<()> {
    // Load session state.
    let session = SessionState::load(&config.data_dir);
    let initial_url = session.server_url.as_deref().unwrap_or("");
    let http_client = conclave_client::api::build_reqwest_client(
        config.accept_invalid_certs,
        conclave_client::api::parse_custom_headers(&config.custom_headers),
        config.proxy_url.as_deref(),
    );
    let api = Arc::new(Mutex::new(ApiClient::new(initial_url, http_client.clone())));

    let mut state = AppState::new();
    state.show_verified_indicator = config.show_verified_indicator;
    let mut input = InputLine::new();
    let mut mls: Option<MlsManager> = None;
    let mut msg_store: Option<MessageStore> = None;

    // Restore session if already logged in.
    if let Some(token) = &session.token {
        api.lock().await.set_token(token.clone());
        state.username = session.username.clone();
        state.user_id = session.user_id;
        state.logged_in = true;

        if let Some(user_id) = session.user_id {
            mls =
                Some(MlsManager::new(&config.data_dir, user_id).map_err(crate::error::Error::Lib)?);

            // Upload key packages so other users can invite us:
            // 1 last-resort (permanent fallback) + 5 regular (single-use).
            if let Some(mls_mgr) = &mls {
                match generate_initial_key_packages(mls_mgr) {
                    Ok(entries) => {
                        let fingerprint = mls_mgr.signing_key_fingerprint();
                        if let Err(e) = api
                            .lock()
                            .await
                            .upload_key_packages(entries, &fingerprint)
                            .await
                        {
                            state.system_messages.push(DisplayMessage::system(&format!(
                                "Warning: failed to upload key packages: {e}"
                            )));
                        }
                    }
                    Err(e) => {
                        state.system_messages.push(DisplayMessage::system(&format!(
                            "Warning: failed to generate key packages: {e}"
                        )));
                    }
                }
            }

            // Open message store and restore persisted last_seen_seq values
            // *before* loading rooms so that load_rooms preserves them.
            if let Ok(store) = MessageStore::open(&config.data_dir) {
                msg_store = Some(store);
            }

            match commands::load_rooms(&api, &mut state, &msg_store).await {
                Ok(room_infos) => {
                    state.group_mapping = build_group_mapping(&room_infos, &config.data_dir);
                }
                Err(e) => {
                    state.system_messages.push(DisplayMessage::system(&format!(
                        "Failed to load rooms: {e}"
                    )));
                }
            }

            // Accept pending welcomes (invites received while offline) so
            // that group mappings exist before we fetch missed messages.
            accept_pending_welcomes(&api, &mut state, &mls, &config.data_dir, &msg_store).await;

            // Restore persisted last_seen_seq and message history per room,
            // then fetch any messages that arrived while we were offline.
            if let Some(store) = &msg_store {
                for (group_id, room) in &mut state.rooms {
                    let persisted_seq = store.get_last_seen_seq(*group_id);
                    room.last_seen_seq = persisted_seq;
                    room.last_read_seq = store.get_last_read_seq(*group_id);

                    // Load persisted message history into memory.
                    let history = store.load_messages(*group_id);
                    if !history.is_empty() {
                        state
                            .room_messages
                            .entry(*group_id)
                            .or_default()
                            .extend(history);
                    }
                }

                // Fetch messages that arrived while offline.
                fetch_missed_messages(&api, &mut state, &mls, &config.data_dir, store).await;
            }
        }
    }

    // Set up terminal.
    terminal::enable_raw_mode().map_err(|e| crate::error::Error::Terminal(e.to_string()))?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, cursor::Show)
        .map_err(|e| crate::error::Error::Terminal(e.to_string()))?;

    // Enable extended keyboard protocol so Shift+Enter can be distinguished
    // from plain Enter. This is best-effort — older terminals will ignore it,
    // and Alt+Enter serves as a universal fallback.
    let _ = execute!(
        stdout,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
    );

    let (cols, rows) =
        terminal::size().map_err(|e| crate::error::Error::Terminal(e.to_string()))?;
    state.terminal_cols = cols;
    state.terminal_rows = rows;

    // Welcome message.
    if state.logged_in {
        let username = state.username.clone().unwrap_or_default();
        state.push_system_message(DisplayMessage::system(&format!(
            "Welcome back, {username}. Type /help for commands."
        )));
    } else {
        state.push_system_message(DisplayMessage::system(
            "Welcome to Conclave. Use /register and /login to get started. Type /help for commands.",
        ));
    }

    render::render_full(&mut stdout, &state, &input)
        .map_err(|e| crate::error::Error::Terminal(e.to_string()))?;

    // Set up event streams.
    let mut term_events = EventStream::new();
    let mut sse_source: Option<EventSource> = None;

    if state.logged_in {
        match api.lock().await.connect_sse() {
            Ok(es) => {
                sse_source = Some(es);
                state.connection_status = ConnectionStatus::Connecting;
            }
            Err(e) => {
                state.push_system_message(DisplayMessage::system(&format!(
                    "SSE connection failed: {e}"
                )));
            }
        }
    }

    // Main event loop.
    let result = main_loop(
        &mut stdout,
        &mut state,
        &mut input,
        &api,
        &mut mls,
        config,
        &mut term_events,
        &mut sse_source,
        &mut msg_store,
        notifications,
    )
    .await;

    // Restore terminal — errors are ignored because we are shutting down
    // and there is no meaningful recovery action if these fail.
    let _ = execute!(stdout, PopKeyboardEnhancementFlags);
    if let Err(error) = terminal::disable_raw_mode() {
        tracing::warn!(%error, "failed to disable raw mode during shutdown");
    }
    if let Err(error) = execute!(stdout, LeaveAlternateScreen) {
        tracing::warn!(%error, "failed to leave alternate screen during shutdown");
    }

    result
}

enum LoopAction {
    Continue,
    Quit,
}

#[allow(clippy::too_many_arguments)]
async fn main_loop(
    stdout: &mut impl Write,
    state: &mut AppState,
    input: &mut InputLine,
    api: &Arc<Mutex<ApiClient>>,
    mls: &mut Option<MlsManager>,
    config: &ClientConfig,
    term_events: &mut EventStream,
    sse_source: &mut Option<EventSource>,
    msg_store: &mut Option<MessageStore>,
    notifications: &NotificationMethod,
) -> crate::error::Result<()> {
    let mut expiry_interval = tokio::time::interval(std::time::Duration::from_secs(1));
    expiry_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            // Terminal key events.
            Some(Ok(ct_event)) = term_events.next() => {
                match ct_event {
                    CtEvent::Key(key_event) => {
                        match handle_key_event(
                            key_event, stdout, state, input, api, mls, config, sse_source,
                            msg_store, notifications,
                        ).await {
                            Ok(LoopAction::Quit) => break,
                            Ok(LoopAction::Continue) => {}
                            Err(e) => {
                                let msg = DisplayMessage::system(&format!("Error: {e}"));
                                add_and_render_message(stdout, state, input, None, msg,
                                    msg_store, notifications);
                            }
                        }
                    }
                    CtEvent::Resize(cols, rows) => {
                        state.terminal_cols = cols;
                        state.terminal_rows = rows;
                        let _ = render::render_full(stdout, state, input);
                    }
                    _ => {}
                }
            }

            // SSE events.
            Some(sse_event) = async {
                match sse_source.as_mut() {
                    Some(es) => es.next().await,
                    None => std::future::pending().await,
                }
            } => {
                match sse_event {
                    Ok(EsEvent::Open) => {
                        state.connection_status = ConnectionStatus::Connected;

                        // Accept any pending welcomes (invites received
                        // while SSE was disconnected) before fetching.
                        accept_pending_welcomes(api, state, mls, &config.data_dir, msg_store).await;

                        // Fetch messages missed while disconnected.
                        if let Some(store) = msg_store {
                            fetch_missed_messages(
                                api, state, mls, &config.data_dir, store,
                            ).await;

                            // Persist updated last_read_seq if the user is
                            // viewing a room (auto-mark as read).
                            if let Some(gid) = &state.active_room
                                && let Some(room) = state.rooms.get_mut(gid)
                            {
                                room.last_read_seq = room.last_seen_seq;
                                store.set_last_read_seq(*gid, room.last_read_seq);
                            }
                        }

                        let _ = render::render_full(stdout, state, input);
                    }
                    Ok(EsEvent::Message(msg)) => {
                        match events::handle_sse_message(
                            &msg.data, api, state, &config.data_dir, msg_store,
                        ).await {
                            Ok(messages) => {
                                if messages.is_empty() {
                                    let _ = render::render_full(stdout, state, input);
                                } else {
                                    let mut cleanup_groups = std::collections::HashSet::new();
                                    for (group_id, display_msg) in messages {
                                        if let Some(gid) = group_id {
                                            cleanup_groups.insert(gid);
                                        }
                                        add_and_render_message(
                                            stdout, state, input,
                                            group_id, display_msg,
                                            msg_store, notifications,
                                        );
                                    }

                                    // Clean up locally cached messages that
                                    // exceed each group's expiry policy.
                                    if let Some(store) = msg_store {
                                        for gid in cleanup_groups {
                                            if let Some(room) = state.rooms.get(&gid) {
                                                store.cleanup_expired_messages(
                                                    gid,
                                                    room.message_expiry_seconds,
                                                );
                                            }
                                        }
                                    }
                                    conclave_client::state::remove_expired_messages(
                                        &state.rooms,
                                        &mut state.room_messages,
                                    );
                                }
                            }
                            Err(e) => {
                                let msg = DisplayMessage::system(
                                    &format!("SSE processing error: {e}"),
                                );
                                add_and_render_message(stdout, state, input, None, msg,
                                    msg_store, notifications);
                            }
                        }
                    }
                    Err(error) => {
                        if is_sse_unauthorized(&error) {
                            auto_logout(
                                stdout, state, input, api, sse_source, msg_store, notifications,
                            ).await;
                        } else {
                            state.connection_status = ConnectionStatus::Disconnected;
                            *sse_source = None;
                            let _ = render::render_status_line(
                                stdout, state, state.terminal_rows.saturating_sub(2),
                            );
                            let _ = render::render_input_line(
                                stdout, state, input, state.terminal_rows.saturating_sub(1),
                            );
                        }
                    }
                }
            }

            // SSE reconnection timer.
            _ = tokio::time::sleep(std::time::Duration::from_secs(5)),
                if sse_source.is_none() && state.logged_in =>
            {
                state.connection_status = ConnectionStatus::Connecting;
                match api.lock().await.connect_sse() {
                    Ok(es) => {
                        *sse_source = Some(es);
                    }
                    Err(_) => {
                        state.connection_status = ConnectionStatus::Disconnected;
                    }
                }
                let _ = render::render_status_line(
                    stdout, state, state.terminal_rows.saturating_sub(2),
                );
                let _ = render::render_input_line(
                    stdout, state, input, state.terminal_rows.saturating_sub(1),
                );
            }

            // Message expiry cleanup timer (1-second tick).
            _ = expiry_interval.tick(),
                if state.logged_in
                    && conclave_client::state::has_expiring_rooms(&state.rooms) =>
            {
                let removed = conclave_client::state::remove_expired_messages(
                    &state.rooms,
                    &mut state.room_messages,
                );

                if let Some(store) = msg_store {
                    for (group_id, room) in &state.rooms {
                        if room.message_expiry_seconds > 0 {
                            store.cleanup_expired_messages(
                                *group_id,
                                room.message_expiry_seconds,
                            );
                        }
                    }
                }

                if removed {
                    let _ = render::render_full(stdout, state, input);
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_key_event(
    key: crossterm::event::KeyEvent,
    stdout: &mut impl Write,
    state: &mut AppState,
    input: &mut InputLine,
    api: &Arc<Mutex<ApiClient>>,
    mls: &mut Option<MlsManager>,
    config: &ClientConfig,
    sse_source: &mut Option<EventSource>,
    msg_store: &mut Option<MessageStore>,
    notifications: &NotificationMethod,
) -> crate::error::Result<LoopAction> {
    let in_password_mode = !matches!(state.input_mode, InputMode::Normal);

    // Handle keys that have special control flow (quit, command dispatch,
    // scrolling) before falling through to the common input-editing path.
    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(LoopAction::Quit),

        // Escape cancels password prompt mode.
        (KeyCode::Esc, _) if in_password_mode => {
            state.input_mode = InputMode::Normal;
            input.submit(true);
            let msg = DisplayMessage::system("Cancelled.");
            add_and_render_message(stdout, state, input, None, msg, msg_store, notifications);
            let _ = render::render_full(stdout, state, input);
            return Ok(LoopAction::Continue);
        }

        // Block multiline in password mode.
        (KeyCode::Enter, m)
            if !in_password_mode
                && (m.contains(KeyModifiers::SHIFT) || m.contains(KeyModifiers::ALT)) =>
        {
            input.insert('\n');
            let _ = render::render_full(stdout, state, input);
            return Ok(LoopAction::Continue);
        }

        // Enter in password mode advances the prompt stage.
        (KeyCode::Enter, _) if in_password_mode => {
            return handle_password_enter(
                stdout,
                state,
                input,
                api,
                mls,
                config,
                sse_source,
                msg_store,
                notifications,
            )
            .await;
        }

        (KeyCode::Enter, _) => {
            return handle_enter(
                stdout,
                state,
                input,
                api,
                mls,
                config,
                sse_source,
                msg_store,
                notifications,
            )
            .await;
        }

        (KeyCode::PageUp, _) => {
            let msg_rows = state.terminal_rows.saturating_sub(2) as usize;
            let total = state.active_messages().len();
            let max_scroll = total.saturating_sub(msg_rows);
            state.scroll_offset = state.scroll_offset.saturating_add(10).min(max_scroll);
            let _ = render::render_full(stdout, state, input);
            return Ok(LoopAction::Continue);
        }
        (KeyCode::PageDown, _) => {
            state.scroll_offset = state.scroll_offset.saturating_sub(10);
            let _ = render::render_full(stdout, state, input);
            return Ok(LoopAction::Continue);
        }

        _ => {}
    }

    // All remaining keys edit the input line, then re-render it.
    // In password mode, block history navigation.
    let edited = match (key.code, key.modifiers) {
        // Block history in password mode.
        (KeyCode::Up | KeyCode::Down, _) if in_password_mode => false,

        (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
            if !input.is_empty() {
                input.delete();
                true
            } else {
                false
            }
        }
        (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
            input.home();
            true
        }
        (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
            input.end();
            true
        }
        (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
            input.move_left();
            true
        }
        (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
            input.move_right();
            true
        }
        (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
            input.kill_to_end();
            true
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            input.kill_to_start();
            true
        }
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
            input.kill_word_backward();
            true
        }
        (KeyCode::Char('b'), KeyModifiers::ALT) => {
            input.move_word_left();
            true
        }
        (KeyCode::Char('f'), KeyModifiers::ALT) => {
            input.move_word_right();
            true
        }
        (KeyCode::Char('d'), KeyModifiers::ALT) => {
            input.delete_word_forward();
            true
        }
        (KeyCode::Backspace, _) => {
            input.backspace();
            true
        }
        (KeyCode::Delete, _) => {
            input.delete();
            true
        }
        (KeyCode::Left, _) => {
            input.move_left();
            true
        }
        (KeyCode::Right, _) => {
            input.move_right();
            true
        }
        (KeyCode::Home, _) => {
            input.home();
            true
        }
        (KeyCode::End, _) => {
            input.end();
            true
        }
        (KeyCode::Up, _) => {
            input.history_up();
            true
        }
        (KeyCode::Down, _) => {
            input.history_down();
            true
        }
        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            input.insert(c);
            true
        }
        _ => false,
    };

    if edited {
        if input.content().contains('\n') {
            let _ = render::render_full(stdout, state, input);
        } else {
            let _ = render::render_input_line(
                stdout,
                state,
                input,
                state.terminal_rows.saturating_sub(1),
            );
        }
    }

    Ok(LoopAction::Continue)
}

#[allow(clippy::too_many_arguments)]
async fn handle_enter(
    stdout: &mut impl Write,
    state: &mut AppState,
    input: &mut InputLine,
    api: &Arc<Mutex<ApiClient>>,
    mls: &mut Option<MlsManager>,
    config: &ClientConfig,
    sse_source: &mut Option<EventSource>,
    msg_store: &mut Option<MessageStore>,
    notifications: &NotificationMethod,
) -> crate::error::Result<LoopAction> {
    if input.is_empty() {
        return Ok(LoopAction::Continue);
    }
    let text = input.submit(false).trim_end_matches('\n').to_string();
    if text.is_empty() {
        let _ = render::render_full(stdout, state, input);
        return Ok(LoopAction::Continue);
    }

    match commands::parse(&text) {
        Ok(Command::Quit) => {
            return Ok(LoopAction::Quit);
        }
        Ok(cmd) => {
            let is_expunge = matches!(cmd, Command::Expunge { .. });
            match commands::execute(cmd, api, state, mls, config, msg_store).await {
                Ok((msgs, should_start_sse)) => {
                    for msg in msgs {
                        add_and_render_message(
                            stdout,
                            state,
                            input,
                            None,
                            msg,
                            msg_store,
                            notifications,
                        );
                    }
                    if is_expunge {
                        return Ok(LoopAction::Quit);
                    }
                    if should_start_sse {
                        // Open the message store after a fresh /login
                        // so messages and seq values are persisted.
                        if msg_store.is_none()
                            && let Ok(store) = MessageStore::open(&config.data_dir)
                        {
                            *msg_store = Some(store);
                        }
                        if sse_source.is_none()
                            && let Ok(es) = api.lock().await.connect_sse()
                        {
                            *sse_source = Some(es);
                            state.connection_status = ConnectionStatus::Connecting;
                        }
                    }
                }
                Err(e) => {
                    if e.is_unauthorized() {
                        auto_logout(
                            stdout,
                            state,
                            input,
                            api,
                            sse_source,
                            msg_store,
                            notifications,
                        )
                        .await;
                    } else {
                        let msg = DisplayMessage::system(&format!("Error: {e}"));
                        add_and_render_message(
                            stdout,
                            state,
                            input,
                            None,
                            msg,
                            msg_store,
                            notifications,
                        );
                    }
                }
            }
            let _ = render::render_full(stdout, state, input);
        }
        Err(e) => {
            let msg = DisplayMessage::system(&format!("{e}"));
            add_and_render_message(stdout, state, input, None, msg, msg_store, notifications);
            let _ = render::render_full(stdout, state, input);
        }
    }

    Ok(LoopAction::Continue)
}

#[allow(clippy::too_many_arguments)]
async fn handle_password_enter(
    stdout: &mut impl Write,
    state: &mut AppState,
    input: &mut InputLine,
    api: &Arc<Mutex<ApiClient>>,
    mls: &mut Option<MlsManager>,
    config: &ClientConfig,
    sse_source: &mut Option<EventSource>,
    msg_store: &mut Option<MessageStore>,
    notifications: &NotificationMethod,
) -> crate::error::Result<LoopAction> {
    let text = input.submit(true);

    // Take the current input mode, temporarily replacing with Normal.
    let mode = std::mem::replace(&mut state.input_mode, InputMode::Normal);

    let InputMode::PasswordPrompt {
        purpose,
        stage,
        current_password,
        new_password,
    } = mode
    else {
        return Ok(LoopAction::Continue);
    };

    match (&purpose, &stage) {
        // ── ChangePassword ───────────────────────────────────────
        (PasswordPromptPurpose::ChangePassword, PasswordPromptStage::Current) => {
            state.input_mode = InputMode::PasswordPrompt {
                purpose,
                stage: PasswordPromptStage::New,
                current_password: text,
                new_password,
            };
        }
        (PasswordPromptPurpose::ChangePassword, PasswordPromptStage::New) => {
            state.input_mode = InputMode::PasswordPrompt {
                purpose,
                stage: PasswordPromptStage::Confirm,
                current_password,
                new_password: text,
            };
        }
        (PasswordPromptPurpose::ChangePassword, PasswordPromptStage::Confirm) => {
            if text != new_password {
                let msg =
                    DisplayMessage::system("Passwords do not match. Password change cancelled.");
                add_and_render_message(stdout, state, input, None, msg, msg_store, notifications);
            } else {
                match api
                    .lock()
                    .await
                    .change_password(&current_password, &new_password)
                    .await
                {
                    Ok(()) => {
                        let msg = DisplayMessage::system(
                            "Password changed successfully. Please log in again.",
                        );
                        add_and_render_message(
                            stdout,
                            state,
                            input,
                            None,
                            msg,
                            msg_store,
                            notifications,
                        );
                    }
                    Err(e) => {
                        let msg = DisplayMessage::system(&format!("Error: {e}"));
                        add_and_render_message(
                            stdout,
                            state,
                            input,
                            None,
                            msg,
                            msg_store,
                            notifications,
                        );
                    }
                }
            }
        }

        // ── Register ─────────────────────────────────────────────
        (PasswordPromptPurpose::Register { .. }, PasswordPromptStage::New) => {
            state.input_mode = InputMode::PasswordPrompt {
                purpose,
                stage: PasswordPromptStage::Confirm,
                current_password,
                new_password: text,
            };
        }
        (PasswordPromptPurpose::Register { .. }, PasswordPromptStage::Confirm) => {
            if text != new_password {
                let msg = DisplayMessage::system("Passwords do not match. Registration cancelled.");
                add_and_render_message(stdout, state, input, None, msg, msg_store, notifications);
            } else {
                // Extract the fields from purpose (already moved out).
                let PasswordPromptPurpose::Register {
                    server,
                    username,
                    token,
                } = purpose
                else {
                    unreachable!()
                };
                let client = conclave_client::api::build_reqwest_client(
                    config.accept_invalid_certs,
                    conclave_client::api::parse_custom_headers(&config.custom_headers),
                    config.proxy_url.as_deref(),
                );
                match operations::register_and_login(
                    &server,
                    &username,
                    &new_password,
                    token.as_deref(),
                    client.clone(),
                    &config.data_dir,
                )
                .await
                {
                    Ok(result) => {
                        *api.lock().await = result.into_api_client(client);
                        state.username = Some(result.username.clone());
                        state.user_id = Some(result.user_id);
                        state.logged_in = true;
                        if let Err(e) = result.save_session(&config.data_dir) {
                            tracing::warn!(error = %e, "failed to save session");
                        }
                        *mls = Some(MlsManager::new(&config.data_dir, result.user_id)?);

                        let room_infos = commands::load_rooms(api, state, msg_store).await?;
                        state.group_mapping = build_group_mapping(&room_infos, &config.data_dir);

                        let msg = DisplayMessage::system(&format!(
                            "Registered and logged in as {} (user ID {})",
                            result.username, result.user_id
                        ));
                        add_and_render_message(
                            stdout,
                            state,
                            input,
                            None,
                            msg,
                            msg_store,
                            notifications,
                        );

                        if msg_store.is_none()
                            && let Ok(store) = MessageStore::open(&config.data_dir)
                        {
                            *msg_store = Some(store);
                        }
                        if sse_source.is_none()
                            && let Ok(es) = api.lock().await.connect_sse()
                        {
                            *sse_source = Some(es);
                            state.connection_status = ConnectionStatus::Connecting;
                        }
                    }
                    Err(e) => {
                        let msg = DisplayMessage::system(&format!("Error: {e}"));
                        add_and_render_message(
                            stdout,
                            state,
                            input,
                            None,
                            msg,
                            msg_store,
                            notifications,
                        );
                    }
                }
            }
        }

        // ── Login ────────────────────────────────────────────────
        (PasswordPromptPurpose::Login { .. }, PasswordPromptStage::New) => {
            let PasswordPromptPurpose::Login { server, username } = purpose else {
                unreachable!()
            };
            let client = conclave_client::api::build_reqwest_client(
                config.accept_invalid_certs,
                conclave_client::api::parse_custom_headers(&config.custom_headers),
                config.proxy_url.as_deref(),
            );
            match operations::login(&server, &username, &text, client.clone(), &config.data_dir)
                .await
            {
                Ok(result) => {
                    *api.lock().await = result.into_api_client(client);
                    state.username = Some(result.username.clone());
                    state.user_id = Some(result.user_id);
                    state.logged_in = true;
                    if let Err(e) = result.save_session(&config.data_dir) {
                        tracing::warn!(error = %e, "failed to save session");
                    }
                    *mls = Some(MlsManager::new(&config.data_dir, result.user_id)?);

                    let room_infos = commands::load_rooms(api, state, msg_store).await?;
                    state.group_mapping = build_group_mapping(&room_infos, &config.data_dir);

                    let unmapped_count = state
                        .rooms
                        .keys()
                        .filter(|gid| !state.group_mapping.contains_key(gid))
                        .count();
                    if unmapped_count > 0 {
                        let msg = DisplayMessage::system(&format!(
                            "{unmapped_count} group(s) have no local encryption state. \
                             Run /reset to rejoin them with a new identity."
                        ));
                        add_and_render_message(
                            stdout,
                            state,
                            input,
                            None,
                            msg,
                            msg_store,
                            notifications,
                        );
                    }

                    let msg = DisplayMessage::system(&format!(
                        "Logged in as {} (user ID {})",
                        result.username, result.user_id
                    ));
                    add_and_render_message(
                        stdout,
                        state,
                        input,
                        None,
                        msg,
                        msg_store,
                        notifications,
                    );

                    if msg_store.is_none()
                        && let Ok(store) = MessageStore::open(&config.data_dir)
                    {
                        *msg_store = Some(store);
                    }
                    if sse_source.is_none()
                        && let Ok(es) = api.lock().await.connect_sse()
                    {
                        *sse_source = Some(es);
                        state.connection_status = ConnectionStatus::Connecting;
                    }
                }
                Err(e) => {
                    let msg = DisplayMessage::system(&format!("Error: {e}"));
                    add_and_render_message(
                        stdout,
                        state,
                        input,
                        None,
                        msg,
                        msg_store,
                        notifications,
                    );
                }
            }
        }

        _ => {}
    }

    let _ = render::render_full(stdout, state, input);
    Ok(LoopAction::Continue)
}

/// Perform auto-logout: clear auth state and stop SSE reconnection.
/// Used when the server returns 401 (expired/invalidated token).
async fn auto_logout(
    stdout: &mut impl Write,
    state: &mut AppState,
    input: &InputLine,
    api: &Arc<Mutex<ApiClient>>,
    sse_source: &mut Option<EventSource>,
    msg_store: &Option<MessageStore>,
    notifications: &NotificationMethod,
) {
    api.lock().await.set_token(String::new());
    state.logged_in = false;
    state.username = None;
    state.user_id = None;
    state.connection_status = ConnectionStatus::Disconnected;
    *sse_source = None;

    let msg = DisplayMessage::system("Session expired. Please log in again.");
    add_and_render_message(stdout, state, input, None, msg, msg_store, notifications);
    let _ = render::render_full(stdout, state, input);
}

/// Check if an SSE error represents an HTTP 401 Unauthorized response.
fn is_sse_unauthorized(error: &reqwest_eventsource::Error) -> bool {
    matches!(
        error,
        reqwest_eventsource::Error::InvalidStatusCode(status, _)
            if *status == reqwest::StatusCode::UNAUTHORIZED
    )
}

/// Add a message to state, persist it, and render it if it belongs to the active view.
fn add_and_render_message(
    stdout: &mut impl Write,
    state: &mut AppState,
    input: &InputLine,
    group_id: Option<Uuid>,
    msg: DisplayMessage,
    msg_store: &Option<MessageStore>,
    notifications: &NotificationMethod,
) {
    // Determine the effective group_id: system messages (group_id=None) are
    // pushed into the active room's message list so they appear inline.
    let effective_gid = group_id.or(state.active_room);

    let is_active_view = match (&state.active_room, &effective_gid) {
        (Some(active), Some(gid)) => active == gid,
        (None, None) => true,
        _ => false,
    };

    // Persist room messages to disk.
    if let (Some(gid), Some(store)) = (&effective_gid, msg_store) {
        if group_id.is_some() {
            store.push_message(*gid, &msg);
        }
        if let Some(room) = state.rooms.get(gid) {
            store.set_last_seen_seq(*gid, room.last_seen_seq);
        }
    }

    match &effective_gid {
        Some(gid) => state.push_room_message(*gid, msg.clone()),
        None => state.push_system_message(msg.clone()),
    }

    if is_active_view {
        // Mark messages as read when the user is viewing the room.
        if let Some(gid) = &effective_gid
            && let Some(room) = state.rooms.get_mut(gid)
        {
            room.last_read_seq = room.last_seen_seq;
            if let Some(store) = msg_store {
                store.set_last_read_seq(*gid, room.last_read_seq);
            }
        }
        // Reset scroll to bottom when new messages arrive.
        state.scroll_offset = 0;
        let _ = render::render_new_message(stdout, state, input);
    } else if !msg.is_system && group_id.is_some() {
        let room_name = group_id
            .and_then(|gid| state.rooms.get(&gid))
            .map(|r| r.display_name())
            .unwrap_or_else(|| "unknown".to_string());
        let use_native = matches!(
            notifications,
            NotificationMethod::Native | NotificationMethod::Both
        );
        let use_bell = matches!(
            notifications,
            NotificationMethod::Bell | NotificationMethod::Both
        );
        if use_native {
            crate::notification::send_notification(
                &format!("#{room_name} - {}", msg.sender),
                &msg.content,
            );
        }
        if use_bell {
            let _ = stdout.write_all(b"\x07");
            let _ = stdout.flush();
        }
    }
}

/// Accept any pending welcomes (group invitations received while offline).
/// Processes each welcome via MLS, updates group mapping, replenishes
/// key packages, and reloads the room list.
async fn accept_pending_welcomes(
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    _mls: &Option<MlsManager>,
    data_dir: &std::path::Path,
    msg_store: &Option<MessageStore>,
) {
    let user_id = match state.user_id {
        Some(id) => id,
        None => return,
    };

    let welcome_result = {
        let api_guard = api.lock().await;
        operations::accept_welcomes(&api_guard, data_dir, user_id).await
    };
    let results = match welcome_result {
        Ok(r) => r,
        Err(error) => {
            state.system_messages.push(DisplayMessage::system(&format!(
                "Failed to accept welcomes: {error}"
            )));
            return;
        }
    };

    if results.is_empty() {
        return;
    }

    for result in &results {
        state
            .group_mapping
            .insert(result.group_id, result.mls_group_id.clone());

        let id_string = result.group_id.to_string();
        let display = result.group_alias.as_deref().unwrap_or(&id_string);
        state.system_messages.push(DisplayMessage::system(&format!(
            "Joined #{display} ({})",
            result.group_id
        )));
    }

    let _ = commands::load_rooms(api, state, msg_store).await;
}

/// Fetch messages that arrived while the client was offline for all rooms.
async fn fetch_missed_messages(
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    _mls: &Option<MlsManager>,
    data_dir: &std::path::Path,
    store: &MessageStore,
) {
    let room_ids: Vec<(Uuid, u64, String, Vec<conclave_client::state::RoomMember>)> = state
        .rooms
        .iter()
        .filter_map(|(id, room)| {
            state.group_mapping.get(id).map(|mls_id| {
                (
                    *id,
                    room.last_seen_seq,
                    mls_id.clone(),
                    room.members.clone(),
                )
            })
        })
        .collect();

    let user_id = match state.user_id {
        Some(id) => id,
        None => return,
    };

    for (group_id, last_seq, mls_group_id, members) in &room_ids {
        let fetch_result = {
            let api_guard = api.lock().await;
            operations::fetch_and_decrypt(
                &api_guard,
                *group_id,
                *last_seq,
                mls_group_id,
                data_dir,
                user_id,
                members,
            )
            .await
        };
        let fetched = match fetch_result {
            Ok(f) => f,
            Err(_) => continue,
        };

        for msg in &fetched.messages {
            let mut display_msg = if msg.is_system {
                DisplayMessage::system(&msg.content)
            } else {
                DisplayMessage::user(
                    msg.sender_id.unwrap_or(Uuid::nil()),
                    &msg.sender,
                    &msg.content,
                    msg.timestamp,
                )
            };
            display_msg.sequence_num = Some(msg.sequence_num);
            display_msg.epoch = Some(msg.epoch);
            if !msg.is_system {
                store.push_message(*group_id, &display_msg);
            }
            state.push_room_message(*group_id, display_msg);

            if let Some(room) = state.rooms.get_mut(group_id) {
                room.last_seen_seq = room.last_seen_seq.max(msg.sequence_num);
            }
        }

        if let Some(room) = state.rooms.get(group_id) {
            store.set_last_seen_seq(*group_id, room.last_seen_seq);

            // Clean up locally cached messages that exceed the group's expiry policy.
            store.cleanup_expired_messages(*group_id, room.message_expiry_seconds);
        }
    }
}
