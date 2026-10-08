//! ANSI/ECMA-48 escape-sequence scanning and the SGR terminal-state model.
//!
//! The highlighter strips escape sequences before matching. Then it re-inserts
//! them verbatim at their original text positions. Only SGR (`CSI … m`)
//! sequences change the tracked [`Attrs`] state.

use crate::color::{Color, flags, push_num};

pub const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;

/// Result of scanning one escape sequence starting at an `ESC` byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Escape {
    /// A complete sequence ending (exclusive) at `end`. `sgr` is true for
    /// `CSI <digits;:> m`.
    Complete { end: usize, sgr: bool },
    /// The buffer ends before the sequence is terminated.
    Incomplete,
}

/// Scan the escape sequence that starts at `buf[start]` (which must be `ESC`).
pub fn scan(buf: &[u8], start: usize) -> Escape {
    debug_assert_eq!(buf[start], ESC);
    let Some(&kind) = buf.get(start + 1) else {
        return Escape::Incomplete;
    };
    match kind {
        // CSI: parameters 0x30–0x3F, intermediates 0x20–0x2F, final 0x40–0x7E.
        b'[' => {
            let mut j = start + 2;
            while let Some(&c) = buf.get(j) {
                match c {
                    0x40..=0x7e => {
                        let params = &buf[start + 2..j];
                        let sgr = c == b'm'
                            && params
                                .iter()
                                .all(|b| b.is_ascii_digit() || *b == b';' || *b == b':');
                        return Escape::Complete { end: j + 1, sgr };
                    }
                    0x20..=0x3f => j += 1,
                    // Malformed: the sequence ends before the unexpected byte.
                    _ => return Escape::Complete { end: j, sgr: false },
                }
            }
            Escape::Incomplete
        }
        // String sequences: OSC, DCS, SOS, PM, APC — terminated by ST (ESC \) or BEL.
        b']' | b'P' | b'X' | b'^' | b'_' => {
            let body = start + 2;
            let Some(off) = memchr::memchr2(BEL, ESC, buf.get(body..).unwrap_or_default()) else {
                return Escape::Incomplete;
            };
            let at = body + off;
            if buf[at] == BEL {
                return Escape::Complete {
                    end: at + 1,
                    sgr: false,
                };
            }
            match buf.get(at + 1) {
                Some(b'\\') => Escape::Complete {
                    end: at + 2,
                    sgr: false,
                },
                // Another ESC aborts the string; it starts a new sequence.
                Some(_) => Escape::Complete {
                    end: at,
                    sgr: false,
                },
                None => Escape::Incomplete,
            }
        }
        // nF: intermediates then a final byte (e.g. charset designation `ESC ( B`).
        0x20..=0x2f => {
            let mut j = start + 1;
            while let Some(&c) = buf.get(j) {
                match c {
                    0x20..=0x2f => j += 1,
                    0x30..=0x7e => {
                        return Escape::Complete {
                            end: j + 1,
                            sgr: false,
                        };
                    }
                    _ => return Escape::Complete { end: j, sgr: false },
                }
            }
            Escape::Incomplete
        }
        // Fp / Fe / Fs two-byte sequences (ESC 7, ESC M, ESC =, …).
        0x30..=0x7e => Escape::Complete {
            end: start + 2,
            sgr: false,
        },
        // Lone ESC followed by a control or non-ASCII byte.
        _ => Escape::Complete {
            end: start + 1,
            sgr: false,
        },
    }
}

/// Offset of a trailing *incomplete* escape sequence in `buf`, if any.
pub fn incomplete_tail(buf: &[u8]) -> Option<usize> {
    let pos = memchr::memrchr(ESC, buf)?;
    // An unterminated string sequence may contain no further ESC, so the last
    // ESC is the only candidate to check.
    match scan(buf, pos) {
        Escape::Incomplete => Some(pos),
        Escape::Complete { .. } => None,
    }
}

/// Offset of a trailing incomplete UTF-8 code point in `buf`, if any.
pub fn incomplete_utf8_tail(buf: &[u8]) -> Option<usize> {
    let n = buf.len();
    for back in 1..=n.min(4) {
        let i = n - back;
        let b = buf[i];
        if b & 0xc0 == 0x80 {
            continue; // continuation byte; keep looking for the lead
        }
        let need = match b {
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf7 => 4,
            _ => return None,
        };
        return (back < need).then_some(i);
    }
    None
}

/// Terminal graphic state tracked across SGR sequences.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Attrs {
    pub fg: Color,
    pub bg: Color,
    pub flags: u16,
}

