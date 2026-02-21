pub mod button;
pub mod config;
pub mod container;
pub mod scrollable;
pub mod text;
pub mod text_input;

use iced::Color;

/// Dark theme for the Conclave GUI (greyscale palette).
#[derive(Debug, Clone)]
pub struct Theme {
    pub background: Color,
    pub surface: Color,
    pub surface_bright: Color,
    pub primary: Color,
    pub text: Color,
    pub text_secondary: Color,
    pub text_muted: Color,
    pub error: Color,
    pub on_error: Color,
    pub warning: Color,
    pub on_warning: Color,
    pub success: Color,
    pub border: Color,
    pub scrollbar: Color,
    pub selection: Color,
    pub title_bar: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: Color::from_rgb8(0x15, 0x15, 0x15),
            surface: Color::from_rgb8(0x0c, 0x12, 0x1a),
            surface_bright: Color::from_rgb8(0x31, 0x31, 0x31),
            primary: Color::from_rgb8(0xd7, 0xd7, 0xd7),
            text: Color::from_rgb8(0xd7, 0xd7, 0xd7),
            text_secondary: Color::from_rgb8(0xab, 0xab, 0xab),
            text_muted: Color::from_rgb8(0x80, 0x80, 0x80),
            error: Color::from_rgb8(0xaa, 0x40, 0x40),
            on_error: Color::from_rgb8(0xff, 0xff, 0xff),
            warning: Color::from_rgb8(0xd4, 0x9e, 0x3c),
            on_warning: Color::from_rgb8(0x00, 0x00, 0x00),
            success: Color::from_rgb8(0xe0, 0xc5, 0x78),
            border: Color::from_rgb8(0x20, 0x20, 0x20),
            scrollbar: Color::from_rgb8(0x31, 0x31, 0x31),
            selection: Color::from_rgb8(0x3f, 0x3f, 0x3f),
            title_bar: Color::from_rgb8(0x20, 0x20, 0x20),
        }
    }
}

impl iced::theme::Base for Theme {
    fn base(&self) -> iced::theme::Style {
        iced::theme::Style {
            background_color: self.background,
            text_color: self.text,
        }
    }

    fn palette(&self) -> Option<iced::theme::Palette> {
        None
    }

    fn default(_preference: iced::theme::Mode) -> Self {
        <Self as Default>::default()
    }

    fn mode(&self) -> iced::theme::Mode {
        iced::theme::Mode::Dark
    }

    fn name(&self) -> &str {
        "Conclave Greyscale"
    }
}

/// Assign a consistent color to a sender based on their user ID.
pub fn nick_color(sender_id: i64) -> Color {
    let colors = [
        Color::from_rgb8(0xe0, 0x6b, 0x75), // red
        Color::from_rgb8(0xb1, 0xb6, 0x95), // green
        Color::from_rgb8(0xfe, 0xcd, 0xb2), // yellow/cream
        Color::from_rgb8(0x87, 0xb0, 0xf9), // blue
        Color::from_rgb8(0xd7, 0xbd, 0xe2), // magenta
        Color::from_rgb8(0x94, 0xe2, 0xd5), // cyan
        Color::from_rgb8(0xf2, 0x8f, 0xad), // pink
        Color::from_rgb8(0xef, 0x9f, 0x76), // orange
        Color::from_rgb8(0xa6, 0xd1, 0x89), // mint
        Color::from_rgb8(0xca, 0x9e, 0xe6), // lavender
        Color::from_rgb8(0xe5, 0xc8, 0x90), // gold
        Color::from_rgb8(0x8c, 0xaa, 0xee), // periwinkle
    ];
    let hash = (sender_id as usize).wrapping_mul(2654435761);
    colors[hash % colors.len()]
}
