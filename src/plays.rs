//! Plays: noticing that a song has been listened to, and remembering it.
//!
//! A song counts as played once you've heard half of it or four minutes, whichever comes first
//! (the rule Last.fm and ListenBrainz both use), and songs of 30 seconds or less never count.
//! Only time actually heard counts: loading, pauses and skipping ahead don't.
//!
//! Plays are kept in `~/.local/share/omaestro/plays.db`, one row per play, away from the
//! library index (which is a cache, safe to delete). They give "Recently played" and "Most
//! played", and how often you've played each song. Star ratings live there too, one row per
//! rated song: never in the music files themselves.

use crate::library::{Db, Source, Track};
use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::path::Path;

/// A song's identity across plays: its file, or its Spotify or YouTube id.
pub fn key(t: &Track) -> String {
    match t.source {
        Source::Local => format!("local:{}", t.path),
        Source::Spotify => t.uri.clone(),
        Source::YouTube => format!("youtube:{}", t.uri),
    }
}

/// Whether `heard` seconds of a song `duration` seconds long make a play (an unknown length
/// counts after four minutes).
pub fn counts(duration: f64, heard: f64) -> bool {
    if duration > 0.0 && duration <= 30.0 {
        return false;
    }
    let needed = if duration > 0.0 {
        (duration / 2.0).min(240.0)
    } else {
        240.0
    };
    heard >= needed
}

/// What the player is doing, as seen on one turn of the main loop.
pub struct Seen<'a> {
    /// The player's number for the song it was last asked to start.
    pub ticket: u64,
    pub track: Option<&'a Track>,
    /// Where in the song, and how long it is, in seconds.
    pub position: f64,
    pub duration: f64,
    /// Playing: not stopped, paused or getting ready.
    pub playing: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Happened {
    /// A song began to play (it's moving): "now playing". With when it began.
    Started(Track, u64),
    /// It's been heard enough to count as a play. With when it began.
    Played(Track, u64),
}

/// One listen to one song.
struct Listen {
    key: String,
    track: Track,
    ticket: u64,
    /// When it began to play (Unix seconds); set once it moves.
    began: Option<u64>,
    heard: f64,
    last_position: f64,
    counted: bool,
}

/// Follows the player from turn to turn and says when a song starts and when it counts.
#[derive(Default)]
pub struct Tracker {
    listen: Option<Listen>,
}

impl Tracker {
    /// `now` is the time (Unix seconds); `elapsed` the seconds since the last call.
    pub fn observe(&mut self, seen: Seen, now: u64, elapsed: f64) -> Vec<Happened> {
        let Some(track) = seen.track else {
            self.listen = None;
            return Vec::new();
        };
        let key = key(track);
        let new = match &self.listen {
            None => true,
            Some(l) if l.key != key => true,
            // Started again from the top. (Picking it up where it was, after switching
            // outputs or updating, is the same listen.)
            Some(l) => l.ticket != seen.ticket && seen.position < 2.0,
        };
        if new {
            self.listen = Some(Listen {
                key,
                track: track.clone(),
                ticket: seen.ticket,
                began: None,
                heard: 0.0,
                last_position: seen.position,
                counted: false,
            });
        }
        let Some(l) = self.listen.as_mut() else {
            return Vec::new();
        };
        l.ticket = seen.ticket;
        let mut happened = Vec::new();
        let step = seen.position - l.last_position;
        l.last_position = seen.position;
        // Moving forward by about the time that passed: heard. A bigger jump is a seek.
        if seen.playing && step > 0.0 && step <= elapsed + 1.0 {
            l.heard += step;
            if l.began.is_none() {
                let began = now.saturating_sub(step as u64);
                l.began = Some(began);
                happened.push(Happened::Started(l.track.clone(), began));
            }
        }
        let duration = if seen.duration > 0.0 {
            seen.duration
        } else {
            l.track.duration_ms as f64 / 1000.0
        };
        if !l.counted && counts(duration, l.heard) {
            l.counted = true;
            happened.push(Happened::Played(l.track.clone(), l.began.unwrap_or(now)));
        }
        happened
    }
}

/// Every play, in its own small database.
pub struct Store {
    conn: Connection,
}

/// "Recently played" or "Most played".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum List {
    Recent,
    Most,
}

impl List {
    pub fn name(self) -> &'static str {
        match self {
            List::Recent => "Recently played",
            List::Most => "Most played",
        }
    }
}

