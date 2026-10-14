//! The sign-in screens for Spotify and YouTube Music, opened from Settings → Sources.
//! Each is a few steps; the slow parts (the browser sign-in, downloading Soloist, reading
//! browser cookies) run in the background and report back as `News`.

use crate::spotify::{self, soloist};
use crate::youtube::Login;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, PartialEq)]
pub enum SpotifyStep {
    /// Paste the Client ID from your Spotify app in the developer dashboard.
    ClientId {
        input: String,
        error: Option<String>,
    },
    /// The browser is showing Spotify's sign-in page; waiting for it to come back.
    Waiting {
        url: String,
    },
    /// Paste the Soloist API key, for playback on this computer.
    Playback {
        input: String,
        error: Option<String>,
    },
    /// Downloading and pairing Soloist; the latest news.
    SettingUp(String),
    Done(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum YouTubeStep {
    /// Pick the browser (or browser profile) you're signed in to YouTube with.
    Browser {
        list: Vec<Login>,
        sel: usize,
        error: Option<String>,
    },
    /// Reading that browser's login; what it's called.
    Connecting(String),
    Done(String),
}

/// Signing in to Last.fm.
#[derive(Debug, Clone, PartialEq)]
pub enum LastFmStep {
    /// Paste the API key and shared secret from your Last.fm API account. `field` is the one
    /// being typed in: 0 the key, 1 the secret.
    Keys {
        key: String,
        secret: String,
        field: usize,
        error: Option<String>,
    },
    /// Getting the page to allow OMAESTRO on (the key and secret are kept, in case it fails).
    Starting {
        key: String,
        secret: String,
    },
    /// Waiting while you allow OMAESTRO on Last.fm's page.
    Allowing {
        url: String,
        key: String,
        secret: String,
    },
    Done(String),
}

/// Signing in to ListenBrainz.
#[derive(Debug, Clone, PartialEq)]
pub enum ListenBrainzStep {
    Token {
        input: String,
        error: Option<String>,
    },
    /// Asking ListenBrainz about this token.
    Checking(String),
    Done(String),
}

/// A web address shown on a sign-in screen: click it to open it, or Copy it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Link {
    Dashboard,
    Redirect,
    SoloistKeys,
    /// The Spotify sign-in page itself, while OMAESTRO waits for it.
    SignIn,
    YouTube,
    /// OTIS, which the screens were designed in (the footer's credit).
    Otis,
    /// Where you make a Last.fm API account.
    LastFmCreate,
    /// Last.fm's page to allow OMAESTRO, while signing in.
    LastFmAllow,
    /// Your ListenBrainz settings, with the token.
    ListenBrainzSettings,
}

pub const DASHBOARD: &str = "https://developer.spotify.com/dashboard";
pub const YOUTUBE: &str = "https://music.youtube.com";
pub const OTIS: &str = "https://github.com/WolfBrandon/otis";

impl Link {
    /// The address, given the Spotify step showing (for the sign-in page).
    pub fn url(self, step: Option<&SpotifyStep>) -> Option<String> {
        Some(match self {
            Link::Dashboard => DASHBOARD.to_string(),
            Link::Redirect => spotify::REDIRECT_URI.to_string(),
            Link::SoloistKeys => soloist::KEY_PAGE.to_string(),
            Link::YouTube => YOUTUBE.to_string(),
            Link::Otis => OTIS.to_string(),
            Link::LastFmCreate => crate::integrations::LASTFM_CREATE.to_string(),
            Link::ListenBrainzSettings => crate::integrations::LISTENBRAINZ_SETTINGS.to_string(),
            // The app knows this one: it's on the Last.fm screen.
            Link::LastFmAllow => return None,
            Link::SignIn => match step {
                Some(SpotifyStep::Waiting { url }) => url.clone(),
                _ => return None,
            },
        })
    }
}

/// What a key did on a sign-in screen.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Stay,
    Close,
    /// Start the browser sign-in with this Client ID.
    SpotifyLogin(String),
    /// Save this Soloist key, then download and pair.
    SoloistKey(String),
    /// Read the login from this browser.
    YouTubeBrowser(Login),
    /// Sign in to Last.fm with this API key and secret.
    LastFmKeys {
        key: String,
        secret: String,
    },
    /// Sign in to ListenBrainz with this token.
    ListenBrainzToken(String),
}

