//! Line framing between raw I/O reads and the [`Highlighter`].
//!
//! Complete lines (terminated by `\n`, `\r\n` or `\r`) are highlighted at once.
//! A trailing partial line is held back so a later read can complete it.
//! The I/O loop calls [`Stream::flush_partial`] when no more data arrives
//! within the read timeout. Escape sequences and UTF-8 code points are never
//! split, and the partial buffer is bounded.

use crate::ansi;
use crate::engine::Highlighter;

/// Default upper bound for a held-back partial line.
pub const DEFAULT_MAX_PENDING: usize = 64 * 1024;
/// A held-back incomplete escape sequence longer than this is flushed anyway.
const MAX_ESCAPE_HOLD: usize = 4096;

/// What a configuration reload produces.
pub struct Reconfig {
    pub highlighter: Highlighter,
    pub read_timeout: std::time::Duration,
    pub max_line_bytes: usize,
}

/// Re-reads the configuration. Called when a reload is requested.
pub type Reloader<'a> = dyn FnMut() -> Result<Reconfig, String> + 'a;

#[derive(Debug)]
pub struct Stream {
    hl: Highlighter,
    pending: Vec<u8>,
    out: Vec<u8>,
    max_pending: usize,
    /// Start of an escape sequence that was force-flushed before it ended.
    /// The bytes that finish it are passed through untouched.
    carry: Vec<u8>,
}

impl Stream {
    pub fn new(hl: Highlighter) -> Self {
        Self::with_max_pending(hl, DEFAULT_MAX_PENDING)
    }

    pub fn with_max_pending(hl: Highlighter, max_pending: usize) -> Self {
        Stream {
            hl,
            pending: Vec::new(),
            out: Vec::with_capacity(64 * 1024),
            max_pending: max_pending.max(16),
            carry: Vec::new(),
        }
    }

    pub fn highlighter(&self) -> &Highlighter {
        &self.hl
    }

    /// Swap in a newly loaded highlighter. Held-back data and the program's
    /// color state carry over.
    pub fn reconfigure(&mut self, mut hl: Highlighter, max_pending: usize) {
        hl.inherit_from(&self.hl);
        self.hl = hl;
        self.max_pending = max_pending.max(16);
    }

    /// Recover the highlighter (it keeps the program's color state).
    pub fn into_highlighter(self) -> Highlighter {
        self.hl
    }

    /// Bytes ready to be written. Call [`Stream::clear_output`] after writing them.
    pub fn output(&self) -> &[u8] {
        &self.out
    }

    pub fn clear_output(&mut self) {
        self.out.clear();
    }

    /// True while a partial line is held back (the I/O loop should use a timeout).
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Feed raw bytes: complete lines go to the output and the rest is held back.
    pub fn feed(&mut self, mut data: &[u8]) {
        if !self.carry.is_empty() {
            data = self.finish_carried_escape(data);
        }
        if !self.pending.is_empty() {
            match memchr::memchr2(b'\n', b'\r', data) {
                Some(i) => {
                    let (head, rest) = data.split_at(i);
                    self.pending.extend_from_slice(head);
                    let line = std::mem::take(&mut self.pending);
                    let term_len = terminator_len(rest);
                    self.hl.highlight_line(&line, &mut self.out);
                    self.out.extend_from_slice(&rest[..term_len]);
                    self.pending = line;
                    self.pending.clear();
                    data = &rest[term_len..];
                }
                None => {
                    self.pending.extend_from_slice(data);
                    if self.pending.len() >= self.max_pending {
                        self.flush_partial(false);
                    }
                    return;
                }
            }
        }
        while let Some(i) = memchr::memchr2(b'\n', b'\r', data) {
            let term_len = terminator_len(&data[i..]);
            self.hl.highlight_line(&data[..i], &mut self.out);
            self.out.extend_from_slice(&data[i..i + term_len]);
            data = &data[i + term_len..];
        }
        if !data.is_empty() {
            self.pending.extend_from_slice(data);
            if self.pending.len() >= self.max_pending {
                self.flush_partial(false);
            }
        }
    }

