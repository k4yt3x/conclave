use iced::Length;
use iced::alignment::{Horizontal, Vertical};
use iced::widget::{Space, button, column, container, row, text, text_input};

use crate::theme;
use crate::widget::Element;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Login,
    Register,
}

#[derive(Debug, Clone)]
pub enum Status {
    Idle,
    Loading,
    Error(String),
}

#[derive(Debug, Clone)]
pub enum Message {
    ServerUrlChanged(String),
    UsernameChanged(String),
    PasswordChanged(String),
    Submit,
    ToggleMode,
    FocusUsername,
    FocusPassword,
}

pub struct Login {
    pub server_url: String,
    pub username: String,
    pub password: String,
    pub status: Status,
    pub mode: Mode,
}

impl Login {
    pub fn new(server_url: String) -> Self {
        Self {
            server_url,
            username: String::new(),
            password: String::new(),
            status: Status::Idle,
            mode: Mode::Login,
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let title = text("Conclave")
            .size(28)
            .class(Box::new(theme::text::primary) as Box<dyn Fn(&theme::Theme) -> _>);

        let subtitle = text("End-to-end encrypted messaging")
            .size(14)
            .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>);

        let server_input = text_input("Server URL", &self.server_url)
            .id("login_server_url")
            .on_input(Message::ServerUrlChanged)
            .on_submit(Message::FocusUsername)
            .padding(10)
            .size(14);

        let username_input = text_input("Username", &self.username)
            .id("login_username")
            .on_input(Message::UsernameChanged)
            .on_submit(Message::FocusPassword)
            .padding(10)
            .size(14);

        let password_input = text_input("Password", &self.password)
            .id("login_password")
            .on_input(Message::PasswordChanged)
            .on_submit(Message::Submit)
            .secure(true)
            .padding(10)
            .size(14);

        let is_loading = matches!(self.status, Status::Loading);

        let submit_label = match (&self.mode, is_loading) {
            (Mode::Login, false) => "Login",
            (Mode::Login, true) => "Logging in...",
            (Mode::Register, false) => "Register",
            (Mode::Register, true) => "Registering...",
        };

        let submit_button = button(
            text(submit_label)
                .size(14)
                .align_x(Horizontal::Center)
                .width(Length::Fill)
                .class(Box::new(theme::text::on_primary) as Box<dyn Fn(&theme::Theme) -> _>),
        )
        .width(Length::Fill)
        .padding(10)
        .on_press_maybe(if is_loading {
            None
        } else {
            Some(Message::Submit)
        });

        let toggle_label = match self.mode {
            Mode::Login => "Switch to Register",
            Mode::Register => "Switch to Login",
        };

        let toggle_button = button(
            text(toggle_label)
                .size(14)
                .align_x(Horizontal::Center)
                .width(Length::Fill),
        )
        .width(Length::Fill)
        .padding(10)
        .class(Box::new(theme::button::secondary) as Box<dyn Fn(&theme::Theme, _) -> _>)
        .on_press_maybe(if is_loading {
            None
        } else {
            Some(Message::ToggleMode)
        });

        let status_text: Element<'_, Message> = match &self.status {
            Status::Idle => Space::new().height(0).into(),
            Status::Loading => text("Connecting...")
                .size(13)
                .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>)
                .into(),
            Status::Error(msg) => text(msg.clone())
                .size(13)
                .class(Box::new(theme::text::error) as Box<dyn Fn(&theme::Theme) -> _>)
                .into(),
        };

        let form = column![
            title,
            subtitle,
            Space::new().height(16),
            text("Server URL")
                .size(12)
                .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>),
            server_input,
            Space::new().height(8),
            text("Username")
                .size(12)
                .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>),
            username_input,
            Space::new().height(8),
            text("Password")
                .size(12)
                .class(Box::new(theme::text::secondary) as Box<dyn Fn(&theme::Theme) -> _>),
            password_input,
            Space::new().height(16),
            row![submit_button, toggle_button].spacing(8),
            Space::new().height(8),
            status_text,
        ]
        .spacing(4)
        .max_width(400);

        let card = container(form)
            .padding(32)
            .class(Box::new(theme::container::card) as Box<dyn Fn(&theme::Theme) -> _>);

        container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .class(Box::new(theme::container::background) as Box<dyn Fn(&theme::Theme) -> _>)
            .into()
    }
}
