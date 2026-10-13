//! Brings the index in line with the music folders.
//!
//! 1. Walk every folder and note each audio file's size and modification time. This is cheap:
//!    about 1.3 s for ~118k files on the NAS.
//! 2. Read tags only from files that are new or changed, many at once. Cover art is skipped.
//! 3. Drop index rows for files that are gone, but only for folders that were read cleanly.
//!    A NAS that isn't mounted looks like an empty folder, and that must not wipe the library.

use super::db::Stamp;
use super::{Db, Track};
use anyhow::Result;
use lofty::config::ParseOptions;
use lofty::file::{AudioFile, TaggedFile, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey, Tag};
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Instant, UNIX_EPOCH};

const AUDIO_EXTENSIONS: &[&str] = &["flac", "mp3", "opus", "ogg", "m4a", "wav", "aiff", "wv"];

/// Where tracks with no artist tags are filed in the library tree.
pub const UNKNOWN_ARTIST: &str = "[Unknown Artist]";

/// Files whose tags are written to the index in one transaction.
const BATCH: usize = 2000;

#[derive(Debug, Clone)]
pub enum Progress {
    /// Walking folders; files found in earlier folders.
    Walking(usize),
    /// Reading tags: done of total.
    Reading(usize, usize),
    /// Updating the album list and search index.
    Finishing,
}

#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub files: usize,
    pub updated: usize,
    pub removed: usize,
    /// Files whose tags couldn't be read; they're still listed, titled by file name.
    pub unreadable: usize,
    /// Folders that were missing or had read errors, so nothing under them was removed.
    pub skipped_roots: Vec<String>,
    pub walk_ms: u128,
    pub read_ms: u128,
    pub total_ms: u128,
}

pub fn scan(
    db: &mut Db,
    roots: &[PathBuf],
    threads: usize,
    mut progress: impl FnMut(Progress),
) -> Result<Summary> {
    let started = Instant::now();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads.max(1))
        .build()?;
    let mut summary = Summary::default();

    // 1. Find every audio file and its stamp.
    let mut found: Vec<(String, Stamp)> = Vec::new();
    let mut clean_roots: Vec<String> = Vec::new();
    for root in roots {
        progress(Progress::Walking(found.len()));
        let (files, clean) = walk(root, &pool);
        if clean {
            clean_roots.push(format!("{}/", root.display()));
        } else {
            summary.skipped_roots.push(root.display().to_string());
        }
        found.extend(files);
    }
    summary.files = found.len();
    summary.walk_ms = started.elapsed().as_millis();

    // 2. Compare with the index.
    let known = db.stamps()?;
    let seen: HashSet<&str> = found.iter().map(|(p, _)| p.as_str()).collect();
    let changed: Vec<&(String, Stamp)> = found
        .iter()
        .filter(|(path, stamp)| known.get(path) != Some(stamp))
        .collect();
    let gone: Vec<String> = known
        .keys()
        .filter(|path| !seen.contains(path.as_str()))
        .filter(|path| {
            clean_roots
                .iter()
                .any(|root| path.starts_with(root.as_str()))
        })
        .cloned()
        .collect();

    // 3. Read tags for new and changed files, a batch at a time.
    let reading = Instant::now();
    let total = changed.len();
    for (n, batch) in changed.chunks(BATCH).enumerate() {
        progress(Progress::Reading(n * BATCH, total));
        let items: Vec<(Track, Stamp, bool)> = pool.install(|| {
            batch
                .par_iter()
                .map(|(path, stamp)| {
                    let (track, ok) = read_track(Path::new(path));
                    (track, *stamp, ok)
                })
                .collect()
        });
        summary.unreadable += items.iter().filter(|(_, _, ok)| !ok).count();
        let rows: Vec<(Track, Stamp)> = items.into_iter().map(|(t, s, _)| (t, s)).collect();
        db.upsert(&rows)?;
    }
    if total > 0 {
        progress(Progress::Reading(total, total));
    }
    summary.updated = total;
    summary.read_ms = reading.elapsed().as_millis();

    db.delete(&gone)?;
    summary.removed = gone.len();

    if summary.updated > 0 || summary.removed > 0 {
        progress(Progress::Finishing);
        db.rebuild_derived()?;
    }
    summary.total_ms = started.elapsed().as_millis();
    Ok(summary)
}

