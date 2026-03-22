use std::collections::HashMap;

use iced::Length;
use iced::Point;
use iced::advanced::text::Span;
use iced::widget::{column, text};
use uuid::Uuid;

use conclave_client::state::{DisplayMessage, RoomMember, VerificationStatus, resolve_sender_name};

use crate::theme;
use crate::widget::Element;
use crate::widget::selectable_rich_text::SelectableRichText;

pub fn message_list<'a, M: Clone + 'a>(
    messages: &'a [DisplayMessage],
    members: &[RoomMember],
    theme: &crate::theme::Theme,
    verification_status: &HashMap<Uuid, VerificationStatus>,
    show_verified_indicator: bool,
    on_link_click: impl Fn(String) -> M + 'a + Clone,
    on_right_click: impl Fn(usize, Point, Option<String>) -> M + 'a + Clone,
) -> iced::widget::Column<'a, M, crate::theme::Theme, crate::widget::Renderer> {
    let mut messages_column = column![].spacing(2).width(Length::Fill);

    for (index, message) in messages.iter().enumerate() {
        let time = format_timestamp(message.timestamp);

        let row_element: Element<'a, M> = if message.is_system {
            let spans = vec![
                Span::new(format!("[{time}] "))
                    .size(13)
                    .color(theme.text_muted),
                Span::new(format!("*** {}", message.content))
                    .size(13)
                    .color(theme.text_secondary),
            ];
            let on_right_click = on_right_click.clone();
            SelectableRichText::new(spans)
                .width(Length::Fill)
                .wrapping(text::Wrapping::Glyph)
                .selection_color(theme.selection)
                .on_right_click(move |point, link| on_right_click(index, point, link))
                .into()
        } else {
            let sender_name = resolve_sender_name(message, members);
            let nick_color = theme::nick_color(message.sender_id.unwrap_or(Uuid::nil()));

            let mut spans = Vec::with_capacity(4);
            spans.push(
                Span::new(format!("[{time}]"))
                    .size(13)
                    .color(theme.text_muted),
            );

            spans.push(
                Span::new(format!(" <{sender_name}>"))
                    .size(13)
                    .color(nick_color),
            );

            // Add verification indicator after the sender name.
            if let Some(sid) = message.sender_id {
                match verification_status.get(&sid) {
                    Some(VerificationStatus::Changed) => {
                        spans.push(Span::new(" [!]").size(13).color(theme.indicator_risky));
                    }
                    Some(VerificationStatus::Unknown | VerificationStatus::Unverified) => {
                        spans.push(Span::new(" [?]").size(13).color(theme.indicator_unverified));
                    }
                    Some(VerificationStatus::Verified) if show_verified_indicator => {
                        spans.push(
                            Span::new(" [\u{2713}]")
                                .size(13)
                                .color(theme.indicator_verified),
                        );
                    }
                    Some(VerificationStatus::Verified) | None => {}
                }
            }

            spans.push(Span::new(" ").size(13));

            // Split content into plain text and URL segments.
            for (segment, is_url) in split_urls(&message.content) {
                let mut span = Span::new(segment).size(13).color(theme.text);
                if is_url {
                    span = span.underline(true).link(segment.to_string());
                }
                spans.push(span);
            }

            let on_right_click = on_right_click.clone();
            SelectableRichText::new(spans)
                .width(Length::Fill)
                .wrapping(text::Wrapping::Glyph)
                .selection_color(theme.selection)
                .on_link_click(on_link_click.clone())
                .on_right_click(move |point, link| on_right_click(index, point, link))
                .into()
        };

        messages_column = messages_column.push(row_element);
    }

    messages_column
}

/// Split text into segments, tagging each as URL or plain text.
/// URLs are detected by `https://` or `http://` prefixes and end at
/// whitespace or trailing punctuation that is unlikely to be part of the URL.
fn split_urls(text: &str) -> Vec<(&str, bool)> {
    let mut segments = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        // Find the next URL start.
        let url_start = remaining
            .find("https://")
            .or_else(|| remaining.find("http://"));

        match url_start {
            Some(start) => {
                // Push any text before the URL.
                if start > 0 {
                    segments.push((&remaining[..start], false));
                }

                // Find the end of the URL (first whitespace or end of string).
                let url_part = &remaining[start..];
                let end = url_part
                    .find(|c: char| c.is_whitespace())
                    .unwrap_or(url_part.len());

                // Trim trailing punctuation that's likely not part of the URL.
                let url = &url_part[..end];
                let trimmed_end = url
                    .trim_end_matches(|c: char| matches!(c, '.' | ',' | ')' | ']' | ';' | '!'))
                    .len();

                segments.push((&url_part[..trimmed_end], true));
                remaining = &url_part[trimmed_end..];
            }
            None => {
                segments.push((remaining, false));
                break;
            }
        }
    }

    segments
}

pub fn format_timestamp(ts: i64) -> String {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "??:??:??".to_string())
}

pub fn format_message_details(
    msg: &DisplayMessage,
    members: &[RoomMember],
    group_id: Option<Uuid>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_urls_no_urls() {
        let result = split_urls("hello world");
        assert_eq!(result, vec![("hello world", false)]);
    }

    #[test]
    fn test_split_urls_single_url() {
        let result = split_urls("check https://example.com please");
        assert_eq!(
            result,
            vec![
                ("check ", false),
                ("https://example.com", true),
                (" please", false),
            ]
        );
    }

    #[test]
    fn test_split_urls_url_at_start() {
        let result = split_urls("https://example.com is great");
        assert_eq!(
            result,
            vec![("https://example.com", true), (" is great", false)]
        );
    }

    #[test]
    fn test_split_urls_url_at_end() {
        let result = split_urls("visit https://example.com");
        assert_eq!(
            result,
            vec![("visit ", false), ("https://example.com", true)]
        );
    }

    #[test]
    fn test_split_urls_multiple_urls() {
        let result = split_urls("see https://a.com and http://b.com");
        assert_eq!(
            result,
            vec![
                ("see ", false),
                ("https://a.com", true),
                (" and ", false),
                ("http://b.com", true),
            ]
        );
    }

    #[test]
    fn test_split_urls_trailing_punctuation() {
        let result = split_urls("see https://example.com.");
        assert_eq!(
            result,
            vec![("see ", false), ("https://example.com", true), (".", false)]
        );
    }

    #[test]
    fn test_split_urls_url_with_path() {
        let result = split_urls("visit https://example.com/path?q=1&r=2#frag");
        assert_eq!(
            result,
            vec![
                ("visit ", false),
                ("https://example.com/path?q=1&r=2#frag", true),
            ]
        );
    }

    #[test]
    fn test_split_urls_empty() {
        let result = split_urls("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_split_urls_only_url() {
        let result = split_urls("https://example.com");
        assert_eq!(result, vec![("https://example.com", true)]);
    }
}
