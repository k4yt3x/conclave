use iced::widget::button::{Catalog, Status, Style};
use iced::{Background, Border, Color};

use super::Theme;

impl Catalog for Theme {
    type Class<'a> = Box<dyn Fn(&Theme, Status) -> Style + 'a>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(primary)
    }

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style {
        class(self, status)
    }
}

pub fn primary(theme: &Theme, status: Status) -> Style {
    let base = Style {
        background: Some(Background::Color(theme.primary)),
        text_color: theme.surface,
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        ..Style::default()
    };

    match status {
        Status::Active => base,
        Status::Hovered => Style {
            background: Some(Background::Color(lighten(theme.primary, 0.1))),
            ..base
        },
        Status::Pressed => Style {
            background: Some(Background::Color(darken(theme.primary, 0.1))),
            ..base
        },
        Status::Disabled => Style {
            background: Some(Background::Color(theme.text_muted)),
            text_color: theme.surface,
            ..base
        },
    }
}

pub fn secondary(theme: &Theme, status: Status) -> Style {
    let base = Style {
        background: Some(Background::Color(theme.surface_bright)),
        text_color: theme.text,
        border: Border {
            radius: 4.0.into(),
            color: theme.border,
            width: 1.0,
        },
        ..Style::default()
    };

    match status {
        Status::Active => base,
        Status::Hovered => Style {
            background: Some(Background::Color(lighten(theme.surface_bright, 0.05))),
            ..base
        },
        Status::Pressed => Style {
            background: Some(Background::Color(darken(theme.surface_bright, 0.05))),
            ..base
        },
        Status::Disabled => Style {
            text_color: theme.text_muted,
            ..base
        },
    }
}

pub fn sidebar(theme: &Theme, status: Status) -> Style {
    let base = Style {
        background: None,
        text_color: theme.text_secondary,
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        ..Style::default()
    };

    match status {
        Status::Active => base,
        Status::Hovered => Style {
            background: Some(Background::Color(theme.surface_bright)),
            text_color: theme.text,
            ..base
        },
        Status::Pressed => Style {
            background: Some(Background::Color(theme.selection)),
            text_color: theme.text,
            ..base
        },
        Status::Disabled => base,
    }
}

pub fn sidebar_active(theme: &Theme, status: Status) -> Style {
    let base = Style {
        background: Some(Background::Color(theme.selection)),
        text_color: theme.text,
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        ..Style::default()
    };

    match status {
        Status::Active => base,
        Status::Hovered => Style {
            background: Some(Background::Color(lighten(theme.selection, 0.05))),
            ..base
        },
        Status::Pressed | Status::Disabled => base,
    }
}

pub fn danger(theme: &Theme, status: Status) -> Style {
    let base = Style {
        background: Some(Background::Color(Color::TRANSPARENT)),
        text_color: theme.error,
        border: Border {
            radius: 4.0.into(),
            color: theme.error,
            width: 1.0,
        },
        ..Style::default()
    };

    match status {
        Status::Active => base,
        Status::Hovered => Style {
            background: Some(Background::Color(theme.error)),
            text_color: theme.surface,
            ..base
        },
        Status::Pressed => Style {
            background: Some(Background::Color(darken(theme.error, 0.1))),
            text_color: theme.surface,
            ..base
        },
        Status::Disabled => Style {
            text_color: theme.text_muted,
            border: Border {
                color: theme.text_muted,
                ..base.border
            },
            ..base
        },
    }
}

pub fn context_menu_item(theme: &Theme, status: Status) -> Style {
    let base = Style {
        background: None,
        text_color: theme.text,
        border: Border {
            radius: 2.0.into(),
            ..Border::default()
        },
        ..Style::default()
    };

    match status {
        Status::Active => base,
        Status::Hovered => Style {
            background: Some(Background::Color(theme.selection)),
            ..base
        },
        Status::Pressed => Style {
            background: Some(Background::Color(lighten(theme.selection, 0.05))),
            ..base
        },
        Status::Disabled => base,
    }
}

fn lighten(color: Color, amount: f32) -> Color {
    Color {
        r: (color.r + amount).min(1.0),
        g: (color.g + amount).min(1.0),
        b: (color.b + amount).min(1.0),
        a: color.a,
    }
}

fn darken(color: Color, amount: f32) -> Color {
    Color {
        r: (color.r - amount).max(0.0),
        g: (color.g - amount).max(0.0),
        b: (color.b - amount).max(0.0),
        a: color.a,
    }
}
