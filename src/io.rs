//! Filter mode: highlight stdin → stdout.

use std::io::{self, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::io::Errno;

use crate::stream::Stream;

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

/// Read until EOF with retries on `EINTR`.
pub(crate) fn read_retry(fd: BorrowedFd<'_>, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        match rustix::io::read(fd, &mut *buf) {
            Ok(n) => return Ok(n),
            Err(Errno::INTR) => continue,
            Err(e) => return Err(e.into()),
        }
    }
}

/// Wait until `fd` is readable or `timeout` passes. Returns false on timeout.
fn wait_readable(fd: BorrowedFd<'_>, timeout: Duration) -> io::Result<bool> {
    let ts = timespec(timeout);
    loop {
        let mut fds = [PollFd::new(&fd, PollFlags::IN)];
        match poll(&mut fds, Some(&ts)) {
            Ok(n) => return Ok(n > 0),
            Err(Errno::INTR) => continue,
            Err(e) => return Err(e.into()),
        }
    }
}

/// Highlight `input` into `out` until EOF.
///
/// While a partial line is pending, the read waits at most `timeout`. Then
/// the partial line is flushed (without splitting escape sequences). A second
/// timeout forces out anything still held back.
pub fn run_filter(
    stream: &mut Stream,
    input: BorrowedFd<'_>,
    out: &mut impl Write,
    timeout: Duration,
) -> io::Result<()> {
    let mut buf = vec![0u8; READ_SIZE];
    let mut stale = false;
    loop {
        if stream.has_pending() && !wait_readable(input, timeout)? {
            stream.flush_partial(stale);
            stale = stream.has_pending();
            drain_output(stream, out)?;
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

/// Filter the process's stdin to its stdout.
pub fn run_stdin(stream: &mut Stream, timeout: Duration) -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    match run_filter(stream, stdin.as_fd(), &mut stdout, timeout) {
        // `ct … | head` closing the pipe is a normal way to stop.
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}
