//! How each source is doing, as a coloured dot and a word: in Settings, and on Search's
//! provider pills. The checks run in the background, each with a time limit: a NAS that's gone
//! away can make even looking at a folder hang for minutes, and the screen never waits for one.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Good,
    Warning,
    Bad,
    /// Not set up, so nothing to check.
    Off,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub health: Health,
    pub word: String,
}

impl Status {
    pub fn new(health: Health, word: impl Into<String>) -> Status {
        Status {
            health,
            word: word.into(),
        }
    }
}

/// The latest check of each source; none until the first one is done.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Statuses {
    pub local: Option<Status>,
    pub spotify: Option<Status>,
    pub youtube: Option<Status>,
    pub sonos: Option<Status>,
}

/// How long one check may take before it counts as "not answering".
pub const LIMIT: Duration = Duration::from_secs(8);

/// Runs `f` on its own thread and waits up to `limit` for it: none if it took longer (the
/// thread finishes on its own, unwatched).
pub fn within<T: Send + 'static>(
    limit: Duration,
    f: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(limit).ok()
}

/// The music folders: each one readable, missing, or not answering (a NAS gone away).
pub fn local(folders: Vec<PathBuf>) -> Status {
    let answers: Vec<Option<bool>> = folders
        .into_iter()
        .map(|folder| {
            within(Duration::from_secs(3), move || {
                std::fs::read_dir(folder).is_ok()
            })
        })
        .collect();
    local_from(&answers)
}

fn local_from(answers: &[Option<bool>]) -> Status {
    let n = answers.len();
    let ready = answers.iter().filter(|a| **a == Some(true)).count();
    let hung = answers.iter().any(Option::is_none);
    match (ready, hung) {
        _ if n == 0 => Status::new(Health::Off, "no music folders set"),
        (r, _) if r == n => Status::new(Health::Good, "ready"),
        (0, true) => Status::new(Health::Bad, "not answering"),
        (0, false) => Status::new(Health::Bad, "can't be read"),
        (r, _) => Status::new(
            Health::Warning,
            format!("{} of {n} folders unreadable", n - r),
        ),
    }
}

/// Spotify, from a check of the sign-in (none: no answer in time), and whether Spotify can
/// play on this computer (Soloist set up).
pub fn spotify_from(checked: Option<Result<(), String>>, plays_here: bool) -> Status {
    match checked {
        None => Status::new(Health::Warning, "not answering"),
        Some(Ok(())) if plays_here => Status::new(Health::Good, "signed in"),
        Some(Ok(())) => Status::new(Health::Warning, "signed in, but can't play here yet"),
        Some(Err(e))
            if e.contains("signed you out")
                || e.contains("refresh the sign-in")
                || e.contains("didn't accept") =>
        {
            Status::new(Health::Bad, "sign-in expired")
        }
        Some(Err(e)) if e.contains("reaching Spotify") => {
            Status::new(Health::Warning, "can't reach Spotify")
        }
        Some(Err(_)) => Status::new(Health::Warning, "Spotify said no"),
    }
}

/// YouTube Music, from asking whose account the saved login is (none: no answer in time).
pub fn youtube_from(checked: Option<Result<String, String>>) -> Status {
    match checked {
        None => Status::new(Health::Warning, "not answering"),
        Some(Ok(_)) => Status::new(Health::Good, "signed in"),
        Some(Err(e)) if e.contains("reaching YouTube") => {
            Status::new(Health::Warning, "can't reach YouTube")
        }
        Some(Err(_)) => Status::new(Health::Bad, "browser login no longer works"),
    }
}

/// Sonos: how many groups' speakers answered, of how many.
pub fn sonos_from(answering: usize, groups: usize) -> Status {
    match (answering, groups) {
        (_, 0) => Status::new(Health::Bad, "no speakers found"),
        (a, g) if a == g => Status::new(
            Health::Good,
            if g == 1 {
                "1 group answering".to_string()
            } else {
                format!("{g} groups answering")
            },
        ),
        (0, _) => Status::new(Health::Bad, "not answering"),
        (a, g) => Status::new(Health::Warning, format!("{a} of {g} groups answering")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_source_gets_a_dot_and_a_word() {
        let word = |s: Status| (s.health, s.word);
        assert_eq!(
            word(local_from(&[Some(true), Some(true)])),
            (Health::Good, "ready".into())
        );
        assert_eq!(
            word(local_from(&[Some(true), Some(false)])),
            (Health::Warning, "1 of 2 folders unreadable".into())
        );
        assert_eq!(
            word(local_from(&[None])),
            (Health::Bad, "not answering".into())
        );
        assert_eq!(
            word(local_from(&[Some(false)])),
            (Health::Bad, "can't be read".into())
        );
        assert_eq!(local_from(&[]).health, Health::Off);

        assert_eq!(spotify_from(Some(Ok(())), true).health, Health::Good);
        assert_eq!(spotify_from(Some(Ok(())), false).health, Health::Warning);
        let expired = "Spotify signed you out (sign-ins last six months): connect again";
        assert_eq!(
            word(spotify_from(Some(Err(expired.into())), true)),
            (Health::Bad, "sign-in expired".into())
        );
        let offline = "reaching Spotify: dns failed";
        assert_eq!(
            spotify_from(Some(Err(offline.into())), true).word,
            "can't reach Spotify"
        );
        assert_eq!(spotify_from(None, true).word, "not answering");

        assert_eq!(youtube_from(Some(Ok("Me".into()))).health, Health::Good);
        assert_eq!(
            word(youtube_from(Some(Err("not signed in".into())))),
            (Health::Bad, "browser login no longer works".into())
        );
        assert_eq!(
            youtube_from(Some(Err("reaching YouTube Music: timeout".into()))).health,
            Health::Warning
        );

        assert_eq!(
            word(sonos_from(3, 3)),
            (Health::Good, "3 groups answering".into())
        );
        assert_eq!(sonos_from(1, 3).word, "1 of 3 groups answering");
        assert_eq!(sonos_from(0, 3).health, Health::Bad);
    }

    #[test]
    fn a_check_that_hangs_doesnt_hold_anything_up() {
        let started = std::time::Instant::now();
        let answer = within(Duration::from_millis(100), || {
            std::thread::sleep(Duration::from_secs(2));
            true
        });
        assert_eq!(answer, None);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(within(Duration::from_secs(1), || 7), Some(7));
    }
}
