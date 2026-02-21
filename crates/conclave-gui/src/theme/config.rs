use std::path::Path;

use iced::Color;
use serde::Deserialize;

use super::Theme;

/// A color parsed from a `"#rrggbb"` hex string.
#[derive(Debug, Clone, Copy)]
struct HexColor(Color);

impl<'de> Deserialize<'de> for HexColor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let s = s.strip_prefix('#').unwrap_or(&s);
        if s.len() != 6 {
            return Err(serde::de::Error::custom("expected 6-digit hex color"));
        }
        let r = u8::from_str_radix(&s[0..2], 16).map_err(serde::de::Error::custom)?;
        let g = u8::from_str_radix(&s[2..4], 16).map_err(serde::de::Error::custom)?;
        let b = u8::from_str_radix(&s[4..6], 16).map_err(serde::de::Error::custom)?;
        Ok(HexColor(Color::from_rgb8(r, g, b)))
    }
}

/// User-configurable theme overrides loaded from the `[theme]` section
/// of `config.toml`. All fields are optional; unset fields keep their
/// defaults from the built-in Ferra palette.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    background: Option<HexColor>,
    surface: Option<HexColor>,
    surface_bright: Option<HexColor>,
    primary: Option<HexColor>,
    text: Option<HexColor>,
    text_secondary: Option<HexColor>,
    text_muted: Option<HexColor>,
    error: Option<HexColor>,
    on_error: Option<HexColor>,
    warning: Option<HexColor>,
    on_warning: Option<HexColor>,
    success: Option<HexColor>,
    border: Option<HexColor>,
    scrollbar: Option<HexColor>,
    selection: Option<HexColor>,
    title_bar: Option<HexColor>,
}

/// Wrapper to extract the `[theme]` section from config.toml.
#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    theme: ThemeConfig,
}

impl ThemeConfig {
    /// Load theme overrides from `<config_dir>/config.toml`.
    ///
    /// Returns default (all `None`) if the file is missing or malformed.
    pub fn load(config_dir: &Path) -> Self {
        let path = config_dir.join("config.toml");
        if path.exists() {
            if let Ok(contents) = std::fs::read_to_string(&path) {
                if let Ok(config) = toml::from_str::<ConfigFile>(&contents) {
                    return config.theme;
                }
            }
        }
        Self::default()
    }

    /// Apply overrides to the base theme, returning the final theme.
    pub fn apply(self, mut theme: Theme) -> Theme {
        if let Some(c) = self.background {
            theme.background = c.0;
        }
        if let Some(c) = self.surface {
            theme.surface = c.0;
        }
        if let Some(c) = self.surface_bright {
            theme.surface_bright = c.0;
        }
        if let Some(c) = self.primary {
            theme.primary = c.0;
        }
        if let Some(c) = self.text {
            theme.text = c.0;
        }
        if let Some(c) = self.text_secondary {
            theme.text_secondary = c.0;
        }
        if let Some(c) = self.text_muted {
            theme.text_muted = c.0;
        }
        if let Some(c) = self.error {
            theme.error = c.0;
        }
        if let Some(c) = self.on_error {
            theme.on_error = c.0;
        }
        if let Some(c) = self.warning {
            theme.warning = c.0;
        }
        if let Some(c) = self.on_warning {
            theme.on_warning = c.0;
        }
        if let Some(c) = self.success {
            theme.success = c.0;
        }
        if let Some(c) = self.border {
            theme.border = c.0;
        }
        if let Some(c) = self.scrollbar {
            theme.scrollbar = c.0;
        }
        if let Some(c) = self.selection {
            theme.selection = c.0;
        }
        if let Some(c) = self.title_bar {
            theme.title_bar = c.0;
        }
        theme
    }
}
