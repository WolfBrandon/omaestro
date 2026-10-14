//! Sending plays to Last.fm and ListenBrainz (scrobbling), on a thread of its own.
//!
//! - **Now playing** goes out as a song starts. If that fails it's forgotten, as both services
//!   ask.
//! - **A play** goes out once it counts (see `plays`). It's first written to
//!   `~/.local/share/omaestro/scrobbles.json`, and only taken off once the service has it. So
//!   plays made offline, or while a service is down, go later, oldest first: tried again
//!   every few minutes, and whenever another play comes in.
//! - **A sign-in that stopped working** is left alone (its plays kept) until you connect
//!   again.
//! - **A batch the service refuses as wrong** (not "not now") is dropped, so one bad play
//!   can't hold up the rest. Why is written to `~/.cache/omaestro/scrobble.log`.
//!
//! What's sent: the title, artist, album, album artist, length, track number, when it began,
//! and the MusicBrainz id and ISRC if the file has them. For Spotify and YouTube Music songs
//! also the song's web address there. Never a file's name or folder.

use crate::integrations::{self, LastFm, ListenBrainz, Service};
use crate::library::{Source, Track};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

/// How long to wait before trying again after "not now".
const RETRY: Duration = Duration::from_secs(5 * 60);
/// Plays per request: Last.fm takes up to 50; ListenBrainz up to 1,000, fewer is kinder.
const LASTFM_BATCH: usize = 50;
const LISTENBRAINZ_BATCH: usize = 100;

/// One play, as the services want it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Listen {
    /// When it began (Unix seconds).
    pub at: u64,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub album_artist: String,
    #[serde(default)]
    pub duration_ms: i64,
    #[serde(default)]
    pub track_no: Option<u32>,
    /// MusicBrainz's recording id, if the file is tagged with it.
    #[serde(default)]
    pub mbid: String,
    #[serde(default)]
    pub isrc: String,
    /// "local", "spotify" or "youtube", and the song's id there (empty for files).
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub uri: String,
}

impl Listen {
    /// None for a song without a title or an artist: both services need them.
    pub fn of(t: &Track, at: u64) -> Option<Listen> {
        if t.title.trim().is_empty() || t.artist.trim().is_empty() {
            return None;
        }
        Some(Listen {
            at,
            title: t.title.clone(),
            artist: t.artist.clone(),
            album: t.album.clone(),
            album_artist: t.album_artist.clone(),
            duration_ms: t.duration_ms,
            track_no: t.track_no,
            mbid: t.mb_track_id.clone(),
            isrc: t.isrc.split(';').next().unwrap_or_default().to_string(),
            source: t.source.key().to_string(),
            uri: if t.source == Source::Local {
                String::new()
            } else {
                t.uri.clone()
            },
        })
    }

    /// The song's address on its service, for a streamed song.
    fn web_address(&self) -> Option<String> {
        match self.source.as_str() {
            "spotify" => self
                .uri
                .strip_prefix("spotify:track:")
                .map(|id| format!("https://open.spotify.com/track/{id}")),
            "youtube" if crate::youtube::is_video_id(&self.uri) => {
                Some(format!("https://music.youtube.com/watch?v={}", self.uri))
            }
            _ => None,
        }
    }
}

/// What a service said to a request.
#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    Sent,
    /// Wrong in itself: sending it again won't help.
    Dropped(String),
    /// Not now (offline, busy, down): try again later.
    Later(String),
    /// The sign-in doesn't work any more.
    SignIn(String),
}

// ---- Last.fm --------------------------------------------------------------------------

