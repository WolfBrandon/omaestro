//! YouTube Music: search, your library, and turning a song into a stream mpv can play.
//!
//! There's no official API. Search and library come from the same web API the YouTube Music
//! site uses ("InnerTube"). Streams come from `yt-dlp`, which keeps up with YouTube's changes.
//!
//! Signing in borrows your browser's YouTube login: `yt-dlp` exports the browser's cookies
//! into a private folder, and OMAESTRO keeps only YouTube's, in
//! `~/.config/omaestro/ytm-cookies.txt` (readable only by you). The rest are deleted.
//! OMAESTRO never sees your password. YouTube may ask for a fresh login now and then; connect
//! again in Settings. The yt-dlp wiki warns that heavy use with cookies could get an account
//! flagged, so OMAESTRO only makes the requests you ask for.

use crate::library::{Source, Track};
use crate::secrets;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

const FILE: &str = "youtube.json";
const COOKIES: &str = "ytm-cookies.txt";
const ORIGIN: &str = "https://music.youtube.com";
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
                          Chrome/141.0.0.0 Safari/537.36";
/// Search filter: songs only.
const SONGS: &str = "EgWKAQIIAWoMEA4QChADEAQQCRAF";
/// Search filter: videos only. Covers, live versions and uploads are often only videos.
const VIDEOS: &str = "EgWKAQIQAWoMEA4QChADEAQQCRAF";
/// How many videos a search adds after the songs.
const VIDEOS_SHOWN: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Account {
    /// The browser the login comes from, as yt-dlp names it.
    pub browser: String,
    pub user: String,
}

pub fn account() -> Option<Account> {
    secrets::load(FILE)
}

pub fn sign_out() {
    secrets::forget(FILE);
    secrets::forget(COOKIES);
}

/// A browser login yt-dlp can read: one per browser, or one per profile for Chromium-style
/// browsers, which keep a separate login in each profile.
#[derive(Debug, Clone, PartialEq)]
pub struct Login {
    /// What yt-dlp's `--cookies-from-browser` takes: "firefox" or "chromium:Profile 1".
    pub arg: String,
    /// What the list shows: "Chromium · Personal · someone@gmail.com".
    pub label: String,
}

/// Browsers yt-dlp can read: its name for them, the programs that mean it's installed, the
/// name to show, and (for Chromium-style ones) the folder that lists their profiles.
const BROWSERS: &[(&str, &[&str], &str, &str)] = &[
    ("chromium", &["chromium"], "Chromium", ".config/chromium"),
    (
        "chrome",
        &["google-chrome-stable", "google-chrome"],
        "Chrome",
        ".config/google-chrome",
    ),
    (
        "brave",
        &["brave", "brave-browser"],
        "Brave",
        ".config/BraveSoftware/Brave-Browser",
    ),
    ("firefox", &["firefox"], "Firefox", ""),
    (
        "vivaldi",
        &["vivaldi-stable", "vivaldi"],
        "Vivaldi",
        ".config/vivaldi",
    ),
    (
        "edge",
        &["microsoft-edge-stable"],
        "Edge",
        ".config/microsoft-edge",
    ),
    ("opera", &["opera"], "Opera", ""),
];

/// Every browser login on this computer that yt-dlp could read.
pub fn logins() -> Vec<Login> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let mut out = Vec::new();
    for (name, programs, shown, dir) in BROWSERS {
        if !programs.iter().any(|p| which(p)) {
            continue;
        }
        let profiles = if dir.is_empty() {
            Vec::new()
        } else {
            std::fs::read_to_string(home.join(dir).join("Local State"))
                .map(|text| chromium_profiles(&text))
                .unwrap_or_default()
        };
        if profiles.is_empty() {
            out.push(Login {
                arg: name.to_string(),
                label: shown.to_string(),
            });
        }
        for (folder, label) in profiles {
            out.push(Login {
                arg: format!("{name}:{folder}"),
                label: format!("{shown} · {label}"),
            });
        }
    }
    out
}

/// The profiles listed in a Chromium "Local State" file: (folder, "name · account"), the
/// default one first.
fn chromium_profiles(local_state: &str) -> Vec<(String, String)> {
    let v: Value = serde_json::from_str(local_state).unwrap_or_default();
    let mut profiles: Vec<(String, String)> = v["profile"]["info_cache"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(folder, info)| {
            let name = info["name"].as_str().unwrap_or(folder);
            let label = match info["user_name"].as_str().filter(|u| !u.is_empty()) {
                Some(account) => format!("{name} · {account}"),
                None => name.to_string(),
            };
            (folder.clone(), label)
        })
        .collect();
    profiles.sort_by_key(|(folder, _)| (folder != "Default", folder.clone()));
    profiles
}

