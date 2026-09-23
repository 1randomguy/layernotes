use iced::Color;

use crate::config::Theme;

/// Colours used for note chrome, derived from the active theme.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub text: Color,
    pub accent: Color,
    pub border: Color,
    pub shadow: Color,
    /// Default note background for the active theme.
    pub note: Color,
}

impl Palette {
    pub fn from_theme(theme: Theme) -> Self {
        // Official Catppuccin palettes: base, surface1, text, mauve.
        let (base, surface1, text, mauve) = match theme {
            Theme::CatppuccinMocha => (0x1e1e2e, 0x45475a, 0xcdd6f4, 0xcba6f7),
            Theme::CatppuccinMacchiato => (0x24273a, 0x494d64, 0xcad3f5, 0xc6a0f6),
            Theme::CatppuccinFrappe => (0x303446, 0x51576d, 0xc6d0f5, 0xca9ee6),
            Theme::CatppuccinLatte => (0xeff1f5, 0xbcc0cc, 0x4c4f69, 0x8839ef),
            Theme::Dark | Theme::Light => {
                let iced_theme = theme.iced();
                let extended = iced_theme.extended_palette();
                return Self {
                    text: extended.background.base.text,
                    accent: extended.primary.base.color,
                    border: extended.background.strong.color,
                    shadow: shadow_for(theme),
                    note: extended.background.base.color,
                };
            }
        };

        Self {
            text: rgb(text),
            accent: rgb(mauve),
            border: rgb(surface1),
            shadow: shadow_for(theme),
            note: rgb(base),
        }
    }
}

/// A slightly shifted version of `color` — darker for light colours and
/// lighter for dark ones — used for the note header so it follows the note.
pub fn shade(color: Color) -> Color {
    let luminance = 0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b;
    let target = if luminance > 0.5 {
        Color::BLACK
    } else {
        Color::WHITE
    };
    mix(color, target, 0.10)
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a,
    }
}

fn rgb(hex: u32) -> Color {
    Color::from_rgb8(
        (hex >> 16) as u8,
        ((hex >> 8) & 0xff) as u8,
        (hex & 0xff) as u8,
    )
}

fn shadow_for(theme: Theme) -> Color {
    match theme {
        Theme::CatppuccinLatte | Theme::Light => Color::from_rgba(0.0, 0.0, 0.0, 0.18),
        _ => Color::from_rgba(0.0, 0.0, 0.0, 0.35),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_forms() {
        assert_eq!(
            parse_hex("#f9e2af"),
            Some(Color::from_rgb8(0xf9, 0xe2, 0xaf))
        );
        assert_eq!(parse_hex("fff"), Some(Color::from_rgb8(0xff, 0xff, 0xff)));
        assert_eq!(parse_hex("nope"), None);
    }

    #[test]
    fn themes_differ() {
        let mocha = Palette::from_theme(Theme::CatppuccinMocha);
        let latte = Palette::from_theme(Theme::CatppuccinLatte);
        assert_ne!(mocha.note, latte.note);
        assert_ne!(mocha.text, latte.text);
    }

    #[test]
    fn mocha_uses_base_and_mauve() {
        let mocha = Palette::from_theme(Theme::CatppuccinMocha);
        assert_eq!(mocha.note, Color::from_rgb8(0x1e, 0x1e, 0x2e));
        assert_eq!(mocha.accent, Color::from_rgb8(0xcb, 0xa6, 0xf7));
        assert_eq!(mocha.text, Color::from_rgb8(0xcd, 0xd6, 0xf4));
    }
}
