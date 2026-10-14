//! The library index: one SQLite file in `~/.cache/omaestro/`. It's a cache of what's on disk,
//! so deleting it only costs one full scan.

use super::{Album, Track};
use anyhow::{Context, Result};
use rusqlite::{Connection, Row, params};
use std::collections::HashMap;
use std::path::Path;

/// Bump when the schema changes; an older index is dropped and rebuilt by the next scan.
const SCHEMA_VERSION: i64 = 2;

const SCHEMA: &str = "
CREATE TABLE tracks (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    size INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    album_dir TEXT NOT NULL,
    title TEXT NOT NULL,
    artist TEXT NOT NULL,
    album TEXT NOT NULL,
    album_artist TEXT NOT NULL,
    album_artist_sort TEXT NOT NULL,
    year INTEGER,
    disc_no INTEGER,
    track_no INTEGER,
    duration_ms INTEGER NOT NULL,
    sample_rate INTEGER,
    bit_depth INTEGER,
    genre TEXT NOT NULL,
    mb_track_id TEXT NOT NULL,
    mb_album_id TEXT NOT NULL,
    isrc TEXT NOT NULL
);
CREATE INDEX tracks_album_dir ON tracks(album_dir);

-- One row per album folder, rebuilt from `tracks` after every scan that changed something.
CREATE TABLE albums (
    dir TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    artist TEXT NOT NULL,
    artist_sort TEXT NOT NULL,
    year INTEGER,
    bit_depth INTEGER,
    sample_rate INTEGER,
    tracks INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL
);
CREATE INDEX albums_artist ON albums(artist);

-- Full-text search over tracks. remove_diacritics lets \"beyonce\" find \"Beyoncé\".
CREATE VIRTUAL TABLE tracks_fts USING fts5(
    title, artist, album, album_artist,
    content='tracks', content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);
";

const TRACK_COLUMNS: &str = "id, path, album_dir, title, artist, album, album_artist, \
    album_artist_sort, year, disc_no, track_no, duration_ms, sample_rate, bit_depth, genre, \
    mb_track_id, mb_album_id, isrc";

pub struct Db {
    conn: Connection,
}

