//! Playback: OMAESTRO's queue, and handing each song to whatever plays it.
//!
//! The queue lives here and can mix sources. Each song goes to one engine, chosen by where
//! it's from and where it's playing:
//! - on this computer: mpv plays local files and YouTube Music streams; Spotify Soloist plays
//!   Spotify songs;
//! - on a Sonos group: the speakers play everything (see `sonos.rs`).
//!
//! mpv holds the song playing now plus the next one when both are local files, which is enough
//! for gapless playback. Other songs play one at a time; the player moves on when one ends.

pub mod mpv;
pub mod soloist;
pub mod sonos;

use crate::library::{Source, Track};
use crate::sonos as sonos_api;
use crate::youtube;
use anyhow::Result;
use mpv::{Mpv, MpvEvent, Options};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub struct AudioDevice {
    /// mpv's name for it, e.g. "pipewire/alsa_output.usb-Schiit_Audio_Schiit_Modi_-00.analog-stereo".
    pub name: String,
    /// Human name, e.g. "Schiit Modi+ Analog Stereo".
    pub description: String,
}

/// Where the music comes out.
#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    /// This computer: an mpv device name such as "pipewire/alsa_output.usb-…", or "" for the
    /// desktop's default.
    Here(String),
    Sonos(sonos_api::Group),
}

/// Everything the player reports.
#[derive(Debug)]
pub enum Event {
    Mpv(MpvEvent),
    Soloist(soloist::Event),
    Sonos(sonos::Event),
    /// A YouTube Music song's stream, looked up for song number `ticket`.
    Resolved {
        ticket: u64,
        result: Result<youtube::Stream, String>,
    },
}

/// What's playing the current song.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Engine {
    None,
    Mpv,
    Soloist,
    Sonos,
}

/// What the player reports back, kept up to date from its engines.
#[derive(Debug, Clone, Default)]
pub struct Status {
    pub paused: bool,
    /// Nothing loaded: before the first track, or after the queue ran out.
    pub idle: bool,
    pub position: f64,
    pub duration: f64,
    /// The rate and sample format actually sent to the output, e.g. 96000 and "s32".
    pub sample_rate: Option<u32>,
    pub sample_format: String,
    pub device: String,
    pub devices: Vec<AudioDevice>,
    /// 0 to 100, for whichever output is in use.
    pub volume: f64,
    /// Getting the current song ready (a YouTube lookup, a conversion for Sonos): what.
    pub preparing: Option<String>,
    pub last_error: Option<String>,
    /// How a streamed song (YouTube, Spotify) is coming in, when known.
    pub quality: Option<Quality>,
}

/// A stream's quality, for the badge in Now Playing: "OPUS 141 kbps", "LOSSLESS 16b-44k".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Quality {
    /// "AAC", "OPUS", "VORBIS", "FLAC"… (see `codec_name`).
    pub codec: String,
    /// Kilobits per second.
    pub kbps: Option<u32>,
    pub bit_depth: Option<u8>,
    pub sample_rate: Option<u32>,
    pub lossless: bool,
    /// The service's own word for it when that's all there is ("VERY HIGH").
    pub level: String,
}

/// A codec's everyday name: "mp4a.40.2" → "AAC", "opus" → "OPUS".
pub fn codec_name(raw: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.starts_with("mp4a") || lower.contains("aac") {
        "AAC".into()
    } else if lower.contains("vorbis") || lower == "ogg" {
        "VORBIS".into()
    } else {
        raw.to_uppercase()
    }
}

/// The saved playlist the queue came from or was saved as.
#[derive(Debug, Clone, PartialEq)]
pub struct Named {
    pub name: String,
    pub path: std::path::PathBuf,
    /// The queue's revision when it was last saved or opened.
    pub saved: u64,
}

/// The queue as it was before an edit, for undo, and what the edit was.
struct Snapshot {
    queue: Vec<Track>,
    current: Option<usize>,
    playlist: Option<Named>,
    what: String,
}

/// How long Soloist gets to start a song before it's asked again, and how many times it's
/// asked before OMAESTRO gives up on the song.
const SOLOIST_RETRY: Duration = Duration::from_secs(4);
const SOLOIST_TRIES: u8 = 3;

/// How many edits undo can go back.
const UNDO_STEPS: usize = 50;

pub struct Player {
    mpv: Mpv,
    events: Sender<Event>,
    soloist: Option<soloist::Soloist>,
    /// The Spotify song Soloist was asked to play, to notice when it moves on by itself.
    soloist_uri: Option<String>,
    /// The one before, whose late news is ignored.
    soloist_previous: Option<String>,
    soloist_held: bool,
    /// Soloist has reported our song: until then, news about any other song (what it was
    /// playing before, or what Spotify resumed as it became the active device) isn't the end
    /// of ours.
    soloist_started: bool,
    /// When Soloist was last asked to play our song, and how many times, while it hasn't.
    soloist_asked: Option<(Instant, u8)>,
    /// Where to seek once Soloist is playing our song (coming back from Sonos mid-song). Sent
    /// with "play", a seek is lost while the song loads.
    soloist_seek: Option<f64>,
    /// The volume on this computer's outputs. While playing on Sonos, the volume shown is the
    /// speaker's own; coming back, this one returns.
    here_volume: f64,
    /// The song restored from last time and how far into it: playing that song carries on
    /// from there (any other starts at the beginning).
    restored: Option<(usize, f64)>,
    sonos: Option<sonos::SonosPlayer>,
    household: Option<Arc<sonos_api::Sonos>>,
    media_port: u16,
    pub output: Output,
    engine: Engine,
    /// Goes up with every song started; news about older songs is ignored.
    ticket: u64,
    /// Where the next song starts (resuming after the output changed).
    start_at: f64,
    /// For Soloist and Sonos, which report the position now and then: the last report, when
    /// it came and whether the song is moving.
    anchor: Option<(f64, Instant)>,
    /// A YouTube stream waiting for its start time (YouTube holds some back for a moment).
    waiting: Option<(youtube::Stream, f64)>,
    queue: Vec<Track>,
    current: Option<usize>,
    undo: Vec<Snapshot>,
    /// Goes up with every change to the queue, so the screen can tell it was edited after saving.
    pub revision: u64,
    /// The playlist the queue holds, if it was opened or saved as one. Part of the queue's
    /// history, so undo brings it back too.
    pub playlist: Option<Named>,
    /// The path handed to mpv as "up next", if any.
    upcoming: Option<String>,
    /// The path just loaded with `jump`. mpv's first report of it isn't a gapless move.
    loading: Option<String>,
    pub status: Status,
}

