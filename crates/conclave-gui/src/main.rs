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

    iced::application(Conclave::new, Conclave::update, Conclave::view)
        .title(Conclave::title)
        .theme(Conclave::theme)
        .subscription(Conclave::subscription)
        .window_size((900.0, 600.0))
        .default_font(iced::Font::MONOSPACE)
        .run()
}
