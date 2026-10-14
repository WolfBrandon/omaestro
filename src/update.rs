//! Update in place: when `./install.sh` puts a new version where this one was started from, a
//! running OMAESTRO notices and restarts itself into it, keeping the queue, the song and the
//! place in it, the output and the volume.
//!
//! The program file is remembered at start: once it's replaced, this process's own path reads
//! "… (deleted)". A new file only counts once it answers `--version`, so a half-written build
//! is never started.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

/// Watches the program file for a new version.
pub struct Watch {
    path: Option<PathBuf>,
    /// The file as it was at start: (inode, modified).
    stamp: Option<(u64, SystemTime)>,
    checked: Instant,
    /// The new version, once there's a working one: "0.9.4".
    pub ready: Option<String>,
}

/// How often the file is looked at.
const EVERY: Duration = Duration::from_secs(3);

fn stamp(path: &Path) -> Option<(u64, SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.ino(), meta.modified().ok()?))
}

impl Watch {
    pub fn new() -> Watch {
        Watch::of(std::env::current_exe().ok())
    }

    fn of(path: Option<PathBuf>) -> Watch {
        let stamp = path.as_deref().and_then(stamp);
        Watch {
            path,
            stamp,
            checked: Instant::now(),
            ready: None,
        }
    }

    /// The program file, to start the new version from.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Looks at the file (every few seconds); true when a new working version has just
    /// appeared.
    pub fn poll(&mut self) -> bool {
        if self.ready.is_some() || self.checked.elapsed() < EVERY {
            return false;
        }
        self.checked = Instant::now();
        let Some(path) = &self.path else {
            return false;
        };
        let now = stamp(path);
        if now.is_none() || now == self.stamp {
            return false;
        }
        match version_of(path) {
            Some(version) => {
                self.ready = Some(version);
                true
            }
            // Still being written, or broken: look again next time.
            None => false,
        }
    }
}

/// "omaestro 0.9.4" from `<path> --version` → "0.9.4".
fn version_of(path: &Path) -> Option<String> {
    let out = Command::new(path).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let version = text.trim().strip_prefix("omaestro ")?.trim();
    (out.status.success() && !version.is_empty()).then(|| version.to_string())
}

/// What to do now that a new version is ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    /// Nothing's playing (or it's paused): switch now, nothing to resume.
    Now,
    /// Playing: wait for a pause, or for U.
    Wait,
}

pub fn when(playing: bool) -> Switch {
    if playing { Switch::Wait } else { Switch::Now }
}

/// Replaces this process with the program at `path` (the new version), in the same terminal.
/// `resume` starts it playing from where this one was. Returns only if that failed.
pub fn restart(path: &Path, resume: bool) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new(path);
    if resume {
        cmd.arg("--resume");
    }
    cmd.exec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::os::unix::fs::PermissionsExt;

    fn program(path: &Path, version: &str) {
        let tmp = path.with_extension("new");
        std::fs::write(&tmp, format!("#!/bin/sh\necho omaestro {version}\n")).unwrap();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).unwrap();
        // install.sh copies and renames, so the file changes in one go.
        std::fs::rename(&tmp, path).unwrap();
    }

    #[test]
    fn notices_a_new_working_version() {
        let tmp = TempDir::new("update");
        let path = tmp.0.join("omaestro");
        program(&path, "0.9.3");
        let mut watch = Watch::of(Some(path.clone()));
        watch.checked = Instant::now() - EVERY;
        assert!(!watch.poll(), "nothing new yet");

        // A broken file isn't taken up.
        std::fs::write(tmp.0.join("broken"), "not a program").unwrap();
        std::fs::rename(tmp.0.join("broken"), &path).unwrap();
        watch.checked = Instant::now() - EVERY;
        assert!(!watch.poll());
        assert_eq!(watch.ready, None);

        program(&path, "0.9.4");
        watch.checked = Instant::now() - EVERY;
        assert!(watch.poll());
        assert_eq!(watch.ready.as_deref(), Some("0.9.4"));
        assert!(!watch.poll(), "said once");

        assert_eq!(when(true), Switch::Wait);
        assert_eq!(when(false), Switch::Now);
    }
}
