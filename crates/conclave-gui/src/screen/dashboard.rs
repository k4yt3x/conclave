use std::collections::HashMap;

use iced::alignment::{Horizontal, Vertical};
use iced::keyboard;
use iced::widget::{
    button, column, container, mouse_area, opaque, row, scrollable, stack, text, text_editor,
    text_input,
};
use iced::{Length, Point};
use uuid::Uuid;

use conclave_client::state::{
    ConnectionStatus, DisplayMessage, Room, RoomTrustLevel, VerificationStatus, room_trust_level,
};

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

#[derive(Debug, Clone, Default)]
pub struct PasswordChangeDialog {
    pub current_password: String,
    pub new_password: String,
    pub confirm_password: String,
    pub error: Option<String>,
    pub loading: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    RoomSelected(Uuid),
    InputAction(text_editor::Action),
    InputSubmitted,
    ToggleUserPopover,
    CloseUserPopover,
    ToggleMembersSidebar,
    SelectedText(Vec<(f32, String)>),
    Logout,
    DragStarted(DragTarget),
    DragUpdate(f32),
    DragEnded,
    VerifyResult(Result<(Option<(Uuid, VerificationStatus)>, Vec<DisplayMessage>), String>),
    PasswordDialogCurrentChanged(String),
    PasswordDialogNewChanged(String),
    PasswordDialogConfirmChanged(String),
    PasswordDialogSubmit,
    PasswordDialogCancel,
    PasswordDialogResult(Result<(), String>),
    LinkClicked(String),
    MessageContextMenu(usize, Point, Option<String>),
    CloseContextMenu,
    CopyToClipboard(String),
    OpenLink(String),
    ShowProperties,
    CloseProperties,
}

pub struct Dashboard {
    pub input_content: text_editor::Content,
    pub show_user_popover: bool,
    pub show_members_sidebar: bool,
    pub left_sidebar_width: f32,
    pub right_sidebar_width: f32,
    pub dragging: Option<DragTarget>,
    pub last_drag_x: f32,
    pub show_password_dialog: bool,
    pub password_dialog: PasswordChangeDialog,
    pub context_menu: Option<ContextMenuState>,
    pub properties_text: Option<String>,
}

pub struct ContextMenuState {
    pub position: Point,
    pub message_content: String,
    pub link_url: Option<String>,
    pub details_text: String,
}