impl Player {
    pub fn start(options: &Options, socket: &Path, events: Sender<Event>) -> Result<Player> {
        let tx = events.clone();
        Ok(Player {
            mpv: Mpv::start(options, socket, move |e| tx.send(Event::Mpv(e)).is_ok())?,
            events,
            soloist: None,
            soloist_uri: None,
            soloist_previous: None,
            soloist_held: false,
            soloist_started: false,
            soloist_asked: None,
            soloist_seek: None,
            here_volume: options.volume as f64,
            restored: None,
            sonos: None,
            household: None,
            media_port: 47100,
            output: Output::Here(options.audio_device.clone()),
            engine: Engine::None,
            ticket: 0,
            start_at: 0.0,
            anchor: None,
            waiting: None,
            queue: Vec::new(),
            current: None,
            undo: Vec::new(),
            revision: 0,
            playlist: None,
            upcoming: None,
            loading: None,
            status: Status {
                idle: true,
                volume: options.volume as f64,
                ..Status::default()
            },
        })
    }

    pub fn queue(&self) -> &[Track] {
        &self.queue
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current
    }

    pub fn current(&self) -> Option<&Track> {
        self.current.and_then(|i| self.queue.get(i))
    }

    /// The Sonos household, once found; and the port speakers fetch songs from.
    pub fn set_sonos(&mut self, household: sonos_api::Sonos, media_port: u16) {
        self.household = Some(Arc::new(household));
        self.media_port = media_port;
    }

    // ---- the queue ---------------------------------------------------------------------

    /// Keeps a copy of the queue for undo, with a few words about the change to come; call
    /// before changing it.
    fn remember(&mut self, what: String) {
        self.undo.push(Snapshot {
            queue: self.queue.clone(),
            current: self.current,
            playlist: self.playlist.clone(),
            what,
        });
        if self.undo.len() > UNDO_STEPS {
            self.undo.remove(0);
        }
        self.revision += 1;
    }

    /// Replaces the queue with `tracks` and plays from `start`. The queue isn't the playlist
    /// it was any more. (Only the tests start a queue this way; the app adds to it instead.)
    #[cfg(test)]
    pub fn play(&mut self, tracks: Vec<Track>, start: usize) -> Result<()> {
        self.remember("replaced the queue".into());
        self.queue = tracks;
        self.playlist = None;
        self.upcoming = None;
        self.jump(start)
    }

    /// Replaces the queue without playing it (an opened playlist). Play starts at the top.
    pub fn load(&mut self, tracks: Vec<Track>, playlist: Option<Named>) -> Result<()> {
        let what = match &playlist {
            Some(p) => format!("opened {}", p.name),
            None => "opened a playlist".into(),
        };
        self.remember(what);
        self.queue = tracks;
        self.current = (!self.queue.is_empty()).then_some(0);
        self.playlist = playlist.map(|p| Named {
            saved: self.revision,
            ..p
        });
        self.stop()
    }

    /// Plays a song right away, putting it after the one playing so the queue carries on.
    pub fn play_now(&mut self, track: Track) -> Result<()> {
        self.remember(format!("played {}", track.title));
        let at = self.current.map_or(self.queue.len(), |c| c + 1);
        self.queue.insert(at, track);
        self.jump(at)
    }

    /// Adds tracks to the end of the queue without playing them. In an empty queue, play
    /// starts with the first of them.
    pub fn enqueue(&mut self, tracks: Vec<Track>) -> Result<()> {
        let what = match tracks.as_slice() {
            [one] => format!("added {}", one.title),
            many => format!("added {} songs", many.len()),
        };
        self.remember(what);
        let first_new = self.queue.len();
        self.queue.extend(tracks);
        if self.current.is_none() && first_new < self.queue.len() {
            self.current = Some(first_new);
        }
        self.sync_upcoming()
    }

    /// Takes a song out of the queue. Taking out the one playing moves on to the next.
    pub fn remove(&mut self, index: usize) -> Result<()> {
        if index >= self.queue.len() {
            return Ok(());
        }
        self.remember(format!("removed {}", self.queue[index].title));
        self.queue.remove(index);
        match self.current {
            Some(c) if c == index => {
                let playing = !self.status.idle;
                if index < self.queue.len() {
                    if playing {
                        return self.jump(index);
                    }
                    self.current = Some(index);
                } else {
                    self.current = self.queue.len().checked_sub(1);
                    if playing {
                        return self.stop();
                    }
                }
                self.sync_upcoming()
            }
            Some(c) if c > index => {
                self.current = Some(c - 1);
                self.sync_upcoming()
            }
            _ => self.sync_upcoming(),
        }
    }

    /// Swaps a song with the one above (`up`) or below it. Returns where the song is now.
    pub fn move_track(&mut self, index: usize, up: bool) -> Result<usize> {
        let other = if up {
            index.checked_sub(1)
        } else {
            (index + 1 < self.queue.len()).then_some(index + 1)
        };
        let Some(other) = other.filter(|_| index < self.queue.len()) else {
            return Ok(index);
        };
        self.remember(format!("moved {}", self.queue[index].title));
        self.queue.swap(index, other);
        self.current = self.current.map(|c| match c {
            c if c == index => other,
            c if c == other => index,
            c => c,
        });
        self.sync_upcoming()?;
        Ok(other)
    }

    /// Moves a song to another place in the queue (a drag). Returns where it is now.
    pub fn move_to(&mut self, from: usize, to: usize) -> Result<usize> {
        if from >= self.queue.len() || from == to {
            return Ok(from);
        }
        let to = to.min(self.queue.len() - 1);
        self.remember(format!("moved {}", self.queue[from].title));
        let track = self.queue.remove(from);
        self.queue.insert(to, track);
        self.current = self.current.map(|c| match c {
            c if c == from => to,
            c if from < c && c <= to => c - 1,
            c if to <= c && c < from => c + 1,
            c => c,
        });
        self.sync_upcoming()?;
        Ok(to)
    }

    /// Empties the queue and stops.
    pub fn clear(&mut self) -> Result<()> {
        if self.queue.is_empty() {
            return Ok(());
        }
        self.remember("cleared the queue".into());
        self.queue.clear();
        self.current = None;
        self.stop()
    }

    /// Puts the queue (and the playlist it was) back as before the last edit. The song playing
    /// carries on; it keeps its ▶ if it's in the restored queue. Returns what was undone, or
    /// None when there's nothing to undo.
    pub fn undo(&mut self) -> Result<Option<String>> {
        let Some(snap) = self.undo.pop() else {
            return Ok(None);
        };
        let playing = self.current().filter(|_| !self.status.idle).cloned();
        self.queue = snap.queue;
        self.playlist = snap.playlist;
        self.revision += 1;
        self.current = match playing {
            Some(track) => {
                let same = |i: &usize| self.queue.get(*i) == Some(&track);
                snap.current
                    .filter(same)
                    .or_else(|| (0..self.queue.len()).find(same))
            }
            None => snap.current.filter(|&i| i < self.queue.len()),
        };
        self.sync_upcoming()?;
        Ok(Some(snap.what))
    }

    // ---- playing -----------------------------------------------------------------------