/// A file's size and modification time (whole seconds), used to spot changed files.
pub type Stamp = (i64, i64);

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("opening library index {}", path.display()))?;
        // WAL lets the screen read while the scanner writes.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;

        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version != SCHEMA_VERSION {
            conn.execute_batch(
                "DROP TABLE IF EXISTS tracks_fts; DROP TABLE IF EXISTS albums; \
                 DROP TABLE IF EXISTS tracks;",
            )?;
            conn.execute_batch(SCHEMA)?;
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        // Marks such as "the album list and search are behind the tracks". Added in 0.9.7,
        // to indexes of any version, so no rescan is needed for it.
        conn.execute_batch("CREATE TABLE IF NOT EXISTS marks (name TEXT PRIMARY KEY)")?;
        Ok(Db { conn })
    }

    /// Whether the album list and search need rebuilding from the tracks: changes were saved
    /// but the rebuild didn't finish (OMAESTRO stopped in between, say), or the album list
    /// doesn't add up to the tracks (an index from before that was noted).
    pub fn behind(&self) -> Result<bool> {
        let marked: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM marks WHERE name = 'behind')",
            [],
            |r| r.get(0),
        )?;
        let (tracks, in_albums): (i64, i64) = self.conn.query_row(
            "SELECT (SELECT COUNT(*) FROM tracks),              (SELECT COALESCE(SUM(tracks), 0) FROM albums)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(marked || tracks != in_albums)
    }

    /// Every indexed file with its size and mtime.
    pub fn stamps(&self) -> Result<HashMap<String, Stamp>> {
        let mut stmt = self.conn.prepare("SELECT path, size, mtime FROM tracks")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Adds or updates tracks in one transaction. A track keeps its id when its file changes.
    pub fn upsert(&mut self, items: &[(Track, Stamp)]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO tracks (path, size, mtime, album_dir, title, artist, album, \
                 album_artist, album_artist_sort, year, disc_no, track_no, duration_ms, \
                 sample_rate, bit_depth, genre, mb_track_id, mb_album_id, isrc)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, \
                 ?17, ?18, ?19)
                 ON CONFLICT(path) DO UPDATE SET size=?2, mtime=?3, album_dir=?4, title=?5, \
                 artist=?6, album=?7, album_artist=?8, album_artist_sort=?9, year=?10, \
                 disc_no=?11, track_no=?12, duration_ms=?13, sample_rate=?14, bit_depth=?15, \
                 genre=?16, mb_track_id=?17, mb_album_id=?18, isrc=?19",
            )?;
            for (t, (size, mtime)) in items {
                stmt.execute(params![
                    t.path,
                    size,
                    mtime,
                    t.album_dir,
                    t.title,
                    t.artist,
                    t.album,
                    t.album_artist,
                    t.album_artist_sort,
                    t.year,
                    t.disc_no,
                    t.track_no,
                    t.duration_ms,
                    t.sample_rate,
                    t.bit_depth,
                    t.genre,
                    t.mb_track_id,
                    t.mb_album_id,
                    t.isrc
                ])?;
            }
        }
        if !items.is_empty() {
            tx.execute("INSERT OR IGNORE INTO marks VALUES ('behind')", [])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete(&mut self, paths: &[String]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare("DELETE FROM tracks WHERE path = ?1")?;
            for path in paths {
                stmt.execute([path])?;
            }
        }
        if !paths.is_empty() {
            tx.execute("INSERT OR IGNORE INTO marks VALUES ('behind')", [])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Rebuilds the album list and the search index from `tracks`.
    /// An album is filed under the artist most of its tracks name, so one stray tag
    /// doesn't split it across two artists.
    pub fn rebuild_derived(&mut self) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut read = tx.prepare(
                "SELECT album_dir, album, album_artist, album_artist_sort, year, bit_depth, \
                 sample_rate, duration_ms FROM tracks ORDER BY album_dir",
            )?;
            let mut rows = read.query([])?;
            tx.execute("DELETE FROM albums", [])?;
            let mut write = tx.prepare(
                "INSERT INTO albums (dir, title, artist, artist_sort, year, bit_depth, \
                 sample_rate, tracks, duration_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;

            let mut current: Option<AlbumAcc> = None;
            while let Some(r) = rows.next()? {
                let dir: String = r.get(0)?;
                if current.as_ref().is_some_and(|a| a.dir != dir) {
                    current.take().unwrap().write(&mut write)?;
                }
                let acc = current.get_or_insert_with(|| AlbumAcc::new(dir));
                acc.add(r)?;
            }
            if let Some(acc) = current {
                acc.write(&mut write)?;
            }
        }
        tx.execute("INSERT INTO tracks_fts(tracks_fts) VALUES('rebuild')", [])?;
        tx.execute("DELETE FROM marks WHERE name = 'behind'", [])?;
        tx.commit()?;
        Ok(())
    }

    #[cfg(test)]
    pub fn conn_for_tests(&self) -> &Connection {
        &self.conn
    }

    pub fn track_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0))?)
    }

    /// Album artists in sort order.
    pub fn artists(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT artist FROM albums GROUP BY artist ORDER BY MIN(artist_sort) COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// An artist's albums, oldest first.
    pub fn albums_by(&self, artist: &str) -> Result<Vec<Album>> {
        let mut stmt = self.conn.prepare(
            "SELECT dir, title, artist, year, bit_depth, sample_rate, tracks, duration_ms \
             FROM albums WHERE artist = ?1 ORDER BY year, title COLLATE NOCASE, dir",
        )?;
        let rows = stmt.query_map([artist], album_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Every album, by artist and then oldest first.
    pub fn albums(&self) -> Result<Vec<Album>> {
        let mut stmt = self.conn.prepare(
            "SELECT dir, title, artist, year, bit_depth, sample_rate, tracks, duration_ms \
             FROM albums ORDER BY artist_sort COLLATE NOCASE, year, title COLLATE NOCASE, dir",
        )?;
        let rows = stmt.query_map([], album_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Every track on an artist's albums, album by album, oldest album first.
    pub fn tracks_by(&self, artist: &str) -> Result<Vec<Track>> {
        let sql = format!(
            "SELECT {} FROM tracks t JOIN albums a ON a.dir = t.album_dir WHERE a.artist = ?1 \
             ORDER BY a.year, a.title COLLATE NOCASE, a.dir, t.disc_no, t.track_no, t.path",
            prefixed("t")
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([artist], track_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The track for a file, if it's in the index.
    pub fn track_at(&self, path: &str) -> Result<Option<Track>> {
        let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE path = ?1");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query_map([path], track_from_row)?;
        Ok(rows.next().transpose()?)
    }

    /// An album's tracks in disc and track order.
    pub fn tracks_in(&self, album_dir: &str) -> Result<Vec<Track>> {
        let sql = format!(
            "SELECT {TRACK_COLUMNS} FROM tracks WHERE album_dir = ?1 \
             ORDER BY disc_no, track_no, path"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([album_dir], track_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Tracks matching every word typed, best matches first. Each word also matches as a
    /// prefix, so "led zep" finds Led Zeppelin.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Track>> {
        self.search_in(query, "", limit)
    }

    /// Songs whose title or artist matches every word typed, best matches first. Album names
    /// are left out, so an album's name finds the album rather than all of its songs.
    pub fn search_songs(&self, query: &str, limit: usize) -> Result<Vec<Track>> {
        self.search_in(query, SONG_COLUMNS, limit)
    }

    /// How many tracks `search` would find without a limit.
    pub fn count(&self, query: &str) -> Result<usize> {
        self.count_in(query, "")
    }

    /// How many songs `search_songs` would find without a limit.
    pub fn count_songs(&self, query: &str) -> Result<usize> {
        self.count_in(query, SONG_COLUMNS)
    }

    fn count_in(&self, query: &str, columns: &str) -> Result<usize> {
        let Some(fts_query) = fts_query(query, columns) else {
            return Ok(0);
        };
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tracks_fts WHERE tracks_fts MATCH ?1",
            [fts_query],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    fn search_in(&self, query: &str, columns: &str, limit: usize) -> Result<Vec<Track>> {
        let Some(fts_query) = fts_query(query, columns) else {
            return Ok(Vec::new());
        };
        let sql = format!(
            "SELECT {} FROM tracks_fts f JOIN tracks t ON t.id = f.rowid \
             WHERE tracks_fts MATCH ?1 ORDER BY bm25(tracks_fts, 4.0, 2.0, 2.0, 2.0) LIMIT ?2",
            prefixed("t")
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![fts_query, limit as i64], track_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

/// The search columns `search_songs` looks in, as an FTS5 column filter.
const SONG_COLUMNS: &str = "{title artist}";

/// TRACK_COLUMNS with a table alias, e.g. "t.id, t.path, …".
fn prefixed(alias: &str) -> String {
    TRACK_COLUMNS
        .split(", ")
        .map(|c| format!("{alias}.{c}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Turns typed words into an FTS5 query: each word quoted (so punctuation is harmless) and
/// matched as a prefix, optionally only in some columns ("{title artist}").
fn fts_query(input: &str, columns: &str) -> Option<String> {
    let filter = if columns.is_empty() {
        String::new()
    } else {
        format!("{columns}: ")
    };
    let words: Vec<String> = input
        .split_whitespace()
        .map(|w| format!("{filter}\"{}\"*", w.replace('"', "\"\"")))
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

fn album_from_row(r: &Row) -> rusqlite::Result<Album> {
    Ok(Album {
        dir: r.get(0)?,
        title: r.get(1)?,
        artist: r.get(2)?,
        year: r.get(3)?,
        bit_depth: r.get(4)?,
        sample_rate: r.get(5)?,
        tracks: r.get(6)?,
        duration_ms: r.get(7)?,
    })
}

fn track_from_row(r: &Row) -> rusqlite::Result<Track> {
    Ok(Track {
        id: r.get(0)?,
        path: r.get(1)?,
        album_dir: r.get(2)?,
        title: r.get(3)?,
        artist: r.get(4)?,
        album: r.get(5)?,
        album_artist: r.get(6)?,
        album_artist_sort: r.get(7)?,
        year: r.get(8)?,
        disc_no: r.get(9)?,
        track_no: r.get(10)?,
        duration_ms: r.get(11)?,
        sample_rate: r.get(12)?,
        bit_depth: r.get(13)?,
        genre: r.get(14)?,
        mb_track_id: r.get(15)?,
        mb_album_id: r.get(16)?,
        isrc: r.get(17)?,
        ..Track::default()
    })
}

/// Collects one album's tracks while `rebuild_derived` walks them in folder order.
struct AlbumAcc {
    dir: String,
    titles: HashMap<String, u32>,
    artists: HashMap<(String, String), u32>,
    year: Option<i32>,
    bit_depth: Option<u8>,
    sample_rate: Option<u32>,
    tracks: u32,
    duration_ms: i64,
}

impl AlbumAcc {
    fn new(dir: String) -> Self {
        AlbumAcc {
            dir,
            titles: HashMap::new(),
            artists: HashMap::new(),
            year: None,
            bit_depth: None,
            sample_rate: None,
            tracks: 0,
            duration_ms: 0,
        }
    }

    fn add(&mut self, r: &Row) -> rusqlite::Result<()> {
        *self.titles.entry(r.get(1)?).or_default() += 1;
        *self.artists.entry((r.get(2)?, r.get(3)?)).or_default() += 1;
        self.year = self.year.or(r.get(4)?);
        self.bit_depth = self.bit_depth.max(r.get(5)?);
        self.sample_rate = self.sample_rate.max(r.get(6)?);
        self.tracks += 1;
        self.duration_ms += r.get::<_, i64>(7)?;
        Ok(())
    }

    fn write(self, stmt: &mut rusqlite::Statement) -> rusqlite::Result<()> {
        let title = most_common(self.titles).unwrap_or_default();
        let (artist, artist_sort) = most_common(self.artists).unwrap_or_default();
        stmt.execute(params![
            self.dir,
            title,
            artist,
            artist_sort,
            self.year,
            self.bit_depth,
            self.sample_rate,
            self.tracks,
            self.duration_ms
        ])?;
        Ok(())
    }
}

/// The value seen most often; ties go to the alphabetically first, so results are stable.
fn most_common<K: Ord + Clone>(counts: HashMap<K, u32>) -> Option<K> {
    counts
        .into_iter()
        .max_by(|(ka, a), (kb, b)| a.cmp(b).then_with(|| kb.cmp(ka)))
        .map(|(k, _)| k)
}