impl LastFmStep {
    pub fn start() -> LastFmStep {
        LastFmStep::Keys {
            key: String::new(),
            secret: String::new(),
            field: 0,
            error: None,
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Outcome {
        match self {
            LastFmStep::Keys {
                key: api_key,
                secret,
                field,
                error,
            } => match key.code {
                KeyCode::Esc => Outcome::Close,
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                    *field = 1 - *field;
                    Outcome::Stay
                }
                // Enter on the key moves on to the secret; on the secret, with both filled
                // in, it signs in.
                KeyCode::Enter if *field == 0 => {
                    *field = 1;
                    Outcome::Stay
                }
                KeyCode::Enter if !api_key.trim().is_empty() && !secret.trim().is_empty() => {
                    *error = None;
                    Outcome::LastFmKeys {
                        key: api_key.trim().to_string(),
                        secret: secret.trim().to_string(),
                    }
                }
                _ => {
                    edit(if *field == 0 { api_key } else { secret }, key);
                    Outcome::Stay
                }
            },
            LastFmStep::Starting { .. } | LastFmStep::Allowing { .. } => match key.code {
                KeyCode::Esc => Outcome::Close,
                _ => Outcome::Stay,
            },
            LastFmStep::Done(_) => Outcome::Close,
        }
    }

    /// Pasted text goes into the field being typed in.
    pub fn paste(&mut self, text: &str) {
        if let LastFmStep::Keys {
            key, secret, field, ..
        } = self
        {
            if *field == 0 { key } else { secret }.push_str(text.trim());
        }
    }
}

impl ListenBrainzStep {
    pub fn start() -> ListenBrainzStep {
        ListenBrainzStep::Token {
            input: String::new(),
            error: None,
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Outcome {
        match self {
            ListenBrainzStep::Token { input, error } => match key.code {
                KeyCode::Esc => Outcome::Close,
                KeyCode::Enter if !input.trim().is_empty() => {
                    *error = None;
                    Outcome::ListenBrainzToken(input.trim().to_string())
                }
                _ => {
                    edit(input, key);
                    Outcome::Stay
                }
            },
            ListenBrainzStep::Checking(_) => match key.code {
                KeyCode::Esc => Outcome::Close,
                _ => Outcome::Stay,
            },
            ListenBrainzStep::Done(_) => Outcome::Close,
        }
    }

    pub fn paste(&mut self, text: &str) {
        if let ListenBrainzStep::Token { input, .. } = self {
            input.push_str(text.trim());
        }
    }
}

/// Typing into a field: letters, Backspace, Ctrl+U to clear. Returns true when it was one of
/// those.
pub fn edit(text: &mut String, key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Backspace => {
            text.pop();
        }
        KeyCode::Char('u') if ctrl => text.clear(),
        KeyCode::Char(c) if !ctrl => text.push(c),
        _ => return false,
    }
    true
}

impl SpotifyStep {
    pub fn on_key(&mut self, key: KeyEvent) -> Outcome {
        match self {
            SpotifyStep::ClientId { input, error } => match key.code {
                KeyCode::Esc => Outcome::Close,
                KeyCode::Enter if !input.trim().is_empty() => {
                    *error = None;
                    Outcome::SpotifyLogin(input.trim().to_string())
                }
                _ => {
                    edit(input, key);
                    Outcome::Stay
                }
            },
            SpotifyStep::Playback { input, error } => match key.code {
                KeyCode::Esc => Outcome::Close,
                KeyCode::Enter if !input.trim().is_empty() => {
                    *error = None;
                    Outcome::SoloistKey(input.trim().to_string())
                }
                _ => {
                    edit(input, key);
                    Outcome::Stay
                }
            },
            SpotifyStep::Waiting { .. } | SpotifyStep::SettingUp(_) => match key.code {
                KeyCode::Esc => Outcome::Close,
                _ => Outcome::Stay,
            },
            SpotifyStep::Done(_) => Outcome::Close,
        }
    }

    /// A pasted Client ID or key goes into whichever field is showing.
    pub fn paste(&mut self, text: &str) {
        if let SpotifyStep::ClientId { input, .. } | SpotifyStep::Playback { input, .. } = self {
            input.push_str(text.trim());
        }
    }