    /// Stops whichever engine isn't `keep`.
    fn hand_over(&mut self, keep: Engine) -> Result<()> {
        if self.engine == keep {
            return Ok(());
        }
        match self.engine {
            Engine::Mpv => {
                self.upcoming = None;
                self.mpv.stop()?;
            }
            Engine::Soloist => {
                if let Some(s) = &self.soloist {
                    self.soloist_held = true;
                    s.pause();
                }
            }
            Engine::Sonos => {
                if let Some(s) = &self.sonos {
                    s.send(sonos::Command::Stop);
                }
            }
            Engine::None => {}
        }
        self.engine = keep;
        Ok(())
    }

    /// Plays the queue entry at `index`.
    pub fn jump(&mut self, index: usize) -> Result<()> {
        let Some(track) = self.queue.get(index).cloned() else {
            return Ok(());
        };
        let start = std::mem::take(&mut self.start_at);
        let start = match self.restored.take() {
            Some((i, at)) if i == index && start == 0.0 => at,
            _ => start,
        };
        self.ticket += 1;
        let ticket = self.ticket;
        self.current = Some(index);
        self.status.idle = false;
        self.status.paused = false;
        self.status.position = start;
        self.status.duration = track.duration_ms as f64 / 1000.0;
        self.status.preparing = None;
        self.status.sample_rate = None;
        self.status.quality = None;
        self.anchor = None;
        self.waiting = None;

        match (&self.output, track.source) {
            (Output::Sonos(group), _) => {
                let group = group.clone();
                self.hand_over(Engine::Sonos)?;
                self.ensure_sonos()?;
                if let Some(s) = &self.sonos {
                    s.send(sonos::Command::Play {
                        ticket,
                        track,
                        group,
                        start,
                    });
                }
            }
            (Output::Here(_), Source::Local) => {
                self.hand_over(Engine::Mpv)?;
                self.loading = Some(track.path.clone());
                self.upcoming = None;
                self.mpv.load_replace_at(&track.path, start)?;
                self.mpv.set("pause", json!(false))?;
                self.sync_upcoming()?;
            }
            (Output::Here(_), Source::YouTube) => {
                self.hand_over(Engine::Mpv)?;
                self.upcoming = None;
                self.loading = None;
                self.mpv.stop()?;
                self.status.preparing = Some("Finding the stream on YouTube Music…".into());
                let events = self.events.clone();
                let id = track.uri.clone();
                std::thread::spawn(move || {
                    let result = youtube::resolve(&id).map_err(|e| format!("{e:#}"));
                    let _ = events.send(Event::Resolved { ticket, result });
                });
                self.start_at = start;
            }
            (Output::Here(device), Source::Spotify) => {
                let device = node_name(device);
                self.hand_over(Engine::Soloist)?;
                self.ensure_soloist(&device)?;
                if self.soloist.is_some() {
                    self.ask_soloist(&track.uri, start);
                }
                if let Some(s) = &self.soloist {
                    s.set_volume(self.status.volume.round() as u8);
                    s.play(&track.uri);
                }
            }
        }
        Ok(())
    }

    /// Notes that Soloist is being asked to play `uri` from `start` seconds.
    fn ask_soloist(&mut self, uri: &str, start: f64) {
        let previous = self.soloist_uri.replace(uri.to_string());
        // The same song again (back from Sonos): its news is wanted, not ignored as late.
        self.soloist_previous = previous.filter(|p| p != uri);
        self.soloist_held = false;
        self.soloist_started = false;
        self.soloist_asked = Some((Instant::now(), 1));
        self.soloist_seek = (start > 1.0).then_some(start);
    }

    /// Starts Soloist on `device`, or restarts it if it's on another device or has stopped.
    fn ensure_soloist(&mut self, device: &str) -> Result<()> {
        let fine = match &mut self.soloist {
            Some(s) => s.device == device && s.alive(),
            None => false,
        };
        if !fine {
            self.soloist = None;
            let events = self.events.clone();
            match soloist::Soloist::start(device, move |e| {
                let _ = events.send(Event::Soloist(e));
            }) {
                Ok(s) => self.soloist = Some(s),
                Err(e) => {
                    self.status.last_error = Some(format!("{e:#}"));
                    self.status.idle = true;
                    self.engine = Engine::None;
                }
            }
        }
        Ok(())
    }

    fn ensure_sonos(&mut self) -> Result<()> {
        if self.sonos.is_some() {
            return Ok(());
        }
        let Some(household) = self.household.clone() else {
            self.status.last_error = Some("The Sonos speakers haven't been found yet".into());
            return Ok(());
        };
        let events = self.events.clone();
        match sonos::SonosPlayer::start(household, self.media_port, move |e| {
            let _ = events.send(Event::Sonos(e));
        }) {
            Ok(s) => self.sonos = Some(s),
            Err(e) => self.status.last_error = Some(format!("{e:#}")),
        }
        Ok(())
    }

    /// Moves on after a song ends by itself: the next one, or stop at the end of the queue.
    fn advance(&mut self) -> Result<()> {
        match self.current {
            Some(i) if i + 1 < self.queue.len() => self.jump(i + 1),
            _ => {
                self.status.idle = true;
                self.status.position = 0.0;
                self.anchor = None;
                self.hand_over(Engine::None)
            }
        }
    }

    pub fn next(&mut self) -> Result<()> {
        match self.current {
            Some(i) if i + 1 < self.queue.len() => self.jump(i + 1),
            _ => Ok(()),
        }
    }

    /// Back to the start of the track, or to the previous track in the first 3 seconds.
    pub fn previous(&mut self) -> Result<()> {
        match self.current {
            Some(i) if self.status.position < 3.0 && i > 0 => self.jump(i - 1),
            Some(_) => self.seek_to(0.0),
            None => Ok(()),
        }
    }

    pub fn toggle_pause(&mut self) -> Result<()> {
        if self.status.idle || self.engine == Engine::None {
            // Finished or never started: play the current entry again.
            return self.current.map_or(Ok(()), |i| self.jump(i));
        }
        let pause = !self.status.paused;
        match self.engine {
            Engine::Mpv => self.mpv.toggle_pause()?,
            Engine::Soloist => {
                if let Some(s) = &self.soloist {
                    self.soloist_held = pause;
                    if pause { s.pause() } else { s.resume() }
                }
                self.status.paused = pause;
            }
            Engine::Sonos => {
                if let Some(s) = &self.sonos {
                    s.send(if pause {
                        sonos::Command::Pause
                    } else {
                        sonos::Command::Resume
                    });
                }
                self.status.paused = pause;
            }
            Engine::None => {}
        }
        self.anchor = self.anchor.map(|_| (self.status.position, Instant::now()));
        Ok(())
    }

    /// Seeks by `seconds`, backwards when negative.
    pub fn seek(&mut self, seconds: f64) -> Result<()> {
        match self.engine {
            Engine::Mpv => self.mpv.seek(seconds),
            _ => self.seek_to(self.status.position + seconds),
        }
    }

