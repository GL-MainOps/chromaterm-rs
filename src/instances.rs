//! Registry of running `ct` instances, used by `ct --reload`.
//!
//! Each highlighting `ct` (filter or program mode) creates
//! `<runtime dir>/chromaterm/<pid>` containing its process start time. The
//! file is removed on exit. `ct --reload` sends the reload signal only to
//! registered processes whose start time still matches. Unrelated processes
//! are never signalled, even when a PID is reused or a process is called
//! `ct`. (SIGUSR1's default action would kill them.)
//!
//! The runtime dir is `$XDG_RUNTIME_DIR` (per-user, mode 0700), or
//! `/tmp/chromaterm-<uid>` as a fallback. The fallback is only used if it is a
//! real directory owned by us and not accessible by others.

use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

use rustix::process::{Pid, Signal};

/// Directory holding the registrations of the current user.
pub fn registry_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
        Some(p) if p.is_absolute() => p.join("chromaterm"),
        _ => PathBuf::from(format!(
            "/tmp/chromaterm-{}",
            rustix::process::getuid().as_raw()
        )),
    };
    Some(base)
}

/// Create (or validate) a private directory: ours, not a symlink, no group/other access.
fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let meta = fs::symlink_metadata(dir)?;
    let uid = rustix::process::getuid().as_raw();
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not a private directory owned by us", dir.display()),
        ));
    }
    Ok(())
}

/// Process start time (clock ticks since boot) from `/proc/<pid>/stat`.
/// Together with the PID it identifies a process uniquely.
fn start_time(pid: i32) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name (field 2) may contain spaces and parentheses, so
    // count fields after the last ')'. starttime is field 22.
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(19).map(str::to_owned)
}

/// Registration of this process. The file is removed on drop.
#[derive(Debug)]
pub struct Registration {
    path: PathBuf,
}

impl Registration {
    /// Register the current process. Returns `None` if the registry is
    /// unavailable (reload then simply won't reach this instance).
    pub fn register() -> Option<Self> {
        let dir = registry_dir()?;
        ensure_private_dir(&dir).ok()?;
        let pid = std::process::id();
        let path = dir.join(pid.to_string());
        let stamp = start_time(pid as i32).unwrap_or_default();
        fs::write(&path, stamp).ok()?;
        Some(Registration { path })
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Outcome of [`signal_all`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReloadReport {
    /// Instances that were sent the reload signal.
    pub signalled: usize,
    /// Stale registrations (dead or reused PIDs) that were cleaned up.
    pub stale: usize,
}

/// Ask every registered, live `ct` instance of this user to reload.
pub fn signal_all() -> io::Result<ReloadReport> {
    let mut report = ReloadReport::default();
    let Some(dir) = registry_dir() else {
        return Ok(report);
    };
    if !dir.is_dir() {
        return Ok(report);
    }
    ensure_private_dir(&dir)?;
    let me = std::process::id() as i32;
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<i32>().ok())
            .filter(|&p| p != me)
        else {
            continue;
        };
        let recorded = fs::read_to_string(entry.path()).unwrap_or_default();
        let alive = Pid::from_raw(pid).filter(|p| rustix::process::test_kill_process(*p).is_ok());
        let same_process = match (alive, start_time(pid)) {
            (None, _) => false,
            // Without /proc (non-Linux) we can only trust liveness.
            (Some(_), None) => recorded.is_empty(),
            (Some(_), Some(now)) => now == recorded.trim(),
        };
        match alive {
            Some(p) if same_process => {
                if rustix::process::kill_process(p, Signal::USR1).is_ok() {
                    report.signalled += 1;
                }
            }
            _ => {
                let _ = fs::remove_file(entry.path());
                report.stale += 1;
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_time_of_self() {
        if Path::new("/proc/self/stat").exists() {
            assert!(start_time(std::process::id() as i32).is_some());
        }
    }

    #[test]
    fn private_dir_is_validated() {
        let tmp = tempfile::tempdir().unwrap();
        let ok = tmp.path().join("ok");
        ensure_private_dir(&ok).unwrap();
        let open = tmp.path().join("open");
        fs::create_dir(&open).unwrap();
        fs::set_permissions(&open, std::os::unix::fs::PermissionsExt::from_mode(0o777)).unwrap();
        assert!(ensure_private_dir(&open).is_err());
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&ok, &link).unwrap();
        assert!(ensure_private_dir(&link).is_err());
    }
}
