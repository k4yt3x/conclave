use std::io::Write;

use crossterm::{
    cursor, queue,
    style::{Attribute, Color, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor},
    terminal::{Clear, ClearType},
};
use uuid::Uuid;

use conclave_client::state::{
    INDICATOR_COLOR_RISKY, INDICATOR_COLOR_UNVERIFIED, INDICATOR_COLOR_VERIFIED, RoomMember,
    RoomTrustLevel, VerificationStatus, resolve_sender_name, room_trust_level,
};

use super::input::InputLine;
use super::state::{AppState, ConnectionStatus, DisplayMessage, InputMode};

/// Return the verification indicator prefix for a user.
fn verification_indicator(
    sender_id: Option<Uuid>,
    verification_status: &std::collections::HashMap<Uuid, VerificationStatus>,
    show_verified: bool,
) -> &'static str {
    match sender_id.and_then(|id| verification_status.get(&id)) {
        Some(VerificationStatus::Changed) => "[!] ",
        Some(VerificationStatus::Unknown | VerificationStatus::Unverified) => "[?] ",
        Some(VerificationStatus::Verified) if show_verified => "[\u{2713}] ",
        Some(VerificationStatus::Verified) | None => "",
    }
}

/// Return the color for a verification indicator.
fn verification_color(
    sender_id: Option<Uuid>,
    verification_status: &std::collections::HashMap<Uuid, VerificationStatus>,
    show_verified: bool,
) -> Option<Color> {
    match sender_id.and_then(|id| verification_status.get(&id)) {
        Some(VerificationStatus::Changed) => {
            let (r, g, b) = INDICATOR_COLOR_RISKY;
            Some(Color::Rgb { r, g, b })
        }
        Some(VerificationStatus::Unknown | VerificationStatus::Unverified) => {
            let (r, g, b) = INDICATOR_COLOR_UNVERIFIED;
            Some(Color::Rgb { r, g, b })
        }
        Some(VerificationStatus::Verified) if show_verified => {
            let (r, g, b) = INDICATOR_COLOR_VERIFIED;
            Some(Color::Rgb { r, g, b })
        }
        Some(VerificationStatus::Verified) | None => None,
    }
}

fn sanitize_for_terminal(input: &str) -> String {
    conclave_client::sanitize_control_chars(input)
}

fn format_time(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|dt| dt.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_else(|| "??:??".to_string())
}

/// Approximate display width using character count.
fn display_width(text: &str) -> usize {
    text.chars().count()
}

/// Number of terminal rows needed to display a line of given width.
fn rows_for_width(width: usize, cols: usize) -> u16 {
    if cols == 0 || width == 0 {
        return 1;
    }
    ((width + cols - 1) / cols) as u16
}

/// Width of the message prefix in terminal columns.
fn message_prefix_width(
    msg: &DisplayMessage,
    members: &[RoomMember],
    verification_status: &std::collections::HashMap<Uuid, VerificationStatus>,
    show_verified: bool,
) -> usize {
    // "[HH:MM] " = 8 columns (time is always 5 chars: HH:MM or ??:??)
    let time_prefix = 8;
    if msg.is_system {
        // "*** " = 4
        time_prefix + 4
    } else {
        let sender = sanitize_for_terminal(&resolve_sender_name(msg, members));
        let indicator = verification_indicator(msg.sender_id, verification_status, show_verified);
        // "<sender> " = 1 + sender_width + 2
        time_prefix + display_width(indicator) + 1 + display_width(&sender) + 2
    }
}

/// Compute how many terminal rows a message occupies.
fn visual_line_count(
    msg: &DisplayMessage,
    cols: u16,
    members: &[RoomMember],
    verification_status: &std::collections::HashMap<Uuid, VerificationStatus>,
    show_verified: bool,
) -> usize {
    let cols_usize = cols as usize;
    if cols_usize == 0 {
        return 1;
    }

    let prefix_width = message_prefix_width(msg, members, verification_status, show_verified);
    let content = sanitize_for_terminal(&msg.content);

    let mut total_rows: usize = 0;
    for (i, line) in content.split('\n').enumerate() {
        let line_width = if i == 0 {
            prefix_width + display_width(line)
        } else {
            display_width(line)
        };
        total_rows += rows_for_width(line_width, cols_usize) as usize;
    }

    total_rows.max(1)
}