/// Lists the audio files under `root` with their stamps. Anything that can't be read is left
/// out, and the bool comes back `false` ("not clean") if the folder is missing or empty or
/// anything under it failed. The caller still indexes what was found but removes nothing,
/// because it only has a partial view.
fn walk(root: &Path, pool: &rayon::ThreadPool) -> (Vec<(String, Stamp)>, bool) {
    if !root.is_dir() {
        return (Vec::new(), false);
    }
    let errors = AtomicUsize::new(0);
    let files = pool.install(|| list_audio(root, &errors));
    let clean = errors.load(Ordering::Relaxed) == 0 && !files.is_empty();
    (files, clean)
}

/// Every audio file under `dir` with its stamp. Sibling folders are read in parallel: over
/// NFS each read is mostly waiting, so threads help. Every folder or file that can't be read
/// is counted in `errors`; nothing is skipped silently.
fn list_audio(dir: &Path, errors: &AtomicUsize) -> Vec<(String, Stamp)> {
    let failed = || {
        errors.fetch_add(1, Ordering::Relaxed);
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        failed();
        return Vec::new();
    };
    let mut files = Vec::new();
    let mut subdirs = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            failed();
            continue;
        };
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        match entry.file_type() {
            Ok(t) if t.is_dir() => subdirs.push(entry.path()),
            Ok(t) if t.is_file() && is_audio(&entry.path()) => {
                match (stamp(&entry.path()), entry.path().to_str()) {
                    (Some(st), Some(path)) => files.push((path.to_string(), st)),
                    _ => failed(),
                }
            }
            Ok(_) => {}
            Err(_) => failed(),
        }
    }
    let nested: Vec<Vec<(String, Stamp)>> =
        subdirs.par_iter().map(|d| list_audio(d, errors)).collect();
    files.extend(nested.into_iter().flatten());
    files
}

/// A file's size and modification time in whole seconds, or `None` if it can't be stat'd
/// (e.g. it was moved away mid-scan).
fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    Some((meta.len() as i64, mtime))
}

fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Reads one file's tags. On failure the track is still returned, titled by its file name,
/// so it shows up and can be played; the bool says whether the tags were read.
pub fn read_track(path: &Path) -> (Track, bool) {
    let mut track = Track {
        path: path.display().to_string(),
        album_dir: album_dir(path),
        title: path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        ..Track::default()
    };
    let options = ParseOptions::new().read_cover_art(false);
    let tagged = Probe::open(path)
        .map(|p| p.options(options))
        .and_then(|p| p.read());
    let ok = match tagged {
        Ok(file) => {
            fill_from(&mut track, &file);
            true
        }
        Err(_) => false,
    };
    if track.album_artist.is_empty() {
        track.album_artist = UNKNOWN_ARTIST.to_string();
        track.album_artist_sort = UNKNOWN_ARTIST.to_string();
    }
    (track, ok)
}

fn fill_from(track: &mut Track, file: &TaggedFile) {
    let props = file.properties();
    track.duration_ms = props.duration().as_millis() as i64;
    track.sample_rate = props.sample_rate();
    track.bit_depth = props.bit_depth();

    let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) else {
        return;
    };
    let text = |key: ItemKey| tag.get_string(key).unwrap_or_default().trim().to_string();

    if let Some(title) = tag.title().filter(|t| !t.trim().is_empty()) {
        track.title = title.trim().to_string();
    }
    track.artist = tag
        .artist()
        .map(|a| a.trim().to_string())
        .unwrap_or_default();
    track.album = tag
        .album()
        .map(|a| a.trim().to_string())
        .unwrap_or_default();
    track.genre = tag
        .genre()
        .map(|g| g.trim().to_string())
        .unwrap_or_default();
    track.track_no = tag.track();
    track.disc_no = tag.disk();
    track.year = year(tag);
    track.mb_track_id = text(ItemKey::MusicBrainzRecordingId);
    track.mb_album_id = text(ItemKey::MusicBrainzReleaseId);
    track.isrc = isrcs(tag);

    let album_artist = text(ItemKey::AlbumArtist);
    if album_artist.is_empty() {
        track.album_artist = track.artist.clone();
        track.album_artist_sort = text(ItemKey::TrackArtistSortOrder);
    } else {
        track.album_artist = album_artist;
        track.album_artist_sort = text(ItemKey::AlbumArtistSortOrder);
    }
    if track.album_artist_sort.is_empty() {
        track.album_artist_sort = track.album_artist.clone();
    }
}

/// The original release year if tagged, otherwise the release date's year.
fn year(tag: &Tag) -> Option<i32> {
    [
        ItemKey::OriginalReleaseDate,
        ItemKey::RecordingDate,
        ItemKey::ReleaseDate,
    ]
    .into_iter()
    .filter_map(|key| tag.get_string(key))
    .find_map(|s| s.get(..4)?.parse().ok())
}

