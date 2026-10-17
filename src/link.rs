//! Add by link: a Spotify or YouTube song's web address, pasted or typed, read into where it's
//! from and its id there. Looking the song up is the services' job (`spotify`, `youtube`).

use crate::library::Source;

/// A song's address, taken apart.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub source: Source,
    /// A Spotify track id, or a YouTube video id.
    pub id: String,
}

/// The Add by link box.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AddLink {
    pub input: String,
    pub error: Option<String>,
    /// Looking the song up.
    pub busy: bool,
}

const NOT_A_SONG: &str = "That isn't a link to a song on Spotify or YouTube";

/// Reads a song's link: open.spotify.com/track/…, spotify:track:…, youtube.com/watch?v=…
/// (music.youtube.com too), youtu.be/…, youtube.com/shorts/…, or a bare video id.
pub fn parse(text: &str) -> Result<Link, String> {
    let text = text.trim();
    if let Some(id) = text.strip_prefix("spotify:track:") {
        return spotify(id);
    }
    if crate::youtube::is_video_id(text) {
        return youtube(text);
    }
    let rest = ["https://", "http://"]
        .iter()
        .find_map(|scheme| {
            let head = text.get(..scheme.len())?;
            head.eq_ignore_ascii_case(scheme)
                .then(|| &text[scheme.len()..])
        })
        .unwrap_or(text);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.to_lowercase();
    let host = host
        .strip_prefix("www.")
        .or_else(|| host.strip_prefix("m."))
        .unwrap_or(&host);
    let path = path.split('#').next().unwrap_or_default();
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    match host {
        "open.spotify.com" | "play.spotify.com" => {
            // Links from another country's site start with "intl-de/" and the like.
            let parts: Vec<&str> = parts
                .into_iter()
                .filter(|p| !p.starts_with("intl-"))
                .collect();
            match parts.as_slice() {
                ["track", id, ..] => spotify(id),
                [kind @ ("album" | "playlist" | "artist"), ..] => Err(format!(
                    "That's a link to a Spotify {kind}: paste a link to one of its songs"
                )),
                _ => Err(NOT_A_SONG.into()),
            }
        }
        "spotify.link" => Err(
            "Short spotify.link links can't be read here: open it in the browser and copy the \
             address it goes to"
                .into(),
        ),
        "youtube.com" | "music.youtube.com" => match parts.as_slice() {
            ["watch"] => {
                let v = query
                    .split('&')
                    .find_map(|pair| pair.strip_prefix("v="))
                    .unwrap_or_default();
                youtube(v)
            }
            ["shorts" | "embed" | "live" | "v", id, ..] => youtube(id),
            ["playlist"] => {
                Err("That's a link to a YouTube playlist: paste a link to one of its songs".into())
            }
            _ => Err(NOT_A_SONG.into()),
        },
        "youtu.be" => match parts.as_slice() {
            [id, ..] => youtube(id),
            _ => Err(NOT_A_SONG.into()),
        },
        _ => Err(NOT_A_SONG.into()),
    }
}

/// A Spotify id is 22 letters and digits.
fn spotify(id: &str) -> Result<Link, String> {
    if id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Ok(Link {
            source: Source::Spotify,
            id: id.to_string(),
        })
    } else {
        Err("That Spotify link looks cut short".into())
    }
}

fn youtube(id: &str) -> Result<Link, String> {
    if crate::youtube::is_video_id(id) {
        Ok(Link {
            source: Source::YouTube,
            id: id.to_string(),
        })
    } else {
        Err("That YouTube link looks cut short".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_song_links_from_spotify_and_youtube() {
        let spotify = |id: &str| {
            Ok(Link {
                source: Source::Spotify,
                id: id.into(),
            })
        };
        let youtube = |id: &str| {
            Ok(Link {
                source: Source::YouTube,
                id: id.into(),
            })
        };
        let track = "4uLU6hMCjMI75M1A2tKUQC";
        for text in [
            "https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC?si=abc123",
            "open.spotify.com/intl-de/track/4uLU6hMCjMI75M1A2tKUQC",
            "  spotify:track:4uLU6hMCjMI75M1A2tKUQC\n",
        ] {
            assert_eq!(parse(text), spotify(track), "{text}");
        }
        let video = "J3S6dppNOcU";
        for text in [
            "https://www.youtube.com/watch?v=J3S6dppNOcU",
            "https://music.youtube.com/watch?v=J3S6dppNOcU&list=RDAMVM&si=x",
            "https://youtube.com/watch?feature=share&v=J3S6dppNOcU#t=30",
            "https://m.youtube.com/watch?v=J3S6dppNOcU",
            "https://youtu.be/J3S6dppNOcU?si=abc",
            "https://www.youtube.com/shorts/J3S6dppNOcU",
            "HTTPS://WWW.YOUTUBE.COM/watch?v=J3S6dppNOcU",
            "J3S6dppNOcU",
        ] {
            assert_eq!(parse(text), youtube(video), "{text}");
        }
    }

    #[test]
    fn other_links_say_what_they_are() {
        let err = |text: &str| parse(text).unwrap_err();
        assert!(err("https://open.spotify.com/album/1DFixLWuPkv3KT3TnV35m3").contains("album"));
        assert!(
            err("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M").contains("playlist")
        );
        assert!(err("https://www.youtube.com/playlist?list=PL123").contains("playlist"));
        assert!(err("https://spotify.link/abc").contains("spotify.link"));
        assert!(err("https://open.spotify.com/track/short").contains("cut short"));
        assert!(err("https://www.youtube.com/watch?v=abc").contains("cut short"));
        assert_eq!(err("https://example.com/watch?v=J3S6dppNOcU"), NOT_A_SONG);
        assert_eq!(err("lithium nirvana"), NOT_A_SONG);
        assert_eq!(err(""), NOT_A_SONG);
    }
}