const MAX_INPUT_HEIGHT: u16 = 10;

/// Number of terminal rows the input area occupies.
fn input_line_count(input: &InputLine) -> u16 {
    let count = input.content().split('\n').count().max(1) as u16;
    count.min(MAX_INPUT_HEIGHT)
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

    // Layout: messages | status bar | input area (1+ rows)
    let input_height = input_line_count(input);
    let status_row = rows.saturating_sub(1 + input_height);
    let input_row = rows.saturating_sub(input_height);
    let msg_rows = status_row as usize;

    let messages = state.active_messages();
    let total = messages.len();
    let end = total.saturating_sub(state.scroll_offset);

    // Walk backward from end to find which messages fit in the available rows.
    let mut lines_accumulated = 0;
    let mut start = end;
    while start > 0 {
        let msg_lines = visual_line_count(
            &messages[start - 1],
            cols,
            &members,
            &state.verification_status,
            state.show_verified_indicator,
        );
        if lines_accumulated + msg_lines > msg_rows && start < end {
            break;
        }
        lines_accumulated += msg_lines;
        start -= 1;
    }

    // Render messages top-to-bottom.
    let mut current_row = 0u16;
    for msg in &messages[start..end] {
        let rows_used = write_message(
            stdout,
            msg,
            cols,
            &members,
            &state.verification_status,
            state.show_verified_indicator,
            current_row,
        )?;
        current_row += rows_used;
    }

    render_status_line(stdout, state, status_row)?;
    render_input_area(stdout, state, input, input_row)?;

    stdout.flush()
}

/// Render the status line (reverse video bar).
pub fn render_status_line(
    stdout: &mut impl Write,
    state: &AppState,
    row: u16,
) -> std::io::Result<()> {
    queue!(stdout, cursor::MoveTo(0, row))?;

    // Room trust badge (colored background) before the reverse-video status bar.
    let badge_width = if let Some(room) = state.active_room_info() {
        let trust = room_trust_level(&room.members, &state.verification_status);
        if matches!(trust, RoomTrustLevel::Verified) && !state.show_verified_indicator {
            0
        } else {
            let (bg, label) = match trust {
                RoomTrustLevel::Risky => (INDICATOR_COLOR_RISKY, "[!]"),
                RoomTrustLevel::Unverified => (INDICATOR_COLOR_UNVERIFIED, "[?]"),
                RoomTrustLevel::Verified => (INDICATOR_COLOR_VERIFIED, "\u{2713}"),
            };
            let (r, g, b) = bg;
            queue!(
                stdout,
                SetBackgroundColor(Color::Rgb { r, g, b }),
                SetForegroundColor(Color::White),
            )?;
            write!(stdout, " {label} ")?;
            queue!(stdout, ResetColor)?;
            // " [?] " = 5 display columns, " ✓ " = 3 display columns
            if matches!(trust, RoomTrustLevel::Verified) {
                3
            } else {
                5
            }
        }
    } else {
        0
    };

    queue!(stdout, SetAttribute(Attribute::Reverse))?;

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

    // Pad to full width, accounting for the trust badge.
    let padding = (state.terminal_cols as usize).saturating_sub(status.len() + badge_width);
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

    let (prefix, display_content) = password_or_normal_input(state, input);

    write!(stdout, "{prefix}{display_content}")?;

    // Position cursor correctly.
    let cursor_col = prefix.len() + input.cursor_position();
    queue!(stdout, cursor::MoveTo(cursor_col as u16, row))?;

    stdout.flush()
}