    /// Plays from this many seconds into the song.
    pub fn seek_to(&mut self, seconds: f64) -> Result<()> {
        if self.status.idle {
            return Ok(());
        }
        let seconds = seconds.clamp(0.0, self.status.duration.max(0.0));
        match self.engine {
            Engine::Mpv => self
                .mpv
                .command(&[json!("seek"), json!(seconds), json!("absolute")])?,
            Engine::Soloist => {
                if let Some(s) = &self.soloist {
                    s.seek(seconds);
                }
            }
            Engine::Sonos => {
                if let Some(s) = &self.sonos {
                    s.send(sonos::Command::Seek(seconds));
                }
            }
            Engine::None => {}
        }
        self.status.position = seconds;
        self.anchor = self.anchor.map(|_| (seconds, Instant::now()));
        Ok(())
    }

    /// Changes where the music plays. Whatever was playing carries on from the same point.
    pub fn set_output(&mut self, output: Output) -> Result<()> {
        if output == self.output {
            return Ok(());
        }
        // From one of this computer's outputs to another while mpv (or nothing) plays: mpv
        // moves over live, without a gap.
        if let (Output::Here(_), Output::Here(device)) = (&self.output, &output)
            && matches!(self.engine, Engine::Mpv | Engine::None)
        {
            let name = if device.is_empty() {
                "auto"
            } else {
                device.as_str()
            };
            self.mpv.set("audio-device", json!(name))?;
            self.output = output;
            return Ok(());
        }
        let resume = (!self.status.idle && self.current.is_some()).then_some(self.status.position);
        let was_sonos = matches!(self.output, Output::Sonos(_));
        self.hand_over(Engine::None)?;
        if let Output::Here(device) = &output {
            self.mpv.set(
                "audio-device",
                json!(if device.is_empty() { "auto" } else { device }),
            )?;
            // Back on this computer: its own volume again, not the speaker's.
            if was_sonos {
                self.status.volume = self.here_volume;
                self.mpv.set("volume", json!(self.here_volume))?;
            }
        }
        self.output = output;
        match (resume, self.current) {
            (Some(position), Some(i)) => {
                self.start_at = position;
                self.jump(i)
            }
            _ => {
                self.status.idle = true;
                Ok(())
            }
        }
    }

    /// The queue from last time, back in place with nothing playing: play carries on from
    /// where it was.
    pub fn restore(
        &mut self,
        tracks: Vec<Track>,
        current: Option<usize>,
        playlist: Option<crate::session::Playlist>,
        position: f64,
    ) {
        self.queue = tracks;
        self.current = current.filter(|&i| i < self.queue.len());
        self.playlist = playlist.map(|p| Named {
            name: p.name,
            path: p.path,
            saved: if p.unsaved { u64::MAX } else { self.revision },
        });
        self.restored = self.current.map(|i| (i, position));
        self.status.position = position;
        self.status.duration = self
            .current()
            .map_or(0.0, |t| t.duration_ms as f64 / 1000.0);
    }

    /// The queue, the song it's on, how far in, and its playlist: what's kept for next time.
    pub fn session(&self) -> crate::session::Session {
        crate::session::Session {
            tracks: self.queue.clone(),
            current: self.current,
            position: match self.restored {
                Some((i, at)) if Some(i) == self.current => at,
                _ => self.status.position,
            },
            playlist: self.playlist.as_ref().map(|p| crate::session::Playlist {
                name: p.name.clone(),
                path: p.path.clone(),
                unsaved: p.saved != self.revision,
            }),
        }
    }

    /// The volume on this computer's outputs (kept for next time), wherever it's playing now.
    pub fn here_volume(&self) -> f64 {
        self.here_volume
    }

    /// Stops playing but keeps the queue and its place, so play starts that track again.
    pub fn stop(&mut self) -> Result<()> {
        self.loading = None;
        self.status.position = 0.0;
        self.status.preparing = None;
        self.waiting = None;
        self.anchor = None;
        self.ticket += 1;
        self.hand_over(Engine::None)?;
        self.status.idle = true;
        self.status.paused = false;
        Ok(())
    }