impl Dashboard {
    pub fn new() -> Self {
        Self {
            input_content: text_editor::Content::new(),
            show_user_popover: false,
            show_members_sidebar: false,
            left_sidebar_width: 200.0,
            right_sidebar_width: 180.0,
            dragging: None,
            last_drag_x: 0.0,
            show_password_dialog: false,
            context_menu: None,
            properties_text: None,
            password_dialog: PasswordChangeDialog::default(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn view<'a>(
        &'a self,
        rooms: &'a HashMap<Uuid, Room>,
        active_room: &'a Option<Uuid>,
        room_messages: &'a HashMap<Uuid, Vec<DisplayMessage>>,
        system_messages: &'a [DisplayMessage],
        connection_status: &'a ConnectionStatus,
        username: &'a Option<String>,
        user_alias: &'a Option<String>,
        user_id: &'a Option<Uuid>,
        server_url: &'a Option<String>,
        accept_invalid_certs: bool,
        theme: &'a crate::theme::Theme,
        verification_status: &'a HashMap<Uuid, VerificationStatus>,
        show_verified_indicator: bool,
        window_size: iced::Size,
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
        let main_area = self.view_main_area(
            rooms,
            active_room,
            room_messages,
            system_messages,
            theme,
            verification_status,
            show_verified_indicator,
        );

        let mut base = row![sidebar, left_handle, main_area];
        if self.show_members_sidebar && active_room.is_some() {
            let right_handle = self.view_drag_handle(DragTarget::RightHandle);
            base = base.push(right_handle);
            base = base.push(self.view_members_sidebar(
                rooms,
                active_room,
                verification_status,
                show_verified_indicator,
            ));
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
        } else if self.show_password_dialog {
            let dialog = self.view_password_dialog();
            stack![base, dialog].into()
        } else if self.show_user_popover {
            let popover = self.view_user_popover(username, user_alias, user_id, server_url);
            stack![base, popover].into()
        } else if let Some(details) = &self.properties_text {
            let dialog = self.view_properties_dialog(details);
            stack![base, dialog].into()
        } else if let Some(ctx) = &self.context_menu {
            let menu = self.view_context_menu(ctx, window_size.width, window_size.height);
            stack![base, menu].into()
        } else {
            base.into()
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn view_sidebar<'a>(
        &'a self,
        rooms: &'a HashMap<Uuid, Room>,
        active_room: &'a Option<Uuid>,
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
        user_id: &'a Option<Uuid>,
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

    fn view_password_dialog(&self) -> Element<'_, Message> {
        use iced::widget::Space;

        let title = text("Change Password")
            .size(18)
            .class(Box::new(theme::text::primary) as Box<dyn Fn(&theme::Theme) -> _>);

        let current_input = text_input("Current password", &self.password_dialog.current_password)
            .on_input(Message::PasswordDialogCurrentChanged)
            .secure(true)
            .padding(10)
            .size(14);

        let new_input = text_input("New password", &self.password_dialog.new_password)
            .on_input(Message::PasswordDialogNewChanged)
            .secure(true)
            .padding(10)
            .size(14);

        let confirm_input = text_input(
            "Confirm new password",
            &self.password_dialog.confirm_password,
        )
        .on_input(Message::PasswordDialogConfirmChanged)
        .on_submit(Message::PasswordDialogSubmit)
        .secure(true)
        .padding(10)
        .size(14);

        let submit_label = if self.password_dialog.loading {
            "Changing..."
        } else {
            "Change Password"
        };

        let submit_btn = button(
            text(submit_label)
                .size(14)
                .align_x(Horizontal::Center)
                .width(Length::Fill)
                .class(Box::new(theme::text::on_primary) as Box<dyn Fn(&theme::Theme) -> _>),
        )
        .width(Length::Fill)
        .padding(10)
        .on_press_maybe(if self.password_dialog.loading {
            None
        } else {
            Some(Message::PasswordDialogSubmit)
        });

        let cancel_btn = button(
            text("Cancel")
                .size(14)
                .align_x(Horizontal::Center)
                .width(Length::Fill),
        )
        .width(Length::Fill)
        .padding(10)
        .class(Box::new(theme::button::secondary) as Box<dyn Fn(&theme::Theme, _) -> _>)
        .on_press_maybe(if self.password_dialog.loading {
            None
        } else {
            Some(Message::PasswordDialogCancel)
        });

        let mut form = column![
            title,
            Space::new().height(16),
            text("Current Password")
                .size(12)
                .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>),
            current_input,
            Space::new().height(8),
            text("New Password")
                .size(12)
                .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>),
            new_input,
            Space::new().height(8),
            text("Confirm New Password")
                .size(12)
                .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>),
            confirm_input,
            Space::new().height(16),
            row![submit_btn, cancel_btn].spacing(8),
        ]
        .spacing(4)
        .max_width(400);

        if let Some(error) = &self.password_dialog.error {
            form = form.push(Space::new().height(8));
            form = form.push(
                text(error.clone())
                    .size(13)
                    .class(Box::new(theme::text::error) as Box<dyn Fn(&theme::Theme) -> _>),
            );
        }

        let card = container(form)
            .padding(32)
            .class(Box::new(theme::container::card) as Box<dyn Fn(&theme::Theme) -> _>);

        let centered = container(opaque(card))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center);

