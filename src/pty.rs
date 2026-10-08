//! Program mode: run a command in a pseudo-terminal and highlight its output.
//!
//! The child gets its own session with the PTY slave as its controlling
//! terminal, so interactive programs (ssh, top, vim, shells) behave normally.
//! `ct` puts the real terminal in raw mode, forwards keystrokes, propagates
//! window-size changes and forwards termination signals. It exits with the
//! child's status.

use std::ffi::OsString;
use std::io::{self, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};
use rustix::event::{PollFd, PollFlags, poll};
use rustix::fs::{Mode, OFlags};
use rustix::io::Errno;
use rustix::process::{Pid, Signal};
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
use rustix::termios::{self, OptionalActions, Termios};

use crate::io::{READ_SIZE, apply_reload, drain_output, read_retry, timespec};
use crate::signals::Signals;
use crate::stream::{Reloader, Stream};

/// Stop reading our stdin while this much input waits for the child.
const MAX_TO_CHILD: usize = 1 << 20;
/// End-of-transmission (Ctrl-D): what a closed, non-tty stdin turns into.
const EOT: u8 = 0x04;

/// Restores the terminal mode on drop.
struct RawGuard<'a> {
    fd: BorrowedFd<'a>,
    saved: Termios,
}

impl Drop for RawGuard<'_> {
    fn drop(&mut self) {
        let _ = termios::tcsetattr(self.fd, OptionalActions::Now, &self.saved);
    }
}

/// Size of the real terminal (stdout first, then stdin).
fn window_size() -> Option<termios::Winsize> {
    termios::tcgetwinsize(io::stdout())
        .or_else(|_| termios::tcgetwinsize(io::stdin()))
        .ok()
}

fn open_pty() -> Result<(OwnedFd, OwnedFd)> {
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
    rustix::io::fcntl_setfd(&master, rustix::io::FdFlags::CLOEXEC)?;
    grantpt(&master)?;
    unlockpt(&master)?;
    let name = ptsname(&master, Vec::new())?;
    let slave = rustix::fs::open(
        name.as_c_str(),
        OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    Ok((master, slave))
}

fn spawn(program: &[OsString], slave: OwnedFd) -> Result<Child> {
    let mut cmd = Command::new(&program[0]);
    cmd.args(&program[1..])
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave));
    // SAFETY: the closure only performs async-signal-safe syscalls (setsid,
    // ioctl) between fork and exec. It does not allocate or take locks.
    unsafe {
        cmd.pre_exec(|| {
            rustix::process::setsid()?;
            // fd 0 is the PTY slave at this point; make it our controlling tty.
            rustix::process::ioctl_tiocsctty(BorrowedFd::borrow_raw(0))?;
            Ok(())
        });
    }
    // `cmd` (holding the parent's copies of the slave) is dropped on return, so
    // reads on the master see EIO once the child's side closes.
    cmd.spawn()
        .with_context(|| format!("failed to run `{}`", program[0].to_string_lossy()))
}