/// How many songs the lists hold.
pub const LIST_SIZE: usize = 100;

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("opening the plays file {}", path.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(2))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS plays (
                 id INTEGER PRIMARY KEY,
                 at INTEGER NOT NULL,
                 key TEXT NOT NULL,
                 -- The song as a JSPF track, as in a playlist.
                 track TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS plays_key ON plays(key);
             CREATE INDEX IF NOT EXISTS plays_at ON plays(at);
             CREATE TABLE IF NOT EXISTS ratings (
                 key TEXT PRIMARY KEY,
                 stars INTEGER NOT NULL,
                 at INTEGER NOT NULL,
                 track TEXT NOT NULL
             );",
        )?;
        Ok(Store { conn })
    }

    /// Remembers one play of `track`, which began at `at` (Unix seconds).
    pub fn record(&self, track: &Track, at: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO plays (at, key, track) VALUES (?1, ?2, ?3)",
            params![
                at as i64,
                key(track),
                crate::playlist::to_jspf(track).to_string()
            ],
        )?;
        Ok(())
    }

    /// Gives `track` 1 to 5 stars, or takes its rating off (0).
    pub fn rate(&self, track: &Track, stars: u8, at: u64) -> Result<()> {
        if stars == 0 {
            self.conn
                .execute("DELETE FROM ratings WHERE key = ?1", [key(track)])?;
        } else {
            self.conn.execute(
                "INSERT OR REPLACE INTO ratings (key, stars, at, track) VALUES (?1, ?2, ?3, ?4)",
                params![
                    key(track),
                    stars.min(5),
                    at as i64,
                    crate::playlist::to_jspf(track).to_string()
                ],
            )?;
        }
        Ok(())
    }

    /// Every rated song's stars, by `key`.
    pub fn ratings(&self) -> Result<HashMap<String, u8>> {
        let mut stmt = self.conn.prepare("SELECT key, stars FROM ratings")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u8>(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// How many times `track` has been played, and when it was last.
    pub fn played(&self, track: &Track) -> Result<(u64, Option<u64>)> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*), MAX(at) FROM plays WHERE key = ?1",
            [key(track)],
            |r| {
                Ok((
                    r.get::<_, i64>(0)? as u64,
                    r.get::<_, Option<i64>>(1)?.map(|t| t as u64),
                ))
            },
        )?)
    }

    /// How many different songs have been played, and when the last play was.
    pub fn summary(&self) -> Result<(usize, Option<u64>)> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(DISTINCT key), MAX(at) FROM plays", [], |r| {
                Ok((
                    r.get::<_, i64>(0)? as usize,
                    r.get::<_, Option<i64>>(1)?.map(|t| t as u64),
                ))
            })?)
    }

    /// One of the lists, as songs to play: each song once, its latest details.
    pub fn list(&self, which: List, db: &Db) -> Result<Vec<Track>> {
        // With MAX(at), SQLite takes the other columns from that newest row.
        let order = match which {
            List::Recent => "MAX(at) DESC",
            List::Most => "COUNT(*) DESC, MAX(at) DESC",
        };
        let mut stmt = self.conn.prepare(&format!(
            "SELECT track, MAX(at) FROM plays GROUP BY key ORDER BY {order} LIMIT ?1"
        ))?;
        let rows = stmt.query_map([LIST_SIZE as i64], |r| r.get::<_, String>(0))?;
        let mut tracks = Vec::new();
        for text in rows {
            if let Ok(v) = serde_json::from_str(&text?)
                && let Some(t) = crate::playlist::from_jspf(&v, db)
            {
                tracks.push(t);
            }
        }
        Ok(tracks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    fn song(title: &str, seconds: i64) -> Track {
        Track {
            source: Source::Spotify,
            uri: format!("spotify:track:{title}"),
            title: title.into(),
            artist: "Artist".into(),
            duration_ms: seconds * 1000,
            ..Track::default()
        }
    }

    /// Plays `track` from `from` for `seconds`, a second at a time; what happened.
    fn listen(
        tracker: &mut Tracker,
        track: &Track,
        ticket: u64,
        from: f64,
        seconds: u32,
        now: &mut u64,
    ) -> Vec<Happened> {
        let mut all = Vec::new();
        for s in 0..=seconds {
            *now += 1;
            all.extend(tracker.observe(
                Seen {
                    ticket,
                    track: Some(track),
                    position: from + s as f64,
                    duration: track.duration_ms as f64 / 1000.0,
                    playing: true,
                },
                *now,
                1.0,
            ));
        }
        all
    }

    #[test]
    fn the_rules() {
        assert!(!counts(30.0, 30.0), "30 seconds or less never counts");
        assert!(counts(200.0, 100.0), "half");
        assert!(!counts(200.0, 99.0));
        assert!(counts(600.0, 240.0), "four minutes of a long song");
        assert!(!counts(600.0, 239.0));
        assert!(counts(0.0, 240.0), "unknown length: four minutes");
    }

    #[test]
    fn a_song_counts_once_after_half_of_it() {
        let mut t = Tracker::default();
        let a = song("a", 200);
        let mut now = 1_000;
        let first = listen(&mut t, &a, 1, 0.0, 99, &mut now);
        assert_eq!(first.len(), 1, "started, not yet played: {first:?}");
        assert!(matches!(&first[0], Happened::Started(s, _) if s.title == "a"));
        let more = listen(&mut t, &a, 1, 100.0, 50, &mut now);
        assert_eq!(
            more.iter()
                .filter(|h| matches!(h, Happened::Played(..)))
                .count(),
            1
        );
        // Played: when it began, not when it counted.
        assert!(more.contains(&Happened::Played(a.clone(), 1_001)));
    }

    #[test]
    fn pauses_loading_and_skipping_ahead_dont_count() {
        let mut t = Tracker::default();
        let a = song("a", 200);
        let mut now = 0;
        // Loading: not moving, nothing happens.
        for _ in 0..30 {
            now += 1;
            let seen = Seen {
                ticket: 1,
                track: Some(&a),
                position: 0.0,
                duration: 200.0,
                playing: true,
            };
            assert!(t.observe(seen, now, 1.0).is_empty());
        }
        listen(&mut t, &a, 1, 0.0, 10, &mut now);
        // Skipping to 150 s and listening a little: only what was heard counts.
        let skipped = listen(&mut t, &a, 1, 150.0, 40, &mut now);
        assert!(skipped.is_empty(), "{skipped:?}");
        // Paused for a long time: nothing.
        for _ in 0..500 {
            now += 1;
            let seen = Seen {
                ticket: 1,
                track: Some(&a),
                position: 190.0,
                duration: 200.0,
                playing: false,
            };
            assert!(t.observe(seen, now, 1.0).is_empty());
        }
    }

    #[test]
    fn picking_up_where_it_was_is_the_same_listen_but_playing_again_is_new() {
        let mut t = Tracker::default();
        let a = song("a", 200);
        let mut now = 0;
        listen(&mut t, &a, 1, 0.0, 60, &mut now);
        // Switched outputs: a new ticket, carrying on from 60 s. Not started again.
        let switched = listen(&mut t, &a, 2, 61.0, 40, &mut now);
        assert!(matches!(switched.as_slice(), [Happened::Played(..)]));
        // Played again from the top: a new listen, and a new play once heard.
        let again = listen(&mut t, &a, 3, 0.0, 100, &mut now);
        assert!(matches!(
            again.as_slice(),
            [Happened::Started(..), Happened::Played(..)]
        ));
        // Another song.
        let b = song("b", 60);
        let other = listen(&mut t, &b, 4, 0.0, 30, &mut now);
        assert!(matches!(
            other.as_slice(),
            [Happened::Started(..), Happened::Played(..)]
        ));
    }

    #[test]
    fn keeps_ratings_until_taken_off() {
        let tmp = TempDir::new("ratings");
        let path = tmp.0.join("plays.db");
        let store = Store::open(&path).unwrap();
        let (a, b) = (song("a", 200), song("b", 200));
        assert!(store.ratings().unwrap().is_empty());
        store.rate(&a, 4, 100).unwrap();
        store.rate(&b, 2, 100).unwrap();
        store.rate(&b, 5, 200).unwrap();
        drop(store);

        let store = Store::open(&path).unwrap();
        let ratings = store.ratings().unwrap();
        assert_eq!(ratings.get(&key(&a)), Some(&4));
        assert_eq!(ratings.get(&key(&b)), Some(&5), "the newest rating wins");
        store.rate(&a, 0, 300).unwrap();
        assert_eq!(store.ratings().unwrap().len(), 1, "0 takes it off");
    }

    #[test]
    fn remembers_plays_and_lists_them() {
        let tmp = TempDir::new("plays");
        let db = Db::open(&tmp.0.join("index.db")).unwrap();
        let store = Store::open(&tmp.0.join("plays.db")).unwrap();
        let (a, b, c) = (song("a", 200), song("b", 200), song("c", 200));
        assert_eq!(store.summary().unwrap(), (0, None));
        assert!(store.list(List::Recent, &db).unwrap().is_empty());
        store.record(&a, 100).unwrap();
        store.record(&b, 200).unwrap();
        store.record(&a, 300).unwrap();
        store.record(&c, 400).unwrap();
        store.record(&a, 500).unwrap();
        store.record(&b, 600).unwrap();

        assert_eq!(store.played(&a).unwrap(), (3, Some(500)));
        assert_eq!(store.played(&song("never", 100)).unwrap(), (0, None));
        assert_eq!(store.summary().unwrap(), (3, Some(600)));
        let titles = |list: Vec<Track>| list.into_iter().map(|t| t.title).collect::<Vec<_>>();
        assert_eq!(
            titles(store.list(List::Recent, &db).unwrap()),
            ["b", "a", "c"]
        );
        assert_eq!(
            titles(store.list(List::Most, &db).unwrap()),
            ["a", "b", "c"]
        );
        let recent = store.list(List::Recent, &db).unwrap();
        assert_eq!(recent[0].uri, "spotify:track:b", "playable again");

        // Kept when reopened.
        drop(store);
        let store = Store::open(&tmp.0.join("plays.db")).unwrap();
        assert_eq!(store.played(&a).unwrap().0, 3);
    }
}