        opaque(mouse_area(centered).on_press(Message::PasswordDialogCancel))
    }

    fn view_context_menu<'a>(
        &'a self,
        ctx: &ContextMenuState,
        window_width: f32,
        window_height: f32,
    ) -> crate::widget::Element<'a, Message> {
        let menu_item = |label: &str, msg: Message| {
            button(
                text(label.to_string())
                    .size(13)
                    .class(Box::new(theme::text::primary) as Box<dyn Fn(&theme::Theme) -> _>),
            )
            .on_press(msg)
            .padding([4, 12])
            .width(Length::Fill)
            .class(Box::new(theme::button::context_menu_item) as Box<dyn Fn(&theme::Theme, _) -> _>)
        };

        let menu_width: f32 = 200.0;
        let mut items = column![].spacing(0).width(menu_width);

        if let Some(url) = &ctx.link_url {
            items = items.push(menu_item("Open Link", Message::OpenLink(url.clone())));
            items = items.push(menu_item(
                "Copy Link",
                Message::CopyToClipboard(url.clone()),
            ));
        }

        items = items.push(menu_item(
            "Copy Message",
            Message::CopyToClipboard(ctx.message_content.clone()),
        ));

        items = items.push(menu_item("Properties", Message::ShowProperties));

        let card = container(items)
            .padding(4)
            .class(Box::new(theme::container::context_menu) as Box<dyn Fn(&theme::Theme) -> _>);

        // Estimate menu height for adaptive positioning.
        let item_count = if ctx.link_url.is_some() { 4 } else { 2 };
        let menu_height = item_count as f32 * 28.0 + 8.0;

        // Flip menu direction when near screen edges.
        let open_left = ctx.position.x + menu_width > window_width;
        let open_up = ctx.position.y + menu_height > window_height;

        let x = if open_left {
            (ctx.position.x - menu_width).max(0.0)
        } else {
            ctx.position.x
        };
        let y = if open_up {
            (ctx.position.y - menu_height).max(0.0)
        } else {
            ctx.position.y
        };

        let positioned = container(opaque(card))
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(iced::Padding {
                top: y,
                right: 0.0,
                bottom: 0.0,
                left: x,
            });

        mouse_area(positioned)
            .on_press(Message::CloseContextMenu)
            .on_right_press(Message::CloseContextMenu)
            .into()
    }

    fn view_properties_dialog<'a>(&'a self, details: &str) -> crate::widget::Element<'a, Message> {
        let content = column![
            text(details.to_string())
                .size(12)
                .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>),
            iced::widget::Space::new().height(8),
            row![
                button(
                    text("Copy")
                        .size(11)
                        .class(Box::new(theme::text::primary) as Box<dyn Fn(&theme::Theme) -> _>),
                )
                .on_press(Message::CopyToClipboard(details.to_string()))
                .padding([4, 12])
                .class(
                    Box::new(theme::button::secondary) as Box<dyn Fn(&theme::Theme, _) -> _>,
                ),
                button(
                    text("Close")
                        .size(11)
                        .class(Box::new(theme::text::primary) as Box<dyn Fn(&theme::Theme) -> _>),
                )
                .on_press(Message::CloseProperties)
                .padding([4, 12])
                .class(
                    Box::new(theme::button::secondary) as Box<dyn Fn(&theme::Theme, _) -> _>,
                ),
            ]
            .spacing(8),
        ]
        .spacing(4)
        .align_x(Horizontal::Right)
        .max_width(360);

        let card = container(content)
            .padding(20)
            .class(Box::new(theme::container::card) as Box<dyn Fn(&theme::Theme) -> _>);

        let centered = container(opaque(card))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center);

        opaque(mouse_area(centered).on_press(Message::CloseProperties))
    }

    #[allow(clippy::too_many_arguments)]
    fn view_main_area<'a>(
        &'a self,
        rooms: &'a HashMap<Uuid, Room>,
        active_room: &'a Option<Uuid>,
        room_messages: &'a HashMap<Uuid, Vec<DisplayMessage>>,
        system_messages: &'a [DisplayMessage],
        theme: &'a crate::theme::Theme,
        verification_status: &'a HashMap<Uuid, VerificationStatus>,
        show_verified_indicator: bool,
    ) -> Element<'a, Message> {
        let messages = self.view_messages(
            rooms,
            active_room,
            room_messages,
            system_messages,
            theme,
            verification_status,
            show_verified_indicator,
        );
        let input = self.view_input();

        let mut main_column = column![].width(Length::Fill).height(Length::Fill);
        if active_room.is_some() {
            main_column = main_column.push(self.view_title_bar(
                rooms,
                active_room,
                verification_status,
                show_verified_indicator,
            ));
        }
        main_column = main_column.push(messages).push(input);
        main_column.into()
    }

    fn view_title_bar<'a>(
        &'a self,
        rooms: &'a HashMap<Uuid, Room>,
        active_room: &'a Option<Uuid>,
        verification_status: &'a HashMap<Uuid, VerificationStatus>,
        show_verified_indicator: bool,
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
                    let trust = room_trust_level(&room.members, verification_status);

                    let name = text(format!("#{}", room.display_name()))
                        .size(14)
                        .class(Box::new(theme::text::primary) as Box<dyn Fn(&theme::Theme) -> _>);
                    let count = room.members.len();
                    let label = if count == 1 { "member" } else { "members" };
                    let members = text(format!("  ({count} {label})"))
                        .size(12)
                        .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>);

                    let title_bar =
                        container(
                            row![
                                name,
                                members,
                                iced::widget::Space::new().width(Length::Fill),
                                toggle_btn
                            ]
                            .align_y(Vertical::Center),
                        )
                        .padding([8, 12])
                        .width(Length::Fill)
                        .class(Box::new(theme::container::title_bar)
                            as Box<dyn Fn(&theme::Theme) -> _>);

                    let show_badge =
                        !matches!(trust, RoomTrustLevel::Verified) || show_verified_indicator;

                    if show_badge {
                        let trust_label = match trust {
                            RoomTrustLevel::Risky => "!",
                            RoomTrustLevel::Unverified => "?",
                            RoomTrustLevel::Verified => "\u{2713}",
                        };
                        let trust_style: Box<dyn Fn(&theme::Theme) -> _> = match trust {
                            RoomTrustLevel::Risky => Box::new(theme::container::trust_risky),
                            RoomTrustLevel::Unverified => {
                                Box::new(theme::container::trust_unverified)
                            }
                            RoomTrustLevel::Verified => Box::new(theme::container::trust_verified),
                        };
                        let badge_size = 30.0;
                        let trust_square = container(
                            text(trust_label)
                                .size(16)
                                .class(Box::new(theme::text::on_error)
                                    as Box<dyn Fn(&theme::Theme) -> _>),
                        )
                        .width(Length::Fixed(badge_size))
                        .height(Length::Fixed(badge_size))
                        .align_x(Horizontal::Center)
                        .align_y(Vertical::Center)
                        .class(trust_style);

                        row![trust_square, title_bar]
                            .align_y(Vertical::Center)
                            .into()
                    } else {
                        title_bar.into()
                    }
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
            .width(Length::Fill)
            .class(Box::new(theme::container::title_bar) as Box<dyn Fn(&theme::Theme) -> _>)
            .into()
    }

    fn view_members_sidebar<'a>(
        &'a self,
        rooms: &'a HashMap<Uuid, Room>,
        active_room: &'a Option<Uuid>,
        verification_status: &'a HashMap<Uuid, VerificationStatus>,
        show_verified_indicator: bool,
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
                let indicator = match verification_status.get(&member.user_id) {
                    Some(VerificationStatus::Changed) => "[!] ",
                    Some(VerificationStatus::Unknown | VerificationStatus::Unverified) => "[?] ",
                    Some(VerificationStatus::Verified) if show_verified_indicator => "[\u{2713}] ",
                    Some(VerificationStatus::Verified) | None => "",
                };
                let member_display = match member.alias.as_deref().filter(|a| !a.is_empty()) {
                    Some(alias) => {
                        format!("{indicator}{alias} (@{}){admin_suffix}", member.username)
                    }
                    None => format!("{indicator}@{}{admin_suffix}", member.username),
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

    #[allow(clippy::too_many_arguments)]
    fn view_messages<'a>(
        &'a self,
        rooms: &'a HashMap<Uuid, Room>,
        active_room: &'a Option<Uuid>,
        room_messages: &'a HashMap<Uuid, Vec<DisplayMessage>>,
        system_messages: &'a [DisplayMessage],
        theme: &'a crate::theme::Theme,
        verification_status: &'a HashMap<Uuid, VerificationStatus>,
        show_verified_indicator: bool,
    ) -> Element<'a, Message> {
        let messages: &[DisplayMessage] = match active_room {
            Some(room_id) => room_messages
                .get(room_id)
                .map(|v| v.as_slice())
                .unwrap_or(&[]),
            None => system_messages,
        };

        let active_room_data = active_room.and_then(|id| rooms.get(&id));

        let members: &[conclave_client::state::RoomMember] = active_room_data
            .map(|r| r.members.as_slice())
            .unwrap_or(&[]);

        let msg_column: iced::widget::Column<'_, Message, theme::Theme, crate::widget::Renderer> =
            message_view::message_list(
                messages,
                members,
                theme,
                verification_status,
                show_verified_indicator,
                Message::LinkClicked,
                |index, point, link| Message::MessageContextMenu(index, point, link),
            );

        let content = container(msg_column.padding([4, 12])).width(Length::Fill);

        scrollable(content)
            .height(Length::Fill)
            .anchor_bottom()
            .into()
    }

    fn view_input(&self) -> Element<'_, Message> {
        let editor = text_editor(&self.input_content)
            .placeholder("Type a message or /command...")
            .on_action(Message::InputAction)
            .key_binding(|key_press| {
                use text_editor::{Binding, Motion};

                match key_press.key {
                    keyboard::Key::Named(keyboard::key::Named::Enter) => {
                        if key_press.modifiers.shift() {
                            Some(Binding::Enter)
                        } else {
                            Some(Binding::Custom(Message::InputSubmitted))
                        }
                    }
                    keyboard::Key::Named(keyboard::key::Named::Backspace)
                        if key_press.modifiers.jump() =>
                    {
                        Some(Binding::Sequence(vec![
                            Binding::Select(Motion::WordLeft),
                            Binding::Backspace,
                        ]))
                    }
                    keyboard::Key::Named(keyboard::key::Named::Delete)
                        if key_press.modifiers.jump() =>
                    {
                        Some(Binding::Sequence(vec![
                            Binding::Select(Motion::WordRight),
                            Binding::Delete,
                        ]))
                    }
                    _ if key_press.key.to_latin(key_press.physical_key) == Some('u')
                        && key_press.modifiers.control() =>
                    {
                        Some(Binding::Sequence(vec![
                            Binding::Select(Motion::Home),
                            Binding::Backspace,
                        ]))
                    }
                    _ if key_press.key.to_latin(key_press.physical_key) == Some('k')
                        && key_press.modifiers.control() =>
                    {
                        Some(Binding::Sequence(vec![
                            Binding::Select(Motion::End),
                            Binding::Delete,
                        ]))
                    }
                    _ if key_press.modifiers.alt() => None,
                    _ => Binding::from_key_press(key_press),
                }
            })
            .id("chat_input")
            .padding(10)
            .size(14)
            .height(Length::Shrink)
            .class(Box::new(theme::text_editor::chat_input) as Box<dyn Fn(&theme::Theme, _) -> _>);

        container(editor)
            .width(Length::Fill)
            .max_height(120)
            .class(Box::new(theme::container::input_area) as Box<dyn Fn(&theme::Theme) -> _>)
            .into()
    }
}