fn write_some(fd: BorrowedFd<'_>, buf: &mut Vec<u8>) -> io::Result<()> {
    match rustix::io::write(fd, buf) {
        Ok(n) => {
            buf.drain(..n);
            Ok(())
        }
        Err(Errno::AGAIN | Errno::INTR) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Run `program` under a PTY, highlighting its output. Returns the exit code.
pub fn run(
    program: &[OsString],
    stream: &mut Stream,
    timeout: Duration,
    reload: &mut Reloader<'_>,
) -> Result<i32> {
    let stdin = io::stdin();
    let stdin_fd = stdin.as_fd();
    let stdin_tty = termios::isatty(stdin_fd);

    let (master, slave) = open_pty().context("cannot allocate a pseudo-terminal")?;
    let saved = if stdin_tty {
        let t = termios::tcgetattr(stdin_fd)?;
        termios::tcsetattr(&slave, OptionalActions::Now, &t)?;
        Some(t)
    } else {
        None
    };
    if let Some(ws) = window_size() {
        let _ = termios::tcsetwinsize(&master, ws);
    }

    let signals = Signals::for_pty().context("cannot install signal handlers")?;
    let mut child = spawn(program, slave)?;
    let pid = Pid::from_child(&child);

    let _raw = match saved {
        Some(saved) => {
            let mut raw = saved.clone();
            raw.make_raw();
            termios::tcsetattr(stdin_fd, OptionalActions::Now, &raw)?;
            Some(RawGuard {
                fd: stdin_fd,
                saved,
            })
        }
        None => None,
    };
    rustix::fs::fcntl_setfl(&master, OFlags::NONBLOCK | OFlags::RDWR)?;

    let result = event_loop(
        &master, stdin_fd, stdin_tty, &signals, &mut child, pid, stream, timeout, reload,
    );
    drop(_raw);
    result?;

    // Reap the child BEFORE closing the master. The child may already have
    // closed the slave (hence EIO) but not exited yet. Closing the master now
    // would hang up its session and kill it with SIGHUP.
    let status = child.wait()?;
    drop(master);
    Ok(status
        .code()
        .or_else(|| status.signal().map(|s| 128 + s))
        .unwrap_or(1))
}

#[allow(clippy::too_many_arguments)]
fn event_loop(
    master: &OwnedFd,
    stdin_fd: BorrowedFd<'_>,
    stdin_tty: bool,
    signals: &Signals,
    child: &mut Child,
    pid: Pid,
    stream: &mut Stream,
    mut timeout: Duration,
    reload: &mut Reloader<'_>,
) -> Result<()> {
    let mut stdout = io::stdout().lock();
    let mut buf = vec![0u8; READ_SIZE];
    let mut to_child: Vec<u8> = Vec::new();
    let mut stdin_open = true;
    let mut stale = false;

    loop {
        let ts = timespec(timeout);
        let want_stdin = stdin_open && to_child.len() < MAX_TO_CHILD;
        let master_flags = if to_child.is_empty() {
            PollFlags::IN
        } else {
            PollFlags::IN | PollFlags::OUT
        };
        let mut fds = [
            PollFd::new(master, master_flags),
            PollFd::new(&signals.wake, PollFlags::IN),
            PollFd::new(
                &stdin_fd,
                if want_stdin {
                    PollFlags::IN
                } else {
                    PollFlags::empty()
                },
            ),
        ];
        let nfds = if want_stdin { 3 } else { 2 };
        let wait = stream.has_pending().then_some(&ts);
        match poll(&mut fds[..nfds], wait) {
            Ok(0) => {
                stream.flush_partial(stale);
                stale = stream.has_pending();
                drain_output(stream, &mut stdout)?;
                continue;
            }
            Ok(_) => {}
            Err(Errno::INTR) => continue,
            Err(e) => return Err(e.into()),
        }
        let (m_ev, s_ev) = (fds[0].revents(), fds[1].revents());
        let i_ev = if nfds == 3 {
            fds[2].revents()
        } else {
            PollFlags::empty()
        };

        if !s_ev.is_empty() {
            signals.drain();
            if signals.reload.take() {
                apply_reload(stream, &mut timeout, reload, stdin_tty);
            }
            if signals.winch.take() {
                if let Some(ws) = window_size() {
                    let _ = termios::tcsetwinsize(master, ws);
                }
            }
            for (sig, flag) in &signals.forward {
                if flag.take() {
                    if let Some(sig) = Signal::from_named_raw(*sig) {
                        let _ = rustix::process::kill_process(pid, sig);
                    }
                }
            }
            if signals.child.take() && child.try_wait()?.is_some() {
                // The child is gone. Drain whatever it left in the PTY.
                // Don't wait on descendants that keep the slave open.
                loop {
                    match rustix::io::read(master, &mut buf) {
                        Ok(n) if n > 0 => stream.feed(&buf[..n]),
                        Err(Errno::INTR) => continue,
                        _ => break,
                    }
                }
                break;
            }
        }

        if m_ev.intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR) {
            match rustix::io::read(master, &mut buf) {
                Ok(0) | Err(Errno::IO) => break,
                Ok(n) => {
                    stale = false;
                    stream.feed(&buf[..n]);
                    drain_output(stream, &mut stdout)?;
                }
                Err(Errno::AGAIN | Errno::INTR) => {}
                Err(e) => return Err(e.into()),
            }
        }
        if m_ev.contains(PollFlags::OUT) && !to_child.is_empty() {
            write_some(master.as_fd(), &mut to_child)?;
        }

        if i_ev.intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR) {
            let n = read_retry(stdin_fd, &mut buf).unwrap_or(0);
            if n == 0 {
                stdin_open = false;
                if !stdin_tty {
                    to_child.push(EOT);
                }
            } else {
                to_child.extend_from_slice(&buf[..n]);
            }
            if !to_child.is_empty() {
                write_some(master.as_fd(), &mut to_child)?;
            }
        }
    }
    stream.finish();
    drain_output(stream, &mut stdout)?;
    stdout.flush()?;
    Ok(())
}
