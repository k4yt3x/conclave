use iced::widget::text_editor::{Catalog, Status, Style};
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
        background: Background::Color(theme.surface_bright),
        border: Border {
            radius: 4.0.into(),
            width: 1.0,
            color: theme.border,
        },
        placeholder: theme.text_muted,
        value: theme.text,
        selection: theme.selection,
    };

    match status {
        Status::Active => base,
        Status::Hovered => Style {
            border: Border {
                color: theme.text_secondary,
                ..base.border
            },
            ..base
        },
        Status::Focused { .. } => Style {
            border: Border {
                color: theme.primary,
                ..base.border
            },
            ..base
        },
        Status::Disabled => Style {
            background: Background::Color(theme.surface),
            value: theme.text_muted,
            ..base
        },
    }
}

pub fn chat_input(theme: &Theme, status: Status) -> Style {
    let base = Style {
        background: Background::Color(theme.input_area),
        border: Border {
            radius: 0.0.into(),
            width: 0.0,
            color: Color::TRANSPARENT,
        },
        placeholder: theme.text_muted,
        value: theme.text,
        selection: theme.selection,
    };

    match status {
        Status::Active | Status::Hovered | Status::Focused { .. } => base,
        Status::Disabled => Style {
            value: theme.text_muted,
            ..base
        },
    }
}
