//! Last.fm and ListenBrainz: the accounts your plays are sent to (Settings → Integrations).
//! This is signing in; sending plays (scrobbling) builds on it.
//!
//! - **Last.fm** uses its desktop sign-in: your own free API key and secret, then you allow
//!   OMAESTRO on Last.fm's site, and it hands over a session key. Every request is signed with
//!   an MD5 of its parameters and the secret.
//! - **ListenBrainz** needs only the token from your profile page.
//!
//! Both are kept in `~/.config/omaestro`, readable only by you.

use crate::secrets;
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const LASTFM_API: &str = "https://ws.audioscrobbler.com/2.0/";
/// Where you make the free API account Last.fm sign-in needs.
pub const LASTFM_CREATE: &str = "https://www.last.fm/api/account/create";
/// Where your ListenBrainz token is.
pub const LISTENBRAINZ_SETTINGS: &str = "https://listenbrainz.org/settings/";
pub const LISTENBRAINZ_API: &str = "https://api.listenbrainz.org/1";

const LASTFM_FILE: &str = "lastfm.json";
const LISTENBRAINZ_FILE: &str = "listenbrainz.json";

/// The services plays can be sent to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    LastFm,
    ListenBrainz,
}

impl Service {
    pub fn name(self) -> &'static str {
        match self {
            Service::LastFm => "Last.fm",
            Service::ListenBrainz => "ListenBrainz",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LastFm {
    pub api_key: String,
    pub secret: String,
    pub session_key: String,
    pub user: String,
}

impl LastFm {
    pub fn load() -> Option<LastFm> {
        secrets::load(LASTFM_FILE)
    }

    pub fn sign_out() {
        secrets::forget(LASTFM_FILE);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListenBrainz {
    pub token: String,
    pub user: String,
}

impl ListenBrainz {
    pub fn load() -> Option<ListenBrainz> {
        secrets::load(LISTENBRAINZ_FILE)
    }

    pub fn sign_out() {
        secrets::forget(LISTENBRAINZ_FILE);
    }
}

/// An agent that hands back every answer, errors included, so a service's reason can be read.
pub(crate) fn http() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .build()
        .into()
}

// ---- Last.fm ---------------------------------------------------------------------------

/// Last.fm's signature for a request: the parameters sorted by name, each name followed by
/// its value, then the secret, as an MD5 in hex. `format` and `callback` don't count.
pub fn api_sig(params: &[(&str, &str)], secret: &str) -> String {
    let mut sorted: Vec<&(&str, &str)> = params
        .iter()
        .filter(|(k, _)| *k != "format" && *k != "callback")
        .collect();
    sorted.sort_by_key(|(k, _)| *k);
    let mut text: String = sorted.iter().map(|(k, v)| format!("{k}{v}")).collect();
    text.push_str(secret);
    Md5::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A signed Last.fm request, as GET (for reading) or POST (for changes). Returns Last.fm's
/// answer, which may be an error it explains.
pub fn lastfm_call(params: &[(&str, &str)], secret: &str, post: bool) -> Result<Value, String> {
    lastfm_call_at(LASTFM_API, params, secret, post)
}

/// `lastfm_call`, to `api` (the tests' stand-in for Last.fm).
pub fn lastfm_call_at(
    api: &str,
    params: &[(&str, &str)],
    secret: &str,
    post: bool,
) -> Result<Value, String> {
    let sig = api_sig(params, secret);
    let mut all: Vec<(&str, &str)> = params.to_vec();
    all.push(("api_sig", &sig));
    all.push(("format", "json"));
    let http = http();
    let reply = if post {
        http.post(api).send_form(all.iter().copied())
    } else {
        let mut req = http.get(api);
        for (k, v) in &all {
            req = req.query(*k, *v);
        }
        req.call()
    };
    let mut res = reply.map_err(|e| format!("reaching Last.fm: {e}"))?;
    res.body_mut()
        .read_json::<Value>()
        .map_err(|e| format!("reading Last.fm's answer: {e}"))
}

/// Last.fm's own error, in words that say what to do.
fn lastfm_error(body: &Value) -> Option<String> {
    let code = body["error"].as_u64()?;
    Some(match code {
        10 => "Last.fm doesn't know that API key: check it on your API account page".into(),
        13 => "That shared secret doesn't go with the API key".into(),
        14 => "Not allowed on Last.fm yet".into(),
        15 => "Last.fm's sign-in link expired: start again".into(),
        26 => "Last.fm has suspended that API key".into(),
        _ => format!(
            "Last.fm said: {}",
            body["message"].as_str().unwrap_or("something went wrong")
        ),
    })
}

/// Step one: a token for you to allow on Last.fm's site.
pub fn lastfm_token(api_key: &str, secret: &str) -> Result<String, String> {
    let body = lastfm_call(
        &[("method", "auth.getToken"), ("api_key", api_key)],
        secret,
        false,
    )?;
    if let Some(e) = lastfm_error(&body) {
        return Err(e);
    }
    body["token"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "Last.fm gave no token".into())
}

/// The page where you allow OMAESTRO.
pub fn lastfm_allow_url(api_key: &str, token: &str) -> String {
    format!("https://www.last.fm/api/auth/?api_key={api_key}&token={token}")
}

/// What `auth.getSession` said.
#[derive(Debug, PartialEq)]
pub enum Session {
    /// Not allowed yet: keep asking.
    Waiting,
    /// Allowed: the session key and your user name.
    Ready {
        key: String,
        user: String,
    },
    Failed(String),
}

pub fn session_from(body: &Value) -> Session {
    match body["error"].as_u64() {
        Some(14) => Session::Waiting,
        Some(_) => Session::Failed(lastfm_error(body).unwrap_or_default()),
        None => match (
            body["session"]["key"].as_str(),
            body["session"]["name"].as_str(),
        ) {
            (Some(key), Some(user)) => Session::Ready {
                key: key.to_string(),
                user: user.to_string(),
            },
            _ => Session::Failed("Last.fm's answer had no session in it".into()),
        },
    }
}

/// Signs in to Last.fm: gets a token, hands the page to allow it on to `allow` (which opens
/// it), then asks every few seconds until you've allowed it, for up to ten minutes, or until
/// `cancel` is set. Saves the account.
pub fn lastfm_sign_in(
    api_key: &str,
    secret: &str,
    allow: impl FnOnce(String),
    cancel: Arc<AtomicBool>,
) -> Result<LastFm, String> {
    let token = lastfm_token(api_key, secret)?;
    allow(lastfm_allow_url(api_key, &token));
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(600) {
        std::thread::sleep(Duration::from_secs(3));
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let body = lastfm_call(
            &[
                ("method", "auth.getSession"),
                ("api_key", api_key),
                ("token", &token),
            ],
            secret,
            false,
        );
        match body.map(|b| session_from(&b)) {
            Ok(Session::Waiting) | Err(_) => continue,
            Ok(Session::Failed(e)) => return Err(e),
            Ok(Session::Ready { key, user }) => {
                let account = LastFm {
                    api_key: api_key.to_string(),
                    secret: secret.to_string(),
                    session_key: key,
                    user,
                };
                secrets::save(LASTFM_FILE, &account).map_err(|e| format!("{e:#}"))?;
                return Ok(account);
            }
        }
    }
    Err("No word from Last.fm in ten minutes: start again".into())
}

// ---- ListenBrainz ----------------------------------------------------------------------

/// Checks a ListenBrainz token; the user name it belongs to.
pub fn listenbrainz_check(token: &str) -> Result<String, String> {
    let mut res = http()
        .get(&format!("{LISTENBRAINZ_API}/validate-token"))
        .header("Authorization", &format!("Token {token}"))
        .call()
        .map_err(|e| format!("reaching ListenBrainz: {e}"))?;
    let body: Value = res
        .body_mut()
        .read_json()
        .map_err(|e| format!("reading ListenBrainz's answer: {e}"))?;
    validated(&body)
}

fn validated(body: &Value) -> Result<String, String> {
    match (body["valid"].as_bool(), body["user_name"].as_str()) {
        (Some(true), Some(user)) => Ok(user.to_string()),
        _ => Err("ListenBrainz doesn't know that token: copy it again from your settings".into()),
    }
}

/// Signs in to ListenBrainz with a token, saving the account.
pub fn listenbrainz_sign_in(token: &str) -> Result<ListenBrainz, String> {
    let user = listenbrainz_check(token)?;
    let account = ListenBrainz {
        token: token.to_string(),
        user,
    };
    secrets::save(LISTENBRAINZ_FILE, &account).map_err(|e| format!("{e:#}"))?;
    Ok(account)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn signs_requests_the_way_last_fm_checks_them() {
        // Worked out separately: md5("api_keyabc123methodauth.getSessiontokentok456secret789").
        let params = [
            ("method", "auth.getSession"),
            ("api_key", "abc123"),
            ("token", "tok456"),
            ("format", "json"),
        ];
        assert_eq!(
            api_sig(&params, "secret789"),
            "be87f72a198902bffdc0c6d3783ee5b0"
        );
    }

    #[test]
    fn reads_last_fm_and_listenbrainz_answers() {
        assert_eq!(
            session_from(&json!({"error": 14, "message": "Unauthorized Token"})),
            Session::Waiting
        );
        assert_eq!(
            session_from(&json!({"session": {"name": "brandon", "key": "sk", "subscriber": 0}})),
            Session::Ready {
                key: "sk".into(),
                user: "brandon".into()
            }
        );
        assert!(matches!(
            session_from(&json!({"error": 15, "message": "expired"})),
            Session::Failed(e) if e.contains("expired")
        ));
        assert!(
            lastfm_error(&json!({"error": 10}))
                .unwrap()
                .contains("API key")
        );
        assert_eq!(lastfm_error(&json!({"token": "t"})), None);
        assert!(
            lastfm_allow_url("k", "t").ends_with("/api/auth/?api_key=k&token=t"),
            "the page you allow OMAESTRO on"
        );

        assert_eq!(
            validated(&json!({"code": 200, "valid": true, "user_name": "brandon"})),
            Ok("brandon".into())
        );
        assert!(validated(&json!({"code": 200, "valid": false})).is_err());
    }
}