    /// Stops everything before OMAESTRO quits (the speakers too).
    pub fn shut_down(&mut self) {
        let on_sonos = self.engine == Engine::Sonos;
        let _ = self.stop();
        self.soloist = None;
        if on_sonos {
            // Give the Sonos thread a moment to send its Stop.
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
        self.sonos = None;
    }

    /// Sets the volume, 0 to 100, of whatever is playing: only OMAESTRO, not the desktop.
    pub fn set_volume(&mut self, volume: f64) -> Result<()> {
        let volume = volume.clamp(0.0, 100.0).round();
        self.status.volume = volume;
        match &self.output {
            Output::Sonos(_) => {
                if let Some(s) = &self.sonos {
                    s.send(sonos::Command::Volume(volume as u8));
                }
            }
            Output::Here(_) => {
                self.here_volume = volume;
                self.mpv.set("volume", json!(volume))?;
                if let Some(s) = &self.soloist {
                    s.set_volume(volume as u8);
                }
            }
        }
        Ok(())
    }

    /// "off", "track" or "album".
    pub fn set_replaygain(&mut self, mode: &str) -> Result<()> {
        let mode = if matches!(mode, "track" | "album") {
            mode
        } else {
            "no"
        };
        self.mpv.set("replaygain", json!(mode))
    }

    pub fn set_gapless(&mut self, on: bool) -> Result<()> {
        self.mpv
            .set("gapless-audio", json!(if on { "weak" } else { "no" }))
    }

    /// Makes sure mpv has the right "up next" track queued behind the current one: only when
    /// both are local files playing on this computer.
    fn sync_upcoming(&mut self) -> Result<()> {
        let want = match (&self.output, self.engine) {
            (Output::Here(_), Engine::Mpv) => self
                .current
                .and_then(|i| self.queue.get(i + 1))
                .filter(|t| t.source == Source::Local)
                .filter(|_| self.current().is_some_and(|t| t.source == Source::Local))
                .map(|t| t.path.clone()),
            _ => None,
        };
        if want != self.upcoming {
            if self.upcoming.is_some() {
                self.mpv.clear_upcoming()?;
            }
            if let Some(path) = &want {
                self.mpv.load_append(path)?;
            }
            self.upcoming = want;
        }
        Ok(())
    }

    /// Keeps the position moving between reports, and starts a held-back YouTube stream when
    /// its time comes. Call a few times a second.
    pub fn tick(&mut self) -> Result<()> {
        if let Some((pos, at)) = self.anchor
            && !self.status.paused
            && !self.status.idle
        {
            self.status.position =
                (pos + at.elapsed().as_secs_f64()).min(self.status.duration.max(pos));
        }
        if let Some((stream, start)) = &self.waiting
            && stream.available_at <= crate::secrets::now_secs()
        {
            let (stream, start) = (stream.clone(), *start);
            self.waiting = None;
            self.play_stream(&stream, start)?;
        }
        // Soloist hasn't started our song yet: ask again every few seconds, then give up.
        if self.engine == Engine::Soloist
            && !self.soloist_started
            && !self.soloist_held
            && let Some((asked, tries)) = self.soloist_asked
            && asked.elapsed() >= SOLOIST_RETRY
        {
            let title = self.current().map(|t| t.title.clone()).unwrap_or_default();
            if tries < SOLOIST_TRIES {
                if let (Some(s), Some(uri)) = (&self.soloist, &self.soloist_uri) {
                    s.play(uri);
                }
                self.soloist_asked = Some((Instant::now(), tries + 1));
            } else {
                self.soloist_asked = None;
                self.status.last_error = Some(format!("Spotify didn't start “{title}”"));
                if let Some(s) = &self.soloist {
                    self.soloist_held = true;
                    s.pause();
                }
                return self.advance();
            }
        }
        // Soloist doesn't report the end of a song; notice it from the position.
        if self.engine == Engine::Soloist
            && self.soloist_started
            && !self.status.paused
            && !self.status.idle
            && self.status.duration > 0.0
            && self.status.position >= self.status.duration + 1.5
        {
            if let Some(s) = &self.soloist {
                self.soloist_held = true;
                s.pause();
            }
            return self.advance();
        }
        Ok(())
    }

    fn play_stream(&mut self, stream: &youtube::Stream, start: f64) -> Result<()> {
        self.status.preparing = None;
        self.status.quality = Some(Quality {
            codec: codec_name(&stream.codec),
            kbps: stream.bitrate,
            sample_rate: stream.sample_rate,
            ..Quality::default()
        });
        self.mpv.set("user-agent", json!(stream.user_agent))?;
        self.loading = Some(stream.url.clone());
        self.mpv.load_replace_at(&stream.url, start)?;
        self.mpv.set("pause", json!(false))
    }

    /// What the visualizer should listen to: mpv's own sound, or for Spotify the device it
    /// plays on. Nothing while paused, stopped or playing on Sonos.
    pub fn listen_target(&self) -> Option<crate::visualizer::Target> {
        use crate::visualizer::Target;
        if self.status.idle || self.status.paused || self.status.preparing.is_some() {
            return None;
        }
        match (self.engine, &self.output) {
            (Engine::Mpv, Output::Here(_)) => Some(Target::Stream("omaestro".into())),
            (Engine::Soloist, Output::Here(device)) => Some(Target::from_mpv_device(device)),
            // On Sonos: a silent copy of the same file, once the speaker is actually playing.
            // Spotify there has no file here to copy.
            (Engine::Sonos, _) if self.anchor.is_some() => {
                let track = self.current()?;
                let file = match track.source {
                    Source::Local => std::path::PathBuf::from(&track.path),
                    Source::YouTube => crate::sonos::server::youtube_file(&track.uri),
                    Source::Spotify => return None,
                };
                file.is_file().then_some(Target::File(file))
            }
            _ => None,
        }
    }

    // ---- news from the engines ----------------------------------------------------------

    pub fn handle(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Mpv(e) => self.mpv_event(e),
            Event::Soloist(e) => self.soloist_event(e),
            Event::Sonos(e) => self.sonos_event(e),
            Event::Resolved { ticket, result } => {
                if ticket != self.ticket || self.engine != Engine::Mpv {
                    return Ok(());
                }
                match result {
                    Ok(stream) => {
                        let start = std::mem::take(&mut self.start_at);
                        if stream.available_at > crate::secrets::now_secs() {
                            self.status.preparing =
                                Some("YouTube is holding the stream back for a moment…".into());
                            self.waiting = Some((stream, start));
                            Ok(())
                        } else {
                            self.play_stream(&stream, start)
                        }
                    }
                    Err(e) => {
                        self.status.preparing = None;
                        self.status.last_error = Some(e);
                        self.advance()
                    }
                }
            }
        }
    }

    fn mpv_event(&mut self, event: MpvEvent) -> Result<()> {
        match event {
            MpvEvent::Property { name, data } => self.property(&name, data)?,
            MpvEvent::Event { name, reason }
                if name == "end-file" && self.engine == Engine::Mpv =>
            {
                match reason.as_deref() {
                    // Played to the end with nothing queued in mpv: move on ourselves.
                    Some("eof") if self.upcoming.is_none() && self.loading.is_none() => {
                        self.advance()?
                    }
                    Some("error") => {
                        let what = self
                            .current()
                            .map(|t| {
                                if t.path.is_empty() {
                                    t.title.clone()
                                } else {
                                    t.path.clone()
                                }
                            })
                            .unwrap_or_default();
                        self.status.last_error = Some(format!("couldn't play {what}"));
                        self.loading = None;
                        self.advance()?;
                    }
                    _ => {}
                }
            }
            MpvEvent::Event { .. } => {}
            MpvEvent::Gone => self.status.last_error = Some("mpv stopped unexpectedly".into()),
        }
        Ok(())
    }

