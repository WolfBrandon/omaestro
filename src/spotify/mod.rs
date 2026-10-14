//! Spotify: signing in, and reading search results, playlists and your library from the
//! Web API. Playback goes through Spotify Soloist (`soloist.rs`) or a Sonos speaker.
//!
//! Signing in uses your own Spotify app's Client ID and the browser ("Authorization Code with
//! PKCE"): OMAESTRO opens Spotify's sign-in page, Spotify sends the browser back to
//! http://127.0.0.1:8898/callback, and OMAESTRO, listening there, swaps the code for tokens.
//! No password ever passes through OMAESTRO. The tokens are kept in
//! `~/.config/omaestro/spotify.json` (readable only by you). Spotify ends sign-ins after six
//! months; then you sign in again.

pub mod soloist;

use crate::library::{Source, Track};
use crate::secrets;
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const PORT: u16 = 8898;
pub const REDIRECT_URI: &str = "http://127.0.0.1:8898/callback";
const SCOPES: &str =
    "user-read-private playlist-read-private playlist-read-collaborative user-library-read";
const FILE: &str = "spotify.json";
const API: &str = "https://api.spotify.com/v1";
/// Spotify's limit for search results per type since February 2026.
pub const SEARCH_LIMIT: usize = 10;

/// What's kept between runs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Account {
    pub client_id: String,
    pub refresh_token: String,
    #[serde(default)]
    pub access_token: String,
    /// When the access token runs out, in seconds since 1970.
    #[serde(default)]
    pub expires_at: u64,
    /// Your Spotify display name, for "Connected as …".
    #[serde(default)]
    pub user: String,
}

/// A Spotify playlist, for the Open list.
#[derive(Debug, Clone, PartialEq)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub tracks: usize,
}

pub struct Spotify {
    account: Account,
    http: ureq::Agent,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .build()
        .into()
}

impl Spotify {
    /// The saved sign-in, if there is one.
    pub fn load() -> Option<Spotify> {
        secrets::load::<Account>(FILE).map(|account| Spotify {
            account,
            http: agent(),
        })
    }

    pub fn user(&self) -> &str {
        &self.account.user
    }

    pub fn sign_out() {
        secrets::forget(FILE);
    }

