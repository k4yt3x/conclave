use std::io::Write;

use crossterm::{
    cursor, queue,
    style::{Attribute, Color, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{Clear, ClearType},
};

use conclave_lib::state::{RoomMember, resolve_sender_name};

use super::input::InputLine;
use super::state::{AppState, ConnectionStatus, DisplayMessage};

fn sanitize_for_terminal(input: &str) -> String {
    conclave_lib::sanitize_control_chars(input)
}

/// Full redraw of the terminal.
pub fn render_full(
    stdout: &mut impl Write,
    state: &AppState,
    input: &InputLine,
) -> std::io::Result<()> {
    let rows = state.terminal_rows;
    let cols = state.terminal_cols;

    queue!(stdout, Clear(ClearType::All))?;

    let members = active_room_members(state);

    // Message area: rows 0..rows-2
    let msg_rows = rows.saturating_sub(2) as usize;
    let messages = state.active_messages();
    let total = messages.len();
    let start = total.saturating_sub(msg_rows + state.scroll_offset);
    let end = total.saturating_sub(state.scroll_offset);

    for (i, msg) in messages[start..end].iter().enumerate() {
        queue!(stdout, cursor::MoveTo(0, i as u16))?;
        write_message(stdout, msg, cols, &members)?;
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
        let name = sanitize_for_terminal(&room.display_name());
        format!("#{name} ({member_count})")
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
        let name = sanitize_for_terminal(&room.display_name());
        format!("[#{name}] ")
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
) -> std::io::Result<()> {
    render_full(stdout, state, input)
}

/// Get the active room's member list (empty slice if no active room).
fn active_room_members(state: &AppState) -> Vec<RoomMember> {
    state
        .active_room
        .and_then(|id| state.rooms.get(&id))
        .map(|r| r.members.clone())
        .unwrap_or_default()
}

/// Write a single formatted message at the current cursor position.
fn write_message(
    stdout: &mut impl Write,
    msg: &DisplayMessage,
    _cols: u16,
    members: &[RoomMember],
) -> std::io::Result<()> {
    let time = chrono::DateTime::from_timestamp(msg.timestamp, 0)
        .map(|dt| dt.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_else(|| "??:??".to_string());

    let content = sanitize_for_terminal(&msg.content);

    if msg.is_system {
        queue!(stdout, SetForegroundColor(Color::DarkYellow))?;
        write!(stdout, "[{time}] *** {content}")?;
        queue!(stdout, ResetColor)?;
    } else {
        let sender = sanitize_for_terminal(&resolve_sender_name(msg, members));
        let nick_color = username_color(msg.sender_id.unwrap_or(0));
        queue!(stdout, SetForegroundColor(Color::DarkGrey))?;
        write!(stdout, "[{time}] ")?;
        queue!(stdout, SetForegroundColor(nick_color))?;
        write!(stdout, "<{sender}>")?;
        queue!(stdout, ResetColor)?;
        write!(stdout, " {content}")?;
    }

    Ok(())
}

/// Assign a consistent ANSI color to a sender based on their user ID.
fn username_color(sender_id: i64) -> Color {
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
    let hash = (sender_id as usize).wrapping_mul(2654435761);
    colors[hash % colors.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_strips_escape_sequences() {
        assert_eq!(sanitize_for_terminal("\x1b[2Jcleared"), "[2Jcleared");
        assert_eq!(sanitize_for_terminal("\x1b[Hhome"), "[Hhome");
        assert_eq!(
            sanitize_for_terminal("\x1b[8mhidden\x1b[0m"),
            "[8mhidden[0m"
        );
        assert_eq!(
            sanitize_for_terminal("before\x1b[31mred\x1b[0mafter"),
            "before[31mred[0mafter"
        );
    }

    #[test]
    fn test_sanitize_preserves_normal_text() {
        assert_eq!(sanitize_for_terminal("hello world"), "hello world");
        assert_eq!(sanitize_for_terminal("café résumé"), "café résumé");
        assert_eq!(sanitize_for_terminal("emoji 🎉🔒"), "emoji 🎉🔒");
        assert_eq!(sanitize_for_terminal("日本語テスト"), "日本語テスト");
        assert_eq!(sanitize_for_terminal("line1\nline2"), "line1\nline2");
    }

    #[test]
    fn test_sanitize_strips_control_chars() {
        assert_eq!(sanitize_for_terminal("\x00null"), "null");
        assert_eq!(sanitize_for_terminal("\x07bell"), "bell");
        assert_eq!(sanitize_for_terminal("\x08backspace"), "backspace");
        assert_eq!(sanitize_for_terminal("\x7fdelete"), "delete");
        assert_eq!(sanitize_for_terminal("\x01\x02\x03"), "");
    }
}