/// The fields both Last.fm calls share, with `[i]` after each name for a scrobble batch.
fn lastfm_fields(l: &Listen, i: Option<usize>) -> Vec<(String, String)> {
    let name = |n: &str| match i {
        Some(i) => format!("{n}[{i}]"),
        None => n.to_string(),
    };
    let mut p = vec![
        (name("artist"), l.artist.clone()),
        (name("track"), l.title.clone()),
    ];
    if !l.album.is_empty() {
        p.push((name("album"), l.album.clone()));
    }
    if !l.album_artist.is_empty() {
        p.push((name("albumArtist"), l.album_artist.clone()));
    }
    if l.duration_ms > 0 {
        p.push((name("duration"), (l.duration_ms / 1000).to_string()));
    }
    if let Some(n) = l.track_no {
        p.push((name("trackNumber"), n.to_string()));
    }
    if !l.mbid.is_empty() {
        p.push((name("mbid"), l.mbid.clone()));
    }
    p
}

fn lastfm_scrobble_params(batch: &[Listen], account: &LastFm) -> Vec<(String, String)> {
    let mut p = vec![
        ("method".to_string(), "track.scrobble".to_string()),
        ("api_key".to_string(), account.api_key.clone()),
        ("sk".to_string(), account.session_key.clone()),
    ];
    for (i, l) in batch.iter().enumerate() {
        p.extend(lastfm_fields(l, Some(i)));
        p.push((format!("timestamp[{i}]"), l.at.to_string()));
    }
    p
}

fn lastfm_now_params(l: &Listen, account: &LastFm) -> Vec<(String, String)> {
    let mut p = vec![
        ("method".to_string(), "track.updateNowPlaying".to_string()),
        ("api_key".to_string(), account.api_key.clone()),
        ("sk".to_string(), account.session_key.clone()),
    ];
    p.extend(lastfm_fields(l, None));
    p
}

/// Last.fm's answer, sorted by what to do about it. Its rules: only errors 11 and 16 (and 9,
/// after signing in again) are worth repeating; the rest mean the request was wrong.
fn lastfm_outcome(reply: Result<Value, String>) -> Outcome {
    let body = match reply {
        Ok(body) => body,
        Err(e) => return Outcome::Later(e),
    };
    let Some(code) = body["error"].as_u64() else {
        return if body.get("scrobbles").is_some() || body.get("nowplaying").is_some() {
            Outcome::Sent
        } else {
            Outcome::Later("Last.fm's answer made no sense".into())
        };
    };
    let said = format!(
        "Last.fm said: {}",
        body["message"].as_str().unwrap_or("something went wrong")
    );
    match code {
        // The session, key or secret no longer works.
        4 | 9 | 10 | 13 | 26 => Outcome::SignIn(
            "Last.fm doesn't accept the sign-in any more: connect again in Settings → \
             Integrations"
                .into(),
        ),
        // Down, busy, or too many requests.
        8 | 11 | 16 | 29 => Outcome::Later(said),
        _ => Outcome::Dropped(said),
    }
}

fn lastfm_send(api: &str, params: &[(String, String)], secret: &str) -> Outcome {
    let params: Vec<(&str, &str)> = params
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    lastfm_outcome(integrations::lastfm_call_at(api, &params, secret, true))
}

// ---- ListenBrainz ---------------------------------------------------------------------

fn listenbrainz_track(l: &Listen) -> Value {
    let mut info = json!({
        "media_player": "OMAESTRO",
        "media_player_version": env!("CARGO_PKG_VERSION"),
        "submission_client": "OMAESTRO",
        "submission_client_version": env!("CARGO_PKG_VERSION"),
    });
    if l.duration_ms > 0 {
        info["duration_ms"] = json!(l.duration_ms);
    }
    if let Some(n) = l.track_no {
        info["tracknumber"] = json!(n);
    }
    if !l.mbid.is_empty() {
        info["recording_mbid"] = json!(l.mbid);
    }
    if !l.isrc.is_empty() {
        info["isrc"] = json!(l.isrc);
    }
    if let Some(address) = l.web_address() {
        info["origin_url"] = json!(address);
        match l.source.as_str() {
            "spotify" => {
                info["music_service"] = json!("spotify.com");
                info["spotify_id"] = json!(address);
            }
            _ => {
                info["music_service"] = json!("music.youtube.com");
                info["music_service_name"] = json!("YouTube Music");
            }
        }
    }
    let mut meta = json!({
        "artist_name": l.artist,
        "track_name": l.title,
        "additional_info": info,
    });
    if !l.album.is_empty() {
        meta["release_name"] = json!(l.album);
    }
    meta
}

