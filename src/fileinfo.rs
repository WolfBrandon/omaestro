//! File Info: everything about one song, in a pop-up (`i`, or a right-click on a song). Most of
//! it is in the library index already. The rest (size, bitrate, channels, ReplayGain) is read
//! from the file in the background, with a time limit: the folder may be on a NAS that's gone
//! away.

use crate::library::{Source, Track};
use lofty::config::ParseOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use std::path::Path;

/// What's read from the file itself.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Details {
    pub size: Option<u64>,
    /// Kilobits per second.
    pub bitrate: Option<u32>,
    pub channels: Option<u8>,
    /// As tagged, e.g. "-7.30 dB".
    pub track_gain: Option<String>,
    pub album_gain: Option<String>,
}

/// Reads a file's details; an error if it can't be read.
pub fn read(path: &Path) -> Result<Details, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    let file = Probe::open(path)
        .map(|p| p.options(ParseOptions::new().read_cover_art(false)))
        .and_then(|p| p.read())
        .map_err(|e| e.to_string())?;
    let props = file.properties();
    let gain = |key: ItemKey| {
        file.tags()
            .iter()
            .find_map(|tag| tag.get_string(key).map(str::to_string))
    };
    Ok(Details {
        size: Some(size),
        bitrate: props.audio_bitrate().or(props.overall_bitrate()),
        channels: props.channels(),
        track_gain: gain(ItemKey::ReplayGainTrackGain),
        album_gain: gain(ItemKey::ReplayGainAlbumGain),
    })
}

/// A streamed song's web address: where it lives on Spotify or YouTube Music.
pub fn link(track: &Track) -> Option<String> {
    match track.source {
        Source::Local => None,
        Source::Spotify => track
            .uri
            .strip_prefix("spotify:track:")
            .map(|id| format!("https://open.spotify.com/track/{id}")),
        Source::YouTube => Some(format!("https://music.youtube.com/watch?v={}", track.uri)),
    }
}

/// 34,211,072 → "32.6 MB".
pub fn size(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1.0 {
        format!("{mb:.1} MB")
    } else {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    }
}

/// 2 → "2 (stereo)".
pub fn channels(n: u8) -> String {
    match n {
        1 => "1 (mono)".into(),
        2 => "2 (stereo)".into(),
        n => n.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{TempDir, flac, have};

    #[test]
    fn reads_what_the_index_doesnt_have() {
        if !have("ffmpeg") {
            eprintln!("skipping: needs ffmpeg");
            return;
        }
        let tmp = TempDir::new("fileinfo");
        let path = tmp.0.join("a.flac");
        flac(
            &path,
            &[("title", "A"), ("REPLAYGAIN_TRACK_GAIN", "-7.30 dB")],
        );
        let d = read(&path).unwrap();
        assert!(d.size.is_some_and(|s| s > 0));
        assert_eq!(d.channels, Some(2));
        assert_eq!(d.track_gain.as_deref(), Some("-7.30 dB"));
        assert_eq!(d.album_gain, None);
        assert!(read(&tmp.0.join("missing.flac")).is_err());
    }

    #[test]
    fn links_sizes_and_channels() {
        let spotify = Track {
            source: Source::Spotify,
            uri: "spotify:track:4uLU6hMCjMI75M1A2tKUQC".into(),
            ..Track::default()
        };
        assert_eq!(
            link(&spotify).as_deref(),
            Some("https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC")
        );
        let youtube = Track {
            source: Source::YouTube,
            uri: "dQw4w9WgXcQ".into(),
            ..Track::default()
        };
        assert_eq!(
            link(&youtube).as_deref(),
            Some("https://music.youtube.com/watch?v=dQw4w9WgXcQ")
        );
        assert_eq!(link(&Track::default()), None);
        assert_eq!(size(34_211_072), "32.6 MB");
        assert_eq!(size(512_000), "500 KB");
        assert_eq!(channels(2), "2 (stereo)");
    }
}
