//! Plays on a Sonos group. Commands run on their own thread, so a slow speaker or a file
//! being converted never freezes the screen; that thread also asks the speaker where it is
//! about once a second while it plays, and notices when a song ends.
//!
//! Each song goes to the group as one link:
//! - local files through OMAESTRO's little web server (converted first if above 48 kHz);
//! - YouTube Music songs downloaded as AAC first, then served the same way;
//! - Spotify songs as a Spotify link the speaker plays itself, with the account linked in the
//!   Sonos app.

use crate::library::{Source, Track};
use crate::sonos::{self, Group, Sonos, server};
use anyhow::{Context, Result, bail};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Command {
    /// Play this song on the group, starting `start` seconds in. `ticket` tells this song's
    /// news apart from older songs'.
    Play {
        ticket: u64,
        track: Track,
        group: Group,
        start: f64,
    },
    Pause,
    Resume,
    Stop,
    Seek(f64),
    Volume(u8),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Getting a song ready (converting, downloading) before it plays.
    Preparing {
        ticket: u64,
        what: String,
    },
    /// Where the group is: "PLAYING", "PAUSED_PLAYBACK", "STOPPED", "TRANSITIONING".
    Status {
        ticket: u64,
        state: String,
        position: f64,
        duration: f64,
    },
    /// The song played to its end.
    Ended {
        ticket: u64,
    },
    /// The group's volume, 0 to 100.
    Volume(u8),
    Error {
        ticket: u64,
        message: String,
    },
    /// Someone else started something on the group (the Sonos app, say); OMAESTRO lets go.
    TakenOver {
        ticket: u64,
    },
}

/// The line to the Sonos thread.
pub struct SonosPlayer {
    commands: Sender<Command>,
}

