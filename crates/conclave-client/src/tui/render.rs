use std::io::Write;

use crossterm::{
    cursor, queue,
    style::{Attribute, Color, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{Clear, ClearType},
};

use super::input::InputLine;
use super::state::{AppState, ConnectionStatus, DisplayMessage};

/// Full redraw of the terminal.
pub fn render_full(
    stdout: &mut impl Write,
    state: &AppState,
    input: &InputLine,
) -> std::io::Result<()> {
    let rows = state.terminal_rows;
    let cols = state.terminal_cols;

    queue!(stdout, Clear(ClearType::All))?;

    // Message area: rows 0..rows-2
    let msg_rows = rows.saturating_sub(2) as usize;
    let messages = state.active_messages();
    let total = messages.len();
    let start = total.saturating_sub(msg_rows + state.scroll_offset);
    let end = total.saturating_sub(state.scroll_offset);

    for (i, msg) in messages[start..end].iter().enumerate() {
        queue!(stdout, cursor::MoveTo(0, i as u16))?;
        write_message(stdout, msg, cols)?;
    }

    render_status_line(stdout, state, rows.saturating_sub(2))?;
    render_input_line(stdout, state, input, rows.saturating_sub(1))?;

    stdout.flush()
}

/// Render the status line (reverse video bar).
pub fn render_status_line(
    stdout: &mut impl Write,
    state: &AppState,
    row: u16,
) -> std::io::Result<()> {
    queue!(
        stdout,
        cursor::MoveTo(0, row),
        SetAttribute(Attribute::Reverse),
    )?;

    let status_icon = match state.connection_status {
        ConnectionStatus::Connected => "connected",
        ConnectionStatus::Connecting => "connecting...",
        ConnectionStatus::Disconnected => "disconnected",
    };

    let room_info = if let Some(room) = state.active_room_info() {
        let member_count = room.members.len();
        format!("#{} ({member_count})", room.name)
    } else {
        String::from("no room")
    };

    let username = state.username.as_deref().unwrap_or("not logged in");

    let status = format!(" [{status_icon}] {room_info} | {username} ");

    // Pad to full width.
    let padding = (state.terminal_cols as usize).saturating_sub(status.len());
    write!(stdout, "{status}{}", " ".repeat(padding))?;

    queue!(stdout, SetAttribute(Attribute::Reset))?;

    Ok(())
}

/// Render the input line.
pub fn render_input_line(
    stdout: &mut impl Write,
    state: &AppState,
    input: &InputLine,
    row: u16,
) -> std::io::Result<()> {
    queue!(
        stdout,
        cursor::MoveTo(0, row),
        Clear(ClearType::CurrentLine),
    )?;

    let prefix = if let Some(room) = state.active_room_info() {
        format!("[#{}] ", room.name)
    } else {
        String::from("> ")
    };

    write!(stdout, "{}{}", prefix, input.content())?;

    // Position cursor correctly.
    let cursor_col = prefix.len() + input.cursor_position();
    queue!(stdout, cursor::MoveTo(cursor_col as u16, row))?;

    stdout.flush()
}

/// Render a newly added message by performing a full redraw.
pub fn render_new_message(
    stdout: &mut impl Write,
    state: &AppState,
    input: &InputLine,
    _msg: &DisplayMessage,
) -> std::io::Result<()> {
    render_full(stdout, state, input)
}

/// Write a single formatted message at the current cursor position.
fn write_message(stdout: &mut impl Write, msg: &DisplayMessage, _cols: u16) -> std::io::Result<()> {
    let time = chrono::DateTime::from_timestamp(msg.timestamp, 0)
        .map(|dt| dt.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_else(|| "??:??".to_string());

    if msg.is_system {
        queue!(stdout, SetForegroundColor(Color::DarkYellow))?;
        write!(stdout, "[{time}] *** {}", msg.content)?;
        queue!(stdout, ResetColor)?;
    } else {
        let nick_color = username_color(&msg.sender);
        queue!(stdout, SetForegroundColor(Color::DarkGrey))?;
        write!(stdout, "[{time}] ")?;
        queue!(stdout, SetForegroundColor(nick_color))?;
        write!(stdout, "<{}>", msg.sender)?;
        queue!(stdout, ResetColor)?;
        write!(stdout, " {}", msg.content)?;
    }

    Ok(())
}

/// Assign a consistent ANSI color to a username based on hash.
fn username_color(username: &str) -> Color {
    let colors = [
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::DarkRed,
        Color::DarkGreen,
        Color::DarkYellow,
        Color::DarkBlue,
        Color::DarkMagenta,
        Color::DarkCyan,
    ];
    let hash: usize = username.bytes().fold(0usize, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(b as usize)
    });
    colors[hash % colors.len()]
}
