//! Signal handling for the I/O loops.
//!
//! Handlers only set atomic flags and write to a self-pipe, so `poll(2)` in the
//! event loop wakes up. All real work happens in the loop.

use std::io;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use signal_hook::consts::{SIGCHLD, SIGHUP, SIGINT, SIGQUIT, SIGTERM, SIGUSR1, SIGWINCH};

/// The signal that asks a running `ct` to reload its configuration
/// (same as Python ChromaTerm).
pub const RELOAD_SIGNAL: i32 = SIGUSR1;

/// A flag set by a signal handler.
#[derive(Clone, Default)]
pub struct Flag(Arc<AtomicBool>);

impl Flag {
    /// Return true (and clear) if the signal arrived since the last check.
    pub fn take(&self) -> bool {
        self.0.swap(false, Ordering::Relaxed)
    }
}

pub struct Signals {
    /// Readable whenever any registered signal arrived.
    pub wake: UnixStream,
    notify: UnixStream,
    pub reload: Flag,
    pub winch: Flag,
    pub child: Flag,
    /// Termination signals to forward to a child process.
    pub forward: Vec<(i32, Flag)>,
}

impl Signals {
    fn new() -> io::Result<Self> {
        let (wake, notify) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        notify.set_nonblocking(true)?;
        Ok(Signals {
            wake,
            notify,
            reload: Flag::default(),
            winch: Flag::default(),
            child: Flag::default(),
            forward: Vec::new(),
        })
    }

    fn register(&self, sig: i32) -> io::Result<Flag> {
        let flag = Flag::default();
        signal_hook::flag::register(sig, Arc::clone(&flag.0))?;
        signal_hook::low_level::pipe::register(sig, self.notify.try_clone()?)?;
        Ok(flag)
    }

    /// Filter mode: only config reload is handled. SIGINT etc. keep their
    /// default action.
    pub fn for_filter() -> io::Result<Self> {
        let mut s = Self::new()?;
        s.reload = s.register(RELOAD_SIGNAL)?;
        Ok(s)
    }

    /// PTY mode: reload, window size, child exit, and forwarding of
    /// termination signals to the child.
    pub fn for_pty() -> io::Result<Self> {
        let mut s = Self::new()?;
        s.reload = s.register(RELOAD_SIGNAL)?;
        s.winch = s.register(SIGWINCH)?;
        s.child = s.register(SIGCHLD)?;
        for sig in [SIGINT, SIGTERM, SIGHUP, SIGQUIT] {
            let flag = s.register(sig)?;
            s.forward.push((sig, flag));
        }
        Ok(s)
    }

    /// Empty the self-pipe after a wake-up.
    pub fn drain(&self) {
        let mut buf = [0u8; 64];
        while rustix::io::read(&self.wake, &mut buf).is_ok_and(|n| n > 0) {}
    }
}
