//! The little web server Sonos speakers fetch songs from, and getting songs ready for them.
//!
//! Speakers can't read the NAS themselves, so OMAESTRO hands them a web address on this
//! computer (port 47100 by default) for each song. It's careful about it:
//! - only the song playing now is offered, under a random name, and nothing once Sonos stops;
//! - only your speakers are answered;
//! - only files inside your music folders (or OMAESTRO's own converted copies) are served,
//!   checked again as they're sent, so a file swapped for a link to somewhere else isn't;
//! - a few connections at a time, with short request lines.
//!
//! The speakers ask for byte ranges when they seek, so files are served whole from disk with
//! their exact size.
//!
//! Sonos plays up to 48 kHz. Files above that are converted once with ffmpeg (to 24-bit 48 kHz,
//! or 44.1 kHz for 88.2 and 176.4 kHz music) into `~/.cache/omaestro/sonos/`, then reused.
//! YouTube Music songs are downloaded there as AAC (no conversion needed) before they play.

use crate::config;
use crate::library::Track;
use anyhow::{Context, Result, bail};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Connections answered at once. A group's coordinator uses one or two; more are refused.
const CONNECTIONS: usize = 16;
/// The longest request or header line accepted, and how many header lines.
const LINE: u64 = 8 * 1024;
const HEADERS: usize = 64;
/// How long a speaker has to finish asking.
const ASKING: Duration = Duration::from_secs(10);

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

/// The song on offer: its random name, and the file (its real path, links resolved).
type Offer = Option<(String, PathBuf)>;

pub struct Server {
    pub port: u16,
    offer: Arc<Mutex<Offer>>,
}

impl Server {
    /// Starts listening at `port` (0: any free port, for the tests), answering only
    /// `speakers`. The listener lasts as long as OMAESTRO; what it serves doesn't.
    pub fn start(port: u16, speakers: Vec<IpAddr>) -> Result<Server> {
        let listener = TcpListener::bind(("0.0.0.0", port)).with_context(|| {
            format!("port {port} is busy, so Sonos can't fetch songs from here")
        })?;
        let port = listener.local_addr()?.port();
        let offer: Arc<Mutex<Offer>> = Arc::default();
        let shared = offer.clone();
        let busy = Arc::new(AtomicUsize::new(0));
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let from = stream.peer_addr().map(|a| a.ip()).ok();
                if !from.is_some_and(|ip| speakers.contains(&ip)) {
                    super::log(&format!(
                        "file server: turned away {}, not one of your speakers",
                        from.map_or("?".into(), |ip| ip.to_string())
                    ));
                    continue;
                }
                if busy.load(Ordering::SeqCst) >= CONNECTIONS {
                    continue;
                }
                busy.fetch_add(1, Ordering::SeqCst);
                let (offer, done) = (shared.clone(), busy.clone());
                let started = std::thread::Builder::new().spawn(move || {
                    let _ = serve(stream, &offer);
                    done.fetch_sub(1, Ordering::SeqCst);
                });
                if started.is_err() {
                    busy.fetch_sub(1, Ordering::SeqCst);
                }
            }
        });
        Ok(Server { port, offer })
    }

    /// Offers `path` (and nothing else from now on) if it's inside one of `folders` or
    /// OMAESTRO's own converted copies. Returns its path on the server, e.g.
    /// "/f/k3j9x2a8qq01mz7d.flac".
    pub fn offer(&self, path: &Path, folders: &[PathBuf]) -> Result<String> {
        let real = inside(path, folders)?;
        let name = format!(
            "{}.{}",
            crate::secrets::random_string(16, b"abcdefghijklmnopqrstuvwxyz0123456789")?,
            extension(path)
        );
        if let Ok(mut offer) = self.offer.lock() {
            *offer = Some((name.clone(), real));
        }
        Ok(format!("/f/{name}"))
    }

    /// Stops offering anything: Sonos has stopped, or plays something else now.
    pub fn withdraw(&self) {
        if let Ok(mut offer) = self.offer.lock() {
            *offer = None;
        }
    }
}

