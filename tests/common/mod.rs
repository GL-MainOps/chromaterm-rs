#![allow(dead_code)]

use assert_cmd::Command;
use tempfile::TempDir;

/// A `ct` command isolated from the user's real configuration.
pub struct Ct {
    pub home: TempDir,
}

impl Ct {
    pub fn new() -> Self {
        Ct {
            home: tempfile::tempdir().unwrap(),
        }
    }

    pub fn cmd(&self) -> Command {
        let mut c = Command::cargo_bin("ct").unwrap();
        c.env("HOME", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join(".config"))
            .env_remove("CHROMATERM_CONFIG")
            .env_remove("CHROMATERM_THEME")
            .env_remove("COLORFGBG")
            .env_remove("COLORTERM")
            .env("TERM", "xterm-256color");
        c
    }
}

/// Inline config with a single rule and no built-in rules.
pub fn only_rule(regex: &str, color: &str) -> String {
    serde_json::json!({ "defaults": false, "rules": [{ "regex": regex, "color": color }] })
        .to_string()
}

/// Remove every SGR (`ESC [ … m`) sequence.
pub fn strip_sgr(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'[') {
            let mut j = i + 2;
            while j < bytes.len()
                && (bytes[j].is_ascii_digit() || bytes[j] == b';' || bytes[j] == b':')
            {
                j += 1;
            }
            if bytes.get(j) == Some(&b'm') {
                i = j + 1;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

impl Ct {
    /// Private runtime dir for the instance registry (used by --reload).
    pub fn runtime_dir(&self) -> std::path::PathBuf {
        let dir = self.home.path().join("run");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .unwrap();
        dir
    }
}

/// How the fake terminal emulator answers queries.
#[derive(Clone, Copy, Default)]
pub struct Terminal {
    /// OSC 11 reply payload, e.g. `rgb:ffff/ffff/ffff` (None: unsupported).
    pub background: Option<&'static str>,
    /// OSC 10 reply payload.
    pub foreground: Option<&'static str>,
    /// Answer DA1 (all real terminals do).
    pub da1: bool,
    /// Keys "typed" by the user while the query is in flight.
    pub typed: &'static [u8],
}

/// Output of [`run_in_terminal`].
pub struct TerminalRun {
    /// Everything ct wrote to the terminal (queries included).
    pub screen: String,
    pub status: std::process::ExitStatus,
    pub elapsed: std::time::Duration,
}

impl TerminalRun {
    /// True if ct asked the terminal for its colors.
    pub fn queried(&self) -> bool {
        self.screen.contains("\x1b]11;?")
    }
}

/// Run `ct` with a pseudo-terminal as its controlling terminal and stdout,
/// while a fake terminal emulator answers color queries. With `stdin`
/// given, ct reads it from a pipe (filter mode). Otherwise the terminal is
/// stdin too (program mode).
pub fn run_in_terminal(
    ct: &Ct,
    args: &[&str],
    envs: &[(&str, &str)],
    stdin: Option<&[u8]>,
    term: Terminal,
) -> TerminalRun {
    use rustix::fs::{Mode, OFlags};
    use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
    use std::io::Write;
    use std::os::fd::BorrowedFd;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

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

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin("ct"));
    cmd.args(args)
        .env("HOME", ct.home.path())
        .env("XDG_CONFIG_HOME", ct.home.path().join(".config"))
        .env("XDG_RUNTIME_DIR", ct.runtime_dir())
        .env("TERM", "xterm-256color")
        .env_remove("CHROMATERM_CONFIG")
        .env_remove("CHROMATERM_THEME")
        .env_remove("COLORFGBG")
        .env_remove("COLORTERM")
        .envs(envs.iter().copied())
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave.try_clone().unwrap()));
    match stdin {
        Some(_) => cmd.stdin(Stdio::piped()),
        None => cmd.stdin(Stdio::from(slave.try_clone().unwrap())),
    };
    // SAFETY: only async-signal-safe syscalls between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            rustix::process::setsid()?;
            rustix::process::ioctl_tiocsctty(BorrowedFd::borrow_raw(1))?;
            Ok(())
        });
    }
    let started = std::time::Instant::now();
    let mut child = cmd.spawn().unwrap();
    drop(cmd);
    drop(slave);

    let emulator = std::thread::spawn(move || {
        let mut screen = Vec::new();
        let mut answered = 0;
        let mut buf = [0u8; 4096];
        loop {
            match rustix::io::read(&master, &mut buf) {
                Ok(n) if n > 0 => screen.extend_from_slice(&buf[..n]),
                _ => break, // EIO: ct exited and closed the terminal
            }
            let queries = screen.windows(3).filter(|w| *w == b"\x1b[c").count();
            while answered < queries {
                answered += 1;
                let mut reply = term.typed.to_vec();
                if let Some(bg) = term.background {
                    reply.extend_from_slice(format!("\x1b]11;{bg}\x1b\\").as_bytes());
                }
                if let Some(fg) = term.foreground {
                    reply.extend_from_slice(format!("\x1b]10;{fg}\x07").as_bytes());
                }
                if term.da1 {
                    reply.extend_from_slice(b"\x1b[?62;22c");
                }
                let _ = rustix::io::write(&master, &reply);
            }
        }
        String::from_utf8_lossy(&screen).into_owned()
    });

    if let Some(data) = stdin {
        let mut pipe = child.stdin.take().unwrap();
        pipe.write_all(data).unwrap();
    }
    let status = child.wait().unwrap();
    let elapsed = started.elapsed();
    TerminalRun {
        screen: emulator.join().unwrap(),
        status,
        elapsed,
    }
}