/// All ISRC codes, whether stored as repeated tags or one tag joined with ';'.
fn isrcs(tag: &Tag) -> String {
    let mut codes: Vec<&str> = tag
        .get_strings(ItemKey::Isrc)
        .flat_map(|s| s.split(';'))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    codes.dedup();
    codes.join(";")
}

/// The album folder for a file. Disc subfolders (`DISC1`, `CD 2`, `Disc 3`) belong to the
/// folder above them, so a multi-disc album is one album.
pub fn album_dir(path: &Path) -> String {
    let parent = path.parent().unwrap_or(Path::new("/"));
    let is_disc_folder = parent
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| {
            let lower = name.to_ascii_lowercase();
            let rest = lower
                .strip_prefix("disc")
                .or_else(|| lower.strip_prefix("cd"));
            rest.is_some_and(|r| {
                let r = r.trim_start_matches([' ', '_', '-']);
                !r.is_empty() && r.chars().all(|c| c.is_ascii_digit())
            })
        });
    let dir = if is_disc_folder {
        parent.parent().unwrap_or(parent)
    } else {
        parent
    };
    dir.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{TempDir, flac, have};

    fn scan_dir(db: &mut Db, root: &Path) -> Summary {
        scan(db, &[root.to_path_buf()], 4, |_| {}).unwrap()
    }

    #[test]
    fn disc_folders_belong_to_their_album() {
        assert_eq!(
            album_dir(Path::new("/m/A/(2001) X/DISC1/01.flac")),
            "/m/A/(2001) X"
        );
        assert_eq!(
            album_dir(Path::new("/m/A/(2001) X/CD 2/01.flac")),
            "/m/A/(2001) X"
        );
        assert_eq!(
            album_dir(Path::new("/m/A/(2001) X/01.flac")),
            "/m/A/(2001) X"
        );
        assert_eq!(
            album_dir(Path::new("/m/A/Discography/01.flac")),
            "/m/A/Discography"
        );
    }

    #[test]
    fn scans_groups_searches_and_rescans() {
        if !have("ffmpeg") {
            eprintln!("skipping: ffmpeg not installed");
            return;
        }
        let tmp = TempDir::new("scan");
        let music = tmp.0.join("music");
        let album = music.join("The Band/(1999) First [16-44 FLAC]");
        flac(
            &album.join("01 - One.flac"),
            &[
                ("title", "One"),
                ("artist", "The Band"),
                ("album", "First"),
                ("album_artist", "The Band"),
                ("ALBUMARTISTSORT", "Band, The"),
                ("date", "1999-05-01"),
                ("track", "1"),
                ("ISRC", "USAAA9900001"),
            ],
        );
        flac(
            &album.join("02 - Two.flac"),
            &[
                ("title", "Two"),
                ("artist", "The Band feat. Guest"),
                ("album", "First"),
                ("album_artist", "The Band"),
                ("ALBUMARTISTSORT", "Band, The"),
                ("date", "1999"),
                ("track", "2"),
            ],
        );
        let double = music.join("Beyoncé/(2003) Double");
        flac(
            &double.join("DISC1/01 - Café.flac"),
            &[
                ("title", "Café"),
                ("artist", "Beyoncé"),
                ("album", "Double"),
                ("disc", "1"),
                ("track", "1"),
            ],
        );
        flac(
            &double.join("DISC2/01 - Encore.flac"),
            &[
                ("title", "Encore"),
                ("artist", "Beyoncé"),
                ("album", "Double"),
                ("disc", "2"),
                ("track", "1"),
            ],
        );

        let mut db = Db::open(&tmp.0.join("index.db")).unwrap();
        let s = scan_dir(&mut db, &music);
        assert_eq!((s.files, s.updated, s.removed, s.unreadable), (4, 4, 0, 0));

        // Sorted by ALBUMARTISTSORT: "Band, The" before "Beyoncé".
        assert_eq!(db.artists().unwrap(), vec!["The Band", "Beyoncé"]);
        let albums = db.albums_by("Beyoncé").unwrap();
        assert_eq!(albums.len(), 1, "two disc folders make one album");
        assert_eq!(albums[0].tracks, 2);
        assert_eq!(
            (albums[0].bit_depth, albums[0].sample_rate),
            (Some(16), Some(44100))
        );
        let first = db.albums_by("The Band").unwrap();
        assert_eq!(first[0].year, Some(1999));
        let tracks = db.tracks_in(&first[0].dir).unwrap();
        assert_eq!(
            tracks.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
            ["One", "Two"]
        );
        assert_eq!(tracks[0].isrc, "USAAA9900001");

        // Accent-insensitive prefix search.
        let hits = db.search("beyon caf", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Café");

        // Nothing changed: nothing re-read.
        let s = scan_dir(&mut db, &music);
        assert_eq!((s.updated, s.removed), (0, 0));

        // Retag one file and delete another.
        let one = album.join("01 - One.flac");
        flac(
            &one,
            &[
                ("title", "One (Remastered)"),
                ("artist", "The Band"),
                ("album", "First"),
                ("album_artist", "The Band"),
                ("track", "1"),
            ],
        );
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&one)
            .unwrap()
            .set_modified(later)
            .unwrap();
        std::fs::remove_file(double.join("DISC2/01 - Encore.flac")).unwrap();
        let id_before = tracks[0].id;
        let s = scan_dir(&mut db, &music);
        assert_eq!((s.updated, s.removed), (1, 1));
        let retagged = db.tracks_in(&first[0].dir).unwrap();
        assert_eq!(retagged[0].title, "One (Remastered)");
        assert_eq!(retagged[0].id, id_before, "a changed file keeps its id");
        assert_eq!(db.albums_by("Beyoncé").unwrap()[0].tracks, 1);
    }

    #[test]
    fn missing_or_empty_root_never_wipes_the_index() {
        if !have("ffmpeg") {
            eprintln!("skipping: ffmpeg not installed");
            return;
        }
        let tmp = TempDir::new("prune");
        let music = tmp.0.join("music");
        flac(
            &music.join("A/(2000) B/01 - C.flac"),
            &[("title", "C"), ("artist", "A")],
        );
        let mut db = Db::open(&tmp.0.join("index.db")).unwrap();
        scan_dir(&mut db, &music);
        assert_eq!(db.track_count().unwrap(), 1);

        // Unmounted NAS: the folder is gone.
        std::fs::rename(&music, tmp.0.join("elsewhere")).unwrap();
        let s = scan_dir(&mut db, &music);
        assert_eq!(s.removed, 0);
        assert_eq!(s.skipped_roots.len(), 1);
        assert_eq!(db.track_count().unwrap(), 1);

        // An autofs mount point with nothing mounted: the folder exists but is empty.
        std::fs::create_dir_all(&music).unwrap();
        let s = scan_dir(&mut db, &music);
        assert_eq!(s.removed, 0);
        assert_eq!(db.track_count().unwrap(), 1);
    }

    #[test]
    fn unreadable_files_are_still_listed() {
        let tmp = TempDir::new("broken");
        let music = tmp.0.join("music");
        std::fs::create_dir_all(music.join("X/Y")).unwrap();
        std::fs::write(music.join("X/Y/01 - Not Really.flac"), b"not audio").unwrap();
        let mut db = Db::open(&tmp.0.join("index.db")).unwrap();
        let s = scan_dir(&mut db, &music);
        assert_eq!((s.files, s.unreadable), (1, 1));
        assert_eq!(db.search("really", 5).unwrap()[0].title, "01 - Not Really");
        assert_eq!(db.artists().unwrap(), vec![UNKNOWN_ARTIST]);
    }

    #[test]
    fn a_file_that_vanished_has_no_stamp() {
        let tmp = TempDir::new("stat");
        let here = tmp.0.join("here.flac");
        std::fs::write(&here, b"x").unwrap();
        assert_eq!(stamp(&here).map(|(size, _)| size), Some(1));
        assert_eq!(stamp(&tmp.0.join("moved-away.flac")), None);
    }

    #[test]
    fn an_unreadable_folder_still_indexes_the_rest_but_removes_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new("perm");
        let music = tmp.0.join("music");
        std::fs::create_dir_all(music.join("Open")).unwrap();
        std::fs::create_dir_all(music.join("Locked")).unwrap();
        std::fs::write(music.join("Open/01 - A.flac"), b"x").unwrap();
        std::fs::write(music.join("Locked/01 - B.flac"), b"x").unwrap();
        let mut db = Db::open(&tmp.0.join("index.db")).unwrap();
        assert_eq!(scan_dir(&mut db, &music).files, 2);

        let locked = music.join("Locked");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        std::fs::write(music.join("Open/02 - C.flac"), b"x").unwrap();
        let s = scan_dir(&mut db, &music);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(
            s.updated, 1,
            "the new file in the readable folder is indexed"
        );
        assert_eq!(s.removed, 0, "B is unreadable, not gone, so it stays");
        assert_eq!(db.track_count().unwrap(), 3);
    }
}
