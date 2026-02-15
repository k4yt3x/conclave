pub mod button;
pub mod container;
pub mod scrollable;
pub mod text;
pub mod text_input;

use iced::Color;

/// Dark theme for the Conclave GUI (Ferra-inspired palette).
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
    pub success: Color,
    pub border: Color,
    pub scrollbar: Color,
    pub selection: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: Color::from_rgb8(0x2b, 0x29, 0x2d),
            surface: Color::from_rgb8(0x24, 0x22, 0x26),
            surface_bright: Color::from_rgb8(0x32, 0x30, 0x34),
            primary: Color::from_rgb8(0xfe, 0xcd, 0xb2),
            text: Color::from_rgb8(0xfe, 0xcd, 0xb2),
            text_secondary: Color::from_rgb8(0xab, 0x8a, 0x79),
            text_muted: Color::from_rgb8(0x68, 0x56, 0x50),
            error: Color::from_rgb8(0xe0, 0x6b, 0x75),
            success: Color::from_rgb8(0xb1, 0xb6, 0x95),
            border: Color::from_rgb8(0x4f, 0x47, 0x4d),
            scrollbar: Color::from_rgb8(0x32, 0x30, 0x34),
            selection: Color::from_rgb8(0x45, 0x3d, 0x41),
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
        "Conclave Dark"
    }
}

/// Assign a consistent color to a username based on hash.
/// Same algorithm as the TUI's username_color function.
pub fn nick_color(username: &str) -> Color {
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
    let hash: usize = username.bytes().fold(0usize, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(b as usize)
    });
    colors[hash % colors.len()]
}
