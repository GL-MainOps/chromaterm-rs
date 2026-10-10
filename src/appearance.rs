//! Automatic dark/light theme selection (the `auto` theme).
//!
//! Detection order:
//! 1. Ask the terminal for its background (OSC 11) and foreground (OSC 10).
//! 2. `$COLORFGBG` (set by rxvt, Konsole and others).
//! 3. Fall back to dark.
//!
//! The query goes to the controlling terminal (`/dev/tty`), followed by a
//! Primary Device Attributes request (DA1, `ESC [ c`). Virtually every
//! terminal answers DA1, and terminals answer in order, so the DA1 reply
//! marks the end of the exchange. Terminals without OSC 10/11 support
//! therefore cost one round-trip, not a timeout.
//!
//! Querying is only safe when nothing else is reading the terminal at that
//! moment, so it is skipped when:
//! - stdout is not a terminal (e.g. `ct | less`: the pager reads the keyboard);
//! - `ct` is not in the terminal's foreground process group (it would be
//!   stopped by SIGTTOU/SIGTTIN);
//! - `TERM` is unset or `dumb`.
//!
//! The caller also picks the moment: program mode queries before starting
//! the child, and filter mode only after the first input arrives, so upstream
//! password prompts (`sudo …| ct`) are already done. See `cli.rs`.

use std::io::{self, IsTerminal};
use std::os::fd::{AsFd, BorrowedFd};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, poll};
use rustix::fs::{Mode, OFlags};
use rustix::io::Errno;
use rustix::termios::{self, LocalModes, OptionalActions, SpecialCodeIndex};

/// How long to wait for the terminal. DA1 normally ends the exchange after one
/// round-trip. This only bounds terminals that never answer.
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(1);
/// OSC 11 (background), OSC 10 (foreground), then DA1.
const QUERY: &[u8] = b"\x1b]11;?\x1b\\\x1b]10;?\x1b\\\x1b[c";
/// Stop reading after this much (a misbehaving terminal or a flood of input).
const MAX_REPLY: usize = 4096;

pub use chromaterm_core::appearance::{Appearance, Rgb, classify, from_colorfgbg, parse_color};

/// Where a detection result came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// The terminal reported its colors.
    Terminal {
        background: Rgb,
        foreground: Option<Rgb>,
    },
    /// `$COLORFGBG`.
    ColorFgBg(String),
    /// Nothing usable. The string says why.
    Fallback(String),
}

/// Result of automatic detection.
#[derive(Clone, Debug)]
pub struct Detection {
    pub appearance: Appearance,
    pub source: Source,
    /// Keyboard input that arrived while waiting for the terminal's reply.
    /// Program mode forwards it to the child, so no keystrokes are lost.
    pub typeahead: Vec<u8>,
}

impl Detection {
    pub fn theme(&self) -> &'static str {
        self.appearance.theme()
    }

    /// Human-readable origin, e.g. `terminal background #0e1317`.
    pub fn describe(&self) -> String {
        let hex = |(r, g, b): Rgb| format!("#{r:02x}{g:02x}{b:02x}");
        match &self.source {
            Source::Terminal {
                background,
                foreground: Some(fg),
            } => format!(
                "terminal background {} / foreground {}",
                hex(*background),
                hex(*fg)
            ),
            Source::Terminal { background, .. } => {
                format!("terminal background {}", hex(*background))
            }
            Source::ColorFgBg(v) => format!("$COLORFGBG={v}"),
            Source::Fallback(why) => format!("default ({why})"),
        }
    }
}

/// Detect without touching the terminal: `$COLORFGBG`, else dark.
pub fn detect_passive(reason: &str) -> Detection {
    let colorfgbg = std::env::var("COLORFGBG").ok();
    if let Some(v) = colorfgbg.as_deref() {
        if let Some(appearance) = from_colorfgbg(v) {
            return Detection {
                appearance,
                source: Source::ColorFgBg(v.to_owned()),
                typeahead: Vec::new(),
            };
        }
    }
    Detection {
        appearance: Appearance::Dark,
        source: Source::Fallback(reason.to_owned()),
        typeahead: Vec::new(),
    }
}

