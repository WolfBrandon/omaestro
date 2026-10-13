//! The sign-in screens for Spotify and YouTube Music, opened from Settings → Sources.
//! Each is a few steps; the slow parts (the browser sign-in, downloading Soloist, reading
//! browser cookies) run in the background and report back as `News`.

use crate::spotify::soloist;
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
    /// Pick the browser you're signed in to YouTube with.
    Browser {
        list: Vec<&'static str>,
        sel: usize,
        error: Option<String>,
    },
    Connecting(&'static str),
    Done(String),
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
    YouTubeBrowser(&'static str),
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
    pub fn start(browsers: Vec<&'static str>) -> YouTubeStep {
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
                    Some(browser) => Outcome::YouTubeBrowser(browser),
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
    fn youtube_picks_a_browser() {
        let mut step = YouTubeStep::start(vec!["chromium", "firefox"]);
        step.on_key(KeyEvent::from(KeyCode::Down));
        step.on_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(
            step.on_key(KeyEvent::from(KeyCode::Enter)),
            Outcome::YouTubeBrowser("firefox")
        );
        assert_eq!(
            YouTubeStep::start(vec![]).on_key(KeyEvent::from(KeyCode::Enter)),
            Outcome::Stay
        );
    }
}
