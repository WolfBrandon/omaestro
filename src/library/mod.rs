//! The local music library: a SQLite index of the music folders, kept fresh by a scanner.

pub mod db;
pub mod scan;

pub use db::Db;

/// Where a track comes from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Source {
    #[default]
    Local,
    Spotify,
    YouTube,
}

impl Source {
    /// The name used in playlist files and settings.
    pub fn key(self) -> &'static str {
        match self {
            Source::Local => "local",
            Source::Spotify => "spotify",
            Source::YouTube => "youtube",
        }
    }

    pub fn from_key(key: &str) -> Option<Source> {
        match key {
            "local" => Some(Source::Local),
            "spotify" => Some(Source::Spotify),
            "youtube" => Some(Source::YouTube),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Source::Local => "Local",
            Source::Spotify => "Spotify",
            Source::YouTube => "YouTube Music",
        }
    }
}

/// One song: an audio file in the index, or a Spotify or YouTube Music track.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Track {
    pub id: i64,
    pub path: String,
    /// The album's folder. Discs in `DISC1/`, `CD2/` … subfolders count as one album.
    pub album_dir: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// ALBUMARTIST, or ARTIST when a file has none. This is what the library tree groups by.
    pub album_artist: String,
    /// Sort key for `album_artist`, e.g. "Beatles, The".
    pub album_artist_sort: String,
    pub year: Option<i32>,
    pub disc_no: Option<u32>,
    pub track_no: Option<u32>,
    pub duration_ms: i64,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    pub genre: String,
    pub mb_track_id: String,
    pub mb_album_id: String,
    /// One or more ISRC codes, joined with ';'.
    pub isrc: String,
    pub source: Source,
    /// The service's id: "spotify:track:…" or a YouTube video id. Empty for local files.
    pub uri: String,
    /// The cover image's web address, for streamed tracks.
    pub art_url: String,
}

/// An album as shown in the library tree: one folder of tracks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Album {
    pub dir: String,
    pub title: String,
    pub artist: String,
    pub year: Option<i32>,
    pub bit_depth: Option<u8>,
    pub sample_rate: Option<u32>,
    pub tracks: u32,
    pub duration_ms: i64,
}
