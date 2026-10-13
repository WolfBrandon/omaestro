//! The little web server Sonos speakers fetch songs from, and getting songs ready for them.
//!
//! Speakers can't read the NAS themselves, so OMAESTRO hands them a web address on this
//! computer (port 47100 by default) for each song. Only songs OMAESTRO offered are served,
//! each under a random name. The speakers ask for byte ranges when they seek, so files are
//! served whole from disk with their exact size.
//!
//! Sonos plays up to 48 kHz. Files above that are converted once with ffmpeg (to 24-bit 48 kHz,
//! or 44.1 kHz for 88.2 and 176.4 kHz music) into `~/.cache/omaestro/sonos/`, then reused.
//! YouTube Music songs are downloaded there as AAC (no conversion needed) before they play.

use crate::config;
use crate::library::Track;
use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Formats Sonos plays as they are.
const PLAYABLE: &[&str] = &["flac", "mp3", "m4a", "aac", "ogg", "wav", "aif", "aiff"];

pub fn cache_dir() -> PathBuf {
    config::cache_dir().join("sonos")
}

pub fn mime(path: &Path) -> &'static str {
    match extension(path).as_str() {
        "flac" => "audio/flac",
        "mp3" => "audio/mpeg",
        "m4a" | "mp4" | "aac" => "audio/mp4",
        "ogg" => "application/ogg",
        "wav" => "audio/wav",
        "aif" | "aiff" => "audio/aiff",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        _ => "application/octet-stream",
    }
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

pub struct Server {
    pub port: u16,
    files: Arc<Mutex<HashMap<String, PathBuf>>>,
}

impl Server {
    /// Starts listening on every network address at `port`.
    pub fn start(port: u16) -> Result<Server> {
        let listener = TcpListener::bind(("0.0.0.0", port)).with_context(|| {
            format!("port {port} is busy, so Sonos can't fetch songs from here")
        })?;
        // Port 0 means "any free port" (the tests use that).
        let port = listener.local_addr()?.port();
        let files: Arc<Mutex<HashMap<String, PathBuf>>> = Arc::default();
        let shared = files.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let files = shared.clone();
                std::thread::spawn(move || {
                    let _ = serve(stream, &files);
                });
            }
        });
        Ok(Server { port, files })
    }

    /// Makes a file available; returns its path on the server, e.g. "/f/k3j9x2a8.flac".
    pub fn offer(&self, path: &Path) -> String {
        let name = format!(
            "{}.{}",
            crate::secrets::random_string(16, b"abcdefghijklmnopqrstuvwxyz0123456789"),
            extension(path)
        );
        if let Ok(mut files) = self.files.lock() {
            // Keep the list from growing forever over a long session.
            if files.len() > 500 {
                files.clear();
            }
            files.insert(name.clone(), path.to_path_buf());
        }
        format!("/f/{name}")
    }
}

fn respond(stream: &mut TcpStream, status: &str) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

