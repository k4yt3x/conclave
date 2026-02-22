use iced::widget::scrollable::{AutoScroll, Catalog, Rail, Scroller, Status, Style};
use iced::{Background, Border, Color, Shadow};

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

fn default_auto_scroll(theme: &Theme) -> AutoScroll {
    AutoScroll {
        background: Background::Color(theme.surface_bright),
        border: Border::default(),
        shadow: Shadow::default(),
        icon: theme.text_muted,
    }
}

pub fn primary(theme: &Theme, status: Status) -> Style {
    let rail = Rail {
        background: None,
        border: Border::default(),
        scroller: Scroller {
            background: Background::Color(theme.scrollbar),
            border: Border {
                radius: 4.0.into(),
                ..Border::default()
            },
        },
    };

    match status {
        Status::Active { .. } => Style {
            container: iced::widget::container::Style::default(),
            vertical_rail: rail,
            horizontal_rail: rail,
            gap: None,
            auto_scroll: default_auto_scroll(theme),
        },
        Status::Hovered {
            is_vertical_scrollbar_hovered,
            is_horizontal_scrollbar_hovered,
            ..
        } => {
            let hovered_rail = |hovered: bool| -> Rail {
                if hovered {
                    Rail {
                        background: Some(Background::Color(Color {
                            a: 0.1,
                            ..theme.text
                        })),
                        scroller: Scroller {
                            background: Background::Color(theme.text_secondary),
                            ..rail.scroller
                        },
                        ..rail
                    }
                } else {
                    rail
                }
            };

            Style {
                container: iced::widget::container::Style::default(),
                vertical_rail: hovered_rail(is_vertical_scrollbar_hovered),
                horizontal_rail: hovered_rail(is_horizontal_scrollbar_hovered),
                gap: None,
                auto_scroll: default_auto_scroll(theme),
            }
        }
        Status::Dragged {
            is_vertical_scrollbar_dragged,
            is_horizontal_scrollbar_dragged,
            ..
        } => {
            let dragged_rail = |dragged: bool| -> Rail {
                if dragged {
                    Rail {
                        background: Some(Background::Color(Color {
                            a: 0.15,
                            ..theme.text
                        })),
                        scroller: Scroller {
                            background: Background::Color(theme.primary),
                            ..rail.scroller
                        },
                        ..rail
                    }
                } else {
                    rail
                }
            };

            Style {
                container: iced::widget::container::Style::default(),
                vertical_rail: dragged_rail(is_vertical_scrollbar_dragged),
                horizontal_rail: dragged_rail(is_horizontal_scrollbar_dragged),
                gap: None,
                auto_scroll: default_auto_scroll(theme),
            }
        }
    }
}
