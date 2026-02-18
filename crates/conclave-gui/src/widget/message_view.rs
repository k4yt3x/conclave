use iced::Length;
use iced::widget::{column, row, text};

use conclave_lib::state::DisplayMessage;

use crate::theme;
use crate::widget::Element;

/// Render a column of messages.
pub fn message_list<'a, M: Clone + 'a>(
    messages: &[DisplayMessage],
) -> iced::widget::Column<'a, M, crate::theme::Theme, crate::widget::Renderer> {
    let mut messages_column = column![].spacing(2).width(Length::Fill);

    for message in messages {
        let time = format_timestamp(message.timestamp);

        let row_element: Element<'a, M> = if message.is_system {
            row![
                text(format!("[{time}]"))
                    .size(13)
                    .class(Box::new(theme::text::muted) as Box<dyn Fn(&theme::Theme) -> _>),
                text(format!(" *** {}", message.content))
                    .size(13)
                    .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>),
            ]
            .into()
        } else {
            let nick_color = theme::nick_color(&message.sender);
            row![
                text(format!("[{time}]"))
                    .size(13)
                    .class(Box::new(theme::text::muted) as Box<dyn Fn(&theme::Theme) -> _>),
                text(format!(" <{}>", message.sender))
                    .size(13)
                    .class(Box::new(move |_theme: &theme::Theme| {
                        iced::widget::text::Style {
                            color: Some(nick_color),
                        }
                    }) as Box<dyn Fn(&theme::Theme) -> _>),
                text(format!(" {}", message.content))
                    .size(13)
                    .class(Box::new(theme::text::primary) as Box<dyn Fn(&theme::Theme) -> _>),
            ]
            .into()
        };

        messages_column = messages_column.push(row_element);
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
