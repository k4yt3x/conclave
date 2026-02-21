use std::collections::HashMap;

use iced::Length;
use iced::alignment::{Horizontal, Vertical};
use iced::widget::{
    button, column, container, mouse_area, opaque, row, scrollable, stack, text, text_input,
};

use conclave_lib::state::{ConnectionStatus, DisplayMessage, Room};

use crate::theme;
use crate::widget::Element;
use crate::widget::message_view;

#[derive(Debug, Clone)]
pub enum Message {
    RoomSelected(String),
    InputChanged(String),
    InputSubmitted,
    ToggleUserPopover,
    CloseUserPopover,
    ToggleMembersSidebar,
    Logout,
}

pub struct Dashboard {
    pub input_value: String,
    pub show_user_popover: bool,
    pub show_members_sidebar: bool,
}

impl Dashboard {
    pub fn new() -> Self {
        Self {
            input_value: String::new(),
            show_user_popover: false,
            show_members_sidebar: false,
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
        server_url: &'a Option<String>,
        accept_invalid_certs: bool,
    ) -> Element<'a, Message> {
        let sidebar = self.view_sidebar(
            rooms,
            active_room,
            connection_status,
            username,
            server_url,
            accept_invalid_certs,
        );
        let main_area = self.view_main_area(rooms, active_room, room_messages, system_messages);

        let mut base = row![sidebar, main_area];
        if self.show_members_sidebar && active_room.is_some() {
            base = base.push(self.view_members_sidebar(rooms, active_room));
        }

        if self.show_user_popover {
            let popover = self.view_user_popover(username, server_url);
            stack![base, popover].into()
        } else {
            base.into()
        }
    }

    fn view_sidebar<'a>(
        &'a self,
        rooms: &'a HashMap<String, Room>,
        active_room: &'a Option<String>,
        connection_status: &'a ConnectionStatus,
        username: &'a Option<String>,
        server_url: &'a Option<String>,
        accept_invalid_certs: bool,
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

            let room_button = button(text(label).size(13).width(Length::Fill))
                .width(Length::Fill)
                .padding([6, 10])
                .class(style)
                .on_press(Message::RoomSelected(room.server_group_id.clone()));

