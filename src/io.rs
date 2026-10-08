//! Filter mode: highlight stdin → stdout.

use std::io::{self, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::io::Errno;

use crate::signals::Signals;
use crate::stream::{Reloader, Stream};

/// Size of each raw read.
pub const READ_SIZE: usize = 64 * 1024;

/// Convert a duration to a poll timeout.
pub(crate) fn timespec(d: Duration) -> Timespec {
    Timespec {
        tv_sec: d.as_secs() as _,
        tv_nsec: d.subsec_nanos() as _,
    }
}

/// Write all pending output of `stream` and flush.
pub(crate) fn drain_output(stream: &mut Stream, out: &mut impl Write) -> io::Result<()> {
    if !stream.output().is_empty() {
        out.write_all(stream.output())?;
        stream.clear_output();
    }
    out.flush()
}

/// Read with retries on `EINTR`.
pub(crate) fn read_retry(fd: BorrowedFd<'_>, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        match rustix::io::read(fd, &mut *buf) {
            Ok(n) => return Ok(n),
            Err(Errno::INTR) => continue,
            Err(e) => return Err(e.into()),
        }
    }
}

/// Reload the configuration into `stream`. On failure, keep the old config
/// and report on stderr (`raw`: the terminal is in raw mode, so use CRLF).
pub(crate) fn apply_reload(
    stream: &mut Stream,
    timeout: &mut Duration,
    reload: &mut Reloader<'_>,
    raw: bool,
) {
    match reload() {
        Ok(r) => {
            stream.reconfigure(r.highlighter, r.max_line_bytes);
            *timeout = r.read_timeout;
        }
        Err(e) => {
            let msg = format!("ct: config reload failed; keeping the previous config.\n{e}");
            let msg = if raw { msg.replace('\n', "\r\n") } else { msg };
            let _ = write!(
                io::stderr(),
                "{}{}",
                msg.trim_end(),
                if raw { "\r\n" } else { "\n" }
            );
        }
    }
}

/// Highlight `input` into `out` until EOF.
///
/// While a partial line is pending, the read waits at most `timeout`. Then the
/// partial line is flushed (without splitting escape sequences). A second
/// timeout forces out anything still held back. If `signals` is given, a
/// reload signal re-reads the config through `reload`.
pub fn run_filter(
    stream: &mut Stream,
    input: BorrowedFd<'_>,
    out: &mut impl Write,
    mut timeout: Duration,
    signals: Option<&Signals>,
    reload: &mut Reloader<'_>,
) -> io::Result<()> {
    let mut buf = vec![0u8; READ_SIZE];
    let mut stale = false;
    loop {
        let ts = timespec(timeout);
        let (in_ev, sig_ev) = {
            let mut fds = [
                PollFd::new(&input, PollFlags::IN),
                match signals {
                    Some(s) => PollFd::new(&s.wake, PollFlags::IN),
                    None => PollFd::new(&input, PollFlags::empty()),
                },
            ];
            let n = if signals.is_some() { 2 } else { 1 };
            match poll(&mut fds[..n], stream.has_pending().then_some(&ts)) {
                Ok(0) => {
                    stream.flush_partial(stale);
                    stale = stream.has_pending();
                    drain_output(stream, out)?;
                    continue;
                }
                Ok(_) => {}
                Err(Errno::INTR) => continue,
                Err(e) => return Err(e.into()),
            }
            (
                fds[0].revents(),
                if n == 2 {
                    fds[1].revents()
                } else {
                    PollFlags::empty()
                },
            )
        };
        if let Some(s) = signals.filter(|_| !sig_ev.is_empty()) {
            s.drain();
            if s.reload.take() {
                apply_reload(stream, &mut timeout, reload, false);
            }
        }
        if !in_ev.intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR) {
            continue;
        }
        let n = read_retry(input, &mut buf)?;
        if n == 0 {
            stream.finish();
            return drain_output(stream, out);
        }
        stale = false;
        stream.feed(&buf[..n]);
        drain_output(stream, out)?;
    }
}

/// Filter the process's stdin to its stdout, reloading on the reload signal.
pub fn run_stdin(
    stream: &mut Stream,
    timeout: Duration,
    reload: &mut Reloader<'_>,
) -> io::Result<()> {
    let signals = Signals::for_filter().ok();
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    match run_filter(
        stream,
        stdin.as_fd(),
        &mut stdout,
        timeout,
        signals.as_ref(),
        reload,
    ) {
        // `ct … | head` closing the pipe is a normal way to stop.
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}
