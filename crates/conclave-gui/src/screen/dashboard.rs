use std::collections::HashMap;

use iced::Length;
use iced::alignment::{Horizontal, Vertical};
use iced::widget::{button, column, container, row, scrollable, text, text_input};

use conclave_lib::state::{ConnectionStatus, DisplayMessage, Room};

use crate::theme;
use crate::widget::Element;
use crate::widget::message_view;

#[derive(Debug, Clone)]
pub enum Message {
    RoomSelected(String),
    InputChanged(String),
    InputSubmitted,
    Logout,
}

pub struct Dashboard {
    pub input_value: String,
}

impl Dashboard {
    pub fn new() -> Self {
        Self {
            input_value: String::new(),
        }
    }

    pub fn view<'a>(
        &'a self,
        rooms: &'a HashMap<String, Room>,
        active_room: &'a Option<String>,
        room_messages: &'a HashMap<String, Vec<DisplayMessage>>,
        system_messages: &'a [DisplayMessage],
        connection_status: &'a ConnectionStatus,
        username: &'a Option<String>,
    ) -> Element<'a, Message> {
        let sidebar = self.view_sidebar(rooms, active_room, connection_status, username);
        let main_area = self.view_main_area(rooms, active_room, room_messages, system_messages);

        row![sidebar, main_area].into()
    }

    fn view_sidebar<'a>(
        &'a self,
        rooms: &'a HashMap<String, Room>,
        active_room: &'a Option<String>,
        connection_status: &'a ConnectionStatus,
        username: &'a Option<String>,
    ) -> Element<'a, Message> {
        let header = container(
            text("Rooms")
                .size(14)
                .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>),
        )
        .padding(12);

        let mut room_list = column![].spacing(2).padding([0, 8]);

        let mut sorted_rooms: Vec<_> = rooms.values().collect();
        sorted_rooms.sort_by(|a, b| a.name.cmp(&b.name));

        for room in sorted_rooms {
            let is_active = active_room.as_ref() == Some(&room.server_group_id);
            let unread = room.last_seen_seq.saturating_sub(room.last_read_seq);

            let label = if unread > 0 {
                format!("# {} ({})", room.name, unread)
            } else {
                format!("# {}", room.name)
            };

            let style: Box<dyn Fn(&theme::Theme, _) -> _> = if is_active {
                Box::new(theme::button::sidebar_active)
            } else {
                Box::new(theme::button::sidebar)
            };

            let btn = button(text(label).size(13).width(Length::Fill))
                .width(Length::Fill)
                .padding([6, 10])
                .class(style)
                .on_press(Message::RoomSelected(room.server_group_id.clone()));

            room_list = room_list.push(btn);
        }

        let status_indicator = {
            let (label, style): (&str, Box<dyn Fn(&theme::Theme) -> _>) = match connection_status {
                ConnectionStatus::Connected => ("Connected", Box::new(theme::text::success)),
                ConnectionStatus::Connecting => ("Connecting...", Box::new(theme::text::secondary)),
                ConnectionStatus::Disconnected => ("Disconnected", Box::new(theme::text::error)),
            };
            text(label).size(11).class(style)
        };

        let user_label = text(username.as_deref().unwrap_or(""))
            .size(12)
            .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>);

        let logout_btn = button(
            text("Logout")
                .size(12)
                .align_x(Horizontal::Center)
                .width(Length::Fill),
        )
        .width(Length::Fill)
        .padding(6)
        .class(Box::new(theme::button::danger) as Box<dyn Fn(&theme::Theme, _) -> _>)
        .on_press(Message::Logout);

        let footer = column![status_indicator, user_label, logout_btn]
            .spacing(4)
            .padding(12);

        let sidebar_content = column![header, scrollable(room_list).height(Length::Fill), footer,]
            .height(Length::Fill);

        container(sidebar_content)
            .width(200)
            .height(Length::Fill)
            .class(Box::new(theme::container::sidebar) as Box<dyn Fn(&theme::Theme) -> _>)
            .into()
    }

    fn view_main_area<'a>(
        &'a self,
        rooms: &'a HashMap<String, Room>,
        active_room: &'a Option<String>,
        room_messages: &'a HashMap<String, Vec<DisplayMessage>>,
        system_messages: &'a [DisplayMessage],
    ) -> Element<'a, Message> {
        let title_bar = self.view_title_bar(rooms, active_room);
        let messages = self.view_messages(active_room, room_messages, system_messages);
        let input = self.view_input();

        column![title_bar, messages, input]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_title_bar<'a>(
        &'a self,
        rooms: &'a HashMap<String, Room>,
        active_room: &'a Option<String>,
    ) -> Element<'a, Message> {
        let content: Element<'a, Message> = match active_room {
            Some(room_id) => {
                if let Some(room) = rooms.get(room_id) {
                    let name = text(format!("#{}", room.name))
                        .size(14)
                        .class(Box::new(theme::text::primary) as Box<dyn Fn(&theme::Theme) -> _>);
                    let members = text(format!("  ({} members)", room.members.len()))
                        .size(12)
                        .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>);
                    row![name, members].align_y(Vertical::Center).into()
                } else {
                    text("Unknown room")
                        .size(14)
                        .class(Box::new(theme::text::muted) as Box<dyn Fn(&theme::Theme) -> _>)
                        .into()
                }
            }
            None => text("No room selected — use /join or click a room")
                .size(14)
                .class(Box::new(theme::text::muted) as Box<dyn Fn(&theme::Theme) -> _>)
                .into(),
        };

        container(content)
            .padding([8, 12])
            .width(Length::Fill)
            .class(Box::new(theme::container::title_bar) as Box<dyn Fn(&theme::Theme) -> _>)
            .into()
    }

    fn view_messages<'a>(
        &'a self,
        active_room: &'a Option<String>,
        room_messages: &'a HashMap<String, Vec<DisplayMessage>>,
        system_messages: &'a [DisplayMessage],
    ) -> Element<'a, Message> {
        let messages: &[DisplayMessage] = match active_room {
            Some(room_id) => room_messages
                .get(room_id)
                .map(|v| v.as_slice())
                .unwrap_or(&[]),
            None => system_messages,
        };

        let msg_column: iced::widget::Column<'_, Message, theme::Theme, crate::widget::Renderer> =
            message_view::message_list(messages);

        let content = container(msg_column.padding([4, 12])).width(Length::Fill);

        scrollable(content)
            .height(Length::Fill)
            .anchor_bottom()
            .into()
    }

    fn view_input(&self) -> Element<'_, Message> {
        let input = text_input("Type a message or /command...", &self.input_value)
            .on_input(Message::InputChanged)
            .on_submit(Message::InputSubmitted)
            .padding(10)
            .size(14)
            .class(Box::new(theme::text_input::chat_input) as Box<dyn Fn(&theme::Theme, _) -> _>);

        container(input)
            .width(Length::Fill)
            .class(Box::new(theme::container::input_area) as Box<dyn Fn(&theme::Theme) -> _>)
            .into()
    }
}