/// Render a multi-line input area starting at `row`.
pub fn render_input_area(
    stdout: &mut impl Write,
    state: &AppState,
    input: &InputLine,
    row: u16,
) -> std::io::Result<()> {
    // In password prompt mode, render a single masked line.
    if !matches!(state.input_mode, InputMode::Normal) {
        return render_input_line(stdout, state, input, row);
    }

    let prefix = if let Some(room) = state.active_room_info() {
        let name = sanitize_for_terminal(&room.display_name());
        format!("[#{name}] ")
    } else {
        String::from("> ")
    };

    let content = input.content();
    let lines: Vec<&str> = content.split('\n').collect();
    let input_height = input_line_count(input);

    for (i, line) in lines.iter().enumerate().take(input_height as usize) {
        let current_row = row + i as u16;
        queue!(
            stdout,
            cursor::MoveTo(0, current_row),
            Clear(ClearType::CurrentLine),
        )?;
        if i == 0 {
            write!(stdout, "{prefix}{line}")?;
        } else {
            write!(stdout, "{line}")?;
        }
    }

    // Compute cursor row and column from the character offset.
    let cursor_pos = input.cursor_position();
    let mut chars_remaining = cursor_pos;
    let mut cursor_line = 0usize;
    let mut cursor_col = 0usize;

    for (i, line) in lines.iter().enumerate() {
        let line_char_count = line.chars().count();
        if chars_remaining <= line_char_count {
            cursor_line = i;
            cursor_col = chars_remaining;
            break;
        }
        // +1 for the '\n' separator
        chars_remaining -= line_char_count + 1;
    }

    let display_col = if cursor_line == 0 {
        prefix.len() + cursor_col
    } else {
        cursor_col
    };

    let cursor_row = row + cursor_line.min(input_height.saturating_sub(1) as usize) as u16;
    queue!(stdout, cursor::MoveTo(display_col as u16, cursor_row))?;

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

/// Return the (prefix, display_content) for the input line, handling
/// password prompt mode (masked with '*') vs normal mode.
fn password_or_normal_input(state: &AppState, input: &InputLine) -> (String, String) {
    if let InputMode::PasswordPrompt { ref stage, .. } = state.input_mode {
        let prefix = stage.label().to_string();
        let masked = "*".repeat(input.content().chars().count());
        (prefix, masked)
    } else {
        let prefix = if let Some(room) = state.active_room_info() {
            let name = sanitize_for_terminal(&room.display_name());
            format!("[#{name}] ")
        } else {
            String::from("> ")
        };
        let content = input.content();
        (prefix, content)
    }
}

/// Get the active room's member list (empty slice if no active room).
fn active_room_members(state: &AppState) -> Vec<RoomMember> {
    state
        .active_room
        .and_then(|id| state.rooms.get(&id))
        .map(|r| r.members.clone())
        .unwrap_or_default()
}

/// Write a single formatted message starting at `row`, handling embedded
/// newlines by rendering each sub-line on its own terminal row with a
/// consistent indent. Returns the number of rows consumed.
fn write_message(
    stdout: &mut impl Write,
    msg: &DisplayMessage,
    cols: u16,
    members: &[RoomMember],
    verification_status: &std::collections::HashMap<Uuid, VerificationStatus>,
    show_verified: bool,
    row: u16,
) -> std::io::Result<u16> {
    let cols_usize = cols as usize;
    let time = format_time(msg.timestamp);
    let content = sanitize_for_terminal(&msg.content);
    let prefix_width = message_prefix_width(msg, members, verification_status, show_verified);

    let mut current_row = row;

    if msg.is_system {
        queue!(stdout, SetForegroundColor(Color::DarkYellow))?;
        for (i, line) in content.split('\n').enumerate() {
            queue!(stdout, cursor::MoveTo(0, current_row))?;
            if i == 0 {
                write!(stdout, "[{time}] *** {line}")?;
                current_row += rows_for_width(prefix_width + display_width(line), cols_usize);
            } else {
                write!(stdout, "{line}")?;
                current_row += rows_for_width(display_width(line), cols_usize);
            }
        }
        queue!(stdout, ResetColor)?;
    } else {
        let sender = sanitize_for_terminal(&resolve_sender_name(msg, members));
        let nick_color = username_color(msg.sender_id.unwrap_or(Uuid::nil()));
        let indicator = verification_indicator(msg.sender_id, verification_status, show_verified);

        for (i, line) in content.split('\n').enumerate() {
            queue!(stdout, cursor::MoveTo(0, current_row))?;
            if i == 0 {
                queue!(stdout, SetForegroundColor(Color::DarkGrey))?;
                write!(stdout, "[{time}] ")?;
                if !indicator.is_empty() {
                    if let Some(color) =
                        verification_color(msg.sender_id, verification_status, show_verified)
                    {
                        queue!(stdout, SetForegroundColor(color))?;
                    }
                    write!(stdout, "{indicator}")?;
                }
                queue!(stdout, SetForegroundColor(nick_color))?;
                write!(stdout, "<{sender}>")?;
                queue!(stdout, ResetColor)?;
                write!(stdout, " {line}")?;
                current_row += rows_for_width(prefix_width + display_width(line), cols_usize);
            } else {
                write!(stdout, "{line}")?;
                current_row += rows_for_width(display_width(line), cols_usize);
            }
        }
    }

    Ok(current_row.saturating_sub(row).max(1))
}

/// Assign a consistent ANSI color to a sender based on their user ID.
fn username_color(sender_id: Uuid) -> Color {
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
    let hash = (sender_id.as_u128() as usize).wrapping_mul(2654435761);
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

    #[test]
    fn test_rows_for_width() {
        assert_eq!(rows_for_width(0, 80), 1);
        assert_eq!(rows_for_width(40, 80), 1);
        assert_eq!(rows_for_width(80, 80), 1);
        assert_eq!(rows_for_width(81, 80), 2);
        assert_eq!(rows_for_width(160, 80), 2);
        assert_eq!(rows_for_width(161, 80), 3);
        assert_eq!(rows_for_width(50, 0), 1);
    }

    #[test]
    fn test_visual_line_count_single_line() {
        let msg = DisplayMessage::system("hello");
        let members = vec![];
        // "[HH:MM] *** hello" = 12 + 5 = 17 chars, fits in 80 cols
        assert_eq!(
            visual_line_count(&msg, 80, &members, &std::collections::HashMap::new(), false),
            1
        );
    }

    #[test]
    fn test_visual_line_count_multiline() {
        let msg = DisplayMessage::system("first\nsecond\nthird");
        let members = vec![];
        // 3 lines, each fits in 80 cols
        assert_eq!(
            visual_line_count(&msg, 80, &members, &std::collections::HashMap::new(), false),
            3
        );
    }

    #[test]
    fn test_visual_line_count_wrapping() {
        let long = "a".repeat(100);
        let msg = DisplayMessage::system(&long);
        let members = vec![];
        // "[HH:MM] *** " (12) + 100 = 112 chars, ceil(112/80) = 2
        assert_eq!(
            visual_line_count(&msg, 80, &members, &std::collections::HashMap::new(), false),
            2
        );
    }

    #[test]
    fn test_visual_line_count_empty_content() {
        let msg = DisplayMessage::system("");
        let members = vec![];
        // "[HH:MM] *** " = 12 chars, fits in 80 cols
        assert_eq!(
            visual_line_count(&msg, 80, &members, &std::collections::HashMap::new(), false),
            1
        );
    }

    #[test]
    fn test_visual_line_count_trailing_newline() {
        let msg = DisplayMessage::system("hello\n");
        let members = vec![];
        // "hello" on line 1 (with prefix), "" on line 2 (no prefix, 0 width → 1 row)
        assert_eq!(
            visual_line_count(&msg, 80, &members, &std::collections::HashMap::new(), false),
            2
        );
    }

    #[test]
    fn test_visual_line_count_continuation_no_prefix() {
        // Continuation lines have no prefix, so a 100-char continuation
        // wraps based on its own width, not prefix + width.
        let long = format!("short\n{}", "a".repeat(100));
        let msg = DisplayMessage::system(&long);
        let members = vec![];
        // Line 1: "[HH:MM] *** short" = 12 + 5 = 17, 1 row
        // Line 2: "aaa...a" = 100 chars (no prefix), ceil(100/80) = 2 rows
        assert_eq!(
            visual_line_count(&msg, 80, &members, &std::collections::HashMap::new(), false),
            3
        );
    }
}
