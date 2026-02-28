use std::time::Duration;

use iced::Length;
use iced::advanced::text::Span;
use iced::widget::{column, container, text, tooltip};

use conclave_client::state::{DisplayMessage, RoomMember, resolve_sender_name};

use crate::theme;
use crate::widget::Element;
use crate::widget::selectable_rich_text::SelectableRichText;

pub fn message_list<'a, M: Clone + 'a>(
    messages: &'a [DisplayMessage],
    members: &[RoomMember],
    group_id: Option<i64>,
    group_name: Option<&str>,
    theme: &crate::theme::Theme,
) -> iced::widget::Column<'a, M, crate::theme::Theme, crate::widget::Renderer> {
    let mut messages_column = column![].spacing(2).width(Length::Fill);

    for message in messages {
        let time = format_timestamp(message.timestamp);
        let tooltip_text = format_tooltip(message, members, group_id, group_name);

        let row_element: Element<'a, M> = if message.is_system {
            let spans = vec![
                Span::new(format!("[{time}] "))
                    .size(13)
                    .color(theme.text_muted),
                Span::new(format!("*** {}", message.content))
                    .size(13)
                    .color(theme.text_secondary),
            ];
            SelectableRichText::new(spans)
                .width(Length::Fill)
                .wrapping(text::Wrapping::Glyph)
                .selection_color(theme.selection)
                .into()
        } else {
            let sender_name = resolve_sender_name(message, members);
            let nick_color = theme::nick_color(message.sender_id.unwrap_or(0));
            let spans = vec![
                Span::new(format!("[{time}]"))
                    .size(13)
                    .color(theme.text_muted),
                Span::new(format!(" <{sender_name}> "))
                    .size(13)
                    .color(nick_color),
                Span::new(message.content.as_str())
                    .size(13)
                    .color(theme.text),
            ];
            SelectableRichText::new(spans)
                .width(Length::Fill)
                .wrapping(text::Wrapping::Glyph)
                .selection_color(theme.selection)
                .into()
        };

        let tooltip_content = container(
            text(tooltip_text)
                .size(12)
                .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>),
        )
        .padding(6)
        .class(Box::new(theme::container::tooltip) as Box<dyn Fn(&theme::Theme) -> _>);

        let with_tooltip: Element<'a, M> =
            tooltip(row_element, tooltip_content, tooltip::Position::FollowCursor)
                .delay(Duration::from_millis(300))
                .into();

        messages_column = messages_column.push(with_tooltip);
    }

    messages_column
}

fn format_timestamp(ts: i64) -> String {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "??:??:??".to_string())
}

fn format_tooltip(
    msg: &DisplayMessage,
    members: &[RoomMember],
    group_id: Option<i64>,
    group_name: Option<&str>,
) -> String {
    use chrono::{Local, TimeZone};
    let datetime = Local
        .timestamp_opt(msg.timestamp, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "Unknown time".to_string());

    let mut lines = vec![format!("Timestamp: {datetime}")];

    if !msg.is_system
        && let Some(sid) = msg.sender_id
    {
        lines.push(format!("Sender ID: {sid}"));
        let member = members.iter().find(|m| m.user_id == sid);
        let username = member.map(|m| m.username.as_str()).unwrap_or(&msg.sender);
        lines.push(format!("Sender username: {username}"));
        if let Some(alias) = member
            .and_then(|m| m.alias.as_deref())
            .filter(|a| !a.is_empty())
        {
            lines.push(format!("Sender alias: {alias}"));
        }
    }

    if let Some(gid) = group_id {
        lines.push(format!("Group ID: {gid}"));
    }

    if let Some(name) = group_name {
        lines.push(format!("Group name: {name}"));
    }

    if let Some(seq) = msg.sequence_num {
        lines.push(format!("Sequence: {seq}"));
    }

    if let Some(epoch) = msg.epoch {
        lines.push(format!("Epoch: {epoch}"));
    }

    lines.join("\n")
}
