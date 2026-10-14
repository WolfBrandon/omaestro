//! Playlists as plain files: one JSPF file (JSON's version of the XSPF playlist format) per
//! playlist in `~/.local/share/omaestro/playlists/`. A playlist can mix local files, Spotify
//! and YouTube Music. Each song keeps every id it has (file, service link, ISRC, MusicBrainz),
//! so it can be matched again elsewhere later.

use crate::config;
use crate::library::{Db, Source, Track};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Where OMAESTRO keeps its own details about a song in the file.
const EXTENSION: &str = "https://github.com/WolfBrandon/omaestro";

pub fn dir() -> PathBuf {
    config::data_dir().join("playlists")
}

/// A saved playlist, for the Open list.
#[derive(Debug, Clone, PartialEq)]
pub struct Saved {
    pub name: String,
    pub path: PathBuf,
    pub tracks: usize,
    pub modified: SystemTime,
}

/// The saved playlists, most recently changed first.
pub fn list(dir: &Path) -> Vec<Saved> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<Saved> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jspf"))
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let v: Value = serde_json::from_str(&text).ok()?;
            let playlist = v.get("playlist")?;
            Some(Saved {
                name: playlist
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| stem(&path)),
                tracks: playlist
                    .get("track")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len),
                modified: std::fs::metadata(&path).and_then(|m| m.modified()).ok()?,
                path,
            })
        })
        .collect();
    out.sort_by_key(|p| std::cmp::Reverse(p.modified));
    out
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A file name for a playlist name: slashes and other awkward characters become "-".
pub fn file_for(dir: &Path, name: &str) -> PathBuf {
    let safe: String = name
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let safe = safe.trim_start_matches('.');
    dir.join(format!(
        "{}.jspf",
        if safe.is_empty() { "Untitled" } else { safe }
    ))
}

/// Writes the playlist, replacing a file of the same name.
pub fn save(path: &Path, name: &str, tracks: &[Track]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let doc = json!({ "playlist": {
        "title": name,
        "creator": "OMAESTRO",
        "track": tracks.iter().map(to_jspf).collect::<Vec<_>>(),
    }});
    let text = serde_json::to_string_pretty(&doc)?;
    // Write a temporary file and rename it, so a crash never leaves half a playlist.
    let tmp = path.with_extension("jspf.tmp");
    std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Renames a saved playlist: its file moves to `to` and the name inside changes. The songs
/// stay as they were saved.
pub fn rename(from: &Path, to: &Path, name: &str) -> Result<()> {
    let text =
        std::fs::read_to_string(from).with_context(|| format!("reading {}", from.display()))?;
    let mut doc: Value = serde_json::from_str(&text)?;
    doc["playlist"]["title"] = json!(name);
    let tmp = to.with_extension("jspf.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&doc)?)
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, to)?;
    if from != to {
        std::fs::remove_file(from)?;
    }
    Ok(())
}

fn to_jspf(t: &Track) -> Value {
    let mut ids = Vec::new();
    match t.source {
        Source::Spotify => ids.push(t.uri.clone()),
        Source::YouTube => ids.push(format!("https://music.youtube.com/watch?v={}", t.uri)),
        Source::Local => {}
    }
    ids.extend(
        t.isrc
            .split(';')
            .filter(|i| !i.is_empty())
            .map(|i| format!("isrc:{i}")),
    );
    if !t.mb_track_id.is_empty() {
        ids.push(format!(
            "https://musicbrainz.org/recording/{}",
            t.mb_track_id
        ));
    }
    let mut v = json!({
        "title": t.title,
        "creator": t.artist,
        "album": t.album,
        "duration": t.duration_ms,
        "identifier": ids,
        "extension": { EXTENSION: [{
            "source": t.source.key(),
            "uri": t.uri,
            "album_artist": t.album_artist,
            "year": t.year,
            "disc": t.disc_no,
            "track": t.track_no,
            "bit_depth": t.bit_depth,
            "sample_rate": t.sample_rate,
            "mb_album_id": t.mb_album_id,
        }]},
    });
    if t.source == Source::Local {
        v["location"] = json!([file_uri(&t.path)]);
    }
    if !t.art_url.is_empty() {
        v["image"] = json!(t.art_url);
    }
    v
}

/// Reads a playlist. Local songs are looked up in the library so their details are current;
/// songs whose file is gone are kept with the details saved in the playlist.
pub fn load(path: &Path, db: &Db) -> Result<(String, Vec<Track>)> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let v: Value =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let playlist = v.get("playlist").context("not a JSPF playlist")?;
    let name = playlist
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| stem(path));
    let tracks = playlist
        .get("track")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| from_jspf(t, db))
        .collect();
    Ok((name, tracks))
}

