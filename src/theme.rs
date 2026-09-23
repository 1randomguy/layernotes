use iced::Color;

use crate::config::ThemeConfig;

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub border: Color,
    pub button_bg: Color,
    pub shadow: Color,
}

impl Palette {
    pub fn from_config(theme: &ThemeConfig) -> Self {
        Self {
            text: parse_hex(&theme.text).unwrap_or(Color::from_rgb8(0xcd, 0xd6, 0xf4)),
            muted: parse_hex(&theme.muted).unwrap_or(Color::from_rgb8(0x7f, 0x84, 0x9c)),
            accent: parse_hex(&theme.accent).unwrap_or(Color::from_rgb8(0x89, 0xb4, 0xfa)),
            border: parse_hex(&theme.border).unwrap_or(Color::from_rgb8(0x45, 0x47, 0x5a)),
            button_bg: parse_hex(&theme.button_bg).unwrap_or(Color::from_rgb8(0x31, 0x32, 0x44)),
            shadow: Color::from_rgba(0.0, 0.0, 0.0, 0.35),
        }
    }
}

/// Parse `#rrggbb` or `#rgb` into an opaque [`Color`].
pub fn parse_hex(input: &str) -> Option<Color> {
    let value = input.trim().trim_start_matches('#');

    let (r, g, b) = match value.len() {
        6 => (
            u8::from_str_radix(&value[0..2], 16).ok()?,
            u8::from_str_radix(&value[2..4], 16).ok()?,
            u8::from_str_radix(&value[4..6], 16).ok()?,
        ),
        3 => {
            let expand = |c: char| u8::from_str_radix(&format!("{c}{c}"), 16).ok();
            let mut chars = value.chars();
            (
                expand(chars.next()?)?,
                expand(chars.next()?)?,
                expand(chars.next()?)?,
            )
        }
        _ => return None,
    };

    Some(Color::from_rgb8(r, g, b))
}

/// Pick a readable foreground color for the given background.
pub fn contrast_text(background: Color) -> Color {
    let luminance = 0.2126 * background.r + 0.7152 * background.g + 0.0722 * background.b;
    if luminance > 0.55 {
        Color::from_rgb8(0x1e, 0x1e, 0x2e)
    } else {
        Color::from_rgb8(0xcd, 0xd6, 0xf4)
    }
}
