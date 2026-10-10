//! Colors, styles and SGR (Select Graphic Rendition) emission.

/// A terminal color as understood by SGR sequences.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Color {
    /// The terminal's default color (`39` / `49`).
    #[default]
    Default,
    /// One of the 16 basic ANSI colors (0–7 normal, 8–15 bright).
    Ansi(u8),
    /// xterm-256 palette index.
    Indexed(u8),
    /// 24-bit truecolor.
    Rgb(u8, u8, u8),
}

/// How highlight colors are emitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ColorMode {
    /// 24-bit `38;2;r;g;b` sequences.
    #[default]
    TrueColor,
    /// Nearest xterm-256 palette entry (`38;5;n`).
    Ansi256,
}

impl ColorMode {
    /// Detect truecolor support from the environment (`COLORTERM`, `TERM`).
    pub fn detect() -> Self {
        Self::detect_from(
            std::env::var("COLORTERM").ok().as_deref(),
            std::env::var("TERM").ok().as_deref(),
        )
    }

    /// Pure variant of [`ColorMode::detect`] for testing.
    pub fn detect_from(colorterm: Option<&str>, term: Option<&str>) -> Self {
        if let Some(ct) = colorterm {
            let ct = ct.to_ascii_lowercase();
            if ct == "truecolor" || ct == "24bit" {
                return ColorMode::TrueColor;
            }
        }
        if let Some(term) = term {
            const TRUECOLOR_TERMS: &[&str] = &[
                "direct",
                "truecolor",
                "24bit",
                "kitty",
                "alacritty",
                "wezterm",
                "foot",
                "ghostty",
                "iterm",
                "contour",
                "rio",
            ];
            if TRUECOLOR_TERMS.iter().any(|t| term.contains(t)) {
                return ColorMode::TrueColor;
            }
        }
        ColorMode::Ansi256
    }
}

impl Color {
    /// Convert this color so it can be rendered in `mode`.
    pub fn for_mode(self, mode: ColorMode) -> Self {
        match (self, mode) {
            (Color::Rgb(r, g, b), ColorMode::Ansi256) => Color::Indexed(rgb_to_256(r, g, b)),
            (c, _) => c,
        }
    }

    /// Parse `#RRGGBB` or `#RGB` (leading `#` required).
    pub fn parse_hex(s: &str) -> Option<Self> {
        let hex = s.strip_prefix('#')?;
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let v = |s: &str| u8::from_str_radix(s, 16).ok();
        match hex.len() {
            6 => Some(Color::Rgb(v(&hex[0..2])?, v(&hex[2..4])?, v(&hex[4..6])?)),
            3 => {
                let d = |i: usize| v(&hex[i..i + 1]).map(|x| x * 17);
                Some(Color::Rgb(d(0)?, d(1)?, d(2)?))
            }
            _ => None,
        }
    }

    /// Approximate RGB value (used for swatches and down-sampling).
    pub fn to_rgb(self) -> Option<(u8, u8, u8)> {
        match self {
            Color::Default => None,
            Color::Rgb(r, g, b) => Some((r, g, b)),
            Color::Ansi(n) => Some(XTERM_16[(n & 15) as usize]),
            Color::Indexed(n) => Some(indexed_to_rgb(n)),
        }
    }

    /// Append the SGR parameters for this color (`fg` or background) to `out`.
    pub fn write_params(self, background: bool, out: &mut Vec<u8>) {
        let base = if background { 40 } else { 30 };
        match self {
            Color::Default => push_num(out, base + 9),
            Color::Ansi(n) if n < 8 => push_num(out, base + n as u16),
            Color::Ansi(n) => push_num(out, base + 60 + (n as u16 & 7)),
            Color::Indexed(n) => {
                push_num(out, base + 8);
                out.extend_from_slice(b";5;");
                push_num(out, n as u16);
            }
            Color::Rgb(r, g, b) => {
                push_num(out, base + 8);
                out.extend_from_slice(b";2;");
                push_num(out, r as u16);
                out.push(b';');
                push_num(out, g as u16);
                out.push(b';');
                push_num(out, b as u16);
            }
        }
    }
}

/// Style flag bits (shared by highlight [`Style`] and terminal state).
pub mod flags {
    pub const BOLD: u16 = 1 << 0;
    pub const DIM: u16 = 1 << 1;
    pub const ITALIC: u16 = 1 << 2;
    pub const UNDERLINE: u16 = 1 << 3;
    pub const BLINK: u16 = 1 << 4;
    pub const INVERT: u16 = 1 << 5;
    pub const STRIKE: u16 = 1 << 6;

    /// `(name, bit, on-code, off-code)` for every flag.
    pub const TABLE: &[(&str, u16, u16, u16)] = &[
        ("bold", BOLD, 1, 22),
        ("dim", DIM, 2, 22),
        ("italic", ITALIC, 3, 23),
        ("underline", UNDERLINE, 4, 24),
        ("blink", BLINK, 5, 25),
        ("invert", INVERT, 7, 27),
        ("strike", STRIKE, 9, 29),
    ];