/// A submit-listens request: "playing_now" (one song, no time), "single" (one play) or
/// "import" (several).
fn listenbrainz_body(listen_type: &str, batch: &[Listen]) -> Value {
    let payload: Vec<Value> = batch
        .iter()
        .map(|l| {
            let mut one = json!({ "track_metadata": listenbrainz_track(l) });
            if listen_type != "playing_now" {
                one["listened_at"] = json!(l.at);
            }
            one
        })
        .collect();
    json!({ "listen_type": listen_type, "payload": payload })
}

/// ListenBrainz's answer (its status, and what it said), sorted by what to do about it.
fn listenbrainz_outcome(reply: Result<(u16, String), String>) -> Outcome {
    let (status, text) = match reply {
        Ok(answer) => answer,
        Err(e) => return Outcome::Later(e),
    };
    let said = || {
        let reason = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_string))
            .unwrap_or_else(|| format!("status {status}"));
        format!("ListenBrainz said: {reason}")
    };
    match status {
        200 => Outcome::Sent,
        401 => Outcome::SignIn(
            "ListenBrainz doesn't accept the token any more: connect again in Settings → \
             Integrations"
                .into(),
        ),
        400 => Outcome::Dropped(said()),
        _ => Outcome::Later(said()),
    }
}

fn listenbrainz_send(api: &str, token: &str, body: &Value) -> Outcome {
    let reply = integrations::http()
        .post(&format!("{api}/submit-listens"))
        .header("Authorization", &format!("Token {token}"))
        .send_json(body)
        .map_err(|e| format!("reaching ListenBrainz: {e}"))
        .map(|mut res| {
            let status = res.status().as_u16();
            let text = res.body_mut().read_to_string().unwrap_or_default();
            (status, text)
        });
    listenbrainz_outcome(reply)
}

// ---- the waiting plays ----------------------------------------------------------------

/// Plays not yet taken by each service, oldest first.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
struct Waiting {
    #[serde(default)]
    lastfm: Vec<Listen>,
    #[serde(default)]
    listenbrainz: Vec<Listen>,
}

impl Waiting {
    fn load(path: &Path) -> Waiting {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Written in one go, readable only by you: it's your listening history.
    fn save(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let _ = std::fs::remove_file(&tmp);
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?
            .write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
        std::fs::rename(&tmp, path)
    }

    fn of(&mut self, service: Service) -> &mut Vec<Listen> {
        match service {
            Service::LastFm => &mut self.lastfm,
            Service::ListenBrainz => &mut self.listenbrainz,
        }
    }
}

/// Why sending stopped.
#[derive(Debug, PartialEq)]
enum Stopped {
    /// Everything was sent (or dropped).
    Done,
    Later(String),
    SignIn(String),
}

/// Sends `waiting` a batch at a time, oldest first, taking each batch off once the service
/// has it (or has refused it as wrong) and saving straight after. Stops at "not now" or
/// "sign in again".
fn send_waiting(
    waiting: &mut Vec<Listen>,
    size: usize,
    mut send: impl FnMut(&[Listen]) -> Outcome,
    mut saved: impl FnMut(&[Listen]),
) -> Stopped {
    while !waiting.is_empty() {
        let n = waiting.len().min(size);
        match send(&waiting[..n]) {
            Outcome::Sent => {}
            Outcome::Dropped(why) => log(&format!("dropped {n} plays: {why}")),
            Outcome::Later(why) => return Stopped::Later(why),
            Outcome::SignIn(why) => return Stopped::SignIn(why),
        }
        waiting.drain(..n);
        saved(waiting);
    }
    Stopped::Done
}

fn log(line: &str) {
    use std::io::Write;
    if cfg!(test) {
        return;
    }
    let path = crate::config::cache_dir().join("scrobble.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{} {line}", crate::secrets::now_secs());
    }
}

// ---- the thread -----------------------------------------------------------------------