impl Attrs {
    /// Apply an SGR parameter string (the bytes between `CSI` and `m`).
    pub fn apply_sgr(&mut self, params: &[u8]) {
        if params.is_empty() {
            *self = Attrs::default();
            return;
        }
        let mut fields = params.split(|&b| b == b';');
        while let Some(field) = fields.next() {
            if field.contains(&b':') {
                self.apply_colon_field(field);
                continue;
            }
            let Some(n) = parse_num(field) else { continue };
            match n {
                38 | 48 | 58 => {
                    let color = match fields.next().and_then(parse_num) {
                        Some(5) => fields.next().and_then(parse_u8).map(Color::Indexed),
                        Some(2) => {
                            let (r, g, b) = (
                                fields.next().and_then(parse_u8),
                                fields.next().and_then(parse_u8),
                                fields.next().and_then(parse_u8),
                            );
                            match (r, g, b) {
                                (Some(r), Some(g), Some(b)) => Some(Color::Rgb(r, g, b)),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    if let Some(c) = color {
                        self.set_extended(n, c);
                    }
                }
                n => self.apply_simple(n),
            }
        }
    }

    fn set_extended(&mut self, which: u16, c: Color) {
        match which {
            38 => self.fg = c,
            48 => self.bg = c,
            _ => {} // 58: underline color — not tracked
        }
    }

    /// Colon (ITU T.416) form: `38:2::r:g:b`, `38:2:r:g:b`, `38:5:n`, `4:3`.
    fn apply_colon_field(&mut self, field: &[u8]) {
        let mut parts = [0u16; 6];
        let mut count = 0;
        for p in field.split(|&b| b == b':') {
            if count == parts.len() {
                return;
            }
            parts[count] = parse_num(p).unwrap_or(0);
            count += 1;
        }
        let parts = &parts[..count];
        let u8_of = |v: u16| u8::try_from(v).ok();
        match parts {
            [4, 0] => self.flags &= !flags::UNDERLINE,
            [4, _] => self.flags |= flags::UNDERLINE,
            [w @ (38 | 48 | 58), 5, n] => {
                if let Some(n) = u8_of(*n) {
                    self.set_extended(*w, Color::Indexed(n));
                }
            }
            [w @ (38 | 48 | 58), 2, r, g, b] | [w @ (38 | 48 | 58), 2, _, r, g, b] => {
                if let (Some(r), Some(g), Some(b)) = (u8_of(*r), u8_of(*g), u8_of(*b)) {
                    self.set_extended(*w, Color::Rgb(r, g, b));
                }
            }
            [n, ..] => self.apply_simple(*n),
            [] => {}
        }
    }

    fn apply_simple(&mut self, n: u16) {
        use flags::*;
        match n {
            0 => *self = Attrs::default(),
            1 => self.flags |= BOLD,
            2 => self.flags |= DIM,
            3 => self.flags |= ITALIC,
            4 | 21 => self.flags |= UNDERLINE,
            5 | 6 => self.flags |= BLINK,
            7 => self.flags |= INVERT,
            9 => self.flags |= STRIKE,
            22 => self.flags &= !(BOLD | DIM),
            23 => self.flags &= !ITALIC,
            24 => self.flags &= !UNDERLINE,
            25 => self.flags &= !BLINK,
            27 => self.flags &= !INVERT,
            29 => self.flags &= !STRIKE,
            30..=37 => self.fg = Color::Ansi((n - 30) as u8),
            39 => self.fg = Color::Default,
            40..=47 => self.bg = Color::Ansi((n - 40) as u8),
            49 => self.bg = Color::Default,
            90..=97 => self.fg = Color::Ansi((n - 90 + 8) as u8),
            100..=107 => self.bg = Color::Ansi((n - 100 + 8) as u8),
            _ => {}
        }
    }

    /// Append the minimal SGR sequence that turns `self` into `to` (nothing if equal).
    pub fn write_transition(&self, to: &Attrs, out: &mut Vec<u8>) {
        if self == to {
            return;
        }
        let start = out.len();
        out.extend_from_slice(b"\x1b[");
        let params_start = out.len();
        let sep = |out: &mut Vec<u8>| {
            if out.len() > params_start {
                out.push(b';');
            }
        };

        let off = self.flags & !to.flags;
        let mut on = to.flags & !self.flags;
        if off & (flags::BOLD | flags::DIM) != 0 {
            // 22 clears both bold and dim; re-enable the one that must stay.
            sep(out);
            push_num(out, 22);
            on |= to.flags & (flags::BOLD | flags::DIM);
        }
        for &(_, bit, _, off_code) in flags::TABLE {
            if bit & (flags::BOLD | flags::DIM) == 0 && off & bit != 0 {
                sep(out);
                push_num(out, off_code);
            }
        }
        for &(_, bit, on_code, _) in flags::TABLE {
            if on & bit != 0 {
                sep(out);
                push_num(out, on_code);
            }
        }
        if self.fg != to.fg {
            sep(out);
            to.fg.write_params(false, out);
        }
        if self.bg != to.bg {
            sep(out);
            to.bg.write_params(true, out);
        }
        if out.len() == params_start {
            out.truncate(start);
        } else {
            out.push(b'm');
        }
    }
}

#[inline]
fn parse_num(field: &[u8]) -> Option<u16> {
    if field.is_empty() {
        return Some(0);
    }
    if field.len() > 5 {
        return None;
    }
    let mut v: u32 = 0;
    for &b in field {
        if !b.is_ascii_digit() {
            return None;
        }
        v = v * 10 + (b - b'0') as u32;
    }
    u16::try_from(v).ok()
}

#[inline]
fn parse_u8(field: &[u8]) -> Option<u8> {
    parse_num(field).and_then(|v| u8::try_from(v).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete(buf: &[u8]) -> (usize, bool) {
        match scan(buf, 0) {
            Escape::Complete { end, sgr } => (end, sgr),
            Escape::Incomplete => panic!("incomplete: {buf:?}"),
        }
    }

    #[test]
    fn scans_csi() {
        assert_eq!(complete(b"\x1b[1;31mX"), (7, true));
        assert_eq!(complete(b"\x1b[mX"), (3, true));
        assert_eq!(complete(b"\x1b[38:2::1:2:3m"), (14, true));
        assert_eq!(complete(b"\x1b[2KX"), (4, false));
        assert_eq!(complete(b"\x1b[?25l"), (6, false));
        assert_eq!(complete(b"\x1b[>4;1m"), (7, false)); // not SGR
        assert_eq!(scan(b"\x1b[1;3", 0), Escape::Incomplete);
        assert_eq!(complete(b"\x1b[1\nX"), (3, false)); // malformed
    }

    #[test]
    fn scans_strings_and_short() {
        assert_eq!(complete(b"\x1b]0;title\x07rest"), (10, false));
        assert_eq!(complete(b"\x1b]8;;http://x\x1b\\link"), (15, false));
        assert_eq!(scan(b"\x1b]0;tit", 0), Escape::Incomplete);
        assert_eq!(scan(b"\x1b]0;tit\x1b", 0), Escape::Incomplete);
        assert_eq!(complete(b"\x1b(Bx"), (3, false));
        assert_eq!(complete(b"\x1b7x"), (2, false));
        assert_eq!(complete(b"\x1b\x1b"), (1, false));
        assert_eq!(scan(b"\x1b", 0), Escape::Incomplete);
    }

    #[test]
    fn incomplete_tails() {
        assert_eq!(incomplete_tail(b"abc\x1b[3"), Some(3));
        assert_eq!(incomplete_tail(b"abc\x1b[3m"), None);
        assert_eq!(incomplete_tail(b"abc"), None);
        assert_eq!(incomplete_utf8_tail("aé".as_bytes()), None);
        assert_eq!(incomplete_utf8_tail(&"aé".as_bytes()[..2]), Some(1));
        assert_eq!(incomplete_utf8_tail(&"a😀".as_bytes()[..4]), Some(1));
        assert_eq!(incomplete_utf8_tail("a😀".as_bytes()), None);
    }

    #[test]
    fn sgr_state() {
        let mut a = Attrs::default();
        a.apply_sgr(b"1;31;48;5;200");
        assert_eq!(a.fg, Color::Ansi(1));
        assert_eq!(a.bg, Color::Indexed(200));
        assert_eq!(a.flags, flags::BOLD);
        a.apply_sgr(b"38;2;1;2;3;22");
        assert_eq!(a.fg, Color::Rgb(1, 2, 3));
        assert_eq!(a.flags, 0);
        a.apply_sgr(b"48:2::9:8:7;4:3");
        assert_eq!(a.bg, Color::Rgb(9, 8, 7));
        assert_eq!(a.flags, flags::UNDERLINE);
        a.apply_sgr(b"4:0;97");
        assert_eq!(a.flags, 0);
        assert_eq!(a.fg, Color::Ansi(15));
        a.apply_sgr(b"");
        assert_eq!(a, Attrs::default());
    }

    #[test]
    fn transitions_are_minimal() {
        let mut out = Vec::new();
        let base = Attrs::default();
        let bold_red = Attrs {
            fg: Color::Ansi(1),
            bg: Color::Default,
            flags: flags::BOLD | flags::DIM,
        };
        base.write_transition(&base, &mut out);
        assert!(out.is_empty());
        base.write_transition(&bold_red, &mut out);
        assert_eq!(out, b"\x1b[1;2;31m");
        out.clear();
        // Turning off bold but keeping dim needs 22 then 2.
        let dim = Attrs {
            flags: flags::DIM,
            ..bold_red
        };
        bold_red.write_transition(&dim, &mut out);
        assert_eq!(out, b"\x1b[22;2m");
        out.clear();
        dim.write_transition(&base, &mut out);
        assert_eq!(out, b"\x1b[22;39m");
    }
}