/// Answers one request: GET or HEAD of an offered file, whole or a byte range.
fn serve(mut stream: TcpStream, files: &Mutex<HashMap<String, PathBuf>>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let mut range = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("range")
        {
            range = Some(value.trim().to_string());
        }
    }
    if method != "GET" && method != "HEAD" {
        return respond(&mut stream, "405 Method Not Allowed");
    }
    let path = target
        .strip_prefix("/f/")
        .and_then(|name| files.lock().ok()?.get(name).cloned());
    let Some(path) = path else {
        return respond(&mut stream, "404 Not Found");
    };
    let mut file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return respond(&mut stream, "404 Not Found"),
    };
    let size = file.metadata()?.len();
    let (start, end, status) = match range.as_deref().map(|r| parse_range(r, size)) {
        None => (0, size.saturating_sub(1), "200 OK"),
        Some(Some((s, e))) => (s, e, "206 Partial Content"),
        Some(None) => {
            return write!(
                stream,
                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{size}\r\n\
                 Content-Length: 0\r\nConnection: close\r\n\r\n"
            );
        }
    };
    let length = if size == 0 { 0 } else { end - start + 1 };
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {}\r\nContent-Length: {length}\r\n\
         Accept-Ranges: bytes\r\nConnection: close\r\n",
        mime(&path)
    );
    if status.starts_with("206") {
        head.push_str(&format!("Content-Range: bytes {start}-{end}/{size}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    if method == "HEAD" {
        return Ok(());
    }
    file.seek(SeekFrom::Start(start))?;
    std::io::copy(&mut file.take(length), &mut stream)?;
    Ok(())
}

/// "bytes=100-", "bytes=100-199" or "bytes=-500" (the last 500) → first and last byte.
fn parse_range(header: &str, size: u64) -> Option<(u64, u64)> {
    let spec = header.strip_prefix("bytes=")?.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    let last = size.checked_sub(1)?;
    let (start, end) = match (a.trim(), b.trim()) {
        ("", n) => {
            let n: u64 = n.parse().ok()?;
            (size.saturating_sub(n), last)
        }
        (s, "") => (s.parse().ok()?, last),
        (s, e) => (s.parse().ok()?, e.parse::<u64>().ok()?.min(last)),
    };
    (start <= end && start <= last).then_some((start, end))
}

/// A short, stable name for a cached copy of something.
fn cache_name(key: &str) -> String {
    // FNV-1a: plenty for telling cache files apart.
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in key.bytes() {
        hash = (hash ^ b as u64).wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// The rate a file above 48 kHz is converted to: 44.1 kHz for its multiples, else 48 kHz.
fn target_rate(rate: u32) -> u32 {
    if rate.is_multiple_of(44100) {
        44100
    } else {
        48000
    }
}

/// A local file Sonos can play: the file itself, or a converted copy in the cache (made the
/// first time; this takes a few seconds). Runs ffmpeg, so call it off the screen's thread.
pub fn prepare_local(track: &Track) -> Result<PathBuf> {
    let path = PathBuf::from(&track.path);
    let rate = track.sample_rate.unwrap_or(44100);
    let fine = PLAYABLE.contains(&extension(&path).as_str())
        && rate <= 48000
        && track.bit_depth.unwrap_or(16) <= 24;
    if fine {
        return Ok(path);
    }
    let dir = cache_dir();
    std::fs::create_dir_all(&dir)?;
    let modified = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    let out = dir.join(format!(
        "{}.flac",
        cache_name(&format!("{}:{modified}", track.path))
    ));
    if out.is_file() {
        return Ok(out);
    }
    let tmp = out.with_extension("part.flac");
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(&path)
        .args([
            "-map",
            "0:a:0",
            "-c:a",
            "flac",
            "-sample_fmt",
            "s32",
            "-bits_per_raw_sample",
            "24",
        ])
        .args(["-ar", &target_rate(rate).to_string(), "-map_metadata", "0"])
        .arg(&tmp)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("running ffmpeg")?;
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        bail!("ffmpeg couldn't convert {} for Sonos", track.path);
    }
    std::fs::rename(&tmp, &out)?;
    Ok(out)
}

/// A YouTube Music song as an AAC file in the cache, for Sonos. Downloads it the first time.
pub fn prepare_youtube(video_id: &str, browser: Option<&str>) -> Result<PathBuf> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir)?;
    let out = dir.join(format!("yt-{video_id}.m4a"));
    if out.is_file() {
        return Ok(out);
    }
    let mut cmd = Command::new("yt-dlp");
    cmd.args(["--ignore-config", "--no-warnings", "--no-playlist", "-f"])
        .arg("bestaudio[ext=m4a]/bestaudio")
        .args(["-x", "--audio-format", "m4a", "-o"])
        .arg(dir.join(format!("yt-{video_id}.%(ext)s")));
    if let Some(browser) = browser {
        cmd.args(["--cookies-from-browser", browser]);
    }
    let out_status = cmd
        .arg(format!("https://music.youtube.com/watch?v={video_id}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("running yt-dlp")?;
    if !out.is_file() {
        let err = String::from_utf8_lossy(&out_status.stderr);
        let last = err
            .lines()
            .rev()
            .find(|l| l.contains("ERROR"))
            .unwrap_or("download failed");
        bail!("{}", last.trim_start_matches("ERROR: "));
    }
    Ok(out)
}

/// Deletes cached copies nobody has used for a week.
pub fn prune_cache() {
    let Ok(entries) = std::fs::read_dir(cache_dir()) else {
        return;
    };
    let week = Duration::from_secs(7 * 86_400);
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.accessed().or_else(|_| m.modified()))
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > week);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn byte_ranges() {
        assert_eq!(parse_range("bytes=0-", 1000), Some((0, 999)));
        assert_eq!(parse_range("bytes=100-199", 1000), Some((100, 199)));
        assert_eq!(parse_range("bytes=900-5000", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=1000-", 1000), None);
        assert_eq!(parse_range("bytes=5-2", 1000), None);
        assert_eq!(target_rate(96000), 48000);
        assert_eq!(target_rate(88200), 44100);
        assert_eq!(target_rate(176400), 44100);
    }

    /// Serves an offered file whole and in part, and nothing else.
    #[test]
    fn serves_only_offered_files_with_ranges() {
        let tmp = TempDir::new("sonos-server");
        let file = tmp.0.join("song.flac");
        std::fs::write(&file, b"0123456789").unwrap();
        let server = Server::start(0).unwrap();
        let port = server.port;
        let path = server.offer(&file);
        let http: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        let url = format!("http://127.0.0.1:{port}{path}");

        let mut res = http.get(&url).call().unwrap();
        assert_eq!(res.status().as_u16(), 200);
        assert_eq!(res.headers().get("content-type").unwrap(), "audio/flac");
        assert_eq!(res.body_mut().read_to_string().unwrap(), "0123456789");

        let mut res = http.get(&url).header("Range", "bytes=2-4").call().unwrap();
        assert_eq!(res.status().as_u16(), 206);
        assert_eq!(res.headers().get("content-range").unwrap(), "bytes 2-4/10");
        assert_eq!(res.body_mut().read_to_string().unwrap(), "234");

        let res = http.head(&url).call().unwrap();
        assert_eq!(res.headers().get("content-length").unwrap(), "10");

        let res = http
            .get(&format!("http://127.0.0.1:{port}/f/../../etc/passwd"))
            .call()
            .unwrap();
        assert_eq!(res.status().as_u16(), 404);
        let res = http
            .get(&format!("http://127.0.0.1:{port}/etc/passwd"))
            .call()
            .unwrap();
        assert_eq!(res.status().as_u16(), 404);
    }
}
