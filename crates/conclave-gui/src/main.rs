#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod screen;
mod subscription;
mod theme;
mod widget;

use app::Conclave;

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "conclave_gui=info".into()),
        )
        .init();

    let icon =
        iced::window::icon::from_file_data(include_bytes!("../../../assets/conclave.png"), None)
            .ok();

    iced::application(Conclave::new, Conclave::update, Conclave::view)
        .title(Conclave::title)
        .theme(Conclave::theme)
        .subscription(Conclave::subscription)
        .window(iced::window::Settings {
            icon,
            size: iced::Size::new(900.0, 600.0),
            ..Default::default()
        })
        .default_font(iced::Font::MONOSPACE)
        .run()
}