/// The accounts signed in now (read afresh each time, so signing in or out just works).
#[derive(Debug, Clone, Default)]
pub struct Accounts {
    pub lastfm: Option<LastFm>,
    pub listenbrainz: Option<ListenBrainz>,
}

impl Accounts {
    pub fn load() -> Accounts {
        Accounts {
            lastfm: LastFm::load(),
            listenbrainz: ListenBrainz::load(),
        }
    }

    /// What identifies the sign-in, to notice a new one.
    fn credential(&self, service: Service) -> Option<String> {
        match service {
            Service::LastFm => self.lastfm.as_ref().map(|a| a.session_key.clone()),
            Service::ListenBrainz => self.listenbrainz.as_ref().map(|a| a.token.clone()),
        }
    }
}

/// Where the services are: the real ones, or the tests' stand-ins.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub lastfm: String,
    pub listenbrainz: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            lastfm: integrations::LASTFM_API.into(),
            listenbrainz: integrations::LISTENBRAINZ_API.into(),
        }
    }
}

/// How a service's plays are going, for Settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    /// Plays not yet taken.
    pub waiting: usize,
    /// Something only you can fix (signing in again).
    pub problem: Option<String>,
}

enum Msg {
    NowPlaying(Listen),
    Played(Listen),
}

/// The line to the scrobbling thread. Dropping it ends the thread; waiting plays are on disk
/// already.
pub struct Scrobbler {
    tx: Sender<Msg>,
}

impl Scrobbler {
    /// Starts the thread, which first sends anything left from last time. `file` keeps the
    /// waiting plays; `report` hears how each connected service is doing.
    pub fn start(
        file: PathBuf,
        endpoints: Endpoints,
        accounts: impl Fn() -> Accounts + Send + 'static,
        report: impl Fn(Service, Report) + Send + 'static,
    ) -> Scrobbler {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || run(&file, &endpoints, accounts, report, rx));
        Scrobbler { tx }
    }

    pub fn now_playing(&self, listen: Listen) {
        let _ = self.tx.send(Msg::NowPlaying(listen));
    }

    pub fn played(&self, listen: Listen) {
        let _ = self.tx.send(Msg::Played(listen));
    }
}

const SERVICES: [Service; 2] = [Service::LastFm, Service::ListenBrainz];