    /// Look up a style name (with common aliases).
    pub fn by_name(name: &str) -> Option<u16> {
        let name = match name {
            "reverse" => "invert",
            "strikethrough" => "strike",
            "faint" => "dim",
            other => other,
        };
        TABLE.iter().find(|t| t.0 == name).map(|t| t.1)
    }
}

/// A highlight style: only the attributes that are `Some`/set are applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub flags: u16,
}

impl Style {
    pub fn is_empty(&self) -> bool {
        self.fg.is_none() && self.bg.is_none() && self.flags == 0
    }

    /// Convert all colors for the target mode.
    pub fn for_mode(self, mode: ColorMode) -> Self {
        Style {
            fg: self.fg.map(|c| c.for_mode(mode)),
            bg: self.bg.map(|c| c.for_mode(mode)),
            flags: self.flags,
        }
    }
}

/// Append a decimal number without allocating.
#[inline]
pub(crate) fn push_num(out: &mut Vec<u8>, n: u16) {
    let mut buf = [0u8; 5];
    let mut i = buf.len();
    let mut n = n;
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    out.extend_from_slice(&buf[i..]);
}

/// Standard xterm values for the 16 ANSI colors (used for previews only).
const XTERM_16: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

fn indexed_to_rgb(n: u8) -> (u8, u8, u8) {
    match n {
        0..=15 => XTERM_16[n as usize],
        16..=231 => {
            let i = n - 16;
            (
                CUBE[(i / 36) as usize],
                CUBE[((i / 6) % 6) as usize],
                CUBE[(i % 6) as usize],
            )
        }
        _ => {
            let v = 8 + (n - 232) * 10;
            (v, v, v)
        }
    }
}

/// Map a truecolor value to the nearest xterm-256 entry (6×6×6 cube or gray ramp).
pub fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    fn cube_index(v: u8) -> u8 {
        // Midpoints between the cube levels 0,95,135,175,215,255.
        match v {
            0..=47 => 0,
            48..=114 => 1,
            115..=154 => 2,
            155..=194 => 3,
            195..=234 => 4,
            _ => 5,
        }
    }
    fn dist(a: (u8, u8, u8), b: (u8, u8, u8)) -> u32 {
        let d = |x: u8, y: u8| (x as i32 - y as i32).pow(2) as u32;
        d(a.0, b.0) + d(a.1, b.1) + d(a.2, b.2)
    }
    let (ri, gi, bi) = (cube_index(r), cube_index(g), cube_index(b));
    let cube = 16 + 36 * ri + 6 * gi + bi;
    let avg = ((r as u16 + g as u16 + b as u16) / 3) as u8;
    let gray = if avg < 4 {
        16 // black from the cube is closer than the darkest gray
    } else if avg > 246 {
        231
    } else {
        232 + ((avg - 3) / 10).min(23)
    };
    let target = (r, g, b);
    if dist(indexed_to_rgb(gray), target) < dist(indexed_to_rgb(cube), target) {
        gray
    } else {
        cube
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_forms() {
        assert_eq!(Color::parse_hex("#ff8000"), Some(Color::Rgb(255, 128, 0)));
        assert_eq!(Color::parse_hex("#F80"), Some(Color::Rgb(255, 136, 0)));
        assert_eq!(Color::parse_hex("ff8000"), None);
        assert_eq!(Color::parse_hex("#ff80"), None);
        assert_eq!(Color::parse_hex("#gg0000"), None);
    }

    #[test]
    fn sgr_params() {
        let mut v = Vec::new();
        Color::Rgb(1, 2, 3).write_params(false, &mut v);
        assert_eq!(v, b"38;2;1;2;3");
        v.clear();
        Color::Indexed(208).write_params(true, &mut v);
        assert_eq!(v, b"48;5;208");
        v.clear();
        Color::Ansi(9).write_params(false, &mut v);
        assert_eq!(v, b"91");
        v.clear();
        Color::Default.write_params(true, &mut v);
        assert_eq!(v, b"49");
    }

    #[test]
    fn down_sampling() {
        assert_eq!(rgb_to_256(0, 0, 0), 16);
        assert_eq!(rgb_to_256(255, 255, 255), 231);
        assert_eq!(rgb_to_256(255, 0, 0), 196);
        assert_eq!(rgb_to_256(128, 128, 128), 244);
        assert_eq!(rgb_to_256(0, 135, 255), 33);
    }

    #[test]
    fn detect_mode() {
        assert_eq!(
            ColorMode::detect_from(Some("truecolor"), None),
            ColorMode::TrueColor
        );
        assert_eq!(
            ColorMode::detect_from(None, Some("xterm-kitty")),
            ColorMode::TrueColor
        );
        assert_eq!(
            ColorMode::detect_from(None, Some("xterm-256color")),
            ColorMode::Ansi256
        );
    }

    #[test]
    fn style_names() {
        assert_eq!(flags::by_name("reverse"), Some(flags::INVERT));
        assert_eq!(flags::by_name("bold"), Some(flags::BOLD));
        assert_eq!(flags::by_name("nope"), None);
    }
}
