use iced::Length;
use iced::widget::{column, row, text};

use conclave_lib::state::DisplayMessage;

use crate::theme;
use crate::widget::Element;

/// Render a column of messages.
pub fn message_list<'a, M: Clone + 'a>(
    messages: &[DisplayMessage],
) -> iced::widget::Column<'a, M, crate::theme::Theme, crate::widget::Renderer> {
    let mut col = column![].spacing(2).width(Length::Fill);

    for msg in messages {
        let time = format_timestamp(msg.timestamp);

        let row_el: Element<'a, M> = if msg.is_system {
            row![
                text(format!("[{time}]"))
                    .size(13)
                    .class(Box::new(theme::text::muted) as Box<dyn Fn(&theme::Theme) -> _>),
                text(format!(" *** {}", msg.content))
                    .size(13)
                    .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>),
            ]
            .into()
        } else {
            let nick_color = theme::nick_color(&msg.sender);
            row![
                text(format!("[{time}]"))
                    .size(13)
                    .class(Box::new(theme::text::muted) as Box<dyn Fn(&theme::Theme) -> _>),
                text(format!(" <{}>", msg.sender)).size(13).class(Box::new(
                    move |_theme: &theme::Theme| {
                        iced::widget::text::Style {
                            color: Some(nick_color),
                        }
                    }
                )
                    as Box<dyn Fn(&theme::Theme) -> _>),
                text(format!(" {}", msg.content))
                    .size(13)
                    .class(Box::new(theme::text::primary) as Box<dyn Fn(&theme::Theme) -> _>),
            ]
            .into()
        };

        col = col.push(row_el);
    }

    col
}

fn format_timestamp(ts: i64) -> String {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "??:??:??".to_string())
}