/// Detect, querying the terminal when that is safe (see the module docs).
pub fn detect() -> Detection {
    match query_controlling_terminal(QUERY_TIMEOUT) {
        Ok(reply) => match reply.background {
            Some(bg) => Detection {
                appearance: classify(bg, reply.foreground),
                source: Source::Terminal {
                    background: bg,
                    foreground: reply.foreground,
                },
                typeahead: reply.typeahead,
            },
            None => {
                let why = if reply.answered {
                    "terminal does not report its colors"
                } else {
                    "no answer from the terminal"
                };
                Detection {
                    typeahead: reply.typeahead,
                    ..detect_passive(why)
                }
            }
        },
        Err(why) => detect_passive(why),
    }
}

/// Query `/dev/tty` if it is safe. `Err` explains why it was skipped.
fn query_controlling_terminal(timeout: Duration) -> Result<Reply, &'static str> {
    if !io::stdout().is_terminal() {
        return Err("stdout is not a terminal");
    }
    match std::env::var("TERM") {
        Ok(t) if !t.is_empty() && t != "dumb" => {}
        _ => return Err("TERM is unset or dumb"),
    }
    let tty = rustix::fs::open(
        "/dev/tty",
        OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| "no controlling terminal")?;
    match termios::tcgetpgrp(&tty) {
        Ok(fg) if fg == rustix::process::getpgrp() => {}
        _ => return Err("not in the terminal's foreground process group"),
    }
    query_fd(tty.as_fd(), timeout).map_err(|_| "terminal query failed")
}

/// Restores the terminal mode on drop.
struct ModeGuard<'a> {
    fd: BorrowedFd<'a>,
    saved: termios::Termios,
}

impl Drop for ModeGuard<'_> {
    fn drop(&mut self) {
        let _ = termios::tcsetattr(self.fd, OptionalActions::Now, &self.saved);
    }
}

