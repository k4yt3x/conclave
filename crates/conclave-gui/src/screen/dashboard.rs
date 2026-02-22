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

pub const SIDEBAR_MIN_WIDTH: f32 = 100.0;
pub const SIDEBAR_MAX_WIDTH: f32 = 500.0;
const DRAG_HANDLE_WIDTH: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DragTarget {
    LeftHandle,
    RightHandle,
}

#[derive(Debug, Clone)]
pub enum Message {
    RoomSelected(i64),
    InputChanged(String),
    InputSubmitted,
    ToggleUserPopover,
    CloseUserPopover,
    ToggleMembersSidebar,
    CopyText(String),
    DismissToast,
    Logout,
    DragStarted(DragTarget),
    DragUpdate(f32),
    DragEnded,
}

pub struct Dashboard {
    pub input_value: String,
    pub show_user_popover: bool,
    pub show_members_sidebar: bool,
    pub toast: Option<String>,
    pub left_sidebar_width: f32,
    pub right_sidebar_width: f32,
    pub dragging: Option<DragTarget>,
    pub last_drag_x: f32,
}

impl Dashboard {
    pub fn new() -> Self {
        Self {
            input_value: String::new(),
            show_user_popover: false,
            show_members_sidebar: false,
            toast: None,
            left_sidebar_width: 200.0,
            right_sidebar_width: 180.0,
            dragging: None,
            last_drag_x: 0.0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn view<'a>(
        &'a self,
        rooms: &'a HashMap<i64, Room>,
        active_room: &'a Option<i64>,
        room_messages: &'a HashMap<i64, Vec<DisplayMessage>>,
        system_messages: &'a [DisplayMessage],
        connection_status: &'a ConnectionStatus,
        username: &'a Option<String>,
        user_alias: &'a Option<String>,
        user_id: &'a Option<i64>,
        server_url: &'a Option<String>,
        accept_invalid_certs: bool,
        theme: &'a crate::theme::Theme,
    ) -> Element<'a, Message> {
        let sidebar = self.view_sidebar(
            rooms,
            active_room,
            connection_status,
            username,
            user_alias,
            server_url,
            accept_invalid_certs,
        );
        let left_handle = self.view_drag_handle(DragTarget::LeftHandle);
        let main_area =
            self.view_main_area(rooms, active_room, room_messages, system_messages, theme);

        let mut base = row![sidebar, left_handle, main_area];
        if self.show_members_sidebar && active_room.is_some() {
            let right_handle = self.view_drag_handle(DragTarget::RightHandle);
            base = base.push(right_handle);
            base = base.push(self.view_members_sidebar(rooms, active_room));
        }

        if self.dragging.is_some() {
            let overlay = mouse_area(
                container(iced::widget::Space::new())
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_move(|point| Message::DragUpdate(point.x))
            .on_release(Message::DragEnded)
            .interaction(iced::mouse::Interaction::ResizingHorizontally);
            stack![base, overlay].into()
        } else if self.show_user_popover {
            let popover = self.view_user_popover(username, user_alias, user_id, server_url);
            stack![base, popover].into()
        } else {
            base.into()
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn view_sidebar<'a>(
        &'a self,
        rooms: &'a HashMap<i64, Room>,
        active_room: &'a Option<i64>,
        connection_status: &'a ConnectionStatus,
        username: &'a Option<String>,
        user_alias: &'a Option<String>,
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
        sorted_rooms.sort_by_key(|a| a.display_name());

        for room in sorted_rooms {
            let is_active = active_room == &Some(room.server_group_id);
            let unread = room.last_seen_seq.saturating_sub(room.last_read_seq);

            let room_display = match room.alias.as_deref().filter(|a| !a.is_empty()) {
                Some(alias) => format!("{alias} (#{})", room.group_name),
                None => format!("#{}", room.group_name),
            };

            let label = if unread > 0 {
                format!("{room_display} ({unread})")
            } else {
                room_display
            };

            let style: Box<dyn Fn(&theme::Theme, _) -> _> = if is_active {
                Box::new(theme::button::sidebar_active)
            } else {
                Box::new(theme::button::sidebar)
            };

            let room_button = button(
                text(label)
                    .size(13)
                    .width(Length::Fill)
                    .wrapping(text::Wrapping::None),
            )
            .width(Length::Fill)
            .padding([6, 10])
            .class(style)
            .on_press(Message::RoomSelected(room.server_group_id));

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

        let user_display = match (
            user_alias.as_deref().filter(|a| !a.is_empty()),
            username.as_deref(),
        ) {
            (Some(alias), Some(uname)) => format!("{alias} (@{uname})"),
            (None, Some(uname)) => format!("@{uname}"),
            _ => String::new(),
        };
        let user_button = button(
            text(user_display)
                .size(14)
                .width(Length::Fill)
                .wrapping(text::Wrapping::None)
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
            .width(self.left_sidebar_width)
            .height(Length::Fill)
            .clip(true)
            .class(Box::new(theme::container::sidebar) as Box<dyn Fn(&theme::Theme) -> _>)
            .into()
    }

    fn view_drag_handle(&self, target: DragTarget) -> Element<'_, Message> {
        let handle = container(
            iced::widget::Space::new()
                .width(DRAG_HANDLE_WIDTH)
                .height(Length::Fill),
        )
        .width(DRAG_HANDLE_WIDTH)
        .height(Length::Fill)
        .class(Box::new(theme::container::drag_handle) as Box<dyn Fn(&theme::Theme) -> _>);

        mouse_area(handle)
            .interaction(iced::mouse::Interaction::ResizingHorizontally)
            .on_press(Message::DragStarted(target))
            .into()
    }

    fn view_user_popover<'a>(
        &'a self,
        username: &'a Option<String>,
        user_alias: &'a Option<String>,
        user_id: &'a Option<i64>,
        server_url: &'a Option<String>,
    ) -> Element<'a, Message> {
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

        let mut card_content = column![].spacing(4).padding(12);

        if let Some(alias) = user_alias.as_deref().filter(|a| !a.is_empty()) {
            card_content = card_content.push(
                text(alias)
                    .size(14)
                    .class(Box::new(theme::text::primary) as Box<dyn Fn(&theme::Theme) -> _>),
            );
        }

        if let Some(uname) = username.as_deref() {
            let label = match user_id {
                Some(uid) => format!("@{uname} ({uid})"),
                None => format!("@{uname}"),
            };
            card_content = card_content.push(
                text(label)
                    .size(13)
                    .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>),
            );
        }

        if let Some(url) = server_url.as_deref() {
            card_content = card_content.push(
                text(url)
                    .size(12)
                    .class(Box::new(theme::text::muted) as Box<dyn Fn(&theme::Theme) -> _>),
            );
        }

        card_content = card_content.push(logout_btn);

        let card = container(card_content)
            .width(Length::Shrink)
            .class(Box::new(theme::container::card) as Box<dyn Fn(&theme::Theme) -> _>);

        let positioned_card = container(opaque(card))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(Vertical::Bottom)
            .padding(iced::Padding::ZERO.bottom(40).left(12));

        opaque(mouse_area(positioned_card).on_press(Message::CloseUserPopover))
    }

