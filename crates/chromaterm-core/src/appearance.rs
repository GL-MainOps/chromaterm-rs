//! Dark or light background classification for automatic theme selection.
//!
//! Pure functions only: the binary queries the terminal (OSC 10/11) and an embedder such as
//! a terminal emulator already knows its colors; both classify them here and pass the theme
//! name to [`crate::config::resolve::ResolveOptions::auto_theme`].

/// An RGB color.
pub type Rgb = (u8, u8, u8);

/// The kind of background the terminal has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Dark,
    Light,
}

impl Appearance {
    /// Name of the built-in theme for this appearance.
    pub fn theme(self) -> &'static str {
        match self {
            Appearance::Dark => "dark",
            Appearance::Light => "light",
        }
    }
}

/// Parse an X11 color spec as sent in OSC replies: `rgb:R/G/B` or
/// `rgba:R/G/B/A`, with 1–4 hex digits per channel.
pub fn parse_color(spec: &[u8]) -> Option<Rgb> {
    let spec = std::str::from_utf8(spec).ok()?;
    let (kind, rest) = spec.split_once(':')?;
    if kind != "rgb" && kind != "rgba" {
        return None;
    }
    let mut parts = rest.split('/');
    let mut channel = || -> Option<u8> {
        let p = parts.next()?;
        if p.is_empty() || p.len() > 4 {
            return None;
        }
        let v = u32::from_str_radix(p, 16).ok()?;
        let max = (1u32 << (4 * p.len())) - 1;
        Some(((v * 255 + max / 2) / max) as u8)
    };
    Some((channel()?, channel()?, channel()?))
}

/// WCAG relative luminance.
fn luminance((r, g, b): Rgb) -> f64 {
    let f = |c: u8| {
        let c = c as f64 / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
}

/// Dark or light, from the terminal's colors.
///
/// With a foreground, "dark" means text lighter than the background (what
/// the dark palette is designed for). Without one, a background with
/// luminance above 0.179 is light. That is where black and white text
/// would have equal contrast.
pub fn classify(background: Rgb, foreground: Option<Rgb>) -> Appearance {
    let bg = luminance(background);
    if let Some(fg) = foreground.map(luminance) {
        if (fg - bg).abs() > 0.05 {
            return if fg > bg {
                Appearance::Dark
            } else {
                Appearance::Light
            };
        }
    }
    if bg > 0.179 {
        Appearance::Light
    } else {
        Appearance::Dark
    }
}

/// `$COLORFGBG` is `fg;bg` (or `fg;default;bg`) with ANSI color indexes.
/// Same convention as Vim: background 0–6 or 8 is dark, 7 and 9–15 light.
pub fn from_colorfgbg(value: &str) -> Option<Appearance> {
    let bg: u8 = value.rsplit(';').next()?.trim().parse().ok()?;
    match bg {
        0..=6 | 8 => Some(Appearance::Dark),
        7 | 9..=15 => Some(Appearance::Light),
        _ => None,
    }
}

/// The built-in theme (`"dark"` or `"light"`) for a background (and, if known, foreground).
pub fn theme_for(background: Rgb, foreground: Option<Rgb>) -> &'static str {
    classify(background, foreground).theme()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_parse() {
        assert_eq!(parse_color(b"rgb:ffff/ffff/ffff"), Some((255, 255, 255)));
        assert_eq!(parse_color(b"rgb:0e0e/1313/1717"), Some((14, 19, 23)));
        assert_eq!(parse_color(b"rgb:0e/13/17"), Some((14, 19, 23)));
        assert_eq!(parse_color(b"rgb:f/0/8"), Some((255, 0, 136)));
        assert_eq!(parse_color(b"rgba:ffff/0000/0000/ffff"), Some((255, 0, 0)));
        assert_eq!(parse_color(b"#ffffff"), None);
        assert_eq!(parse_color(b"rgb:ffff/ffff"), None);
        assert_eq!(parse_color(b"rgb:fffff/0/0"), None);
    }

    #[test]
    fn classification() {
        let (black, white) = ((0, 0, 0), (255, 255, 255));
        assert_eq!(classify((14, 19, 23), None), Appearance::Dark);
        assert_eq!(classify(white, None), Appearance::Light);
        assert_eq!(classify((0xfd, 0xf6, 0xe3), None), Appearance::Light); // solarized light
        assert_eq!(classify((0x00, 0x2b, 0x36), None), Appearance::Dark); // solarized dark
        // Mid-gray: the text color decides.
        assert_eq!(classify((128, 128, 128), Some(white)), Appearance::Dark);
        assert_eq!(classify((128, 128, 128), Some(black)), Appearance::Light);
    }

    #[test]
    fn colorfgbg() {
        assert_eq!(from_colorfgbg("15;0"), Some(Appearance::Dark));
        assert_eq!(from_colorfgbg("0;15"), Some(Appearance::Light));
        assert_eq!(from_colorfgbg("0;default;7"), Some(Appearance::Light));
        assert_eq!(from_colorfgbg("7;8"), Some(Appearance::Dark));
        assert_eq!(from_colorfgbg("15;default"), None);
        assert_eq!(from_colorfgbg(""), None);
    }

    #[test]
    fn theme_names() {
        assert_eq!(theme_for((14, 19, 23), None), "dark");
        assert_eq!(theme_for((255, 255, 255), None), "light");
    }
}
