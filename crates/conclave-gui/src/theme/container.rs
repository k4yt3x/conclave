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
        background: Some(Background::Color(theme.title_bar)),
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

pub fn error_banner(theme: &Theme) -> Style {
    Style {
        background: Some(Background::Color(theme.error)),
        text_color: Some(theme.on_error),
        ..Style::default()
    }
}

pub fn warning_banner(theme: &Theme) -> Style {
    Style {
        background: Some(Background::Color(theme.warning)),
        text_color: Some(theme.on_warning),
        ..Style::default()
    }
}

pub fn tooltip(theme: &Theme) -> Style {
    Style {
        background: Some(Background::Color(theme.surface)),
        border: Border {
            color: theme.border,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..Style::default()
    }
}

pub fn input_area(theme: &Theme) -> Style {
    Style {
        background: Some(Background::Color(theme.input_area)),
        border: Border {
            color: theme.border,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Style::default()
    }
}

pub fn drag_handle(theme: &Theme) -> Style {
    Style {
        background: Some(Background::Color(theme.border)),
        ..Style::default()
    }
}

pub fn trust_risky(theme: &Theme) -> Style {
    Style {
        background: Some(Background::Color(theme.indicator_risky)),
        text_color: Some(theme.on_indicator_risky),
        ..Style::default()
    }
}

pub fn trust_unverified(theme: &Theme) -> Style {
    Style {
        background: Some(Background::Color(theme.indicator_unverified)),
        text_color: Some(theme.on_indicator_unverified),
        ..Style::default()
    }
}

pub fn trust_verified(theme: &Theme) -> Style {
    Style {
        background: Some(Background::Color(theme.indicator_verified)),
        text_color: Some(theme.on_indicator_verified),
        ..Style::default()
    }
}