impl SonosPlayer {
    pub fn start(
        sonos: Arc<Sonos>,
        port: u16,
        events: impl Fn(Event) + Send + 'static,
    ) -> Result<SonosPlayer> {
        let server = server::Server::start(port)?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || run(&sonos, &server, rx, events));
        Ok(SonosPlayer { commands: tx })
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

/// What the thread is playing now.
struct Playing {
    ticket: u64,
    group: Group,
    /// The link we gave the speaker, to notice if something else takes over.
    link: String,
    /// It has been seen playing; "STOPPED" after that means the song ended.
    started: bool,
    /// We paused or stopped it ourselves.
    held: bool,
    duration: f64,
    /// When the speaker was told to play, to notice one that never starts.
    asked: Instant,
}

/// A speaker that hasn't started within this long after being asked probably can't fetch
/// the song (most often: the firewall).
const START_TIMEOUT: Duration = Duration::from_secs(15);

fn run(
    sonos: &Sonos,
    server: &server::Server,
    commands: Receiver<Command>,
    events: impl Fn(Event),
) {
    let mut playing: Option<Playing> = None;
    let mut last_poll = Instant::now();
    loop {
        let wait = if playing.is_some() {
            Duration::from_millis(1000).saturating_sub(last_poll.elapsed())
        } else {
            Duration::from_secs(3600)
        };
        match commands.recv_timeout(wait) {
            Ok(command) => {
                let ticket = playing.as_ref().map_or(0, |p| p.ticket);
                let result = (|| -> Result<()> {
                    let group = playing.as_ref().map(|p| p.group.clone());
                    match command {
                        Command::Play {
                            ticket,
                            track,
                            group,
                            start,
                        } => {
                            playing = None;
                            match start_song(sonos, server, &track, &group, ticket, &events) {
                                Ok(link) => {
                                    if start > 1.0 {
                                        let _ = sonos.seek(&group, start);
                                    }
                                    if let Ok(v) = sonos.volume(&group) {
                                        events(Event::Volume(v));
                                    }
                                    playing = Some(Playing {
                                        ticket,
                                        group,
                                        link,
                                        started: false,
                                        held: false,
                                        duration: track.duration_ms as f64 / 1000.0,
                                        asked: Instant::now(),
                                    });
                                }
                                Err(e) => events(Event::Error {
                                    ticket,
                                    message: format!("{e:#}"),
                                }),
                            }
                        }
                        Command::Pause => {
                            if let (Some(g), Some(p)) = (group, playing.as_mut()) {
                                p.held = true;
                                sonos.pause(&g)?;
                            }
                        }
                        Command::Resume => {
                            if let (Some(g), Some(p)) = (group, playing.as_mut()) {
                                p.held = false;
                                sonos.resume(&g)?;
                            }
                        }
                        Command::Stop => {
                            if let Some(g) = group {
                                playing = None;
                                sonos.stop(&g)?;
                            }
                        }
                        Command::Seek(seconds) => {
                            if let Some(g) = group {
                                sonos.seek(&g, seconds)?;
                            }
                        }
                        Command::Volume(v) => {
                            if let Some(g) = group {
                                sonos.set_volume(&g, v)?;
                                events(Event::Volume(v));
                            }
                        }
                    }
                    Ok(())
                })();
                if let Err(e) = result {
                    events(Event::Error {
                        ticket,
                        message: format!("{e:#}"),
                    });
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                if let Some(p) = playing {
                    let _ = sonos.stop(&p.group);
                }
                return;
            }
        }
        if last_poll.elapsed() >= Duration::from_millis(900)
            && let Some(p) = playing.as_mut()
        {
            last_poll = Instant::now();
            match sonos.position(&p.group) {
                Ok(pos) => {
                    if p.started && !pos.uri.is_empty() && !same_link(&pos.uri, &p.link) {
                        events(Event::TakenOver { ticket: p.ticket });
                        playing = None;
                        continue;
                    }
                    if pos.state == "PLAYING" {
                        p.started = true;
                    }
                    if !p.started && !p.held && p.asked.elapsed() > START_TIMEOUT {
                        events(Event::Error {
                            ticket: p.ticket,
                            message: "the speaker didn't start. If this was a local or YouTube \
                                      song, it probably couldn't fetch it from this computer: \
                                      allow port 47100 in the firewall (see the README)"
                                .into(),
                        });
                        playing = None;
                        continue;
                    }
                    let duration = if pos.duration > 0.0 {
                        pos.duration
                    } else {
                        p.duration
                    };
                    let ended = p.started
                        && !p.held
                        && pos.state == "STOPPED"
                        // A radio-style stream has no length; it ends when it stops.
                        && (duration <= 0.0 || pos.position == 0.0 || pos.position >= duration - 3.0);
                    events(Event::Status {
                        ticket: p.ticket,
                        state: pos.state,
                        position: pos.position,
                        duration,
                    });
                    if ended {
                        events(Event::Ended { ticket: p.ticket });
                        playing = None;
                    }
                }
                Err(e) => events(Event::Error {
                    ticket: p.ticket,
                    message: format!("{e:#}"),
                }),
            }
        }
    }
}

/// Speakers sometimes report a link back slightly reworded (case, escaping); compare loosely.
fn same_link(reported: &str, ours: &str) -> bool {
    let norm = |s: &str| s.to_lowercase().replace("%3a", ":").replace("&amp;", "&");
    let (a, b) = (norm(reported), norm(ours));
    a == b
        || a.starts_with(&b)
        || b.starts_with(&a)
        || a.contains(b.split('?').next().unwrap_or(&b))
}

/// Gets a song ready and starts it on the group. Returns the link it played.
fn start_song(
    sonos: &Sonos,
    server: &server::Server,
    track: &Track,
    group: &Group,
    ticket: u64,
    events: &impl Fn(Event),
) -> Result<String> {
    let me = sonos::local_ip(&group.coordinator.ip).context("no network address to serve from")?;
    let base = format!("http://{me}:{}", server.port);
    let (link, metadata) = match track.source {
        Source::Local => {
            let needs_work = track.sample_rate.is_some_and(|r| r > 48000);
            if needs_work {
                events(Event::Preparing {
                    ticket,
                    what: "Converting for Sonos (above 48 kHz)…".into(),
                });
            }
            let file = server::prepare_local(track)?;
            let url = format!("{base}{}", server.offer(&file));
            let meta = sonos::didl_http(
                &track.title,
                &track.artist,
                &track.album,
                "",
                &url,
                server::mime(&file),
                (track.duration_ms / 1000) as u64,
            );
            (url, meta)
        }
        Source::YouTube => {
            events(Event::Preparing {
                ticket,
                what: "Downloading from YouTube Music for Sonos…".into(),
            });
            let browser = crate::youtube::account().map(|a| a.browser);
            let file = server::prepare_youtube(&track.uri, browser.as_deref())?;
            let url = format!("{base}{}", server.offer(&file));
            let meta = sonos::didl_http(
                &track.title,
                &track.artist,
                &track.album,
                &track.art_url,
                &url,
                server::mime(&file),
                (track.duration_ms / 1000) as u64,
            );
            (url, meta)
        }
        Source::Spotify => {
            let Some(account) = sonos.spotify_account() else {
                bail!(
                    "OMAESTRO can't find your Spotify account on Sonos yet. Play any Spotify \
                     song once from the Sonos app (any room), then try again"
                );
            };
            sonos::spotify_link(
                &track.uri,
                &track.title,
                &track.artist,
                &track.album,
                account,
            )
        }
    };
    if let Err(e) = sonos.play_uri(group, &link, &metadata) {
        if track.source == Source::Spotify {
            // Maybe Spotify was linked again in the Sonos app: look for the account afresh.
            sonos::forget_spotify_account();
        }
        return Err(e);
    }
    Ok(link)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_compare_loosely() {
        assert!(same_link(
            "http://192.168.1.60:47100/f/abc.flac",
            "http://192.168.1.60:47100/f/abc.flac"
        ));
        assert!(same_link(
            "x-sonos-spotify:spotify%3atrack%3aABC?sid=9&flags=8224&sn=1",
            "x-sonos-spotify:spotify%3atrack%3aABC?sid=9&flags=8224&sn=1"
        ));
        assert!(same_link(
            "x-sonos-spotify:spotify:track:ABC?sid=9",
            "x-sonos-spotify:spotify%3atrack%3aABC?sid=9&flags=8224&sn=1"
        ));
        assert!(!same_link(
            "x-sonos-vli:RINCON_1:2",
            "http://192.168.1.60:47100/f/abc.flac"
        ));
    }
}
