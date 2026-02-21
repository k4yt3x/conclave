use iced::widget::text::{Catalog, Style};

use super::Theme;

impl Catalog for Theme {
    type Class<'a> = Box<dyn Fn(&Theme) -> Style + 'a>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(primary)
    }

    fn style(&self, class: &Self::Class<'_>) -> Style {
        class(self)
    }
}

pub fn primary(theme: &Theme) -> Style {
    Style {
        color: Some(theme.text),
    }
}

pub fn secondary(theme: &Theme) -> Style {
    Style {
        color: Some(theme.text_secondary),
    }
}

pub fn muted(theme: &Theme) -> Style {
    Style {
        color: Some(theme.text_muted),
    }
}

pub fn error(theme: &Theme) -> Style {
    Style {
        color: Some(theme.error),
    }
}

pub fn on_primary(theme: &Theme) -> Style {
    Style {
        color: Some(theme.surface),
    }
}

pub fn on_error(theme: &Theme) -> Style {
    Style {
        color: Some(theme.on_error),
    }
}

pub fn on_warning(theme: &Theme) -> Style {
    Style {
        color: Some(theme.on_warning),
    }
}