fn run(
    file: &Path,
    endpoints: &Endpoints,
    accounts: impl Fn() -> Accounts,
    report: impl Fn(Service, Report),
    rx: Receiver<Msg>,
) {
    let mut waiting = Waiting::load(file);
    // Sign-ins a service refused, with why: not tried again until they change.
    let mut refused: [Option<(String, String)>; 2] = [None, None];
    let mut next_try = Some(Instant::now());
    loop {
        let wait = next_try.map_or(Duration::from_secs(3600), |t| {
            t.saturating_duration_since(Instant::now())
        });
        match rx.recv_timeout(wait) {
            Ok(Msg::NowPlaying(listen)) => {
                let accounts = accounts();
                for (i, service) in SERVICES.into_iter().enumerate() {
                    let ok = accounts.credential(service).is_some_and(|c| {
                        refused[i].as_ref().is_none_or(|(refused, _)| *refused != c)
                    });
                    if ok {
                        let _ = now_playing(service, &listen, &accounts, endpoints);
                    }
                }
                continue;
            }
            Ok(Msg::Played(listen)) => {
                let accounts = accounts();
                for service in SERVICES {
                    if accounts.credential(service).is_some() {
                        waiting.of(service).push(listen.clone());
                    }
                }
                if let Err(e) = waiting.save(file) {
                    log(&format!("couldn't keep the waiting plays: {e}"));
                }
                next_try = Some(Instant::now());
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if next_try.is_some_and(|t| t <= Instant::now()) {
            let accounts = accounts();
            let mut again = false;
            for (i, service) in SERVICES.into_iter().enumerate() {
                let Some(credential) = accounts.credential(service) else {
                    continue;
                };
                if let Some((refused_one, why)) = &refused[i]
                    && *refused_one == credential
                {
                    report(
                        service,
                        Report {
                            waiting: waiting.of(service).len(),
                            problem: Some(why.clone()),
                        },
                    );
                    continue;
                }
                refused[i] = None;
                let mut list = std::mem::take(waiting.of(service));
                let stopped = send_service(service, &mut list, &accounts, endpoints, |left| {
                    *waiting.of(service) = left.to_vec();
                    let _ = waiting.save(file);
                });
                *waiting.of(service) = list;
                let _ = waiting.save(file);
                let problem = match stopped {
                    Stopped::Done => None,
                    Stopped::Later(why) => {
                        log(&format!("{}: trying again later: {why}", service.name()));
                        again = true;
                        None
                    }
                    Stopped::SignIn(why) => {
                        refused[i] = Some((credential, why.clone()));
                        Some(why)
                    }
                };
                report(
                    service,
                    Report {
                        waiting: waiting.of(service).len(),
                        problem,
                    },
                );
            }
            next_try = again.then(|| Instant::now() + RETRY);
        }
    }
}

/// Sends one service's waiting plays.
fn send_service(
    service: Service,
    list: &mut Vec<Listen>,
    accounts: &Accounts,
    endpoints: &Endpoints,
    saved: impl FnMut(&[Listen]),
) -> Stopped {
    match service {
        Service::LastFm => {
            let Some(account) = &accounts.lastfm else {
                return Stopped::Done;
            };
            send_waiting(
                list,
                LASTFM_BATCH,
                |batch| {
                    lastfm_send(
                        &endpoints.lastfm,
                        &lastfm_scrobble_params(batch, account),
                        &account.secret,
                    )
                },
                saved,
            )
        }
        Service::ListenBrainz => {
            let Some(account) = &accounts.listenbrainz else {
                return Stopped::Done;
            };
            let mut first = true;
            send_waiting(
                list,
                LISTENBRAINZ_BATCH,
                |batch| {
                    // ListenBrainz asks for no more than one request a second.
                    if !first {
                        std::thread::sleep(Duration::from_secs(1));
                    }
                    first = false;
                    let kind = if batch.len() == 1 { "single" } else { "import" };
                    listenbrainz_send(
                        &endpoints.listenbrainz,
                        &account.token,
                        &listenbrainz_body(kind, batch),
                    )
                },
                saved,
            )
        }
    }
}

fn now_playing(
    service: Service,
    listen: &Listen,
    accounts: &Accounts,
    endpoints: &Endpoints,
) -> Outcome {
    match (service, &accounts.lastfm, &accounts.listenbrainz) {
        (Service::LastFm, Some(account), _) => lastfm_send(
            &endpoints.lastfm,
            &lastfm_now_params(listen, account),
            &account.secret,
        ),
        (Service::ListenBrainz, _, Some(account)) => listenbrainz_send(
            &endpoints.listenbrainz,
            &account.token,
            &listenbrainz_body("playing_now", std::slice::from_ref(listen)),
        ),
        _ => Outcome::Sent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    fn listen(title: &str, at: u64) -> Listen {
        Listen::of(
            &Track {
                source: Source::Spotify,
                uri: "spotify:track:4uLU6hMCjMI75M1A2tKUQC".into(),
                title: title.into(),
                artist: "Rick".into(),
                album: "Hits".into(),
                duration_ms: 213_000,
                track_no: Some(1),
                isrc: "GBARL9300135;X".into(),
                ..Track::default()
            },
            at,
        )
        .unwrap()
    }

    fn lastfm() -> LastFm {
        LastFm {
            api_key: "key".into(),
            secret: "secret".into(),
            session_key: "sk".into(),
            user: "me".into(),
        }
    }

    #[test]
    fn what_a_play_carries() {
        let l = listen("Roll", 1_000);
        assert_eq!(l.isrc, "GBARL9300135", "the first ISRC");
        assert_eq!(Listen::of(&Track::default(), 1), None, "no title or artist");
        let local = Listen::of(
            &Track {
                path: "/home/me/music/a.flac".into(),
                title: "A".into(),
                artist: "B".into(),
                ..Track::default()
            },
            1,
        )
        .unwrap();
        assert_eq!(local.uri, "", "never a file's path");
        assert!(
            !listenbrainz_body("single", &[local])
                .to_string()
                .contains("/home")
        );
    }

    #[test]
    fn last_fm_requests() {
        let batch = [listen("One", 100), listen("Two", 200)];
        let p = lastfm_scrobble_params(&batch, &lastfm());
        let get = |k: &str| p.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("method"), Some("track.scrobble"));
        assert_eq!(get("sk"), Some("sk"));
        assert_eq!(get("track[0]"), Some("One"));
        assert_eq!(get("track[1]"), Some("Two"));
        assert_eq!(get("timestamp[1]"), Some("200"));
        assert_eq!(get("duration[0]"), Some("213"));
        assert_eq!(get("album[0]"), Some("Hits"));
        assert_eq!(get("mbid[0]"), None, "only when tagged");
        let now = lastfm_now_params(&batch[0], &lastfm());
        assert!(now.contains(&("track".into(), "One".into())));
        assert!(!now.iter().any(|(k, _)| k.starts_with("timestamp")));

        let reply = |v: Value| lastfm_outcome(Ok(v));
        assert_eq!(
            reply(json!({"scrobbles": {"@attr": {"accepted": 1, "ignored": 1}}})),
            Outcome::Sent,
            "ignored ones are done too"
        );
        assert!(matches!(reply(json!({"error": 9})), Outcome::SignIn(_)));
        assert!(matches!(reply(json!({"error": 11})), Outcome::Later(_)));
        assert!(matches!(reply(json!({"error": 16})), Outcome::Later(_)));
        assert!(matches!(
            reply(json!({"error": 6, "message": "x"})),
            Outcome::Dropped(_)
        ));
        assert!(matches!(
            lastfm_outcome(Err("offline".into())),
            Outcome::Later(_)
        ));
    }

    #[test]
    fn listenbrainz_requests() {
        let body = listenbrainz_body("import", &[listen("One", 100), listen("Two", 200)]);
        assert_eq!(body["listen_type"], "import");
        let first = &body["payload"][0];
        assert_eq!(first["listened_at"], 100);
        assert_eq!(first["track_metadata"]["artist_name"], "Rick");
        assert_eq!(first["track_metadata"]["track_name"], "One");
        assert_eq!(first["track_metadata"]["release_name"], "Hits");
        let info = &first["track_metadata"]["additional_info"];
        assert_eq!(info["duration_ms"], 213_000);
        assert_eq!(info["submission_client"], "OMAESTRO");
        assert_eq!(
            info["spotify_id"],
            "https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC"
        );
        let now = listenbrainz_body("playing_now", &[listen("One", 100)]);
        assert!(now["payload"][0].get("listened_at").is_none());

        let reply = |status: u16, text: &str| listenbrainz_outcome(Ok((status, text.into())));
        assert_eq!(reply(200, "{\"status\":\"ok\"}"), Outcome::Sent);
        assert!(matches!(reply(401, ""), Outcome::SignIn(_)));
        assert_eq!(
            reply(400, "{\"error\":\"bad listen\"}"),
            Outcome::Dropped("ListenBrainz said: bad listen".into())
        );
        assert!(matches!(reply(429, ""), Outcome::Later(_)));
        assert!(matches!(reply(503, ""), Outcome::Later(_)));
    }

    #[test]
    fn sends_oldest_first_and_keeps_what_isnt_taken() {
        let mut waiting: Vec<Listen> = (0..5).map(|i| listen(&i.to_string(), i)).collect();
        let mut sent = Vec::new();
        let mut answers = vec![
            Outcome::Sent,
            Outcome::Dropped("bad".into()),
            Outcome::Later("down".into()),
        ];
        answers.reverse();
        let stopped = send_waiting(
            &mut waiting,
            2,
            |batch| {
                sent.push(batch.iter().map(|l| l.at).collect::<Vec<_>>());
                answers.pop().unwrap()
            },
            |_| {},
        );
        assert_eq!(stopped, Stopped::Later("down".into()));
        assert_eq!(sent, vec![vec![0, 1], vec![2, 3], vec![4]]);
        assert_eq!(waiting.iter().map(|l| l.at).collect::<Vec<_>>(), vec![4]);
    }

    /// A stand-in for Last.fm and ListenBrainz on this computer: answers each request with
    /// the next status and body from `answers`, and keeps what it was sent.
    fn stand_in(answers: Vec<(u16, &'static str)>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let got = Arc::new(Mutex::new(Vec::new()));
        let kept = got.clone();
        std::thread::spawn(move || {
            for (stream, (status, body)) in listener.incoming().flatten().zip(answers) {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                let mut length = 0;
                let mut request = String::new();
                reader.read_line(&mut line).unwrap();
                request.push_str(&line);
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if let Some(n) = header.to_lowercase().strip_prefix("content-length:") {
                        length = n.trim().parse().unwrap();
                    }
                    if header.trim().is_empty() {
                        break;
                    }
                    request.push_str(&header);
                }
                let mut sent = vec![0; length];
                reader.read_exact(&mut sent).unwrap();
                request.push_str(&String::from_utf8_lossy(&sent));
                kept.lock().unwrap().push(request);
                let mut stream = stream;
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (address, got)
    }

    /// The whole thread: plays wait on disk while ListenBrainz is down, go once it's back,
    /// and a token it refuses isn't tried again until it changes.
    #[test]
    fn plays_wait_on_disk_until_the_service_takes_them() {
        let tmp = TempDir::new("scrobble");
        let file = tmp.0.join("scrobbles.json");
        let (address, got) = stand_in(vec![
            (503, "{\"error\":\"down\"}"),
            (200, "{\"status\":\"ok\"}"),
            (401, "{\"error\":\"bad token\"}"),
        ]);
        let endpoints = Endpoints {
            lastfm: "http://127.0.0.1:9/never".into(),
            listenbrainz: address,
        };
        let accounts = || Accounts {
            lastfm: None,
            listenbrainz: Some(ListenBrainz {
                token: "t".into(),
                user: "me".into(),
            }),
        };
        let reports = Arc::new(Mutex::new(Vec::new()));
        let heard = reports.clone();
        let (tx, rx) = mpsc::channel();
        let thread_file = file.clone();
        let thread = std::thread::spawn(move || {
            run(
                &thread_file,
                &endpoints,
                accounts,
                move |_, r| heard.lock().unwrap().push(r),
                rx,
            )
        });
        let wait_for = |n: usize| {
            for _ in 0..100 {
                if got.lock().unwrap().len() >= n {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            std::thread::sleep(Duration::from_millis(100));
        };

        // Down: the play is kept on disk.
        tx.send(Msg::Played(listen("One", 100))).unwrap();
        wait_for(1);
        assert_eq!(Waiting::load(&file).listenbrainz.len(), 1);
        assert!(
            got.lock().unwrap()[0]
                .to_lowercase()
                .contains("authorization: token t")
        );

        // The next play brings a retry: both go, oldest first, in one request.
        tx.send(Msg::Played(listen("Two", 200))).unwrap();
        wait_for(2);
        let sent = got.lock().unwrap()[1].clone();
        assert!(sent.contains("\"import\"") && sent.find("One") < sent.find("Two"));
        assert_eq!(Waiting::load(&file).listenbrainz.len(), 0);

        // Refused: kept, with the problem reported, and not tried again.
        tx.send(Msg::Played(listen("Three", 300))).unwrap();
        wait_for(3);
        tx.send(Msg::Played(listen("Four", 400))).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(
            got.lock().unwrap().len(),
            3,
            "a refused token isn't used again"
        );
        assert_eq!(Waiting::load(&file).listenbrainz.len(), 2);
        let last = reports.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last.waiting, 2);
        assert!(last.problem.unwrap().contains("connect again"));

        drop(tx);
        thread.join().unwrap();
    }
}