    fn property(&mut self, name: &str, data: Value) -> Result<()> {
        let mine = self.engine == Engine::Mpv;
        match name {
            "path" => {
                let Some(path) = data.as_str() else {
                    return Ok(());
                };
                // The first path reported after a load is that load, even when it's the same
                // file as before; only after that can a path mean a gapless move.
                let loaded = self.loading.take();
                if loaded.as_deref() == Some(path) {
                    // Our own load arriving.
                } else if self.upcoming.as_deref() == Some(path) {
                    // mpv moved to the queued track by itself, without a gap.
                    self.current = self.current.map(|i| i + 1);
                    self.ticket += 1;
                    self.upcoming = None;
                    self.mpv.clear_upcoming()?;
                    self.sync_upcoming()?;
                }
            }
            "pause" if mine => self.status.paused = data.as_bool().unwrap_or(false),
            "time-pos" if mine => {
                if let Some(t) = data.as_f64() {
                    self.status.position = t;
                }
            }
            "duration" if mine => {
                if let Some(d) = data.as_f64() {
                    self.status.duration = d;
                }
            }
            "idle-active" if mine => {
                // Between songs that aren't both local files mpv goes idle for a moment;
                // that's only "stopped" once the queue has run out.
                let idle = data.as_bool().unwrap_or(false);
                if !idle {
                    self.status.idle = false;
                }
            }
            "audio-params" if mine => {
                self.status.sample_rate = data
                    .get("samplerate")
                    .and_then(Value::as_u64)
                    .map(|r| r as u32);
                self.status.sample_format = data
                    .get("format")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
            }
            "audio-device" => self.status.device = data.as_str().unwrap_or_default().to_string(),
            "audio-device-list" => self.status.devices = pipewire_devices(&data),
            "volume" if matches!(self.output, Output::Here(_)) => {
                if let Some(v) = data.as_f64() {
                    self.status.volume = v;
                    self.here_volume = v;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn soloist_event(&mut self, event: soloist::Event) -> Result<()> {
        match event {
            soloist::Event::State(s) => {
                if self.engine != Engine::Soloist
                    || (s.uri.is_some() && s.uri == self.soloist_previous)
                {
                    return Ok(());
                }
                // Until Soloist reports our song, its news is about something else.
                if !self.soloist_started {
                    match (&s.uri, &self.soloist_uri) {
                        (Some(uri), Some(ours)) if uri == ours => {
                            self.soloist_started = true;
                            self.soloist_asked = None;
                        }
                        _ => return Ok(()),
                    }
                }
                // Playing our song: now a seek sticks.
                if s.status.as_deref() == Some("playing")
                    && let Some(at) = self.soloist_seek.take()
                    && let Some(sol) = &self.soloist
                {
                    sol.seek(at);
                }
                // Soloist moved on to a song we didn't ask for: ours has ended.
                if let (Some(uri), Some(ours)) = (&s.uri, &self.soloist_uri)
                    && uri != ours
                    && !self.soloist_held
                {
                    if let Some(sol) = &self.soloist {
                        self.soloist_held = true;
                        sol.pause();
                    }
                    return self.advance();
                }
                if let Some(d) = s.duration_ms.filter(|d| *d > 0.0) {
                    self.status.duration = d / 1000.0;
                }
                if s.quality.is_some() {
                    self.status.quality = s.quality;
                }
                if let Some(status) = &s.status {
                    self.status.paused = status == "paused" || status == "idle";
                    // Stopped at the very end: the song finished.
                    let at_end = self.status.duration > 0.0
                        && self.status.position >= self.status.duration - 2.0;
                    if (status == "idle" || status == "paused") && at_end && !self.soloist_held {
                        self.soloist_held = true;
                        return self.advance();
                    }
                }
                if let (Some(pos), Some(at)) = (s.position_ms, s.at_ms) {
                    let lag = (crate::secrets::now_secs() as f64 * 1000.0 - at).max(0.0);
                    let speed = s.speed.unwrap_or(1.0);
                    let now = (pos + lag * speed) / 1000.0;
                    self.status.position = now;
                    self.anchor = Some((now, Instant::now()));
                    if speed == 0.0 {
                        self.status.paused = true;
                    }
                }
            }
            soloist::Event::LoggedOut => {
                self.status.last_error = Some(
                    "Soloist isn't signed in to Spotify: pair it again in Settings → Sources"
                        .into(),
                );
            }
            soloist::Event::Error(e) => self.status.last_error = Some(format!("Spotify: {e}")),
            soloist::Event::Gone(e) => {
                self.soloist = None;
                if self.engine == Engine::Soloist {
                    self.status.last_error = Some(e);
                    self.status.idle = true;
                    self.engine = Engine::None;
                }
            }
        }
        Ok(())
    }

    fn sonos_event(&mut self, event: sonos::Event) -> Result<()> {
        let current = |t: u64| t == self.ticket && self.engine == Engine::Sonos;
        match event {
            sonos::Event::Preparing { ticket, what } if current(ticket) => {
                self.status.preparing = Some(what);
            }
            sonos::Event::Status {
                ticket,
                state,
                position,
                duration,
            } if current(ticket) => {
                self.status.preparing = None;
                self.status.paused = state == "PAUSED_PLAYBACK";
                self.status.position = position;
                if duration > 0.0 {
                    self.status.duration = duration;
                }
                // A YouTube song plays on Sonos from an AAC copy here: its size says its rate.
                if self.status.quality.is_none()
                    && let Some(track) = self.current().filter(|t| t.source == Source::YouTube)
                    && let Ok(meta) =
                        std::fs::metadata(crate::sonos::server::youtube_file(&track.uri))
                    && track.duration_ms > 0
                {
                    let kbps = meta.len() * 8 / track.duration_ms as u64;
                    self.status.quality = Some(Quality {
                        codec: "AAC".into(),
                        kbps: Some(kbps as u32),
                        ..Quality::default()
                    });
                }
                self.anchor = (state == "PLAYING").then(|| (position, Instant::now()));
            }
            sonos::Event::Ended { ticket } if current(ticket) => self.advance()?,
            sonos::Event::Error { ticket, message } if current(ticket) || ticket == 0 => {
                self.status.preparing = None;
                self.status.last_error = Some(format!("Sonos: {message}"));
            }
            sonos::Event::TakenOver { ticket } if current(ticket) => {
                self.engine = Engine::None;
                self.status.idle = true;
                self.anchor = None;
                self.status.last_error =
                    Some("The Sonos group is playing something else now".into());
            }
            sonos::Event::Volume(v) => {
                if matches!(self.output, Output::Sonos(_)) {
                    self.status.volume = v as f64;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// mpv's device name → the PipeWire node name Soloist wants ("" for the default).
fn node_name(device: &str) -> String {
    device
        .strip_prefix("pipewire/")
        .unwrap_or_default()
        .to_string()
}

/// mpv lists every device three times (PipeWire, PulseAudio, ALSA). Keep the PipeWire ones.
fn pipewire_devices(list: &Value) -> Vec<AudioDevice> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| {
            let name = d.get("name")?.as_str()?;
            (name == "pipewire" || name.starts_with("pipewire/")).then(|| AudioDevice {
                name: name.to_string(),
                description: d
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or(name)
                    .to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{TempDir, flac, have};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    pub fn silent(tmp: &TempDir) -> (Player, mpsc::Receiver<Event>) {
        let (tx, rx) = mpsc::channel();
        let options = Options {
            audio_device: String::new(),
            replaygain: "off".into(),
            gapless: true,
            volume: 100,
            silent: true,
        };
        (
            Player::start(&options, &tmp.0.join("mpv.sock"), tx).unwrap(),
            rx,
        )
    }

    /// Spotify on this computer: news from Soloist about some other song (what Spotify
    /// resumed as Soloist became the active device) before ours has started isn't the end of
    /// ours. Soloist is asked again a few times, then the song is given up on. There's no real
    /// Soloist here, and the queue has one song, so nothing can play.
    #[test]
    fn soloist_news_about_another_song_isnt_the_end_of_ours() {
        if !have("mpv") {
            eprintln!("skipping: needs mpv");
            return;
        }
        let tmp = TempDir::new("player-soloist");
        let (mut player, _rx) = silent(&tmp);
        let ours = Track {
            source: Source::Spotify,
            uri: "spotify:track:ours".into(),
            title: "Ours".into(),
            duration_ms: 200_000,
            ..Track::default()
        };
        player.queue = vec![ours.clone()];
        player.current = Some(0);
        player.engine = Engine::Soloist;
        player.status.idle = false;
        player.status.duration = 200.0;
        player.soloist_uri = Some(ours.uri.clone());
        player.soloist_asked = Some((Instant::now(), 1));
        let state = |uri: &str| {
            Event::Soloist(soloist::Event::State(soloist::State {
                status: Some("playing".into()),
                uri: Some(uri.into()),
                duration_ms: Some(227_000.0),
                position_ms: Some(0.0),
                at_ms: Some(crate::secrets::now_secs() as f64 * 1000.0),
                speed: Some(1.0),
                ..Default::default()
            }))
        };

        player.handle(state("spotify:track:resumed")).unwrap();
        assert_eq!(player.current_index(), Some(0));
        assert!(!player.status.idle, "not taken for the end of ours");
        assert_eq!(
            player.status.duration, 200.0,
            "the other song's length isn't ours"
        );
        player.handle(state("spotify:track:ours")).unwrap();
        assert!(player.soloist_started);
        assert_eq!(player.status.duration, 227.0);

        // Never starting: asked again after a few seconds, then given up on, with a reason.
        player.soloist_started = false;
        player.soloist_asked = Some((Instant::now() - SOLOIST_RETRY, 1));
        player.tick().unwrap();
        assert_eq!(player.soloist_asked.map(|(_, n)| n), Some(2));
        assert!(!player.status.idle);
        player.soloist_asked = Some((Instant::now() - SOLOIST_RETRY, SOLOIST_TRIES));
        player.tick().unwrap();
        let error = player.status.last_error.clone().unwrap_or_default();
        assert!(error.contains("didn't start “Ours”"), "{error}");
        assert!(player.status.idle, "the queue had nothing after it");
    }

    /// Back from Sonos mid-song, Soloist is asked for the same song again: its news counts
    /// (it isn't "the previous song's"), and the seek to where Sonos was waits until Soloist is
    /// playing it. No real Soloist here, so nothing plays.
    #[test]
    fn the_same_spotify_song_again_carries_on_where_it_was() {
        if !have("mpv") {
            eprintln!("skipping: needs mpv");
            return;
        }
        let tmp = TempDir::new("player-soloist-again");
        let (mut player, _rx) = silent(&tmp);
        let song = Track {
            source: Source::Spotify,
            uri: "spotify:track:same".into(),
            title: "Same".into(),
            duration_ms: 240_000,
            ..Track::default()
        };
        player.queue = vec![song.clone(), song.clone()];
        player.current = Some(0);
        player.engine = Engine::Soloist;
        player.status.idle = false;
        player.ask_soloist(&song.uri, 0.0);
        // Back from Sonos, 90 s in: the same song again.
        player.ask_soloist(&song.uri, 90.0);
        assert_eq!(player.soloist_previous, None);
        assert_eq!(player.soloist_seek, Some(90.0));
        let state = |status: &str, pos: f64| {
            Event::Soloist(soloist::Event::State(soloist::State {
                status: Some(status.into()),
                uri: Some(song.uri.clone()),
                duration_ms: Some(240_000.0),
                position_ms: Some(pos * 1000.0),
                at_ms: Some(crate::secrets::now_secs() as f64 * 1000.0),
                speed: Some(if status == "playing" { 1.0 } else { 0.0 }),
                ..Default::default()
            }))
        };
        // Its news counts: paused where it was left, then playing.
        player.handle(state("paused", 45.0)).unwrap();
        assert!(player.soloist_started, "not ignored as the previous song's");
        assert_eq!(player.soloist_seek, Some(90.0), "no seek until it plays");
        player.handle(state("playing", 45.0)).unwrap();
        assert_eq!(player.soloist_seek, None, "the seek went once it played");
        assert_eq!(player.current_index(), Some(0), "nothing skipped");
    }

    /// The queue from last time: in place, nothing playing. Playing the song it was on carries
    /// on from there; any other song starts at the beginning.
    #[test]
    fn a_restored_queue_carries_on_from_its_place() {
        if !have("mpv") {
            eprintln!("skipping: needs mpv");
            return;
        }
        let tmp = TempDir::new("player-restore");
        let (mut player, _rx) = silent(&tmp);
        let songs = named(&["a", "b", "c"]);
        let mix = crate::session::Playlist {
            name: "Mix".into(),
            path: tmp.0.join("Mix.jspf"),
            unsaved: true,
        };
        player.restore(songs, Some(1), Some(mix.clone()), 83.0);
        assert!(player.status.idle, "nothing plays until asked");
        assert_eq!(player.current_index(), Some(1));
        assert_eq!(player.status.position, 83.0);
        let kept = player.session();
        assert_eq!((kept.current, kept.position), (Some(1), 83.0));
        assert_eq!(kept.playlist, Some(mix), "still unsaved");

        // Another song first: from its start.
        player.jump(2).unwrap();
        assert_eq!(player.status.position, 0.0);
        // The restored one: from where it was.
        player.restore(named(&["a", "b", "c"]), Some(1), None, 83.0);
        player.jump(1).unwrap();
        assert_eq!(player.status.position, 83.0);
        player.stop().unwrap();
    }

    /// Sonos has its own volume. Coming back to this computer brings back this computer's,
    /// rather than handing Soloist or mpv the speaker's (which went almost silent).
    #[test]
    fn each_output_keeps_its_own_volume() {
        if !have("mpv") {
            eprintln!("skipping: needs mpv");
            return;
        }
        let tmp = TempDir::new("player-volume");
        let (mut player, _rx) = silent(&tmp);
        player.set_volume(80.0).unwrap();
        let office = sonos_api::Group {
            id: "RINCON_1:1".into(),
            coordinator: sonos_api::Speaker {
                uuid: "RINCON_1".into(),
                room: "Office".into(),
                ip: "192.0.2.1".into(),
            },
            rooms: vec!["Office".into()],
            home_theater: false,
        };
        player.set_output(Output::Sonos(office)).unwrap();
        player
            .handle(Event::Sonos(sonos::Event::Volume(30)))
            .unwrap();
        assert_eq!(player.status.volume, 30.0, "the speaker's own volume");
        player.set_volume(27.0).unwrap();
        assert_eq!(
            player.here_volume(),
            80.0,
            "changing Sonos's leaves this computer's"
        );
        player.set_output(Output::Here(String::new())).unwrap();
        assert_eq!(player.status.volume, 80.0, "this computer's again");
    }

    #[test]
    fn keeps_only_pipewire_devices() {
        let list = json!([
            {"name": "auto", "description": "Autoselect device"},
            {"name": "pipewire", "description": "Default (pipewire)"},
            {"name": "pipewire/alsa_output.usb-Schiit", "description": "Schiit Modi+ Analog Stereo"},
            {"name": "pulse/alsa_output.usb-Schiit", "description": "Schiit Modi+ Analog Stereo"},
        ]);
        let names: Vec<_> = pipewire_devices(&list)
            .into_iter()
            .map(|d| d.description)
            .collect();
        assert_eq!(names, ["Default (pipewire)", "Schiit Modi+ Analog Stereo"]);
        assert_eq!(
            node_name("pipewire/alsa_output.usb-Schiit"),
            "alsa_output.usb-Schiit"
        );
        assert_eq!(node_name("pipewire"), "");
    }

    fn named(titles: &[&str]) -> Vec<Track> {
        titles
            .iter()
            .map(|t| Track {
                title: t.to_string(),
                path: format!("/nowhere/{t}.flac"),
                ..Track::default()
            })
            .collect()
    }

    fn titles(player: &Player) -> Vec<&str> {
        player.queue().iter().map(|t| t.title.as_str()).collect()
    }

    /// Queue edits keep the ▶ on the right song, and undo walks them back. The files don't
    /// exist; only the bookkeeping is checked.
    #[test]
    fn queue_edits_keep_the_playing_song_and_undo() {
        if !have("mpv") {
            eprintln!("skipping: needs mpv");
            return;
        }
        let tmp = TempDir::new("player-edit");
        let (mut player, _rx) = silent(&tmp);
        player.play(named(&["a", "b", "c", "d"]), 2).unwrap();
        assert_eq!(player.current().unwrap().title, "c");

        player.remove(0).unwrap();
        assert_eq!(titles(&player), ["b", "c", "d"]);
        assert_eq!(player.current().unwrap().title, "c", "still on c");

        assert_eq!(player.move_track(1, true).unwrap(), 0);
        assert_eq!(titles(&player), ["c", "b", "d"]);
        assert_eq!(player.current_index(), Some(0), "the ▶ moved with c");
        assert_eq!(player.move_track(0, true).unwrap(), 0, "top can't go up");

        // Dragging: c (playing) to the end and back keeps the ▶ on c.
        assert_eq!(player.move_to(0, 2).unwrap(), 2);
        assert_eq!(titles(&player), ["b", "d", "c"]);
        assert_eq!(player.current_index(), Some(2));
        player.move_to(2, 0).unwrap();
        assert_eq!(titles(&player), ["c", "b", "d"]);
        assert_eq!(player.current_index(), Some(0));
        player.undo().unwrap();
        player.undo().unwrap();

        // Removing the song that's playing moves on to the next one.
        player.remove(0).unwrap();
        assert_eq!(player.current().unwrap().title, "b");

        let before = player.revision;
        player.clear().unwrap();
        assert!(player.queue().is_empty());
        assert!(player.revision > before);

        let mut steps = Vec::new();
        while player.undo().unwrap().is_some() {
            steps.push(titles(&player).join(""));
        }
        assert_eq!(
            steps,
            ["bd", "cbd", "bcd", "abcd", ""],
            "each edit, newest first"
        );

        player.load(named(&["x", "y"]), None).unwrap();
        assert_eq!(
            player.current_index(),
            Some(0),
            "an opened playlist starts at the top"
        );
    }

    /// Moving between this computer's outputs doesn't restart the song.
    #[test]
    fn switching_outputs_here_keeps_playing() {
        if !have("mpv") {
            eprintln!("skipping: needs mpv");
            return;
        }
        let tmp = TempDir::new("player-output");
        let (mut player, _rx) = silent(&tmp);
        player.play(named(&["a", "b"]), 1).unwrap();
        let ticket = player.ticket;
        player
            .set_output(Output::Here("pipewire/alsa_output.usb-Schiit".into()))
            .unwrap();
        assert_eq!(player.ticket, ticket, "no new song started");
        assert_eq!(player.current_index(), Some(1));
        assert_eq!(
            player.output,
            Output::Here("pipewire/alsa_output.usb-Schiit".into())
        );
    }

    fn run_until(
        player: &mut Player,
        rx: &mpsc::Receiver<Event>,
        secs: u64,
        done: impl Fn(&Player) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < deadline && !done(player) {
            if let Ok(event) = rx.recv_timeout(Duration::from_millis(50)) {
                player.handle(event).unwrap();
            }
            player.tick().unwrap();
        }
    }

    /// Plays two one-second files into mpv's null output (silent) and checks that the queue
    /// moves on by itself and that mpv reports what it's sending to the output.
    #[test]
    fn plays_through_the_queue_silently() {
        if !have("mpv") || !have("ffmpeg") {
            eprintln!("skipping: needs mpv and ffmpeg");
            return;
        }
        let tmp = TempDir::new("player");
        let tracks: Vec<Track> = ["a", "b"]
            .iter()
            .map(|name| {
                let path = tmp.0.join(format!("{name}.flac"));
                flac(&path, &[("title", name)]);
                Track {
                    path: path.display().to_string(),
                    duration_ms: 1000,
                    ..Track::default()
                }
            })
            .collect();
        let (mut player, rx) = silent(&tmp);
        player.play(tracks, 0).unwrap();

        let mut saw_rate = None;
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline
            && !(player.current_index() == Some(1) && player.status.idle)
        {
            if let Ok(event) = rx.recv_timeout(Duration::from_millis(50)) {
                player.handle(event).unwrap();
                saw_rate = saw_rate.or(player.status.sample_rate);
            }
        }
        assert_eq!(
            player.current_index(),
            Some(1),
            "didn't move on to the second track"
        );
        assert!(player.status.idle, "stops at the end of the queue");
        assert_eq!(saw_rate, Some(44100), "mpv never reported its output rate");
        assert!(
            player.status.last_error.is_none(),
            "{:?}",
            player.status.last_error
        );
    }

    /// Finds and plays a YouTube Music song into mpv's silent output, signed out:
    /// `cargo test live_youtube_plays -- --ignored --nocapture`. Needs the network.
    #[test]
    #[ignore]
    fn live_youtube_plays_silently() {
        let tmp = TempDir::new("player-yt");
        let (mut player, rx) = silent(&tmp);
        let song = Track {
            source: Source::YouTube,
            uri: "xfSskvyxYGo".into(),
            title: "Tighten Up".into(),
            duration_ms: 212_000,
            ..Track::default()
        };
        player.play(vec![song], 0).unwrap();
        assert!(
            player.status.preparing.is_some(),
            "looks the stream up first"
        );
        run_until(&mut player, &rx, 30, |p| p.status.position > 2.0);
        println!(
            "position {:.1}s, output {:?} Hz, error {:?}",
            player.status.position, player.status.sample_rate, player.status.last_error
        );
        assert!(
            player.status.position > 2.0,
            "{:?}",
            player.status.last_error
        );
        assert_eq!(player.current_index(), Some(0));
    }

    /// A missing file is skipped, so a gap in the queue doesn't stop the music.
    #[test]
    fn skips_songs_that_cant_play() {
        if !have("mpv") || !have("ffmpeg") {
            eprintln!("skipping: needs mpv and ffmpeg");
            return;
        }
        let tmp = TempDir::new("player-skip");
        let good = tmp.0.join("good.flac");
        flac(&good, &[("title", "good")]);
        let mut tracks = named(&["missing"]);
        tracks.push(Track {
            title: "good".into(),
            path: good.display().to_string(),
            ..Track::default()
        });
        let (mut player, rx) = silent(&tmp);
        player.play(tracks, 0).unwrap();
        run_until(&mut player, &rx, 10, |p| {
            p.current_index() == Some(1) && p.status.position > 0.2
        });
        assert_eq!(player.current_index(), Some(1));
        assert!(
            player
                .status
                .last_error
                .as_deref()
                .unwrap_or("")
                .contains("missing")
        );
    }
}