    /// Where to start: the browser sign-in, or straight to playback if that's all that's left.
    pub fn start(signed_in: bool) -> SpotifyStep {
        if signed_in {
            SpotifyStep::Playback {
                input: soloist::key().unwrap_or_default(),
                error: None,
            }
        } else {
            SpotifyStep::ClientId {
                input: String::new(),
                error: None,
            }
        }
    }
}

impl YouTubeStep {
    pub fn start(browsers: Vec<Login>) -> YouTubeStep {
        YouTubeStep::Browser {
            list: browsers,
            sel: 0,
            error: None,
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Outcome {
        match self {
            YouTubeStep::Browser { list, sel, .. } => match key.code {
                KeyCode::Esc => Outcome::Close,
                KeyCode::Up | KeyCode::Char('k') => {
                    *sel = sel.saturating_sub(1);
                    Outcome::Stay
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    *sel = (*sel + 1).min(list.len().saturating_sub(1));
                    Outcome::Stay
                }
                KeyCode::Enter => match list.get(*sel) {
                    Some(login) => Outcome::YouTubeBrowser(login.clone()),
                    None => Outcome::Stay,
                },
                _ => Outcome::Stay,
            },
            YouTubeStep::Connecting(_) => match key.code {
                KeyCode::Esc => Outcome::Close,
                _ => Outcome::Stay,
            },
            YouTubeStep::Done(_) => Outcome::Close,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spotify_steps_take_typed_or_pasted_ids() {
        let mut step = SpotifyStep::start(false);
        assert_eq!(
            step.on_key(KeyEvent::from(KeyCode::Enter)),
            Outcome::Stay,
            "nothing typed yet"
        );
        step.paste("  0123456789abcdef0123456789abcdef \n");
        assert_eq!(
            step.on_key(KeyEvent::from(KeyCode::Enter)),
            Outcome::SpotifyLogin("0123456789abcdef0123456789abcdef".into())
        );
        let mut step = SpotifyStep::Playback {
            input: String::new(),
            error: None,
        };
        for c in "key123".chars() {
            step.on_key(KeyEvent::from(KeyCode::Char(c)));
        }
        assert_eq!(
            step.on_key(KeyEvent::from(KeyCode::Enter)),
            Outcome::SoloistKey("key123".into())
        );
        assert_eq!(step.on_key(KeyEvent::from(KeyCode::Esc)), Outcome::Close);
    }

    #[test]
    fn last_fm_and_listenbrainz_steps() {
        let key = |code| KeyEvent::from(code);
        let mut step = LastFmStep::start();
        step.paste(" abc123 ");
        assert_eq!(
            step.on_key(key(KeyCode::Enter)),
            Outcome::Stay,
            "on to the secret"
        );
        step.paste("s3cret");
        assert_eq!(
            step.on_key(key(KeyCode::Enter)),
            Outcome::LastFmKeys {
                key: "abc123".into(),
                secret: "s3cret".into()
            }
        );
        // Tab goes back to the key; a missing secret doesn't sign in.
        let mut step = LastFmStep::start();
        step.paste("abc");
        step.on_key(key(KeyCode::Tab));
        assert_eq!(step.on_key(key(KeyCode::Enter)), Outcome::Stay);
        assert_eq!(step.on_key(key(KeyCode::Esc)), Outcome::Close);

        let mut step = ListenBrainzStep::start();
        step.paste(" tok-123 ");
        assert_eq!(
            step.on_key(key(KeyCode::Enter)),
            Outcome::ListenBrainzToken("tok-123".into())
        );
    }

    #[test]
    fn links_give_their_addresses() {
        assert_eq!(
            Link::Redirect.url(None).as_deref(),
            Some("http://127.0.0.1:8898/callback")
        );
        assert_eq!(Link::Dashboard.url(None).as_deref(), Some(DASHBOARD));
        assert_eq!(Link::Otis.url(None).as_deref(), Some(OTIS));
        assert_eq!(
            Link::SignIn.url(None),
            None,
            "only while waiting for the sign-in"
        );
        let waiting = SpotifyStep::Waiting {
            url: "https://accounts.spotify.com/authorize?x".into(),
        };
        assert_eq!(
            Link::SignIn.url(Some(&waiting)).as_deref(),
            Some("https://accounts.spotify.com/authorize?x")
        );
    }

    /// Lists this computer's browser logins by yt-dlp name and profile folder only:
    /// `cargo test live_logins -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_logins() {
        for login in crate::youtube::logins() {
            println!("{}", login.arg);
        }
    }

    #[test]
    fn youtube_picks_a_browser() {
        let login = |arg: &str| Login {
            arg: arg.into(),
            label: arg.into(),
        };
        let mut step = YouTubeStep::start(vec![login("chromium:Profile 1"), login("firefox")]);
        step.on_key(KeyEvent::from(KeyCode::Down));
        step.on_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(
            step.on_key(KeyEvent::from(KeyCode::Enter)),
            Outcome::YouTubeBrowser(login("firefox"))
        );
        assert_eq!(
            YouTubeStep::start(vec![]).on_key(KeyEvent::from(KeyCode::Enter)),
            Outcome::Stay
        );
    }
}
