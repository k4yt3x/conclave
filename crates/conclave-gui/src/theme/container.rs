use iced::widget::container::{Catalog, Style};
use iced::{Background, Border};

use super::Theme;

impl Catalog for Theme {
    type Class<'a> = Box<dyn Fn(&Theme) -> Style + 'a>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(transparent)
    }

    fn style(&self, class: &Self::Class<'_>) -> Style {
        class(self)
    }
}

pub fn transparent(_theme: &Theme) -> Style {
    Style::default()
}

pub fn background(theme: &Theme) -> Style {
    Style {
        background: Some(Background::Color(theme.background)),
        ..Style::default()
    }
}

pub fn sidebar(theme: &Theme) -> Style {
    Style {
        background: Some(Background::Color(theme.surface)),
        border: Border {
            color: theme.border,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..Style::default()
    }
}

pub fn title_bar(theme: &Theme) -> Style {
    Style {
        background: Some(Background::Color(theme.surface)),
        border: Border {
            color: theme.border,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Style::default()
    }
}

pub fn card(theme: &Theme) -> Style {
    Style {
        background: Some(Background::Color(theme.surface)),
        border: Border {
            color: theme.border,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Style::default()
    }
}

pub fn input_area(theme: &Theme) -> Style {
    Style {
        background: Some(Background::Color(theme.surface)),
        border: Border {
            color: theme.border,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Style::default()
    }
}