/// Run the query/answer exchange on a terminal file descriptor.
///
/// Only canonical mode and echo are switched off. Signals (Ctrl-C) keep
/// working, and the mode is restored before returning.
pub fn query_fd(fd: BorrowedFd<'_>, timeout: Duration) -> io::Result<Reply> {
    let saved = termios::tcgetattr(fd)?;
    let mut mode = saved.clone();
    mode.local_modes
        .remove(LocalModes::ICANON | LocalModes::ECHO);
    mode.special_codes[SpecialCodeIndex::VMIN] = 1;
    mode.special_codes[SpecialCodeIndex::VTIME] = 0;
    termios::tcsetattr(fd, OptionalActions::Now, &mode)?;
    let _guard = ModeGuard { fd, saved };

    let mut sent = 0;
    while sent < QUERY.len() {
        match rustix::io::write(fd, &QUERY[sent..]) {
            Ok(n) => sent += n,
            Err(Errno::INTR) => {}
            Err(e) => return Err(e.into()),
        }
    }

    let deadline = Instant::now() + timeout;
    let mut buf = Vec::with_capacity(256);
    let mut chunk = [0u8; 512];
    while buf.len() < MAX_REPLY && !contains_da1(&buf) {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        let ts = crate::io::timespec(left);
        let mut fds = [PollFd::new(&fd, PollFlags::IN)];
        match poll(&mut fds, Some(&ts)) {
            Ok(0) => break,
            Ok(_) => {}
            Err(Errno::INTR) => continue,
            Err(e) => return Err(e.into()),
        }
        match rustix::io::read(fd, &mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(Errno::INTR | Errno::AGAIN) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(parse_reply(&buf))
}

/// What the terminal sent back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reply {
    pub background: Option<Rgb>,
    pub foreground: Option<Rgb>,
    /// True if the DA1 reply arrived (the terminal is responsive).
    pub answered: bool,
    /// Everything that was not part of a reply (user keystrokes).
    pub typeahead: Vec<u8>,
}

/// Length of a DA1 reply (`ESC [ ? <digits;> c`) starting at `i`, if any.
fn da1_len(buf: &[u8], i: usize) -> Option<usize> {
    if buf.get(i..i + 3)? != b"\x1b[?" {
        return None;
    }
    let mut j = i + 3;
    while let Some(&b) = buf.get(j) {
        match b {
            b'0'..=b'9' | b';' => j += 1,
            b'c' => return Some(j + 1 - i),
            _ => return None,
        }
    }
    None
}

fn contains_da1(buf: &[u8]) -> bool {
    memchr::memchr_iter(0x1b, buf).any(|i| da1_len(buf, i).is_some())
}

/// Split a raw reply into colors, the DA1 marker, and user typeahead.
pub fn parse_reply(buf: &[u8]) -> Reply {
    let mut reply = Reply::default();
    let mut i = 0;
    while i < buf.len() {
        if let Some(len) = da1_len(buf, i) {
            reply.answered = true;
            i += len;
            continue;
        }
        if buf[i..].starts_with(b"\x1b]") {
            // OSC reply, terminated by BEL or ST (ESC \). An unterminated
            // tail is a reply cut off by the timeout: drop it.
            let body = &buf[i + 2..];
            let (end, term_len) = match memchr::memchr2(0x07, 0x1b, body) {
                Some(p) if body[p] == 0x07 => (p, 1),
                Some(p) if body.get(p + 1) == Some(&b'\\') => (p, 2),
                _ => (body.len(), 0),
            };
            let content = &body[..end];
            if let Some(color) = content.strip_prefix(b"11;").and_then(parse_color) {
                reply.background = Some(color);
            } else if let Some(color) = content.strip_prefix(b"10;").and_then(parse_color) {
                reply.foreground = Some(color);
            }
            i += 2 + end + term_len;
            continue;
        }
        reply.typeahead.push(buf[i]);
        i += 1;
    }
    reply
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::OwnedFd;

    #[test]
    fn replies_parse_with_typeahead() {
        let r = parse_reply(
            b"ab\x1b]11;rgb:ffff/ffff/ffff\x1b\\\x1b]10;rgb:0000/0000/0000\x07c\x1b[?62;22c\x1b[A",
        );
        assert_eq!(r.background, Some((255, 255, 255)));
        assert_eq!(r.foreground, Some((0, 0, 0)));
        assert!(r.answered);
        // User keys (including an arrow key) are kept, replies are not.
        assert_eq!(r.typeahead, b"abc\x1b[A");

        let r = parse_reply(b"\x1b[?1;2c");
        assert_eq!((r.background, r.answered), (None, true));
        // A reply cut off by the timeout is dropped, not treated as typing.
        let r = parse_reply(b"x\x1b]11;rgb:ff");
        assert_eq!((r.background, r.typeahead.as_slice()), (None, &b"x"[..]));
    }

    /// A pseudo-terminal pair: (master = "terminal emulator", slave = our tty).
    fn pty() -> (OwnedFd, OwnedFd) {
        use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
        grantpt(&master).unwrap();
        unlockpt(&master).unwrap();
        let name = ptsname(&master, Vec::new()).unwrap();
        let slave = rustix::fs::open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY,
            Mode::empty(),
        )
        .unwrap();
        (master, slave)
    }

    /// Emulate a terminal: wait for the query, then send `answer`.
    fn answer_with(master: OwnedFd, answer: &'static [u8]) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let mut seen = Vec::new();
            let mut buf = [0u8; 256];
            while !seen.ends_with(b"\x1b[c") {
                match rustix::io::read(&master, &mut buf) {
                    Ok(n) if n > 0 => seen.extend_from_slice(&buf[..n]),
                    _ => return,
                }
            }
            let _ = rustix::io::write(&master, answer);
            std::thread::sleep(Duration::from_millis(300));
        })
    }

    #[test]
    fn query_reads_colors_and_restores_mode() {
        let (master, slave) = pty();
        let before = termios::tcgetattr(&slave).unwrap();
        let t = answer_with(
            master,
            b"hi\x1b]11;rgb:ffff/ffff/ffff\x1b\\\x1b]10;rgb:1f1f/2323/2828\x1b\\\x1b[?62;c",
        );
        let reply = query_fd(slave.as_fd(), Duration::from_secs(2)).unwrap();
        assert_eq!(reply.background, Some((255, 255, 255)));
        assert_eq!(reply.foreground, Some((0x1f, 0x23, 0x28)));
        assert_eq!(reply.typeahead, b"hi");
        let after = termios::tcgetattr(&slave).unwrap();
        assert_eq!(before.local_modes, after.local_modes);
        t.join().unwrap();
    }

    #[test]
    fn terminal_without_osc_support_ends_at_da1() {
        let (master, slave) = pty();
        let t = answer_with(master, b"\x1b[?1;2c");
        let started = Instant::now();
        let reply = query_fd(slave.as_fd(), Duration::from_secs(5)).unwrap();
        assert!(reply.answered && reply.background.is_none());
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "must not wait for the timeout"
        );
        t.join().unwrap();
    }

    #[test]
    fn silent_terminal_times_out() {
        let (_master, slave) = pty();
        let started = Instant::now();
        let reply = query_fd(slave.as_fd(), Duration::from_millis(150)).unwrap();
        assert_eq!(reply, Reply::default());
        assert!(started.elapsed() >= Duration::from_millis(150));
    }
}