fn which(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

/// Whether `id` looks like a YouTube video id: 11 letters, digits, `-` or `_`. Ids from a
/// playlist file could be anything, and they end up in file names and yt-dlp's arguments.
pub fn is_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// An error unless `id` is a YouTube video id.
pub fn check_id(id: &str) -> Result<()> {
    if !is_video_id(id) {
        bail!("{id:?} isn't a YouTube Music song id");
    }
    Ok(())
}

/// Copies the YouTube login from `browser` and checks it works. Returns the account name.
/// Takes a few seconds (yt-dlp may unlock the browser's keyring).
pub fn connect(browser: &str) -> Result<String> {
    use std::os::unix::fs::DirBuilderExt;
    let path = secrets::path(COOKIES);
    let config = crate::config::private_dir(&crate::config::config_dir())?;
    // yt-dlp exports every site's cookies. They go into a folder only you can open, made
    // before yt-dlp writes, and removed however this ends. (The file itself must not exist
    // yet: yt-dlp reads an existing one first, and refuses an empty one.)
    let export = Export(config.join(format!(
        ".cookies-{}",
        secrets::random_string(12, b"abcdefghijklmnopqrstuvwxyz0123456789")?
    )));
    std::fs::DirBuilder::new().mode(0o700).create(&export.0)?;
    let tmp = export.0.join("cookies.txt");
    // With no URL yt-dlp writes the cookie file and then complains there's nothing to do.
    let out = Command::new("yt-dlp")
        .args([
            "--ignore-config",
            "--cookies-from-browser",
            browser,
            "--cookies",
        ])
        .arg(&tmp)
        .stdin(Stdio::null())
        .output()
        .context("running yt-dlp (is it installed?)")?;
    let exported = std::fs::read_to_string(&tmp).unwrap_or_default();
    if exported.trim().is_empty() {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err
            .lines()
            .rev()
            .find(|l| l.contains("ERROR"))
            .unwrap_or("no cookie file");
        bail!("Couldn't read {browser}'s cookies: {last}");
    }
    let kept = youtube_only(&exported);
    if !Cookies::parse(&kept).signed_in() {
        bail!("{browser} isn't signed in to YouTube. Sign in at music.youtube.com there first.");
    }
    write_private(&path, &kept)?;
    drop(export);
    let mut client = Client::new()?;
    let user = client
        .account_name()
        .unwrap_or_else(|_| "YouTube Music".into());
    secrets::save(
        FILE,
        &Account {
            browser: browser.to_string(),
            user: user.clone(),
        },
    )?;
    Ok(user)
}

/// The folder yt-dlp exports into; removed, with everything in it, when dropped.
struct Export(PathBuf);

impl Drop for Export {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Writes `text` to `path`, readable only by you, in one go.
fn write_private(path: &std::path::Path, text: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension("new");
    let _ = std::fs::remove_file(&tmp);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?
        .write_all(text.as_bytes())?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Whether a cookie-file domain is YouTube's: "youtube.com", ".youtube.com",
/// "music.youtube.com"; not "notyoutube.com".
fn youtube_domain(domain: &str) -> bool {
    let domain = domain.trim_start_matches('.');
    domain == "youtube.com" || domain.ends_with(".youtube.com")
}

/// A cookie file (Netscape format) cut down to YouTube's cookies; comments are kept.
pub fn youtube_only(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let cookie = line.strip_prefix("#HttpOnly_").unwrap_or(line);
        let keep = if cookie.starts_with('#') || cookie.trim().is_empty() {
            true
        } else {
            youtube_domain(cookie.split('\t').next().unwrap_or(""))
        };
        if keep {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Cuts a saved cookie file from before 0.9.7, which held every site's cookies, down to
/// YouTube's. Does nothing if there's no file or it's already cut down.
pub fn tidy_cookies() -> Result<()> {
    let path = secrets::path(COOKIES);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let kept = youtube_only(&text);
    if kept != text {
        write_private(&path, &kept)?;
    }
    Ok(())
}

/// Cookies from a Netscape-format cookie file, the ones YouTube Music needs.
#[derive(Debug, Default, Clone)]
pub struct Cookies(Vec<(String, String)>);

impl Cookies {
    pub fn parse(text: &str) -> Cookies {
        let now = secrets::now_secs();
        let mut out = Vec::new();
        for line in text.lines() {
            // "#HttpOnly_" lines are real cookies; other "#" lines are comments.
            let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
            if line.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() != 7 {
                continue;
            }
            if !youtube_domain(fields[0]) {
                continue;
            }
            let expires: u64 = fields[4].parse().unwrap_or(0);
            if expires != 0 && expires < now {
                continue;
            }
            out.push((fields[5].to_string(), fields[6].to_string()));
        }
        Cookies(out)
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn signed_in(&self) -> bool {
        self.get("LOGIN_INFO").is_some()
            && ["SAPISID", "__Secure-3PAPISID", "__Secure-1PAPISID"]
                .iter()
                .any(|n| self.get(n).is_some())
    }

    fn header(&self) -> String {
        let mut parts: Vec<String> = self.0.iter().map(|(n, v)| format!("{n}={v}")).collect();
        parts.push("SOCS=CAI".into());
        parts.join("; ")
    }

    /// The Authorization header YouTube expects from a signed-in browser:
    /// "SAPISIDHASH <time>_<sha1(time + ' ' + SAPISID + ' ' + origin)>" and its two siblings.
    fn authorization(&self, now: u64) -> Option<String> {
        let hash = |cookie: &str| {
            let input = format!("{now} {cookie} {ORIGIN}");
            format!("{now}_{}", sha1_smol::Sha1::from(input).digest())
        };
        let sapisid = self.get("SAPISID").or(self.get("__Secure-3PAPISID"))?;
        let mut parts = vec![format!("SAPISIDHASH {}", hash(sapisid))];
        if let Some(c) = self.get("__Secure-1PAPISID") {
            parts.push(format!("SAPISID1PHASH {}", hash(c)));
        }
        if let Some(c) = self.get("__Secure-3PAPISID") {
            parts.push(format!("SAPISID3PHASH {}", hash(c)));
        }
        Some(parts.join(" "))
    }
}

/// Talks to YouTube Music's web API.
pub struct Client {
    http: ureq::Agent,
    version: String,
    visitor: String,
    cookies: Option<Cookies>,
}

impl Client {
    /// Reads the site's current API version (it changes every few days) and, if signed in,
    /// the saved cookies.
    pub fn new() -> Result<Client> {
        let http: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .build()
            .into();
        let mut res = http
            .get(&format!("{ORIGIN}/"))
            .header("User-Agent", USER_AGENT)
            .header("Cookie", "SOCS=CAI")
            .call()
            .context("reaching YouTube Music")?;
        let page = res
            .body_mut()
            .with_config()
            .limit(8 * 1024 * 1024)
            .read_to_string()?;
        let find = |key: &str| {
            let start = page.find(&format!("\"{key}\":\""))? + key.len() + 4;
            let end = page[start..].find('"')? + start;
            Some(page[start..end].to_string())
        };
        let cookies = std::fs::read_to_string(secrets::path(COOKIES))
            .ok()
            .map(|text| Cookies::parse(&text))
            .filter(Cookies::signed_in);
        Ok(Client {
            version: find("INNERTUBE_CLIENT_VERSION")
                .context("YouTube Music's page has changed")?,
            visitor: find("VISITOR_DATA").unwrap_or_default(),
            cookies,
            http,
        })
    }

    pub fn signed_in(&self) -> bool {
        self.cookies.is_some()
    }

    fn post(&self, endpoint: &str, body: Value) -> Result<Value> {
        let mut payload = json!({
            "context": {"client": {"clientName": "WEB_REMIX", "clientVersion": self.version,
                                   "hl": "en", "gl": "US"}, "user": {}}
        });
        if let (Some(p), Some(b)) = (payload.as_object_mut(), body.as_object()) {
            p.extend(b.clone());
        }
        let mut req = self
            .http
            .post(&format!(
                "{ORIGIN}/youtubei/v1/{endpoint}?prettyPrint=false"
            ))
            .header("User-Agent", USER_AGENT)
            .header("Origin", ORIGIN)
            .header("X-Origin", ORIGIN)
            .header("Referer", &format!("{ORIGIN}/"))
            .header("X-Goog-Visitor-Id", &self.visitor)
            .header("X-YouTube-Client-Name", "67")
            .header("X-YouTube-Client-Version", &self.version);
        match &self.cookies {
            Some(c) => {
                req = req
                    .header("Cookie", &c.header())
                    .header("X-Goog-AuthUser", "0");
                if let Some(auth) = c.authorization(secrets::now_secs()) {
                    req = req.header("Authorization", &auth);
                }
            }
            None => req = req.header("Cookie", "SOCS=CAI"),
        }
        let mut res = req.send_json(&payload).context("reaching YouTube Music")?;
        match res.status().as_u16() {
            200 => Ok(res
                .body_mut()
                .with_config()
                .limit(16 * 1024 * 1024)
                .read_json()?),
            401 | 403 => bail!("YouTube Music didn't accept the login; connect again in Settings"),
            status => bail!("YouTube Music replied {status}"),
        }
    }

    /// Songs matching the words, then videos (asked for at the same time), best first.
    pub fn search(&self, query: &str) -> Result<Vec<Track>> {
        let (songs, videos) = std::thread::scope(|s| {
            let videos = s.spawn(|| self.post("search", json!({"query": query, "params": VIDEOS})));
            let songs = self.post("search", json!({"query": query, "params": SONGS}));
            (songs, videos.join())
        });
        // Songs are what matters; videos only if they came.
        let videos = match videos {
            Ok(Ok(body)) => search_songs(&body),
            _ => Vec::new(),
        };
        Ok(best_first(query, search_songs(&songs?), videos))
    }

    /// One song or video by its id, for Add by link: it's first in its own "up next" list.
    pub fn song(&self, video_id: &str) -> Result<Track> {
        check_id(video_id)?;
        let body = self.post("next", json!({"videoId": video_id, "isAudioOnly": true}))?;
        next_song(&body, video_id).context("YouTube has nothing at that link")
    }

    /// "Connected as …".
    pub fn account_name(&mut self) -> Result<String> {
        let body = self.post("account/account_menu", json!({}))?;
        let header = at(
            &body,
            "actions.0.openPopupAction.popup.multiPageMenuRenderer.header.activeAccountHeaderRenderer",
        );
        at(header, "accountName.runs.0.text")
            .as_str()
            .map(str::to_string)
            .context("not signed in")
    }

    /// Your library's playlists, starting with Liked Music.
    pub fn playlists(&self) -> Result<Vec<Playlist>> {
        if !self.signed_in() {
            bail!("Connect YouTube Music in Settings to see your playlists");
        }
        let body = self.post("browse", json!({"browseId": "FEmusic_liked_playlists"}))?;
        let mut out = vec![Playlist {
            id: "VLLM".into(),
            name: "Liked Music".into(),
        }];
        let sections = at(
            &body,
            "contents.singleColumnBrowseResultsRenderer.tabs.0.tabRenderer.content.sectionListRenderer.contents",
        );
        for section in list(sections) {
            let grid = at(section, "itemSectionRenderer.contents.0.gridRenderer.items");
            for item in list(grid) {
                let item = &item["musicTwoRowItemRenderer"];
                let run = at(item, "title.runs.0");
                let id = at(run, "navigationEndpoint.browseEndpoint.browseId")
                    .as_str()
                    .unwrap_or("");
                if id.starts_with("VL") && id != "VLLM" {
                    out.push(Playlist {
                        id: id.to_string(),
                        name: run["text"].as_str().unwrap_or_default().to_string(),
                    });
                }
            }
        }
        Ok(out)
    }

    /// A playlist's songs (follows "load more" up to `limit` songs).
    pub fn playlist_tracks(&self, id: &str, limit: usize) -> Result<Vec<Track>> {
        let body = self.post("browse", json!({"browseId": id}))?;
        let shelf = at(
            &body,
            "contents.twoColumnBrowseResultsRenderer.secondaryContents.sectionListRenderer.contents.0.musicPlaylistShelfRenderer.contents",
        );
        let mut out = Vec::new();
        let mut items = list(shelf).cloned().collect::<Vec<_>>();
        loop {
            let mut next = None;
            for item in &items {
                if let Some(t) = playlist_track(&item["musicResponsiveListItemRenderer"]) {
                    out.push(t);
                }
                if let Some(token) = continuation(item) {
                    next = Some(token);
                }
            }
            let Some(token) = next.filter(|_| out.len() < limit) else {
                break;
            };
            let more = self.post("browse", json!({"continuation": token}))?;
            items = list(at(
                &more,
                "onResponseReceivedActions.0.appendContinuationItemsAction.continuationItems",
            ))
            .cloned()
            .collect();
        }
        out.truncate(limit);
        Ok(out)
    }
}

/// A YouTube Music playlist, for the Open list.
#[derive(Debug, Clone, PartialEq)]
pub struct Playlist {
    pub id: String,
    pub name: String,
}

/// Walks a dotted path through JSON: "a.0.b" is `v["a"][0]["b"]`. Missing parts give Null.
fn at<'a>(v: &'a Value, path: &str) -> &'a Value {
    path.split('.')
        .fold(v, |v, key| match key.parse::<usize>() {
            Ok(i) => v.get(i).unwrap_or(&Value::Null),
            Err(_) => v.get(key).unwrap_or(&Value::Null),
        })
}

fn list(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten()
}

fn text_of(column: &Value) -> Vec<&Value> {
    list(at(
        column,
        "musicResponsiveListItemFlexColumnRenderer.text.runs",
    ))
    .collect()
}

/// "3:38" or "1:02:05" → milliseconds.
fn duration_ms(s: &str) -> Option<i64> {
    let parts: Vec<i64> = s
        .split(':')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    Some(parts.iter().fold(0, |acc, p| acc * 60 + p) * 1000)
}

fn largest_thumbnail(v: &Value) -> String {
    list(at(
        v,
        "thumbnail.musicThumbnailRenderer.thumbnail.thumbnails",
    ))
    .filter_map(|t| t["url"].as_str())
    .last()
    .unwrap_or_default()
    .to_string()
}

/// Fills artist, album, year and duration from a row of runs like "Artist • Album • 3:38".
/// Runs are told apart by what they link to and look like, not by position.
fn read_runs(runs: &[&Value], t: &mut Track) {
    let mut artists = Vec::new();
    for run in runs {
        let text = run["text"].as_str().unwrap_or_default();
        let browse = at(run, "navigationEndpoint.browseEndpoint.browseId").as_str();
        match browse {
            Some(id) if id.starts_with("MPRE") => t.album = text.to_string(),
            Some(_) => artists.push(text),
            None if text.trim() == "•" || text.trim().is_empty() => {}
            None if duration_ms(text).is_some() => t.duration_ms = duration_ms(text).unwrap_or(0),
            None if text.len() == 4 && text.chars().all(|c| c.is_ascii_digit()) => {
                t.year = text.parse().ok()
            }
            None if matches!(text, "Song" | "Video" | "Album" | "Single" | "EP") => {}
            // A video's "653K views" or "14K likes", a song's "180K plays".
            None if [" views", " likes", " plays"]
                .iter()
                .any(|s| text.ends_with(s)) => {}
            None => artists.push(text),
        }
    }
    t.artist = artists.join(", ");
    t.album_artist = artists.first().map(|a| a.to_string()).unwrap_or_default();
}

/// Songs from a search reply.
fn search_songs(body: &Value) -> Vec<Track> {
    let sections = at(
        body,
        "contents.tabbedSearchResultsRenderer.tabs.0.tabRenderer.content.sectionListRenderer.contents",
    );
    let Some(shelf) = list(sections).find(|s| s.get("musicShelfRenderer").is_some()) else {
        return Vec::new();
    };
    list(&shelf["musicShelfRenderer"]["contents"])
        .filter_map(|item| {
            let it = &item["musicResponsiveListItemRenderer"];
            let columns: Vec<&Value> = list(&it["flexColumns"]).collect();
            let title = text_of(columns.first()?);
            let id = at(
                it,
                "overlay.musicItemThumbnailOverlayRenderer.content.musicPlayButtonRenderer.playNavigationEndpoint.watchEndpoint.videoId",
            )
            .as_str()
            .or(at(title.first()?, "navigationEndpoint.watchEndpoint.videoId").as_str())?;
            let mut t = Track {
                source: Source::YouTube,
                uri: id.to_string(),
                title: title.first()?["text"].as_str()?.to_string(),
                art_url: largest_thumbnail(it),
                ..Track::default()
            };
            if let Some(details) = columns.get(1) {
                read_runs(&text_of(details), &mut t);
            }
            Some(t)
        })
        .collect()
}

/// Songs, then the first few videos not among them. Those with every word typed in their
/// title or artist go first, so a video's exact title finds it above songs that only share a
/// word with it.
fn best_first(query: &str, songs: Vec<Track>, videos: Vec<Track>) -> Vec<Track> {
    let words = |s: &str| -> Vec<String> {
        s.split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect()
    };
    let wanted = words(query);
    let mut all = songs;
    for v in videos.into_iter().take(VIDEOS_SHOWN) {
        if !all.iter().any(|t| t.uri == v.uri) {
            all.push(v);
        }
    }
    let (mut full, rest): (Vec<Track>, Vec<Track>) = all.into_iter().partition(|t| {
        let have = words(&format!("{} {}", t.title, t.artist));
        wanted.iter().all(|w| have.contains(w))
    });
    full.extend(rest);
    full
}

/// The song `video_id` from an "up next" reply.
fn next_song(body: &Value, video_id: &str) -> Option<Track> {
    fn find<'a>(v: &'a Value, id: &str) -> Option<&'a Value> {
        match v {
            Value::Object(map) => map
                .get("playlistPanelVideoRenderer")
                .filter(|it| it["videoId"].as_str() == Some(id))
                .or_else(|| map.values().find_map(|x| find(x, id))),
            Value::Array(items) => items.iter().find_map(|x| find(x, id)),
            _ => None,
        }
    }
    let it = find(body, video_id)?;
    let mut t = Track {
        source: Source::YouTube,
        uri: video_id.to_string(),
        title: at(it, "title.runs.0.text").as_str()?.to_string(),
        art_url: list(at(it, "thumbnail.thumbnails"))
            .filter_map(|t| t["url"].as_str())
            .last()
            .unwrap_or_default()
            .to_string(),
        ..Track::default()
    };
    read_runs(
        &list(at(it, "longBylineText.runs")).collect::<Vec<_>>(),
        &mut t,
    );
    if let Some(ms) = at(it, "lengthText.runs.0.text")
        .as_str()
        .and_then(duration_ms)
    {
        t.duration_ms = ms;
    }
    Some(t)
}

/// A song row from a playlist page.
fn playlist_track(it: &Value) -> Option<Track> {
    let id = at(it, "playlistItemData.videoId").as_str()?;
    let columns: Vec<&Value> = list(&it["flexColumns"]).collect();
    let mut t = Track {
        source: Source::YouTube,
        uri: id.to_string(),
        title: text_of(columns.first()?).first()?["text"]
            .as_str()?
            .to_string(),
        art_url: largest_thumbnail(it),
        ..Track::default()
    };
    let mut runs: Vec<&Value> = Vec::new();
    for column in columns.iter().skip(1) {
        runs.extend(text_of(column));
    }
    read_runs(&runs, &mut t);
    let fixed = at(
        it,
        "fixedColumns.0.musicResponsiveListItemFixedColumnRenderer.text",
    );
    let length = fixed["simpleText"]
        .as_str()
        .or(at(fixed, "runs.0.text").as_str());
    if let Some(ms) = length.and_then(duration_ms) {
        t.duration_ms = ms;
    }
    Some(t)
}

/// The "load more" token at the end of a playlist page.
fn continuation(item: &Value) -> Option<String> {
    let endpoint = &item["continuationItemRenderer"]["continuationEndpoint"];
    if let Some(token) = at(endpoint, "continuationCommand.token").as_str() {
        return Some(token.to_string());
    }
    list(at(endpoint, "commandExecutorCommand.commands"))
        .find_map(|c| at(c, "continuationCommand.token").as_str())
        .map(str::to_string)
}

/// A song ready for mpv: a direct audio address, good for about six hours.
#[derive(Debug, Clone, PartialEq)]
pub struct Stream {
    pub url: String,
    pub user_agent: String,
    pub headers: Vec<(String, String)>,
    /// YouTube sometimes holds a stream back while an ad would play; not before this time
    /// (seconds since 1970).
    pub available_at: u64,
    pub codec: String,
    pub sample_rate: Option<u32>,
    pub bitrate: Option<u32>,
}

/// Asks yt-dlp for the best audio of a song, with your login when connected (Premium quality).
pub fn resolve(video_id: &str) -> Result<Stream> {
    check_id(video_id)?;
    let mut cmd = Command::new("yt-dlp");
    cmd.args(["--ignore-config", "-j", "-f", "bestaudio", "--no-warnings"]);
    if let Some(account) = account() {
        cmd.args(["--cookies-from-browser", &account.browser]);
    }
    let out = cmd
        .arg(format!("{ORIGIN}/watch?v={video_id}"))
        .stdin(Stdio::null())
        .output()
        .context("running yt-dlp")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err
            .lines()
            .rev()
            .find(|l| l.contains("ERROR"))
            .unwrap_or("unknown error");
        bail!(
            "YouTube wouldn't play this one: {}",
            last.trim_start_matches("ERROR: ")
        );
    }
    let v: Value = serde_json::from_slice(&out.stdout).context("reading yt-dlp's answer")?;
    stream_from(&v)
}

fn stream_from(v: &Value) -> Result<Stream> {
    let headers = v["http_headers"].as_object().cloned().unwrap_or_default();
    let header_list: Vec<(String, String)> = headers
        .iter()
        .filter(|(k, _)| *k != "User-Agent")
        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
        .collect();
    Ok(Stream {
        url: v["url"]
            .as_str()
            .context("yt-dlp gave no stream address")?
            .to_string(),
        user_agent: headers
            .get("User-Agent")
            .and_then(Value::as_str)
            .unwrap_or(USER_AGENT)
            .to_string(),
        headers: header_list,
        available_at: v["available_at"].as_u64().unwrap_or(0),
        codec: v["acodec"].as_str().unwrap_or_default().to_string(),
        sample_rate: v["asr"].as_u64().map(|r| r as u32),
        bitrate: v["abr"].as_f64().map(|b| b.round() as u32),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_songs_from_a_real_search_reply() {
        // A trimmed copy of a live, signed-out search for "black keys" (2026-10-13).
        let body: Value = serde_json::from_str(include_str!("test-search-songs.json")).unwrap();
        let songs = search_songs(&body);
        assert_eq!(songs.len(), 3);
        let first = &songs[0];
        assert_eq!(first.source, Source::YouTube);
        assert_eq!(first.uri.len(), 11, "a video id: {}", first.uri);
        assert_eq!(first.artist, "The Black Keys");
        assert_eq!(first.album, "No Rain, No Flowers");
        assert_eq!(first.duration_ms, 218_000);
        assert!(first.art_url.starts_with("https://"));
    }

    #[test]
    fn finds_videos_and_puts_an_exact_title_first() {
        // A trimmed copy of a live, signed-out video search for a cover that YouTube Music
        // only has as a video (2026-10-17).
        let body: Value = serde_json::from_str(include_str!("test-search-videos.json")).unwrap();
        let videos = search_songs(&body);
        let cover = &videos[0];
        assert_eq!(
            (
                cover.uri.as_str(),
                cover.title.as_str(),
                cover.artist.as_str()
            ),
            (
                "J3S6dppNOcU",
                "Lithium by Nirvana (Morgan James Cover)",
                "Morgan James"
            ),
            "\"653K views\" isn't an artist"
        );
        assert_eq!(cover.duration_ms, 256_000);
        let songs =
            search_songs(&serde_json::from_str(include_str!("test-search-songs.json")).unwrap());
        let first_song = songs[0].clone();
        let both = best_first(
            "Lithium by Nirvana (Morgan James Cover)",
            songs.clone(),
            videos.clone(),
        );
        assert_eq!(both[0].uri, "J3S6dppNOcU", "the exact title goes first");
        assert_eq!(both.len(), songs.len() + videos.len());
        // Songs keep their place when the words fit them as well as a video.
        let both = best_first("black keys", songs.clone(), videos.clone());
        assert_eq!(both[0], first_song);
        assert_eq!(
            both.last().map(|t| t.uri.as_str()),
            Some(videos[2].uri.as_str())
        );
        // A video that's also a song isn't listed twice.
        let again = best_first("x", songs.clone(), vec![first_song]);
        assert_eq!(again.len(), songs.len());
    }

    #[test]
    fn reads_a_song_from_its_up_next_list() {
        // A trimmed copy of a live, signed-out "next" reply for a video (2026-10-17).
        let body: Value = serde_json::from_str(include_str!("test-next.json")).unwrap();
        let t = next_song(&body, "J3S6dppNOcU").unwrap();
        assert_eq!(
            (t.title.as_str(), t.artist.as_str(), t.duration_ms),
            (
                "Lithium by Nirvana (Morgan James Cover)",
                "Morgan James",
                256_000
            )
        );
        assert_eq!(t.source, Source::YouTube);
        assert!(t.art_url.starts_with("https://i.ytimg.com/"));
        assert_eq!(
            next_song(&body, "AAAAAAAAAAA"),
            None,
            "only the song asked for"
        );
    }

    #[test]
    fn lists_each_chromium_profile() {
        let state = r#"{"profile":{"info_cache":{
            "Profile 1":{"name":"Personal","user_name":"me@example.com"},
            "Default":{"name":"Work","user_name":""}}}}"#;
        assert_eq!(
            chromium_profiles(state),
            [
                ("Default".to_string(), "Work".to_string()),
                (
                    "Profile 1".to_string(),
                    "Personal · me@example.com".to_string()
                ),
            ]
        );
        assert!(chromium_profiles("not json").is_empty());
    }

    /// Only YouTube's cookies are kept from an export of every site's.
    #[test]
    fn keeps_only_youtube_cookies() {
        let export = "# Netscape HTTP Cookie File\n\
            .youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\tyt\n\
            #HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t0\tLOGIN_INFO\tyt2\n\
            music.youtube.com\tFALSE\t/\tTRUE\t0\tPREF\tyt3\n\
            .bank.example\tTRUE\t/\tTRUE\t0\tsession\tsecret\n\
            #HttpOnly_.mail.example\tTRUE\t/\tTRUE\t0\tSID\tsecret\n\
            notyoutube.com\tTRUE\t/\tTRUE\t0\tx\tsecret\n\
            .google.com\tTRUE\t/\tTRUE\t0\tSID\tsecret\n";
        let kept = youtube_only(export);
        assert!(!kept.contains("secret"), "{kept}");
        for name in [
            "SAPISID",
            "LOGIN_INFO",
            "PREF",
            "# Netscape HTTP Cookie File",
        ] {
            assert!(kept.contains(name), "{name} kept");
        }
        assert_eq!(
            youtube_only(&kept),
            kept,
            "cutting down again changes nothing"
        );
        assert!(is_video_id("dQw4w9WgXcQ"));
        for bad in [
            "",
            "dQw4w9WgXc",
            "/../../tmp/x",
            "dQw4w9WgXc/",
            "dQw4w9WgXcQQ",
        ] {
            assert!(!is_video_id(bad), "{bad:?}");
        }
        assert!(resolve("../../etc/x").is_err());
        assert!(crate::sonos::server::prepare_youtube("-o/tmp/x", None).is_err());
    }

    #[test]
    fn cookie_files_and_the_login_header() {
        let file = "# Netscape HTTP Cookie File\n\
            .youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\tabc/def\n\
            #HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t0\tLOGIN_INFO\tlogin\n\
            .youtube.com\tTRUE\t/\tTRUE\t1\tOLD\texpired\n\
            .google.com\tTRUE\t/\tTRUE\t0\tSID\tnot-ours\n\
            broken line\n";
        let cookies = Cookies::parse(file);
        assert!(cookies.signed_in());
        assert_eq!(
            cookies.header(),
            "SAPISID=abc/def; LOGIN_INFO=login; SOCS=CAI"
        );
        // sha1("1700000000 abc/def https://music.youtube.com")
        let auth = cookies.authorization(1_700_000_000).unwrap();
        let expected =
            sha1_smol::Sha1::from("1700000000 abc/def https://music.youtube.com").digest();
        assert_eq!(auth, format!("SAPISIDHASH 1700000000_{expected}"));
        assert!(!Cookies::parse("").signed_in());
    }

    /// One live, signed-out search: `cargo test live_youtube -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_youtube_search() {
        let client = Client::new().unwrap();
        assert!(!client.version.is_empty());
        let songs = client.search("tighten up black keys").unwrap();
        for t in songs.iter().take(5) {
            println!(
                "{} | {} | {} | {} ms | {}",
                t.uri, t.title, t.artist, t.duration_ms, t.album
            );
        }
        assert!(songs.iter().any(|t| t.title.contains("Tighten Up")));
        // Only a video on YouTube Music.
        let songs = client
            .search("Lithium by Nirvana (Morgan James Cover)")
            .unwrap();
        println!(
            "{} | {} | {}",
            songs[0].uri, songs[0].title, songs[0].artist
        );
        assert_eq!(songs[0].uri, "J3S6dppNOcU");
        // Add by link.
        let t = client.song("J3S6dppNOcU").unwrap();
        println!("{} | {} | {} ms", t.title, t.artist, t.duration_ms);
        assert_eq!(t.artist, "Morgan James");
        assert!(client.song("AAAAAAAAAAA").is_err());
    }

    #[test]
    fn reads_durations_and_stream_details() {
        assert_eq!(duration_ms("3:38"), Some(218_000));
        assert_eq!(duration_ms("1:02:05"), Some(3_725_000));
        assert_eq!(duration_ms("2010"), None);
        let v = serde_json::json!({"url": "https://rr1.googlevideo.com/x", "acodec": "opus", "asr": 48000,
            "abr": 122.6, "available_at": 1791927356u64,
            "http_headers": {"User-Agent": "UA", "Accept": "*/*"}});
        let s = stream_from(&v).unwrap();
        assert_eq!(
            (s.user_agent.as_str(), s.codec.as_str(), s.bitrate),
            ("UA", "opus", Some(123))
        );
        assert_eq!(s.headers, vec![("Accept".to_string(), "*/*".to_string())]);
    }
}