    /// A current access token, refreshing it when it's about to run out.
    fn token(&mut self) -> Result<String> {
        if !self.account.access_token.is_empty()
            && self.account.expires_at > secrets::now_secs() + 60
        {
            return Ok(self.account.access_token.clone());
        }
        let mut res = self
            .http
            .post("https://accounts.spotify.com/api/token")
            .send_form([
                ("grant_type", "refresh_token"),
                ("refresh_token", self.account.refresh_token.as_str()),
                ("client_id", self.account.client_id.as_str()),
            ])
            .context("reaching Spotify")?;
        let status = res.status().as_u16();
        let body: Value = res.body_mut().read_json().unwrap_or(Value::Null);
        if status != 200 {
            if body["error"] == "invalid_grant" {
                bail!(
                    "Spotify signed you out (sign-ins last six months): connect again in Settings"
                );
            }
            bail!(
                "Spotify refused to refresh the sign-in ({status}): {}",
                body["error"]
            );
        }
        self.account.access_token = body["access_token"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        self.account.expires_at = secrets::now_secs() + body["expires_in"].as_u64().unwrap_or(3600);
        // Spotify sometimes hands out a new refresh token; keep whichever is current.
        if let Some(refresh) = body["refresh_token"].as_str() {
            self.account.refresh_token = refresh.to_string();
        }
        secrets::save(FILE, &self.account)?;
        Ok(self.account.access_token.clone())
    }

    /// GET an API path such as "/me" or a full "next" URL from a paged reply.
    fn get(&mut self, path: &str) -> Result<Value> {
        let url = if path.starts_with("https://") {
            path.to_string()
        } else {
            format!("{API}{path}")
        };
        for attempt in 0..2 {
            let token = self.token()?;
            let mut res = self
                .http
                .get(&url)
                .header("Authorization", &format!("Bearer {token}"))
                .call()
                .context("reaching Spotify")?;
            match res.status().as_u16() {
                200 => return Ok(res.body_mut().read_json()?),
                401 if attempt == 0 => self.account.expires_at = 0,
                403 => bail!(
                    "Spotify said no (403). Is your account on the app's user list in the Spotify \
                     dashboard?"
                ),
                429 => bail!("Spotify asks to slow down; try again in a minute"),
                status => bail!("Spotify replied {status}"),
            }
        }
        bail!("Spotify didn't accept the sign-in")
    }

    /// Whether Spotify still accepts the sign-in (a look at your profile).
    pub fn check(&mut self) -> Result<()> {
        self.get("/me").map(|_| ())
    }

    /// Songs matching the words, best first (Spotify returns at most 10).
    pub fn search(&mut self, query: &str) -> Result<Vec<Track>> {
        let q = form_encode(query);
        let body = self.get(&format!("/search?type=track&limit={SEARCH_LIMIT}&q={q}"))?;
        Ok(items(&body["tracks"]["items"]).filter_map(track).collect())
    }

    /// Your playlists, the ones you made and the ones you follow.
    pub fn playlists(&mut self) -> Result<Vec<Playlist>> {
        let mut out = Vec::new();
        let mut next = Some("/me/playlists?limit=50".to_string());
        while let Some(page) = next.take() {
            let body = self.get(&page)?;
            out.extend(items(&body["items"]).map(|p| {
                Playlist {
                    id: p["id"].as_str().unwrap_or_default().to_string(),
                    name: p["name"].as_str().unwrap_or_default().to_string(),
                    tracks: p["items"]["total"]
                        .as_u64()
                        .or(p["tracks"]["total"].as_u64())
                        .unwrap_or(0) as usize,
                }
            }));
            next = body["next"].as_str().map(str::to_string);
        }
        Ok(out)
    }

    /// A playlist's songs. Spotify only shows the songs of playlists you own or collaborate on.
    pub fn playlist_tracks(&mut self, id: &str) -> Result<Vec<Track>> {
        let mut out = Vec::new();
        let mut next = Some(format!("/playlists/{id}/items?limit=50"));
        while let Some(page) = next.take() {
            let body = self.get(&page)?;
            out.extend(
                items(&body["items"])
                    .filter_map(|entry| track(entry.get("item").or(entry.get("track"))?)),
            );
            next = body["next"].as_str().map(str::to_string);
        }
        Ok(out)
    }

    /// Your Liked Songs, newest first (up to `limit`).
    pub fn liked(&mut self, limit: usize) -> Result<Vec<Track>> {
        let mut out = Vec::new();
        let mut next = Some("/me/tracks?limit=50".to_string());
        while let Some(page) = next.take() {
            let body = self.get(&page)?;
            out.extend(items(&body["items"]).filter_map(|entry| track(&entry["track"])));
            next = body["next"]
                .as_str()
                .filter(|_| out.len() < limit)
                .map(str::to_string);
        }
        out.truncate(limit);
        Ok(out)
    }
}

fn items(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten()
}

/// A Spotify track object as a queue entry.
pub fn track(v: &Value) -> Option<Track> {
    let uri = v["uri"].as_str()?;
    if !uri.starts_with("spotify:track:") || v["is_local"].as_bool() == Some(true) {
        return None;
    }
    let artists: Vec<&str> = items(&v["artists"])
        .filter_map(|a| a["name"].as_str())
        .collect();
    let album = &v["album"];
    let album_artist = items(&album["artists"])
        .find_map(|a| a["name"].as_str())
        .or(artists.first().copied())
        .unwrap_or_default();
    // Spotify lists covers largest first; a mid-sized one is plenty for the terminal.
    let images: Vec<&str> = items(&album["images"])
        .filter_map(|i| i["url"].as_str())
        .collect();
    Some(Track {
        source: Source::Spotify,
        uri: uri.to_string(),
        title: v["name"].as_str().unwrap_or_default().to_string(),
        artist: artists.join(", "),
        album: album["name"].as_str().unwrap_or_default().to_string(),
        album_artist: album_artist.to_string(),
        year: album["release_date"]
            .as_str()
            .and_then(|d| d.get(..4))
            .and_then(|y| y.parse().ok()),
        disc_no: v["disc_number"].as_u64().map(|n| n as u32),
        track_no: v["track_number"].as_u64().map(|n| n as u32),
        duration_ms: v["duration_ms"].as_i64().unwrap_or(0),
        isrc: v["external_ids"]["isrc"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        art_url: images
            .get(1)
            .or(images.first())
            .map(|s| s.to_string())
            .unwrap_or_default(),
        ..Track::default()
    })
}

/// Percent-encoding for form fields and query strings.
pub fn form_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// What the sign-in page needs: the code verifier we keep and the challenge we send.
fn pkce_pair() -> (String, String) {
    let verifier = secrets::random_string(
        64,
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~",
    );
    (verifier.clone(), challenge(&verifier))
}

fn challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn authorize_url(client_id: &str, challenge: &str, state: &str) -> String {
    format!(
        "https://accounts.spotify.com/authorize?response_type=code&client_id={}&redirect_uri={}\
         &scope={}&state={state}&code_challenge_method=S256&code_challenge={challenge}",
        form_encode(client_id),
        form_encode(REDIRECT_URI),
        form_encode(SCOPES)
    )
}

/// A sign-in in progress. Dropping it, or `cancel`, stops listening.
pub struct Login {
    pub url: String,
    stop: Arc<AtomicBool>,
}

impl Login {
    pub fn cancel(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for Login {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Starts signing in: listens on 127.0.0.1:8898 and returns the page to open in the browser.
/// `done` is called once, from a background thread, with the result.
pub fn login(
    client_id: &str,
    done: impl FnOnce(Result<String, String>) + Send + 'static,
) -> Result<Login> {
    let client_id = client_id.trim().to_string();
    if client_id.len() < 16 || !client_id.chars().all(|c| c.is_ascii_alphanumeric()) {
        bail!(
            "That doesn't look like a Client ID (32 letters and digits from the Spotify dashboard)"
        );
    }
    let listener = TcpListener::bind(("127.0.0.1", PORT))
        .with_context(|| format!("port {PORT} is busy, so Spotify can't send the sign-in back"))?;
    listener.set_nonblocking(true)?;
    let (verifier, challenge) = pkce_pair();
    let state = secrets::random_string(24, b"abcdefghijklmnopqrstuvwxyz0123456789");
    let url = authorize_url(&client_id, &challenge, &state);
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    std::thread::spawn(move || {
        let result = wait_for_code(&listener, &state, &stopped)
            .and_then(|code| finish(&client_id, &code, &verifier))
            .map_err(|e| format!("{e:#}"));
        done(result);
    });
    Ok(Login { url, stop })
}

/// Waits up to 10 minutes for the browser to come back with `?code=…`.
fn wait_for_code(listener: &TcpListener, state: &str, stop: &AtomicBool) -> Result<String> {
    let deadline = Instant::now() + Duration::from_secs(600);
    while Instant::now() < deadline {
        if stop.load(Ordering::Relaxed) {
            bail!("Sign-in cancelled");
        }
        let (mut stream, _) = match listener.accept() {
            Ok(conn) => conn,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line)?;
        // "GET /callback?code=…&state=… HTTP/1.1"
        let target = line.split_whitespace().nth(1).unwrap_or_default();
        let Some(query) = target.strip_prefix("/callback?") else {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
            continue;
        };
        let param = |key: &str| {
            query.split('&').find_map(|kv| {
                let (k, v) = kv.split_once('=')?;
                (k == key).then(|| v.to_string())
            })
        };
        let (ok, message) = match (param("code"), param("state"), param("error")) {
            (Some(code), Some(s), _) if s == state => {
                (Some(code), "OMAESTRO is connected to Spotify.")
            }
            (_, _, Some(_)) => (None, "Spotify sign-in was cancelled."),
            _ => (
                None,
                "That sign-in didn't match. Please start again from OMAESTRO.",
            ),
        };
        let page = format!(
            "<!doctype html><meta charset=utf-8><title>OMAESTRO</title>\
             <body style=\"font:16px sans-serif;background:#050a06;color:#a1dfb4;padding:3em\">\
             <h2>{message}</h2><p>You can close this tab and go back to OMAESTRO.</p>"
        );
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{page}",
            page.len()
        );
        return ok.ok_or_else(|| anyhow!("{message}"));
    }
    bail!("Sign-in timed out after 10 minutes")
}

/// Swaps the code for tokens, reads your name, and saves the sign-in. Returns the name.
fn finish(client_id: &str, code: &str, verifier: &str) -> Result<String> {
    let http = agent();
    let mut res = http
        .post("https://accounts.spotify.com/api/token")
        .send_form([
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT_URI),
            ("client_id", client_id),
            ("code_verifier", verifier),
        ])
        .context("reaching Spotify")?;
    let status = res.status().as_u16();
    let body: Value = res.body_mut().read_json().unwrap_or(Value::Null);
    if status != 200 {
        bail!(
            "Spotify didn't accept the sign-in ({status} {}). Check the Redirect URI in the \
             dashboard is exactly {REDIRECT_URI}",
            body["error_description"]
                .as_str()
                .or(body["error"].as_str())
                .unwrap_or("")
        );
    }
    let mut spotify = Spotify {
        account: Account {
            client_id: client_id.to_string(),
            refresh_token: body["refresh_token"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            access_token: body["access_token"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            expires_at: secrets::now_secs() + body["expires_in"].as_u64().unwrap_or(3600),
            user: String::new(),
        },
        http,
    };
    let me = spotify.get("/me")?;
    spotify.account.user = me["display_name"]
        .as_str()
        .or(me["id"].as_str())
        .unwrap_or("Spotify user")
        .to_string();
    secrets::save(FILE, &spotify.account)?;
    Ok(spotify.account.user)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pkce_challenge_matches_the_spec_example() {
        // RFC 7636, appendix B.
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let (verifier, ch) = pkce_pair();
        assert_eq!(verifier.len(), 64);
        assert_eq!(ch, challenge(&verifier));
    }

    #[test]
    fn the_sign_in_page_asks_for_read_access_and_comes_back_to_us() {
        let url = authorize_url("abc123", "CH", "st");
        assert!(url.starts_with("https://accounts.spotify.com/authorize?response_type=code"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A8898%2Fcallback"));
        assert!(url.contains("scope=user-read-private%20playlist-read-private"));
        assert!(url.contains("code_challenge=CH") && url.contains("state=st"));
    }

    #[test]
    fn reads_a_track_object() {
        let v = json!({
            "uri": "spotify:track:5Z01UMMf7V1o0MzF86s6WJ", "name": "Tighten Up",
            "duration_ms": 211000, "track_number": 2, "disc_number": 1,
            "artists": [{"name": "The Black Keys"}],
            "external_ids": {"isrc": "USNO11000135"},
            "album": {"name": "Brothers", "release_date": "2010-05-18",
                      "artists": [{"name": "The Black Keys"}],
                      "images": [{"url": "https://i.scdn.co/640"}, {"url": "https://i.scdn.co/300"}]}
        });
        let t = track(&v).unwrap();
        assert_eq!(t.source, Source::Spotify);
        assert_eq!(
            (t.title.as_str(), t.artist.as_str(), t.album.as_str()),
            ("Tighten Up", "The Black Keys", "Brothers")
        );
        assert_eq!(
            (t.year, t.track_no, t.duration_ms),
            (Some(2010), Some(2), 211000)
        );
        assert_eq!(t.isrc, "USNO11000135");
        assert_eq!(t.art_url, "https://i.scdn.co/300");
        assert!(track(&json!({"uri": "spotify:local:x", "is_local": true})).is_none());
    }

    #[test]
    fn a_bad_client_id_is_refused_before_listening() {
        assert!(login("not an id", |_| {}).is_err());
    }
}
