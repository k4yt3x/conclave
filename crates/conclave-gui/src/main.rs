#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod notification;
mod screen;
mod subscription;
mod theme;
mod widget;

use app::Conclave;
use conclave_client::config::{ClientConfig, acquire_instance_lock};

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                if cfg!(debug_assertions) {
                    "conclave_gui=info".into()
                } else {
                    "conclave_gui=warn".into()
                }
            }),
        )
        .init();

    let config = ClientConfig::load();
    let _lock = match acquire_instance_lock(&config.data_dir) {
        Ok(lock) => lock,
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(1);
        }
    };

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