            room_list = room_list.push(room_button);
        }

        let status_banner: Option<Element<'a, Message>> = match connection_status {
            ConnectionStatus::Connected => None,
            ConnectionStatus::Connecting => Some(
                container(
                    text("Reconnecting...")
                        .size(12)
                        .width(Length::Fill)
                        .align_x(Horizontal::Center)
                        .class(Box::new(theme::text::on_error) as Box<dyn Fn(&theme::Theme) -> _>),
                )
                .width(Length::Fill)
                .padding([6, 10])
                .class(Box::new(theme::container::error_banner) as Box<dyn Fn(&theme::Theme) -> _>)
                .into(),
            ),
            ConnectionStatus::Disconnected => Some(
                container(
                    text("Disconnected")
                        .size(12)
                        .width(Length::Fill)
                        .align_x(Horizontal::Center)
                        .class(Box::new(theme::text::on_error) as Box<dyn Fn(&theme::Theme) -> _>),
                )
                .width(Length::Fill)
                .padding([6, 10])
                .class(Box::new(theme::container::error_banner) as Box<dyn Fn(&theme::Theme) -> _>)
                .into(),
            ),
        };

        let user_display = username
            .as_ref()
            .map(|u| format!("@{u}"))
            .unwrap_or_default();
        let user_button = button(
            text(user_display)
                .size(14)
                .width(Length::Fill)
                .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>),
        )
        .width(Length::Fill)
        .padding([8, 10])
        .class(Box::new(theme::button::sidebar) as Box<dyn Fn(&theme::Theme, _) -> _>)
        .on_press(Message::ToggleUserPopover);

        let is_insecure = accept_invalid_certs
            || server_url
                .as_ref()
                .is_some_and(|u| u.starts_with("http://"));

        let warning_banner: Option<Element<'a, Message>> =
            if is_insecure && matches!(connection_status, ConnectionStatus::Connected) {
                Some(
                    container(
                        text("Insecure")
                            .size(12)
                            .width(Length::Fill)
                            .align_x(Horizontal::Center)
                            .class(Box::new(theme::text::on_warning)
                                as Box<dyn Fn(&theme::Theme) -> _>),
                    )
                    .width(Length::Fill)
                    .padding([6, 10])
                    .class(Box::new(theme::container::warning_banner)
                        as Box<dyn Fn(&theme::Theme) -> _>)
                    .into(),
                )
            } else {
                None
            };

        let mut footer = column![].spacing(4).padding([8, 0]);
        if let Some(banner) = status_banner {
            footer = footer.push(banner);
        }
        if let Some(banner) = warning_banner {
            footer = footer.push(banner);
        }
        footer = footer.push(user_button);

        let sidebar_content = column![header, scrollable(room_list).height(Length::Fill), footer]
            .height(Length::Fill);

        container(sidebar_content)
            .width(200)
            .height(Length::Fill)
            .class(Box::new(theme::container::sidebar) as Box<dyn Fn(&theme::Theme) -> _>)
            .into()
    }

    fn view_user_popover<'a>(
        &'a self,
        username: &'a Option<String>,
        server_url: &'a Option<String>,
    ) -> Element<'a, Message> {
        let identity_display = format!(
            "{}@{}",
            username.as_deref().unwrap_or(""),
            server_url.as_deref().unwrap_or("")
        );

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

        let card_content = column![
            text(identity_display)
                .size(13)
                .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>),
            logout_btn,
        ]
        .spacing(6)
        .padding(12)
        .width(176);

        let card = container(card_content)
            .class(Box::new(theme::container::card) as Box<dyn Fn(&theme::Theme) -> _>);

        let positioned_card = container(opaque(card))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(Vertical::Bottom)
            .padding(iced::Padding::ZERO.bottom(40).left(12));

        opaque(mouse_area(positioned_card).on_press(Message::CloseUserPopover)).into()
    }

    fn view_main_area<'a>(
        &'a self,
        rooms: &'a HashMap<String, Room>,
        active_room: &'a Option<String>,
        room_messages: &'a HashMap<String, Vec<DisplayMessage>>,
        system_messages: &'a [DisplayMessage],
    ) -> Element<'a, Message> {
        let messages = self.view_messages(active_room, room_messages, system_messages);
        let input = self.view_input();

        let mut main_column = column![].width(Length::Fill).height(Length::Fill);
        if active_room.is_some() {
            main_column = main_column.push(self.view_title_bar(rooms, active_room));
        }
        main_column = main_column.push(messages).push(input);
        main_column.into()
    }

    fn view_title_bar<'a>(
        &'a self,
        rooms: &'a HashMap<String, Room>,
        active_room: &'a Option<String>,
    ) -> Element<'a, Message> {
        let toggle_label = if self.show_members_sidebar { ">" } else { "<" };
        let toggle_btn = button(
            text(toggle_label)
                .size(14)
                .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>),
        )
        .padding([4, 8])
        .class(Box::new(theme::button::sidebar) as Box<dyn Fn(&theme::Theme, _) -> _>)
        .on_press(Message::ToggleMembersSidebar);

        let content: Element<'a, Message> = match active_room {
            Some(room_id) => {
                if let Some(room) = rooms.get(room_id) {
                    let name = text(format!("#{}", room.name))
                        .size(14)
                        .class(Box::new(theme::text::primary) as Box<dyn Fn(&theme::Theme) -> _>);
                    let members = text(format!("  ({} members)", room.members.len()))
                        .size(12)
                        .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>);
                    row![
                        name,
                        members,
                        iced::widget::Space::new().width(Length::Fill),
                        toggle_btn
                    ]
                    .align_y(Vertical::Center)
                    .into()
                } else {
                    row![
                        text("Unknown room")
                            .size(14)
                            .class(Box::new(theme::text::muted) as Box<dyn Fn(&theme::Theme) -> _>),
                        iced::widget::Space::new().width(Length::Fill),
                        toggle_btn,
                    ]
                    .align_y(Vertical::Center)
                    .into()
                }
            }
            None => text("").size(14).into(),
        };

        container(content)
            .padding([8, 12])
            .width(Length::Fill)
            .class(Box::new(theme::container::title_bar) as Box<dyn Fn(&theme::Theme) -> _>)
            .into()
    }

    fn view_members_sidebar<'a>(
        &'a self,
        rooms: &'a HashMap<String, Room>,
        active_room: &'a Option<String>,
    ) -> Element<'a, Message> {
        let mut member_list = column![].spacing(2).padding([8, 12]);

        if let Some(room) = active_room.as_ref().and_then(|id| rooms.get(id)) {
            let mut sorted_indices: Vec<usize> = (0..room.members.len()).collect();
            sorted_indices.sort_by(|a, b| room.members[*a].cmp(&room.members[*b]));
            for index in sorted_indices {
                member_list = member_list.push(
                    text(&room.members[index])
                        .size(13)
                        .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>),
                );
            }
        }

        container(scrollable(member_list).height(Length::Fill))
            .width(180)
            .height(Length::Fill)
            .class(Box::new(theme::container::sidebar) as Box<dyn Fn(&theme::Theme) -> _>)
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