    fn view_main_area<'a>(
        &'a self,
        rooms: &'a HashMap<i64, Room>,
        active_room: &'a Option<i64>,
        room_messages: &'a HashMap<i64, Vec<DisplayMessage>>,
        system_messages: &'a [DisplayMessage],
        theme: &'a crate::theme::Theme,
    ) -> Element<'a, Message> {
        let messages =
            self.view_messages(rooms, active_room, room_messages, system_messages, theme);
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
        rooms: &'a HashMap<i64, Room>,
        active_room: &'a Option<i64>,
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
                    let name = text(format!("#{}", room.display_name()))
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
        rooms: &'a HashMap<i64, Room>,
        active_room: &'a Option<i64>,
    ) -> Element<'a, Message> {
        let member_count = active_room
            .and_then(|id| rooms.get(&id))
            .map(|r| r.members.len())
            .unwrap_or(0);

        let header = container(
            text(format!("{member_count} Members"))
                .size(14)
                .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>),
        )
        .padding(12);

        let mut member_list = column![].spacing(2).padding([0, 12]);

        if let Some(room) = active_room.and_then(|id| rooms.get(&id)) {
            let mut sorted_members: Vec<&_> = room.members.iter().collect();
            sorted_members.sort_by(|a, b| {
                // Sort admins before regular members, then alphabetically.
                let role_ord = if a.role == "admin" { 0 } else { 1 };
                let role_ord_b = if b.role == "admin" { 0 } else { 1 };
                role_ord
                    .cmp(&role_ord_b)
                    .then_with(|| a.display_name().cmp(b.display_name()))
            });
            for member in sorted_members {
                let admin_suffix = if member.role == "admin" { " *" } else { "" };
                let member_display = match member.alias.as_deref().filter(|a| !a.is_empty()) {
                    Some(alias) => format!("{alias} (@{}){admin_suffix}", member.username),
                    None => format!("@{}{admin_suffix}", member.username),
                };
                member_list = member_list.push(
                    text(member_display)
                        .size(13)
                        .wrapping(text::Wrapping::None)
                        .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>),
                );
            }
        }

        let sidebar_content =
            column![header, scrollable(member_list).height(Length::Fill)].height(Length::Fill);

        container(sidebar_content)
            .width(self.right_sidebar_width)
            .height(Length::Fill)
            .clip(true)
            .class(Box::new(theme::container::sidebar) as Box<dyn Fn(&theme::Theme) -> _>)
            .into()
    }

    fn view_messages<'a>(
        &'a self,
        rooms: &'a HashMap<i64, Room>,
        active_room: &'a Option<i64>,
        room_messages: &'a HashMap<i64, Vec<DisplayMessage>>,
        system_messages: &'a [DisplayMessage],
        theme: &'a crate::theme::Theme,
    ) -> Element<'a, Message> {
        let messages: &[DisplayMessage] = match active_room {
            Some(room_id) => room_messages
                .get(room_id)
                .map(|v| v.as_slice())
                .unwrap_or(&[]),
            None => system_messages,
        };

        let members: &[conclave_lib::state::RoomMember] = match active_room {
            Some(room_id) => rooms
                .get(room_id)
                .map(|r| r.members.as_slice())
                .unwrap_or(&[]),
            None => &[],
        };

        let msg_column: iced::widget::Column<'_, Message, theme::Theme, crate::widget::Renderer> =
            message_view::message_list(messages, members, *active_room, theme, Message::CopyText);

        let content = container(msg_column.padding([4, 12])).width(Length::Fill);

        let messages_area = scrollable(content).height(Length::Fill).anchor_bottom();

        if let Some(toast_text) = &self.toast {
            let toast_badge = container(
                text(toast_text.as_str())
                    .size(12)
                    .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>),
            )
            .padding([6, 12])
            .class(Box::new(theme::container::toast) as Box<dyn Fn(&theme::Theme) -> _>);

            let toast_overlay = container(toast_badge)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Bottom)
                .padding(iced::Padding::ZERO.bottom(8));

            stack![messages_area, toast_overlay].into()
        } else {
            messages_area.into()
        }
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
