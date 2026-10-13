//! Helpers shared by the tests.

use crate::library::{Db, Track};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A temporary folder that deletes itself.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(name: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!("omaestro-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Whether a program is installed.
pub fn have(program: &str) -> bool {
    Command::new(program).arg("--version").output().is_ok()
}

/// Writes a one-second silent 16-bit/44.1 kHz FLAC with the given tags.
pub fn flac(path: &Path, tags: &[(&str, &str)]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut cmd = Command::new("ffmpeg");
    cmd.args([
        "-v",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        "anullsrc=r=44100:cl=stereo",
        "-t",
        "1",
    ]);
    cmd.args(["-sample_fmt", "s16"]);
    for (k, v) in tags {
        cmd.arg("-metadata").arg(format!("{k}={v}"));
    }
    let ok = cmd.arg(path).status().unwrap().success();
    assert!(ok, "ffmpeg failed for {}", path.display());
}

fn sample_track(artist: &str, album: &str, dir: &str, no: u32, title: &str) -> Track {
    Track {
        path: format!("{dir}/{no:02} - {title}.flac"),
        album_dir: dir.into(),
        title: title.into(),
        artist: artist.into(),
        album: album.into(),
        album_artist: artist.into(),
        album_artist_sort: artist.into(),
        track_no: Some(no),
        year: Some(2000),
        duration_ms: 180_000,
        sample_rate: Some(44100),
        bit_depth: Some(16),
        ..Track::default()
    }
}

/// A small index: Abba (one album, two tracks) and Zappa (two albums). No audio files.
pub fn sample_library(dir: &Path) -> Db {
    let mut db = Db::open(&dir.join("index.db")).unwrap();
    let rows: Vec<_> = [
        sample_track("Abba", "Gold", "/m/Abba/Gold", 1, "Dancing Queen"),
        sample_track("Abba", "Gold", "/m/Abba/Gold", 2, "Knowing Me"),
        sample_track(
            "Zappa",
            "Apostrophe",
            "/m/Zappa/Apostrophe",
            1,
            "Cosmik Debris",
        ),
        sample_track("Zappa", "Hot Rats", "/m/Zappa/Hot Rats", 1, "Peaches"),
    ]
    .into_iter()
    .map(|t| (t, (1, 1)))
    .collect();
    db.upsert(&rows).unwrap();
    db.rebuild_derived().unwrap();
    db
}
