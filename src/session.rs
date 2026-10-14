//! The queue as it was when OMAESTRO last closed, so it comes back on the next start: the
//! songs, the one it was on and how far in, and the playlist they came from. Nothing plays
//! until asked; then it carries on from there.
//!
//! The songs are kept as a playlist (`last-queue.jspf`, the same format as saved playlists,
//! so Spotify and YouTube Music songs come back too) and the rest beside it
//! (`last-queue.json`). Both live in `~/.local/share/omaestro`, outside the playlists folder,
//! and are written whenever the queue or the current song changes, so a crash loses at most
//! the position in the song.

use crate::library::{Db, Track};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const SONGS: &str = "last-queue.jspf";
const PLACE: &str = "last-queue.json";

/// What's restored.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub tracks: Vec<Track>,
    pub current: Option<usize>,
    /// Seconds into the current song.
    pub position: f64,
    pub playlist: Option<Playlist>,
}

/// The saved playlist the queue came from (or was saved as).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Playlist {
    pub name: String,
    pub path: PathBuf,
    /// Changed since it was last saved.
    pub unsaved: bool,
}

/// Everything but the songs.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Place {
    current: Option<usize>,
    position: f64,
    playlist: Option<Playlist>,
}

pub fn save(dir: &Path, session: &Session) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let name = session
        .playlist
        .as_ref()
        .map_or("Last queue", |p| p.name.as_str());
    crate::playlist::save(&dir.join(SONGS), name, &session.tracks)?;
    let place = Place {
        current: session.current,
        position: session.position,
        playlist: session.playlist.clone(),
    };
    let tmp = dir.join(format!("{PLACE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(&place)?)?;
    std::fs::rename(tmp, dir.join(PLACE))?;
    Ok(())
}

/// The last session, if there's one. Local songs get their details from the library index.
pub fn load(dir: &Path, db: &Db) -> Option<Session> {
    let (_, tracks) = crate::playlist::load(&dir.join(SONGS), db).ok()?;
    let place: Place = std::fs::read(dir.join(PLACE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let current = place.current.filter(|&i| i < tracks.len());
    Some(Session {
        position: if current.is_some() {
            place.position.max(0.0)
        } else {
            0.0
        },
        current,
        tracks,
        playlist: place.playlist,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Source;
    use crate::testutil::{TempDir, sample_library};

    #[test]
    fn the_queue_comes_back_as_it_was() {
        let tmp = TempDir::new("session");
        let db = sample_library(&tmp.0);
        let mut tracks = db.tracks_in("/m/Abba/Gold").unwrap();
        tracks.push(Track {
            source: Source::Spotify,
            uri: "spotify:track:4uLU6hMCjMI75M1A2tKUQC".into(),
            title: "Never Gonna Give You Up".into(),
            ..Track::default()
        });
        let session = Session {
            tracks,
            current: Some(1),
            position: 83.5,
            playlist: Some(Playlist {
                name: "Mix".into(),
                path: tmp.0.join("Mix.jspf"),
                unsaved: true,
            }),
        };
        let dir = tmp.0.join("state");
        assert_eq!(load(&dir, &db), None, "nothing saved yet");
        save(&dir, &session).unwrap();
        assert_eq!(load(&dir, &db), Some(session.clone()));

        // A shorter queue than the saved place (edited by hand): no current song.
        let short = Session {
            tracks: session.tracks[..1].to_vec(),
            ..session.clone()
        };
        save(&dir, &short).unwrap();
        let mut place: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join(PLACE)).unwrap()).unwrap();
        place["current"] = 5.into();
        std::fs::write(dir.join(PLACE), place.to_string()).unwrap();
        let loaded = load(&dir, &db).unwrap();
        assert_eq!((loaded.current, loaded.position), (None, 0.0));

        // An empty queue stays empty.
        let empty = Session {
            tracks: Vec::new(),
            current: None,
            position: 0.0,
            playlist: None,
        };
        save(&dir, &empty).unwrap();
        assert_eq!(load(&dir, &db), Some(empty));
    }
}