fn from_jspf(v: &Value, db: &Db) -> Option<Track> {
    let s = |key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let ext = v
        .get("extension")
        .and_then(|e| e.get(EXTENSION))
        .and_then(|e| e.get(0))
        .cloned()
        .unwrap_or(Value::Null);
    let es = |key: &str| {
        ext.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let en = |key: &str| ext.get(key).and_then(Value::as_u64);
    let ids: Vec<String> = v
        .get("identifier")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|i| i.as_str().map(str::to_string))
        .collect();
    let location = v
        .get("location")
        .and_then(|l| l.get(0).or(Some(l)))
        .and_then(Value::as_str)
        .and_then(path_from_uri);

    // What kind of song: our own note, else the ids or a file location.
    let source = Source::from_key(&es("source")).unwrap_or(if location.is_some() {
        Source::Local
    } else if ids.iter().any(|i| i.starts_with("spotify:")) {
        Source::Spotify
    } else if ids.iter().any(|i| i.contains("youtube.com/watch?v=")) {
        Source::YouTube
    } else {
        return None;
    });

    if source == Source::Local {
        let path = location?;
        if let Ok(Some(track)) = db.track_at(&path) {
            return Some(track);
        }
        // Not in the library (moved, or the NAS is offline): keep what the playlist says.
        return Some(Track {
            path,
            title: s("title"),
            artist: s("creator"),
            album: s("album"),
            album_artist: es("album_artist"),
            duration_ms: v.get("duration").and_then(Value::as_i64).unwrap_or(0),
            ..Track::default()
        });
    }

    let uri = match es("uri") {
        uri if !uri.is_empty() => uri,
        _ => match source {
            Source::Spotify => ids.iter().find(|i| i.starts_with("spotify:"))?.clone(),
            _ => ids
                .iter()
                .find_map(|i| i.split("watch?v=").nth(1))?
                .split('&')
                .next()?
                .to_string(),
        },
    };
    Some(Track {
        source,
        uri,
        title: s("title"),
        artist: s("creator"),
        album: s("album"),
        album_artist: es("album_artist"),
        duration_ms: v.get("duration").and_then(Value::as_i64).unwrap_or(0),
        year: en("year").map(|y| y as i32),
        disc_no: en("disc").map(|n| n as u32),
        track_no: en("track").map(|n| n as u32),
        art_url: s("image"),
        isrc: ids
            .iter()
            .filter_map(|i| i.strip_prefix("isrc:"))
            .collect::<Vec<_>>()
            .join(";"),
        ..Track::default()
    })
}

/// "/music/AC⁄DC/01 Back in Black.flac" → "file:///music/AC%E2%81%84DC/01%20Back%20in%20Black.flac".
pub fn file_uri(path: &str) -> String {
    let mut out = String::from("file://");
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The reverse of `file_uri`. Plain paths are accepted too.
pub fn path_from_uri(uri: &str) -> Option<String> {
    let encoded = match uri.strip_prefix("file://") {
        Some(rest) => rest,
        None if uri.starts_with('/') => return Some(uri.to_string()),
        None => return None,
    };
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?, 16)
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{TempDir, sample_library};

    #[test]
    fn file_uris_survive_odd_characters() {
        let path = "/music/AC⁄DC/01 Back in Black (100%).flac";
        let uri = file_uri(path);
        assert!(uri.starts_with("file:///music/AC%E2%81%84DC/01%20Back"));
        assert_eq!(path_from_uri(&uri).as_deref(), Some(path));
        assert_eq!(
            path_from_uri("/plain/path.flac").as_deref(),
            Some("/plain/path.flac")
        );
        assert_eq!(path_from_uri("http://x"), None);
    }

    #[test]
    fn renaming_moves_the_file_and_changes_the_name_inside() {
        let tmp = TempDir::new("playlist-rename");
        let db = sample_library(&tmp.0);
        let tracks = db.tracks_in("/m/Abba/Gold").unwrap();
        let (old, new) = (file_for(&tmp.0, "Mix"), file_for(&tmp.0, "Road trip"));
        save(&old, "Mix", &tracks).unwrap();
        rename(&old, &new, "Road trip").unwrap();
        assert!(!old.exists());
        let (name, loaded) = load(&new, &db).unwrap();
        assert_eq!(name, "Road trip");
        assert_eq!(loaded.len(), tracks.len());
    }

    #[test]
    fn saves_and_loads_a_mixed_playlist() {
        let tmp = TempDir::new("playlist");
        let db = sample_library(&tmp.0);
        let mut tracks = db.tracks_in("/m/Abba/Gold").unwrap();
        tracks.push(Track {
            source: Source::Spotify,
            uri: "spotify:track:4uLU6hMCjMI75M1A2tKUQC".into(),
            title: "Never Gonna Give You Up".into(),
            artist: "Rick Astley".into(),
            duration_ms: 213_000,
            isrc: "GBARL9300135".into(),
            ..Track::default()
        });
        tracks.push(Track {
            source: Source::YouTube,
            uri: "dQw4w9WgXcQ".into(),
            title: "Never Gonna Give You Up".into(),
            artist: "Rick Astley".into(),
            ..Track::default()
        });
        // A file that left the library still loads, with what the playlist knew about it.
        tracks.push(Track {
            path: "/m/gone.flac".into(),
            title: "Gone".into(),
            ..Track::default()
        });

        let dir = tmp.0.join("playlists");
        let path = file_for(&dir, "Saturday / Mix");
        assert_eq!(path.file_name().unwrap(), "Saturday - Mix.jspf");
        save(&path, "Saturday / Mix", &tracks).unwrap();

        let (name, loaded) = load(&path, &db).unwrap();
        assert_eq!(name, "Saturday / Mix");
        assert_eq!(loaded.len(), 5);
        assert_eq!(
            loaded[0], tracks[0],
            "local songs come back from the library"
        );
        assert_eq!(loaded[2].source, Source::Spotify);
        assert_eq!(loaded[2].uri, tracks[2].uri);
        assert_eq!(loaded[2].isrc, "GBARL9300135");
        assert_eq!(loaded[3].source, Source::YouTube);
        assert_eq!(loaded[3].uri, "dQw4w9WgXcQ");
        assert_eq!(loaded[4].title, "Gone");

        let saved = list(&dir);
        assert_eq!(saved.len(), 1);
        assert_eq!(
            (saved[0].name.as_str(), saved[0].tracks),
            ("Saturday / Mix", 5)
        );
    }
}