    /// Highlight and emit the held-back partial line.
    ///
    /// With `force == false`, a trailing incomplete escape sequence or UTF-8
    /// code point stays held back. With `force == true` (EOF, or a second
    /// timeout), everything is emitted.
    pub fn flush_partial(&mut self, force: bool) {
        if self.pending.is_empty() {
            return;
        }
        let mut cut = self.pending.len();
        if !force {
            if let Some(i) = ansi::incomplete_tail(&self.pending) {
                if self.pending.len() - i <= MAX_ESCAPE_HOLD {
                    cut = i;
                }
            }
            if let Some(i) = ansi::incomplete_utf8_tail(&self.pending[..cut]) {
                cut = i;
            }
        }
        if cut == 0 {
            return;
        }
        let pending = std::mem::take(&mut self.pending);
        self.hl.highlight_line(&pending[..cut], &mut self.out);
        if force {
            if let Some(i) = ansi::incomplete_tail(&pending[..cut]) {
                self.carry.clear();
                self.carry.extend_from_slice(&pending[i..cut]);
            }
        }
        self.pending = pending;
        self.pending.drain(..cut);
    }

    /// Pass through the bytes that complete a force-flushed escape sequence.
    /// Return the rest of `data`.
    fn finish_carried_escape<'d>(&mut self, data: &'d [u8]) -> &'d [u8] {
        let take = data.len().min(MAX_ESCAPE_HOLD);
        let mut seq = std::mem::take(&mut self.carry);
        let carried = seq.len();
        seq.extend_from_slice(&data[..take]);
        let used = match ansi::scan(&seq, 0) {
            ansi::Escape::Complete { end, .. } => {
                self.hl.observe_escape(&seq[..end]);
                end.saturating_sub(carried)
            }
            ansi::Escape::Incomplete if seq.len() < MAX_ESCAPE_HOLD => {
                seq.truncate(carried + take);
                self.out.extend_from_slice(&data[..take]);
                self.carry = seq;
                return &data[take..];
            }
            ansi::Escape::Incomplete => take,
        };
        self.out.extend_from_slice(&data[..used]);
        &data[used..]
    }

    /// Flush everything (end of input).
    pub fn finish(&mut self) {
        self.flush_partial(true);
    }
}

/// Length of the line terminator at the start of `rest` (`\r\n` → 2, else 1).
#[inline]
fn terminator_len(rest: &[u8]) -> usize {
    if rest[0] == b'\r' && rest.get(1) == Some(&b'\n') {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{Color, Style};
    use crate::engine::{Matcher, Rule, RuleStyles};

    fn stream(max: usize) -> Stream {
        let rule = Rule {
            description: "num".into(),
            matcher: Matcher::new(r"\d+").unwrap(),
            styles: RuleStyles::Whole(Style {
                fg: Some(Color::Ansi(1)),
                ..Style::default()
            }),
            exclusive: false,
        };
        Stream::with_max_pending(Highlighter::new(vec![rule]), max)
    }
    fn take(s: &mut Stream) -> String {
        let o = String::from_utf8_lossy(s.output()).replace('\x1b', "E");
        s.clear_output();
        o
    }

    #[test]
    fn splits_lines_and_holds_partials() {
        let mut s = stream(1024);
        s.feed(b"a1\r\nb2\rc");
        assert_eq!(take(&mut s), "aE[31m1E[39m\r\nbE[31m2E[39m\r");
        assert!(s.has_pending());
        s.feed(b"3");
        assert_eq!(take(&mut s), "");
        s.feed(b"4\n");
        // The partial line is joined, so "34" is one match.
        assert_eq!(take(&mut s), "cE[31m34E[39m\n");
        assert!(!s.has_pending());
    }

    #[test]
    fn flush_holds_incomplete_escape_and_utf8() {
        let mut s = stream(1024);
        s.feed(b"x1\x1b[3");
        s.flush_partial(false);
        assert_eq!(take(&mut s), "xE[31m1E[39m");
        s.feed(b"1mz\n");
        assert_eq!(take(&mut s), "E[31mz\n");

        s.feed("é".as_bytes().split_at(1).0);
        s.flush_partial(false);
        assert_eq!(take(&mut s), "");
        s.flush_partial(true);
        assert_eq!(s.output(), &[0xc3]);
    }

    #[test]
    fn bounded_pending() {
        let mut s = stream(16);
        s.feed(&[b'a'; 40]);
        assert!(s.output().len() >= 16);
        assert!(s.pending.len() < 16);
    }
}