/// `path` with links resolved, if that's inside one of `folders` or OMAESTRO's converted
/// copies; else an error saying so.
pub fn inside(path: &Path, folders: &[PathBuf]) -> Result<PathBuf> {
    let real = path
        .canonicalize()
        .with_context(|| format!("can't read {}", path.display()))?;
    let ok = folders
        .iter()
        .chain([&cache_dir()])
        .filter_map(|f| f.canonicalize().ok())
        .any(|f| real.starts_with(f));
    if !ok {
        bail!(
            "{} isn't in your music folders, so it isn't offered to Sonos",
            path.display()
        );
    }
    Ok(real)
}

fn respond(stream: &mut TcpStream, status: &str) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

/// One line of the request, up to `LINE` long; None if it's longer, or the speaker is too
/// slow.
fn read_line(reader: &mut impl BufRead, started: Instant) -> std::io::Result<Option<String>> {
    if started.elapsed() > ASKING {
        return Ok(None);
    }
    let mut line = String::new();
    reader.take(LINE).read_line(&mut line)?;
    Ok((line.ends_with('\n') || line.is_empty()).then_some(line))
}

/// Answers one request: GET or HEAD of the offered file, whole or a byte range.
fn serve(mut stream: TcpStream, offer: &Mutex<Offer>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let started = Instant::now();
    let mut reader = BufReader::new(stream.try_clone()?);
    let Some(line) = read_line(&mut reader, started)? else {
        return respond(&mut stream, "400 Bad Request");
    };
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let mut range = None;
    let mut headers = 0;
    loop {
        let Some(header) = read_line(&mut reader, started)? else {
            return respond(&mut stream, "400 Bad Request");
        };
        if header.trim().is_empty() {
            break;
        }
        headers += 1;
        if headers > HEADERS {
            return respond(&mut stream, "400 Bad Request");
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
    let path = target.strip_prefix("/f/").and_then(|name| {
        let offer = offer.lock().ok()?;
        let (offered, path) = offer.as_ref()?;
        (offered == name).then(|| path.clone())
    });
    // Checked again now: the file must still be the one offered, not since swapped for a
    // link to somewhere else.
    let Some(path) = path.filter(|p| p.canonicalize().is_ok_and(|now| now == *p)) else {
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
pub fn prepare_local(track: &Track, folders: &[PathBuf]) -> Result<PathBuf> {
    let path = PathBuf::from(&track.path);
    // Only songs from your music folders (a playlist file could name any file at all).
    inside(&path, folders)?;
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

/// Where a YouTube Music song is kept once downloaded for Sonos.
pub fn youtube_file(video_id: &str) -> PathBuf {
    cache_dir().join(format!("yt-{video_id}.m4a"))
}

/// A YouTube Music song as an AAC file in the cache, for Sonos. Downloads it the first time.
pub fn prepare_youtube(video_id: &str, browser: Option<&str>) -> Result<PathBuf> {
    crate::youtube::check_id(video_id)?;
    let dir = cache_dir();
    std::fs::create_dir_all(&dir)?;
    let out = youtube_file(video_id);
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

    fn local() -> Vec<IpAddr> {
        vec!["127.0.0.1".parse().unwrap()]
    }

    fn http() -> ureq::Agent {
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into()
    }

    fn status(url: &str) -> u16 {
        http().get(url).call().unwrap().status().as_u16()
    }

    /// Serves the offered file whole and in part, and nothing else.
    #[test]
    fn serves_only_the_offered_file_with_ranges() {
        let tmp = TempDir::new("sonos-server");
        let file = tmp.0.join("song.flac");
        std::fs::write(&file, b"0123456789").unwrap();
        let server = Server::start(0, local()).unwrap();
        let port = server.port;
        let folders = [tmp.0.clone()];
        let url = format!(
            "http://127.0.0.1:{port}{}",
            server.offer(&file, &folders).unwrap()
        );

        let mut res = http().get(&url).call().unwrap();
        assert_eq!(res.status().as_u16(), 200);
        assert_eq!(res.headers().get("content-type").unwrap(), "audio/flac");
        assert_eq!(res.body_mut().read_to_string().unwrap(), "0123456789");

        let mut res = http()
            .get(&url)
            .header("Range", "bytes=2-4")
            .call()
            .unwrap();
        assert_eq!(res.status().as_u16(), 206);
        assert_eq!(res.headers().get("content-range").unwrap(), "bytes 2-4/10");
        assert_eq!(res.body_mut().read_to_string().unwrap(), "234");

        let res = http().head(&url).call().unwrap();
        assert_eq!(res.headers().get("content-length").unwrap(), "10");

        assert_eq!(
            status(&format!("http://127.0.0.1:{port}/f/../../etc/passwd")),
            404
        );
        assert_eq!(status(&format!("http://127.0.0.1:{port}/etc/passwd")), 404);

        // Only the latest song is offered, and nothing once withdrawn.
        let next = tmp.0.join("next.flac");
        std::fs::write(&next, b"abc").unwrap();
        let next_url = format!(
            "http://127.0.0.1:{port}{}",
            server.offer(&next, &folders).unwrap()
        );
        assert_eq!(status(&url), 404, "the earlier song isn't offered any more");
        assert_eq!(status(&next_url), 200);
        server.withdraw();
        assert_eq!(status(&next_url), 404);
    }

    /// Files outside the music folders aren't offered, and a file swapped for a link to
    /// somewhere else after it was offered isn't served.
    #[test]
    fn serves_only_files_in_the_music_folders() {
        let tmp = TempDir::new("sonos-server-folders");
        let music = tmp.0.join("music");
        std::fs::create_dir_all(&music).unwrap();
        let secret = tmp.0.join("secret.json");
        std::fs::write(&secret, b"{}").unwrap();
        let server = Server::start(0, local()).unwrap();
        let folders = [music.clone()];

        let err = server.offer(&secret, &folders).unwrap_err();
        assert!(format!("{err:#}").contains("isn't in your music folders"));
        // A link inside the folder that points outside it doesn't count either.
        let link = music.join("song.flac");
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        assert!(server.offer(&link, &folders).is_err());
        assert!(
            prepare_local(
                &Track {
                    path: link.display().to_string(),
                    ..Track::default()
                },
                &folders
            )
            .is_err()
        );

        // Offered as a real file, then swapped for a link.
        std::fs::remove_file(&link).unwrap();
        std::fs::write(&link, b"music").unwrap();
        let url = format!(
            "http://127.0.0.1:{}{}",
            server.port,
            server.offer(&link, &folders).unwrap()
        );
        assert_eq!(status(&url), 200);
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        assert_eq!(status(&url), 404);
    }

    /// Only the speakers are answered; overlong or endless requests are turned away.
    #[test]
    fn answers_only_speakers_and_short_requests() {
        let tmp = TempDir::new("sonos-server-limits");
        let file = tmp.0.join("song.flac");
        std::fs::write(&file, b"0123456789").unwrap();

        // Not one of the speakers: the connection is closed without an answer.
        let elsewhere = Server::start(0, vec!["192.0.2.10".parse().unwrap()]).unwrap();
        let path = elsewhere
            .offer(&file, std::slice::from_ref(&tmp.0))
            .unwrap();
        assert!(
            http()
                .get(&format!("http://127.0.0.1:{}{path}", elsewhere.port))
                .call()
                .is_err()
        );

        let server = Server::start(0, local()).unwrap();
        let path = server.offer(&file, std::slice::from_ref(&tmp.0)).unwrap();
        let ask = |request: &[u8]| {
            let mut s = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
            let _ = s.write_all(request);
            let mut answer = String::new();
            let _ = s.read_to_string(&mut answer);
            answer
        };
        let long = format!("GET /f/{} HTTP/1.1\r\n\r\n", "x".repeat(10_000));
        assert!(ask(long.as_bytes()).starts_with("HTTP/1.1 400"));
        let many = format!("GET {path} HTTP/1.1\r\n{}\r\n", "X-A: b\r\n".repeat(100));
        assert!(ask(many.as_bytes()).starts_with("HTTP/1.1 400"));
        let fine = format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(ask(fine.as_bytes()).starts_with("HTTP/1.1 200"));
    }
}
