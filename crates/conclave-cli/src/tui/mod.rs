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
    event::{Event as CtEvent, EventStream, KeyCode, KeyModifiers},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::StreamExt;
use reqwest_eventsource::{Event as EsEvent, EventSource};
use tokio::sync::Mutex;

use conclave_lib::api::ApiClient;
use conclave_lib::config::{ClientConfig, SessionState};
use conclave_lib::mls::MlsManager;

use self::commands::Command;
use self::input::InputLine;
use self::state::{AppState, ConnectionStatus, DisplayMessage};
use self::store::MessageStore;

/// Run the interactive TUI.
pub async fn run(config: &ClientConfig) -> crate::error::Result<()> {
    // Load session state.
    let session = SessionState::load(&config.data_dir);
    let initial_url = session.server_url.as_deref().unwrap_or("");
    let api = Arc::new(Mutex::new(ApiClient::new(
        initial_url,
        config.accept_invalid_certs,
    )));

    let mut state = AppState::new();
    let mut input = InputLine::new();
    let mut mls: Option<MlsManager> = None;
    let mut msg_store: Option<MessageStore> = None;

    // Restore session if already logged in.
    if let Some(token) = &session.token {
        api.lock().await.set_token(token.clone());
        state.username = session.username.clone();
        state.user_id = session.user_id;
        state.logged_in = true;

        if let Some(username) = &session.username {
            mls = Some(
                MlsManager::new(&config.data_dir, username).map_err(crate::error::Error::Lib)?,
            );
            state.group_mapping = commands::load_group_mapping(&config.data_dir);

            // Open message store and restore persisted last_seen_seq values
            // *before* loading rooms so that load_rooms preserves them.
            if let Ok(store) = MessageStore::open(&config.data_dir) {
                msg_store = Some(store);
            }

            if let Err(e) = commands::load_rooms(&api, &mut state).await {
                state.system_messages.push(DisplayMessage::system(&format!(
                    "Failed to load rooms: {e}"
                )));
            }

            // Restore persisted last_seen_seq and message history per room,
            // then fetch any messages that arrived while we were offline.
            if let Some(store) = &msg_store {
                for (group_id, room) in &mut state.rooms {
                    let persisted_seq = store.get_last_seen_seq(group_id);
                    room.last_seen_seq = persisted_seq;
                    room.last_read_seq = store.get_last_read_seq(group_id);

                    // Load persisted message history into memory.
                    let history = store.load_messages(group_id);
                    if !history.is_empty() {
                        state
                            .room_messages
                            .entry(group_id.clone())
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
        &msg_store,
    )
    .await;

    // Restore terminal.
    let _ = terminal::disable_raw_mode();
    let _ = execute!(stdout, LeaveAlternateScreen);

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
    msg_store: &Option<MessageStore>,
) -> crate::error::Result<()> {
    loop {
        tokio::select! {
            // Terminal key events.
            Some(Ok(ct_event)) = term_events.next() => {
                match ct_event {
                    CtEvent::Key(key_event) => {
                        match handle_key_event(
                            key_event, stdout, state, input, api, mls, config, sse_source,
                            msg_store,
                        ).await {
                            Ok(LoopAction::Quit) => break,
                            Ok(LoopAction::Continue) => {}
                            Err(e) => {
                                let msg = DisplayMessage::system(&format!("Error: {e}"));
                                add_and_render_message(stdout, state, input, None, msg,
                                    msg_store);
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
                        let _ = render::render_status_line(
                            stdout, state, state.terminal_rows.saturating_sub(2),
                        );
                        let _ = render::render_input_line(
                            stdout, state, input, state.terminal_rows.saturating_sub(1),
                        );
                    }
                    Ok(EsEvent::Message(msg)) => {
                        match events::handle_sse_message(
                            &msg.data, api, state, &config.data_dir,
                        ).await {
                            Ok(messages) => {
                                for (group_id, display_msg) in messages {
                                    add_and_render_message(
                                        stdout, state, input,
                                        Some(&group_id), display_msg,
                                        msg_store,
                                    );
                                }
                            }
                            Err(e) => {
                                let msg = DisplayMessage::system(
                                    &format!("SSE processing error: {e}"),
                                );
                                add_and_render_message(stdout, state, input, None, msg,
                                    msg_store);
                            }
                        }
                    }
                    Err(_) => {
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
    msg_store: &Option<MessageStore>,
) -> crate::error::Result<LoopAction> {
    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL)
        | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
            return Ok(LoopAction::Quit);
        }

        (KeyCode::Enter, _) => {
            if input.is_empty() {
                return Ok(LoopAction::Continue);
            }
            let text = input.submit();

            match commands::parse(&text) {
                Ok(Command::Quit) => {
                    return Ok(LoopAction::Quit);
                }
                Ok(cmd) => {
                    match commands::execute(cmd, api, state, mls, config, msg_store).await {
                        Ok((msgs, should_start_sse)) => {
                            for msg in msgs {
                                add_and_render_message(stdout, state, input, None, msg, msg_store);
                            }
                            if should_start_sse && sse_source.is_none() {
                                if let Ok(es) = api.lock().await.connect_sse() {
                                    *sse_source = Some(es);
                                    state.connection_status = ConnectionStatus::Connecting;
                                }
                            }
                        }
                        Err(e) => {
                            let msg = DisplayMessage::system(&format!("Error: {e}"));
                            add_and_render_message(stdout, state, input, None, msg, msg_store);
                        }
                    }
                    let _ = render::render_full(stdout, state, input);
                }
                Err(e) => {
                    let msg = DisplayMessage::system(&format!("{e}"));
                    add_and_render_message(stdout, state, input, None, msg, msg_store);
                    let _ = render::render_full(stdout, state, input);
                }
            }
        }

        (KeyCode::Backspace, _) => {
            input.backspace();
            let _ = render::render_input_line(
                stdout,
                state,
                input,
                state.terminal_rows.saturating_sub(1),
            );
        }
        (KeyCode::Delete, _) => {
            input.delete();
            let _ = render::render_input_line(
                stdout,
                state,
                input,
                state.terminal_rows.saturating_sub(1),
            );
        }
        (KeyCode::Left, _) => {
            input.move_left();
            let _ = render::render_input_line(
                stdout,
                state,
                input,
                state.terminal_rows.saturating_sub(1),
            );
        }
        (KeyCode::Right, _) => {
            input.move_right();
            let _ = render::render_input_line(
                stdout,
                state,
                input,
                state.terminal_rows.saturating_sub(1),
            );
        }
        (KeyCode::Home, _) => {
            input.home();
            let _ = render::render_input_line(
                stdout,
                state,
                input,
                state.terminal_rows.saturating_sub(1),
            );
        }
        (KeyCode::End, _) => {
            input.end();
            let _ = render::render_input_line(
                stdout,
                state,
                input,
                state.terminal_rows.saturating_sub(1),
            );
        }
        (KeyCode::Up, _) => {
            input.history_up();
            let _ = render::render_input_line(
                stdout,
                state,
                input,
                state.terminal_rows.saturating_sub(1),
            );
        }
        (KeyCode::Down, _) => {
            input.history_down();
            let _ = render::render_input_line(
                stdout,
                state,
                input,
                state.terminal_rows.saturating_sub(1),
            );
        }
        (KeyCode::PageUp, _) => {
            state.scroll_offset = state.scroll_offset.saturating_add(10);
            let _ = render::render_full(stdout, state, input);
        }
        (KeyCode::PageDown, _) => {
            state.scroll_offset = state.scroll_offset.saturating_sub(10);
            let _ = render::render_full(stdout, state, input);
        }

        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            input.insert(c);
            let _ = render::render_input_line(
                stdout,
                state,
                input,
                state.terminal_rows.saturating_sub(1),
            );
        }

        _ => {}
    }

    Ok(LoopAction::Continue)
}

/// Add a message to state, persist it, and render it if it belongs to the active view.
fn add_and_render_message(
    stdout: &mut impl Write,
    state: &mut AppState,
    input: &InputLine,
    group_id: Option<&str>,
    msg: DisplayMessage,
    msg_store: &Option<MessageStore>,
) {
    // Determine the effective group_id: system messages (group_id=None) are
    // pushed into the active room's message list so they appear inline.
    let effective_gid = group_id
        .map(|s| s.to_string())
        .or_else(|| state.active_room.clone());

    let is_active_view = match (&state.active_room, &effective_gid) {
        (Some(active), Some(gid)) => active == gid,
        (None, None) => true,
        _ => false,
    };

    // Persist room messages to disk.
    if let (Some(gid), Some(store)) = (&effective_gid, msg_store) {
        if group_id.is_some() {
            // Only persist actual room messages, not ephemeral system messages.
            store.push_message(gid, &msg);
        }
        // Persist the updated last_seen_seq.
        if let Some(room) = state.rooms.get(gid.as_str()) {
            store.set_last_seen_seq(gid, room.last_seen_seq);
        }
    }

    match &effective_gid {
        Some(gid) => state.push_room_message(gid, msg.clone()),
        None => state.push_system_message(msg.clone()),
    }

    if is_active_view {
        // Mark messages as read when the user is viewing the room.
        if let Some(gid) = &effective_gid {
            if let Some(room) = state.rooms.get_mut(gid.as_str()) {
                room.last_read_seq = room.last_seen_seq;
                if let Some(store) = msg_store {
                    store.set_last_read_seq(gid, room.last_read_seq);
                }
            }
        }
        // Reset scroll to bottom when new messages arrive.
        state.scroll_offset = 0;
        let _ = render::render_new_message(stdout, state, input, &msg);
    }
}

/// Fetch messages that arrived while the client was offline for all rooms.
async fn fetch_missed_messages(
    api: &Arc<Mutex<ApiClient>>,
    state: &mut AppState,
    _mls: &Option<MlsManager>,
    data_dir: &std::path::Path,
    store: &MessageStore,
) {
    let room_ids: Vec<(String, u64)> = state
        .rooms
        .iter()
        .map(|(id, room)| (id.clone(), room.last_seen_seq))
        .collect();

    for (group_id, last_seq) in room_ids {
        let mls_group_id = match state.group_mapping.get(&group_id) {
            Some(id) => id.clone(),
            None => continue,
        };

        let username = match &state.username {
            Some(u) => u.clone(),
            None => continue,
        };

        let resp = match api
            .lock()
            .await
            .get_messages(&group_id, last_seq as i64)
            .await
        {
            Ok(r) => r,
            Err(_) => continue,
        };

        for stored_msg in &resp.messages {
            let data_dir_owned = data_dir.to_path_buf();
            let username_clone = username.clone();
            let mls_group_id_clone = mls_group_id.clone();
            let mls_bytes = stored_msg.mls_message.clone();

            let decrypted = match tokio::task::spawn_blocking(move || {
                let mls = MlsManager::new(&data_dir_owned, &username_clone)?;
                mls.decrypt_message(&mls_group_id_clone, &mls_bytes)
            })
            .await
            {
                Ok(Ok(d)) => d,
                _ => continue,
            };

            match decrypted {
                conclave_lib::mls::DecryptedMessage::Application(plaintext) => {
                    let text = String::from_utf8_lossy(&plaintext).to_string();
                    let msg = DisplayMessage::user(
                        &stored_msg.sender_username,
                        &text,
                        stored_msg.created_at as i64,
                    );
                    store.push_message(&group_id, &msg);
                    state.push_room_message(&group_id, msg);
                }
                _ => {}
            }

            if let Some(room) = state.rooms.get_mut(&group_id) {
                room.last_seen_seq = room.last_seen_seq.max(stored_msg.sequence_num);
            }
        }

        // Persist updated last_seen_seq after processing all messages for this room.
        if let Some(room) = state.rooms.get(&group_id) {
            store.set_last_seen_seq(&group_id, room.last_seen_seq);
        }
    }
}
